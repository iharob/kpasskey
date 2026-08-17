//! The FIDO helper's line to the daemon.
//!
//! `/dev/uhid` is 0600 root:root so the helper runs as root, but the phone belongs to the
//! daemon's session. Rather than give the helper the daemon's privileges — or the daemon
//! root — it asks over the same Unix socket PAM uses.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

pub const DEFAULT_SOCKET: &str = "/run/kpasskey/pam.sock";

pub struct NewCredential {
    pub credential_id: String,
    pub public_key_pem: String,
}

/// Asks the phone for a fresh per-site credential. Blocks until the user answers.
///
/// # Errors
/// Fails if the daemon is unreachable, the user declines, or the reply is malformed.
pub fn make_credential(rp_id: &str, user_name: &str) -> Result<NewCredential> {
    let reply = request(&serde_json::json!({
        "op": "webauthn.makecred",
        "rpId": rp_id,
        "userName": user_name,
    }))?;

    Ok(NewCredential {
        credential_id: field(&reply, "credId")?,
        public_key_pem: field(&reply, "publicKeyPem")?,
    })
}

/// Asks the phone to sign `authData ‖ clientDataHash`.
///
/// # Errors
/// Fails if the daemon is unreachable, the user declines, or no signature comes back.
pub fn assert(rp_id: &str, credential_id: &str, payload_b64: &str) -> Result<String> {
    let reply = request(&serde_json::json!({
        "op": "webauthn.assert",
        "rpId": rp_id,
        "credId": credential_id,
        "payload": payload_b64,
    }))?;
    field(&reply, "sig")
}

fn request(body: &serde_json::Value) -> Result<serde_json::Value> {
    let stream = UnixStream::connect(DEFAULT_SOCKET)
        .with_context(|| format!("connecting to {DEFAULT_SOCKET}"))?;
    // Generous: the far end is a human looking at a phone, not a machine.
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok();

    let mut writer = stream.try_clone().context("cloning the daemon socket")?;
    writer
        .write_all(format!("{body}\n").as_bytes())
        .context("sending the request to the daemon")?;
    writer.flush().ok();

    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .context("reading the daemon's reply")?;

    let reply: serde_json::Value =
        serde_json::from_str(line.trim()).context("daemon sent invalid JSON")?;
    if !reply
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        let reason = reply
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("refused");
        bail!("{reason}");
    }
    Ok(reply)
}

fn field(reply: &serde_json::Value, name: &str) -> Result<String> {
    reply
        .get(name)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .with_context(|| format!("daemon reply has no {name}"))
}
