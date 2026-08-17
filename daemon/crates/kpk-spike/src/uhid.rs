//! A virtual FIDO HID authenticator via `/dev/uhid`.
//!
//! Chrome and Firefox on Linux already speak USB-HID CTAP2, so presenting a HID device with
//! the FIDO usage page makes the phone reachable from any site with no extension and no
//! browser cooperation. This first increment answers one question: is the device recognised
//! by the FIDO stack at all? It implements CTAPHID framing plus `authenticatorGetInfo`;
//! credential creation and assertions come next.
//!
//! `/dev/uhid` is 0600 root:root, so this must run as root — which is why the real design
//! puts it in a small separate helper rather than the sandboxed daemon.

use std::fs::OpenOptions;
use std::io::{Read as _, Write as _};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD as BASE64URL};
use p256::elliptic_curve::sec1::ToEncodedPoint as _;
use p256::pkcs8::DecodePublicKey as _;

use crate::{fidolink, webauthn};

const UHID_PATH: &str = "/dev/uhid";

// linux/uhid.h event types.
const UHID_START: u32 = 2;
const UHID_STOP: u32 = 3;
const UHID_OPEN: u32 = 4;
const UHID_CLOSE: u32 = 5;
const UHID_OUTPUT: u32 = 6;
const UHID_CREATE2: u32 = 11;
const UHID_INPUT2: u32 = 12;

const UHID_DATA_MAX: usize = 4096;
/// `struct uhid_event` is packed: u32 type + the largest union member (`create2_req`, 4372).
const UHID_EVENT_SIZE: usize = 4 + 4372;

const HID_REPORT_SIZE: usize = 64;

// CTAPHID commands (high bit set on the wire).
const CTAPHID_PING: u8 = 0x01;
const CTAPHID_INIT: u8 = 0x06;
const CTAPHID_CBOR: u8 = 0x10;
const CTAPHID_ERROR: u8 = 0x3f;
const CTAP2_ERR_OPERATION_DENIED: u8 = 0x27;

const CTAP_BROADCAST_CID: u32 = 0xffff_ffff;
const CTAP2_MAKE_CREDENTIAL: u8 = 0x01;
const CTAP2_GET_ASSERTION: u8 = 0x02;
const CTAP2_GET_INFO: u8 = 0x04;

const CTAP1_ERR_INVALID_COMMAND: u8 = 0x01;
const CTAP2_ERR_NO_CREDENTIALS: u8 = 0x2e;
const CTAP2_OK: u8 = 0x00;

/// How often to reassure the browser while the user is deciding on their phone.
const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(100);
const CTAPHID_KEEPALIVE: u8 = 0x3b;
/// KEEPALIVE status: waiting for the user, as opposed to merely busy.
const KEEPALIVE_UP_NEEDED: u8 = 0x02;

/// Standard FIDO HID report descriptor: 64-byte input and output reports on usage page
/// 0xF1D0, which is what browsers scan for.
const FIDO_REPORT_DESCRIPTOR: [u8; 34] = [
    0x06, 0xd0, 0xf1, // Usage Page (FIDO Alliance)
    0x09, 0x01, // Usage (CTAPHID)
    0xa1, 0x01, // Collection (Application)
    0x09, 0x20, //   Usage (Input Report Data)
    0x15, 0x00, //   Logical Minimum (0)
    0x26, 0xff, 0x00, //   Logical Maximum (255)
    0x75, 0x08, //   Report Size (8)
    0x95, 0x40, //   Report Count (64)
    0x81, 0x02, //   Input (Data,Var,Abs)
    0x09, 0x21, //   Usage (Output Report Data)
    0x15, 0x00, //   Logical Minimum (0)
    0x26, 0xff, 0x00, //   Logical Maximum (255)
    0x75, 0x08, //   Report Size (8)
    0x95, 0x40, //   Report Count (64)
    0x91, 0x02, //   Output (Data,Var,Abs)
    0xc0, // End Collection
];

