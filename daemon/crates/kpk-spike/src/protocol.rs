//! The signed request. Everything the phone displays is inside the bytes it signs, so an
//! assertion cannot be lifted onto a different user, host or action.

use anyhow::{Context as _, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use rand::TryRngCore as _;

pub const CHALLENGE_LEN: usize = 32;

pub struct VerifyRequest {
    pub id: u64,
    pub challenge: [u8; CHALLENGE_LEN],
    pub user: String,
    pub host: String,
    pub action: String,
    pub detail: String,
}

impl VerifyRequest {
    pub fn new(id: u64, user: String, action: String, detail: String) -> Result<Self> {
        let mut challenge = [0u8; CHALLENGE_LEN];
        rand::rngs::OsRng
            .try_fill_bytes(&mut challenge)
            .context("drawing a challenge from the OS CSPRNG")?;
        let host = hostname();
        Ok(Self {
            id,
            challenge,
            user,
            host,
            action,
            detail,
        })
    }

    /// The exact bytes sent on the wire, and therefore the exact bytes signed. Neither side
    /// re-serialises before hashing.
    pub fn to_line(&self) -> String {
        serde_json::json!({
            "v": 1,
            "type": "verify.request",
            "id": self.id,
            "challenge": BASE64.encode(self.challenge),
            "user": self.user,
            "host": self.host,
            "action": self.action,
            "detail": self.detail,
        })
        .to_string()
    }
}

pub struct VerifyResponse {
    pub id: u64,
    pub signature: Vec<u8>,
    pub denied: Option<String>,
}

pub fn parse_response(line: &str) -> Result<VerifyResponse> {
    let value: serde_json::Value = serde_json::from_str(line).context("phone sent invalid JSON")?;
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .context("response has no id")?;

    if let Some(reason) = value.get("denied").and_then(serde_json::Value::as_str) {
        return Ok(VerifyResponse {
            id,
            signature: Vec::new(),
            denied: Some(reason.to_owned()),
        });
    }

    let encoded = value
        .get("sig")
        .and_then(serde_json::Value::as_str)
        .context("response has neither sig nor denied")?;
    let signature = BASE64.decode(encoded).context("sig is not valid base64")?;

    Ok(VerifyResponse {
        id,
        signature,
        denied: None,
    })
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map_or_else(|_| "unknown-host".to_owned(), |name| name.trim().to_owned())
}
