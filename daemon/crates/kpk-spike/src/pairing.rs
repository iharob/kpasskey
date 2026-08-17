//! Pairing windows.
//!
//! A pairing is a short, explicitly opened window carrying a single-use token. The QR
//! carries the desktop's key hash out of band (screen to camera), so the phone authenticates
//! the desktop on its very first connection rather than trusting it and verifying later.

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use rand::TryRngCore as _;

/// Long enough to walk to the phone and scan, short enough that a forgotten window closes.
const PAIRING_WINDOW: Duration = Duration::from_secs(120);
const TOKEN_LEN: usize = 32;

pub struct PairingSession {
    pub token: [u8; TOKEN_LEN],
    pub uri: String,
    /// Six digits shown on both screens so the user can confirm they paired the right pair.
    pub short_code: String,
    pub label: String,
    opened: Instant,
}

impl PairingSession {
    fn expired(&self) -> bool {
        self.opened.elapsed() >= PAIRING_WINDOW
    }
}

pub struct Pairings {
    current: Option<PairingSession>,
}

impl Pairings {
    pub fn new() -> Self {
        Self { current: None }
    }

    /// Opens a window, replacing any existing one — a second Pair click should restart
    /// rather than silently reuse a token the user has already walked away from.
    ///
    /// # Errors
    /// Fails if the OS entropy source is unavailable.
    pub fn begin(&mut self, label: String) -> Result<&PairingSession> {
        let mut token = [0u8; TOKEN_LEN];
        rand::rngs::OsRng
            .try_fill_bytes(&mut token)
            .context("drawing a pairing token from the OS CSPRNG")?;

        let host = hostname();
        let encoded = BASE64URL.encode(token);
        let short_code = short_code(&token);
        let uri = format!(
            "kpk://pair?v=1&h={host}&t={encoded}&a={address}:{port}",
            address = "127.0.0.1",
            port = crate::DEFAULT_PHONE_PORT,
        );

        self.current = Some(PairingSession {
            token,
            uri,
            short_code,
            label,
            opened: Instant::now(),
        });
        self.current.as_ref().context("pairing session vanished")
    }

    pub fn cancel(&mut self) {
        self.current = None;
    }

    pub fn seconds_left(&mut self) -> u32 {
        self.expire_if_due();
        self.current.as_ref().map_or(0, |session| {
            let elapsed = session.opened.elapsed();
            u32::try_from(PAIRING_WINDOW.saturating_sub(elapsed).as_secs()).unwrap_or(0)
        })
    }

    /// Consumes the window if `token` matches, so a token can pair exactly one device.
    pub fn take_matching(&mut self, token: &[u8]) -> Option<PairingSession> {
        self.expire_if_due();
        let matches = self
            .current
            .as_ref()
            .is_some_and(|session| session.token.as_slice() == token);
        if matches { self.current.take() } else { None }
    }

    fn expire_if_due(&mut self) {
        if self.current.as_ref().is_some_and(PairingSession::expired) {
            self.current = None;
        }
    }
}

fn short_code(token: &[u8]) -> String {
    let mut value: u32 = 0;
    for byte in token.iter().take(4) {
        value = value.wrapping_mul(31).wrapping_add(u32::from(*byte));
    }
    format!("{:06}", value % 1_000_000)
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map_or_else(|_| "unknown-host".to_owned(), |name| name.trim().to_owned())
}
