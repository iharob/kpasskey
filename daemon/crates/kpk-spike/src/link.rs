//! Signature checking and key loading.

use anyhow::{Context as _, Result};
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::{DecodePublicKey as _, EncodePublicKey as _};
use sha2::{Digest as _, Sha256};

/// 128 bits of SHA-256 over the SPKI. Long enough that two devices cannot collide, so a
/// re-pair of the same phone overwrites its record instead of adding a second one.
const ID_BYTES: usize = 16;
/// 48 bits, rendered as three groups of four hex digits. This is read off two screens and
/// compared by eye, so it trades length for legibility — it identifies a device, it does not
/// authenticate one. Attestation and the pairing code do that.
const FINGERPRINT_BYTES: usize = 6;

/// Checks an assertion against the enrolled device key over the exact bytes that were sent.
/// A reply cannot be moved onto a different request, because the request *is* the message.
pub fn verify_assertion(key: &VerifyingKey, sent_line: &str, der_signature: &[u8]) -> Result<()> {
    let signature = Signature::from_der(der_signature).context("signature is not valid DER ECDSA")?;
    key.verify(sent_line.as_bytes(), &signature)
        .context("signature does not verify against the enrolled device key")
}

pub fn load_verifying_key(pem: &str) -> Result<VerifyingKey> {
    let public = p256::PublicKey::from_public_key_pem(pem)
        .context("device public key is not a P-256 SPKI PEM")?;
    Ok(VerifyingKey::from(&public))
}

/// The device id, derived from the key rather than from the clock, so the same phone always
/// lands on the same record.
///
/// # Errors
/// Fails if the PEM is not a P-256 SPKI.
pub fn key_id(pem: &str) -> Result<String> {
    let digest = spki_digest(pem)?;
    Ok(hex(digest.iter().take(ID_BYTES)))
}

/// The same identity in a form a person can compare against the phone's screen.
///
/// # Errors
/// Fails if the PEM is not a P-256 SPKI.
pub fn key_fingerprint(pem: &str) -> Result<String> {
    let digest = spki_digest(pem)?;
    let bytes: Vec<u8> = digest.iter().take(FINGERPRINT_BYTES).copied().collect();
    let groups: Vec<String> = bytes
        .chunks(2)
        .map(|pair| hex(pair.iter()).to_uppercase())
        .collect();
    Ok(groups.join(" "))
}

fn hex<'a>(bytes: impl Iterator<Item = &'a u8>) -> String {
    use std::fmt::Write as _;
    bytes.fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// Hashes the re-encoded SPKI, never the PEM text: whitespace and line wrapping must not
/// change a device's identity.
fn spki_digest(pem: &str) -> Result<[u8; 32]> {
    let public = p256::PublicKey::from_public_key_pem(pem)
        .context("device public key is not a P-256 SPKI PEM")?;
    let der = public
        .to_public_key_der()
        .context("re-encoding the device key as SPKI DER")?;
    Ok(Sha256::digest(der.as_bytes()).into())
}