fn put(buffer: &mut [u8], offset: usize, bytes: &[u8]) -> Result<()> {
    let end = offset
        .checked_add(bytes.len())
        .context("uhid buffer offset overflow")?;
    buffer
        .get_mut(offset..end)
        .context("uhid buffer too small")?
        .copy_from_slice(bytes);
    Ok(())
}

fn take(buffer: &[u8], offset: usize, length: usize) -> Result<&[u8]> {
    let end = offset.checked_add(length).context("slice overflow")?;
    buffer.get(offset..end).context("slice out of range")
}

fn create_event() -> Result<Vec<u8>> {
    let mut event = vec![0u8; UHID_EVENT_SIZE];
    put(&mut event, 0, &UHID_CREATE2.to_ne_bytes())?;

    // create2_req starts at offset 4: name[128], phys[64], uniq[64], rd_size u16,
    // bus u16, vendor u32, product u32, version u32, country u32, rd_data[4096].
    put(&mut event, 4, b"kpasskey virtual authenticator")?;

    let rd_size_offset = 4 + 128 + 64 + 64;
    let descriptor_length = u16::try_from(FIDO_REPORT_DESCRIPTOR.len())
        .context("report descriptor is too long")?;
    put(&mut event, rd_size_offset, &descriptor_length.to_ne_bytes())?;
    put(&mut event, rd_size_offset + 2, &3u16.to_ne_bytes())?; // BUS_USB
    put(&mut event, rd_size_offset + 4, &0x1209u32.to_ne_bytes())?; // pid.codes
    put(&mut event, rd_size_offset + 8, &0x0001u32.to_ne_bytes())?;
    put(&mut event, rd_size_offset + 12, &1u32.to_ne_bytes())?;
    put(&mut event, rd_size_offset + 16, &0u32.to_ne_bytes())?;
    put(&mut event, rd_size_offset + 20, &FIDO_REPORT_DESCRIPTOR)?;

    Ok(event)
}

fn input_event(report: &[u8]) -> Result<Vec<u8>> {
    if report.len() > UHID_DATA_MAX {
        bail!("report is larger than the uhid maximum");
    }
    let mut event = vec![0u8; UHID_EVENT_SIZE];
    put(&mut event, 0, &UHID_INPUT2.to_ne_bytes())?;
    let size = u16::try_from(report.len()).context("report length overflow")?;
    put(&mut event, 4, &size.to_ne_bytes())?;
    put(&mut event, 6, report)?;
    Ok(event)
}

/// `authenticatorGetInfo`: versions, our AAGUID, and the options a phone-backed
/// authenticator can honestly claim. `uv` is true because the fingerprint *is* the
/// verification.
fn get_info_response() -> Vec<u8> {
    let mut cbor = vec![
        0xa3, // map(3)
        0x01, 0x81, 0x68, // 1 => ["FIDO_2_0"]
        b'F', b'I', b'D', b'O', b'_', b'2', b'_', b'0', 0x03, 0x50, // 3 => bytes(16)
    ];
    cbor.extend_from_slice(&webauthn::AAGUID);
    cbor.extend_from_slice(&[
        0x04, 0xa3, // 4 => map(3)
        0x62, b'r', b'k', 0xf4, // "rk": false — no discoverable credentials, see get_assertion
        0x62, b'u', b'p', 0xf5, // "up": true
        0x62, b'u', b'v', 0xf5, // "uv": true
    ]);

    let mut response = vec![CTAP2_OK];
    response.extend_from_slice(&cbor);
    response
}

/// A response that is ready now, or one a person still has to approve.
enum Outcome {
    Ready(Vec<Vec<u8>>),
    Waiting(mpsc::Receiver<Vec<u8>>),
}

struct Assembly {
    channel: u32,
    command: u8,
    expected: usize,
    payload: Vec<u8>,
}

/// Prefixes a CTAP2 status byte, which every response carries ahead of its CBOR body.
fn ctap_reply(result: Result<Vec<u8>>, failure: u8) -> Vec<u8> {
    match result {
        Ok(body) => {
            let mut response = vec![CTAP2_OK];
            response.extend_from_slice(&body);
            response
        }
        Err(error) => {
            println!("[ctap2]   failed: {error:#}");
            vec![failure]
        }
    }
}

/// Creates a credential whose private half is generated inside the phone's TEE.
fn make_credential(body: &[u8]) -> Vec<u8> {
    ctap_reply(create_credential(body), CTAP2_ERR_OPERATION_DENIED)
}

fn create_credential(body: &[u8]) -> Result<Vec<u8>> {
    let request = webauthn::parse_make_credential(body)?;
    println!(
        "[ctap2]   authenticatorMakeCredential rp={} user={}",
        request.rp_id, request.user_name
    );

    let created = fidolink::make_credential(&request.rp_id, &request.user_name)?;
    let credential_id = BASE64URL
        .decode(&created.credential_id)
        .context("phone returned a credential id that is not base64url")?;

    let public = p256::PublicKey::from_public_key_pem(&created.public_key_pem)
        .context("phone returned a key that is not a P-256 SPKI PEM")?;
    let point = public.to_encoded_point(false);
    let cose = webauthn::cose_key(
        point.x().context("credential key has no x coordinate")?,
        point.y().context("credential key has no y coordinate")?,
    )?;

    let attested = webauthn::attested_credential_data(&credential_id, &cose)?;
    let auth_data = webauthn::authenticator_data(
        &request.rp_id,
        webauthn::FLAG_USER_PRESENT | webauthn::FLAG_USER_VERIFIED | webauthn::FLAG_ATTESTED,
        0,
        Some(&attested),
    );

    println!("[ctap2]   credential created on the phone for {}", request.rp_id);
    webauthn::make_credential_response(&auth_data)
}

fn get_assertion(body: &[u8]) -> Vec<u8> {
    ctap_reply(assert_credential(body), CTAP2_ERR_NO_CREDENTIALS)
}

fn assert_credential(body: &[u8]) -> Result<Vec<u8>> {
    let request = webauthn::parse_get_assertion(body)?;
    println!("[ctap2]   authenticatorGetAssertion rp={}", request.rp_id);

    // No credential is stored here, so the site must name one. That is why getInfo reports
    // rk:false — claiming discoverable credentials would promise a lookup we cannot do.
    let credential_id = request
        .allowed
        .first()
        .context("no credential named, and this authenticator stores none")?;

    // Sign count stays 0: the spec's value for an authenticator that keeps no counter, which
    // is honest here because the helper holds no per-credential state at all.
    let auth_data = webauthn::authenticator_data(
        &request.rp_id,
        webauthn::FLAG_USER_PRESENT | webauthn::FLAG_USER_VERIFIED,
        0,
        None,
    );

    // WebAuthn signs the authenticator data concatenated with the client data hash, in that
    // order. Getting this wrong produces a signature the site rejects with no diagnostic.
    let mut message = auth_data.clone();
    message.extend_from_slice(&request.client_data_hash);

    let signature = fidolink::assert(
        &request.rp_id,
        &BASE64URL.encode(credential_id),
        &BASE64.encode(&message),
    )?;
    let signature = BASE64
        .decode(signature)
        .context("phone returned a signature that is not base64")?;

    println!("[ctap2]   assertion signed on the phone for {}", request.rp_id);
    webauthn::get_assertion_response(credential_id, &auth_data, &signature)
}

/// Splits a response into 64-byte CTAPHID packets: one init frame then continuations.
fn frame(channel: u32, command: u8, payload: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut packets = Vec::new();
    let length = u16::try_from(payload.len()).context("response too long for CTAPHID")?;

    let first_chunk = payload.len().min(HID_REPORT_SIZE - 7);
    let mut packet = vec![0u8; HID_REPORT_SIZE];
    put(&mut packet, 0, &channel.to_be_bytes())?;
    put(&mut packet, 4, &[command | 0x80])?;
    put(&mut packet, 5, &length.to_be_bytes())?;
    put(&mut packet, 7, take(payload, 0, first_chunk)?)?;
    packets.push(packet);

    let mut offset = first_chunk;
    let mut sequence: u8 = 0;
    while offset < payload.len() {
        let chunk = (payload.len() - offset).min(HID_REPORT_SIZE - 5);
        let mut packet = vec![0u8; HID_REPORT_SIZE];
        put(&mut packet, 0, &channel.to_be_bytes())?;
        put(&mut packet, 4, &[sequence])?;
        put(&mut packet, 5, take(payload, offset, chunk)?)?;
        packets.push(packet);
        offset += chunk;
        sequence = sequence.wrapping_add(1);
    }

    Ok(packets)
}

fn write_packets(device: &mut std::fs::File, packets: &[Vec<u8>]) -> Result<()> {
    for packet in packets {
        device
            .write_all(&input_event(packet)?)
            .context("writing a report to the host")?;
    }
    Ok(())
}

/// Holds the browser open while the phone is asked. Without these frames Chrome gives up
/// long before a person can pick up their phone and touch the sensor.
fn await_phone(
    device: &mut std::fs::File,
    channel: u32,
    receiver: &mpsc::Receiver<Vec<u8>>,
) -> Result<Vec<u8>> {
    loop {
        match receiver.recv_timeout(KEEPALIVE_INTERVAL) {
            Ok(body) => return Ok(body),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let packets = frame(channel, CTAPHID_KEEPALIVE, &[KEEPALIVE_UP_NEEDED])?;
                write_packets(device, &packets)?;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("the phone request ended without an answer");
            }
        }
    }
}

pub fn run() -> Result<()> {
    let mut device = OpenOptions::new()
        .read(true)
        .write(true)
        .open(UHID_PATH)
        .with_context(|| format!("opening {UHID_PATH} (needs root)"))?;

    device
        .write_all(&create_event()?)
        .context("creating the uhid device")?;
    println!("virtual FIDO authenticator created; waiting for the host");
    println!("test with:  fido2-token -L        (expect a new hidraw device)");

    let mut next_channel: u32 = 1;
    let mut assembly: Option<Assembly> = None;
    let mut buffer = vec![0u8; UHID_EVENT_SIZE + 16];

    loop {
        let read = device.read(&mut buffer).context("reading a uhid event")?;
        if read < 4 {
            continue;
        }
        let kind = u32::from_ne_bytes(take(&buffer, 0, 4)?.try_into()?);

        match kind {
            UHID_START => println!("[uhid] host started the device"),
            UHID_OPEN => println!("[uhid] host opened the device"),
            UHID_CLOSE => println!("[uhid] host closed the device"),
            UHID_STOP => {
                println!("[uhid] host stopped the device");
                return Ok(());
            }
            UHID_OUTPUT => {
                // output_req: data[4096], size u16, rtype u8.
                let size = usize::from(u16::from_ne_bytes(
                    take(&buffer, 4 + UHID_DATA_MAX, 2)?.try_into()?,
                ));
                // The host prepends a report-number byte when the descriptor declares no
                // numbered reports, so a 64-byte packet arrives as 65 bytes.
                let (offset, length) = if size > HID_REPORT_SIZE {
                    (5, size - 1)
                } else {
                    (4, size)
                };
                let report = take(&buffer, offset, length.min(HID_REPORT_SIZE))?.to_vec();
                let channel = u32::from_be_bytes(take(&report, 0, 4)?.try_into()?);

                match handle_report(&report, &mut assembly, &mut next_channel)? {
                    Some(Outcome::Ready(packets)) => write_packets(&mut device, &packets)?,
                    Some(Outcome::Waiting(receiver)) => {
                        let body = await_phone(&mut device, channel, &receiver)?;
                        write_packets(&mut device, &frame(channel, CTAPHID_CBOR, &body)?)?;
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }
}

fn handle_report(
    report: &[u8],
    assembly: &mut Option<Assembly>,
    next_channel: &mut u32,
) -> Result<Option<Outcome>> {
    let channel = u32::from_be_bytes(take(report, 0, 4)?.try_into()?);
    let marker = *report.get(4).context("short report")?;

    if marker & 0x80 != 0 {
        let command = marker & 0x7f;
        let high = u16::from(*report.get(5).context("short report")?);
        let low = u16::from(*report.get(6).context("short report")?);
        let expected = usize::from((high << 8) | low);
        let available = expected.min(HID_REPORT_SIZE - 7);
        let payload = take(report, 7, available)?.to_vec();

        if payload.len() < expected {
            *assembly = Some(Assembly {
                channel,
                command,
                expected,
                payload,
            });
            return Ok(None);
        }
        return dispatch(channel, command, &payload, next_channel).map(Some);
    }

    // Continuation frame.
    let Some(state) = assembly.as_mut() else {
        return Ok(None);
    };
    if state.channel != channel {
        return Ok(None);
    }
    let remaining = state.expected.saturating_sub(state.payload.len());
    let available = remaining.min(HID_REPORT_SIZE - 5);
    state.payload.extend_from_slice(take(report, 5, available)?);
    if state.payload.len() < state.expected {
        return Ok(None);
    }

    let Some(done) = assembly.take() else {
        return Ok(None);
    };
    dispatch(done.channel, done.command, &done.payload, next_channel).map(Some)
}

fn dispatch(
    channel: u32,
    command: u8,
    payload: &[u8],
    next_channel: &mut u32,
) -> Result<Outcome> {
    match command {
        CTAPHID_INIT => {
            let nonce = take(payload, 0, 8)?;
            let assigned = if channel == CTAP_BROADCAST_CID {
                *next_channel = next_channel.wrapping_add(1);
                *next_channel
            } else {
                channel
            };
            println!("[ctaphid] INIT -> channel {assigned:#x}");

            let mut body = nonce.to_vec();
            body.extend_from_slice(&assigned.to_be_bytes());
            body.extend_from_slice(&[
                2,    // CTAPHID protocol version
                1, 0, 0, // device version major/minor/build
                0x04, // capabilities: CBOR
            ]);
            frame(channel, CTAPHID_INIT, &body).map(Outcome::Ready)
        }
        CTAPHID_PING => {
            println!("[ctaphid] PING ({} bytes)", payload.len());
            frame(channel, CTAPHID_PING, payload).map(Outcome::Ready)
        }
        CTAPHID_CBOR => {
            let ctap_command = payload.first().copied().unwrap_or(0);
            let body = payload.get(1..).unwrap_or_default();
            println!("[ctaphid] CBOR command {ctap_command:#04x}");
            match ctap_command {
                CTAP2_GET_INFO => {
                    println!("[ctap2]   authenticatorGetInfo");
                    frame(channel, CTAPHID_CBOR, &get_info_response()).map(Outcome::Ready)
                }
                // These wait on a human holding a phone, so they run off the event loop and
                // the browser is kept alive with KEEPALIVE frames meanwhile.
                CTAP2_MAKE_CREDENTIAL | CTAP2_GET_ASSERTION => {
                    let owned = body.to_vec();
                    let (sender, receiver) = mpsc::channel();
                    std::thread::spawn(move || {
                        let response = if ctap_command == CTAP2_MAKE_CREDENTIAL {
                            make_credential(&owned)
                        } else {
                            get_assertion(&owned)
                        };
                        sender.send(response).ok();
                    });
                    Ok(Outcome::Waiting(receiver))
                }
                other => {
                    println!("[ctap2]   unhandled command {other:#04x}");
                    frame(channel, CTAPHID_CBOR, &[CTAP1_ERR_INVALID_COMMAND]).map(Outcome::Ready)
                }
            }
        }
        other => {
            println!("[ctaphid] unhandled command {other:#04x}");
            frame(channel, CTAPHID_ERROR, &[CTAP1_ERR_INVALID_COMMAND]).map(Outcome::Ready)
        }
    }
}
