//! Wiring: one TCP listener for the phone, one Unix socket for PAM, one D-Bus object for
//! System Settings — sharing a device store and a pairing window.
//!
//! Replies are routed by request id rather than tied to the socket they arrived on, so the
//! phone can drop and reconnect mid-verification without orphaning an assertion the user
//! already approved with their finger.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::sync::{Mutex, oneshot};

use crate::link::{key_id, load_verifying_key, verify_assertion};
use crate::pairing::Pairings;
use crate::protocol::{VerifyRequest, parse_response};
use crate::store::{Device, Store};

type Pending = oneshot::Sender<std::result::Result<Vec<u8>, String>>;
/// WebAuthn replies carry more than a signature (a new credential brings its id and public
/// key back), so they get their own channel rather than being squeezed into `Pending`.
type PendingJson = oneshot::Sender<serde_json::Value>;

pub struct Daemon {
    pub store: Arc<Store>,
    pub pairings: Arc<Mutex<Pairings>>,
    verify_timeout: Duration,
    next_id: AtomicU64,
    outbound: Mutex<Option<OwnedWriteHalf>>,
    pending: Mutex<HashMap<u64, Pending>>,
    pending_json: Mutex<HashMap<u64, PendingJson>>,
}

impl Daemon {
    pub fn new(store: Arc<Store>, pairings: Arc<Mutex<Pairings>>, verify_timeout: Duration) -> Self {
        Self {
            store,
            pairings,
            verify_timeout,
            next_id: AtomicU64::new(1),
            outbound: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            pending_json: Mutex::new(HashMap::new()),
        }
    }

    /// The device an authorisation is checked against. The spike uses the first paired
    /// device; the real daemon selects by owner and online state.
    fn active_device(&self) -> Result<Device> {
        self.store
            .list()?
            .into_iter()
            .next()
            .context("no paired device; pair one from System Settings")
    }

    async fn adopt_phone(self: &Arc<Self>, stream: TcpStream) {
        stream.set_nodelay(true).ok();
        let peer = stream
            .peer_addr()
            .map_or_else(|_| "unknown".to_owned(), |address| address.to_string());
        let (read_half, write_half) = stream.into_split();
        *self.outbound.lock().await = Some(write_half);
        println!("[phone] connected from {peer}");

        let daemon = Arc::clone(self);
        tokio::spawn(async move {
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let trimmed = line.trim().to_owned();
                if !trimmed.is_empty() {
                    daemon.on_phone_message(&trimmed).await;
                }
            }
            println!("[phone] {peer} disconnected");
        });
    }

    async fn on_phone_message(&self, line: &str) {
        let kind = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|value| {
                value
                    .get("t")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
            });

        if kind.as_deref() == Some("pair.request") {
            if let Err(error) = self.complete_pairing(line).await {
                println!("[pair] rejected: {error:#}");
                self.send_line(
                    &serde_json::json!({"t": "pair.result", "ok": false, "reason": format!("{error:#}")})
                        .to_string(),
                )
                .await
                .ok();
            }
            return;
        }

        self.deliver(line).await;
    }

    /// Consumes the open pairing window and records the device. The token is single-use, so
    /// a replayed request finds no window and is refused.
    async fn complete_pairing(&self, line: &str) -> Result<()> {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;

        let value: serde_json::Value = serde_json::from_str(line).context("invalid pair.request")?;
        let text = |name: &str| -> String {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };

        let token = BASE64URL
            .decode(text("token"))
            .context("pairing token is not base64url")?;
        let session = {
            let mut pairings = self.pairings.lock().await;
            pairings
                .take_matching(&token)
                .context("no matching pairing window is open")?
        };

        let public_key_pem = text("publicKeyPem");
        load_verifying_key(&public_key_pem).context("device sent an unusable public key")?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or_default();
        let label = if session.label.is_empty() {
            text("name")
        } else {
            session.label.clone()
        };

        let device = Device {
            id: key_id(&public_key_pem)?,
            name: label,
            model: text("model"),
            owner: std::env::var("USER").unwrap_or_else(|_| "unknown".to_owned()),
            security_level: text("securityLevel"),
            verified_boot: text("verifiedBoot"),
            paired_at: now,
            public_key_pem,
        };

        self.store.save(&device)?;
        println!(
            "[pair] paired '{}' ({}) security={} id={}",
            device.name, device.model, device.security_level, device.id
        );

        self.send_line(
            &serde_json::json!({"t": "pair.result", "ok": true, "id": device.id}).to_string(),
        )
        .await?;
        Ok(())
    }

    async fn send_line(&self, line: &str) -> Result<()> {
        let mut guard = self.outbound.lock().await;
        let writer = guard.as_mut().context("no phone connected")?;
        writer.write_all(format!("{line}\n").as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Hands a reply to whichever verification is waiting for that id. A reply with no
    /// waiter is a late answer to an abandoned request, and is dropped.
    async fn deliver(&self, line: &str) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            let id = value.get("id").and_then(serde_json::Value::as_u64);
            if let Some(id) = id {
                let waiter = self.pending_json.lock().await.remove(&id);
                if let Some(waiter) = waiter {
                    waiter.send(value).ok();
                    return;
                }
            }
        }

        let response = match parse_response(line) {
            Ok(response) => response,
            Err(error) => {
                println!("[phone] unparseable reply: {error:#}");
                return;
            }
        };

        let Some(waiter) = self.pending.lock().await.remove(&response.id) else {
            println!("[phone] reply {} arrived with nobody waiting", response.id);
            return;
        };

        let outcome = response.denied.map_or_else(|| Ok(response.signature), Err);
        waiter.send(outcome).ok();
    }

    async fn run_verification(&self, user: String, action: String, detail: String) -> Result<()> {
        let device = self.active_device()?;
        let key = load_verifying_key(&device.public_key_pem)?;

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = VerifyRequest::new(id, user, action, detail)?;
        let line = request.to_line();

        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);

        if self.send_line(&line).await.is_err() {
            self.pending.lock().await.remove(&id);
            bail!("no phone connected");
        }
        println!("[phone] sent request {id} to '{}', awaiting approval", device.name);

        let outcome = match tokio::time::timeout(self.verify_timeout, receiver).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => {
                self.pending.lock().await.remove(&id);
                bail!("verification was abandoned");
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                bail!("phone did not respond within {:?}", self.verify_timeout);
            }
        };

        match outcome {
            Ok(signature) => verify_assertion(&key, &line, &signature),
            Err(reason) => bail!("denied on the phone: {reason}"),
        }
    }

    /// Sends a request the phone answers with JSON, and waits for that answer.
    async fn ask_phone(&self, mut request: serde_json::Value) -> Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        if let Some(map) = request.as_object_mut() {
            map.insert("id".to_owned(), serde_json::json!(id));
        }

        let (sender, receiver) = oneshot::channel();
        self.pending_json.lock().await.insert(id, sender);

        if self.send_line(&request.to_string()).await.is_err() {
            self.pending_json.lock().await.remove(&id);
            bail!("no phone connected");
        }

        let reply = match tokio::time::timeout(self.verify_timeout, receiver).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                self.pending_json.lock().await.remove(&id);
                bail!("the request was abandoned");
            }
            Err(_) => {
                self.pending_json.lock().await.remove(&id);
                bail!("phone did not respond within {:?}", self.verify_timeout);
            }
        };

        if let Some(reason) = reply.get("denied").and_then(serde_json::Value::as_str) {
            bail!("denied on the phone: {reason}");
        }
        Ok(reply)
    }

    /// Asks the phone to mint a per-site credential key. The private half never leaves it.
    async fn webauthn_make_credential(
        &self,
        rp_id: &str,
        user_name: &str,
    ) -> Result<serde_json::Value> {
        let device = self.active_device()?;
        println!("[webauthn] makeCredential rp={rp_id} -> '{}'", device.name);

        let reply = self
            .ask_phone(serde_json::json!({
                "v": 1,
                "type": "webauthn.makecred",
                "rpId": rp_id,
                "userName": user_name,
            }))
            .await?;

        let credential_id = reply
            .get("credId")
            .and_then(serde_json::Value::as_str)
            .context("phone returned no credential id")?
            .to_owned();
        let public_key_pem = reply
            .get("publicKeyPem")
            .and_then(serde_json::Value::as_str)
            .context("phone returned no public key")?
            .to_owned();
        load_verifying_key(&public_key_pem).context("phone returned an unusable credential key")?;

        println!("[webauthn] credential created on the phone for {rp_id}");
        Ok(serde_json::json!({
            "ok": true,
            "credId": credential_id,
            "publicKeyPem": public_key_pem,
        }))
    }

    /// Asks the phone to sign `authData ‖ clientDataHash` with a credential key. The
    /// fingerprint touch is what authorises it.
    async fn webauthn_assert(
        &self,
        rp_id: &str,
        credential_id: &str,
        payload_b64: &str,
    ) -> Result<serde_json::Value> {
        println!("[webauthn] getAssertion rp={rp_id}");
        let reply = self
            .ask_phone(serde_json::json!({
                "v": 1,
                "type": "webauthn.assert",
                "rpId": rp_id,
                "credId": credential_id,
                "payload": payload_b64,
            }))
            .await?;

        let signature = reply
            .get("sig")
            .and_then(serde_json::Value::as_str)
            .context("phone returned no signature")?
            .to_owned();
        println!("[webauthn] assertion signed on the phone for {rp_id}");
        Ok(serde_json::json!({"ok": true, "sig": signature}))
    }

    async fn handle_pam(&self, stream: UnixStream) -> Result<()> {
        let peer_uid = stream
            .peer_cred()
            .map(|cred| cred.uid())
            .context("reading peer credentials")?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .context("reading the PAM request")?;

        let request: serde_json::Value =
            serde_json::from_str(line.trim()).context("PAM client sent invalid JSON")?;
        let field = |name: &str, fallback: &str| -> String {
            request
                .get(name)
                .and_then(serde_json::Value::as_str)
                .unwrap_or(fallback)
                .to_owned()
        };
        // The FIDO helper shares this socket: it runs as root and needs the phone, but must
        // not gain the daemon's own privileges to reach it.
        let operation = field("op", "verify");
        if operation.starts_with("webauthn.") {
            let outcome = match operation.as_str() {
                "webauthn.makecred" => {
                    self.webauthn_make_credential(&field("rpId", ""), &field("userName", "user"))
                        .await
                }
                "webauthn.assert" => {
                    self.webauthn_assert(
                        &field("rpId", ""),
                        &field("credId", ""),
                        &field("payload", ""),
                    )
                    .await
                }
                other => bail!("unknown operation {other}"),
            };
            let reply = outcome.unwrap_or_else(|error| {
                println!("[webauthn] -> FAILED: {error:#}");
                serde_json::json!({"ok": false, "error": format!("{error:#}")})
            });
            let mut stream = reader.into_inner();
            stream
                .write_all(format!("{reply}\n").as_bytes())
                .await
                .context("replying to the FIDO helper")?;
            stream.flush().await.context("flushing the reply")?;
            return Ok(());
        }

        let user = field("user", "unknown");
        let action = field("action", "unknown");
        let detail = field("detail", "");

        println!("[pam] uid={peer_uid} user={user} action={action} detail={detail}");
        let reply = match self.run_verification(user, action, detail).await {
            Ok(()) => {
                println!("[pam] -> APPROVED (signature verified)");
                serde_json::json!({"ok": true, "reason": "success"})
            }
            Err(error) => {
                println!("[pam] -> DENIED: {error:#}");
                serde_json::json!({"ok": false, "reason": format!("{error:#}")})
            }
        };

        let mut stream = reader.into_inner();
        stream
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .context("replying to the PAM client")?;
        stream.flush().await.context("flushing the PAM reply")?;
        Ok(())
    }
}

pub async fn serve(
    devices_dir: PathBuf,
    socket_path: &Path,
    port: u16,
    timeout_secs: u64,
) -> Result<()> {
    let store = Arc::new(Store::open(&devices_dir)?);
    let pairings = Arc::new(Mutex::new(Pairings::new()));
    let daemon = Arc::new(Daemon::new(
        Arc::clone(&store),
        Arc::clone(&pairings),
        Duration::from_secs(timeout_secs),
    ));

    let _bus = crate::dbus::serve(store, pairings).await?;

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let _ = std::fs::remove_file(socket_path);
    let pam_listener = UnixListener::bind(socket_path)
        .with_context(|| format!("binding {}", socket_path.display()))?;
    let phone_listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .with_context(|| format!("binding 0.0.0.0:{port}"))?;

    println!("kpk-spike listening");
    println!("  phone   : 0.0.0.0:{port}  (LAN, or adb reverse tcp:{port} tcp:{port})");
    println!("  pam     : {}", socket_path.display());
    println!("  devices : {}", devices_dir.display());
    println!("  wait    : {timeout_secs}s per verification");

    loop {
        tokio::select! {
            accepted = phone_listener.accept() => {
                let (stream, _) = accepted.context("accepting a phone connection")?;
                daemon.adopt_phone(stream).await;
            }
            accepted = pam_listener.accept() => {
                let (stream, _) = accepted.context("accepting a PAM connection")?;
                let daemon = Arc::clone(&daemon);
                tokio::spawn(async move {
                    if let Err(error) = daemon.handle_pam(stream).await {
                        println!("[pam] connection failed: {error:#}");
                    }
                });
            }
        }
    }
}

/// Stands in for the PAM module: triggers one verification and prints the verdict.
pub async fn probe(socket_path: &Path, action: &str, detail: &str) -> Result<()> {
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_owned());
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connecting to {}", socket_path.display()))?;

    let request = serde_json::json!({"user": user, "action": action, "detail": detail});
    let mut reader = BufReader::new(stream);
    reader
        .get_mut()
        .write_all(format!("{request}\n").as_bytes())
        .await
        .context("sending the probe request")?;

    let mut reply = String::new();
    reader
        .read_line(&mut reply)
        .await
        .context("reading the verdict")?;
    let verdict: serde_json::Value =
        serde_json::from_str(reply.trim()).context("daemon sent invalid JSON")?;

    let approved = verdict
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let reason = verdict
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("no reason");

    if approved {
        println!("APPROVED ({reason})");
        Ok(())
    } else {
        bail!("DENIED ({reason})")
    }
}

/// Brings a device paired by hand into the store, so the manual `openssl` bootstrap has a
/// migration path instead of being re-done.
pub fn import(devices_dir: &Path, pubkey_path: &Path, name: &str, model: &str) -> Result<()> {
    let store = Store::open(devices_dir)?;
    let public_key_pem = std::fs::read_to_string(pubkey_path)
        .with_context(|| format!("reading {}", pubkey_path.display()))?;
    load_verifying_key(&public_key_pem).context("that file is not a P-256 public key")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();
    let device = Device {
        id: key_id(&public_key_pem)?,
        name: name.to_owned(),
        model: model.to_owned(),
        owner: std::env::var("USER").unwrap_or_else(|_| "unknown".to_owned()),
        security_level: "TrustedEnvironment".to_owned(),
        verified_boot: "green".to_owned(),
        paired_at: now,
        public_key_pem,
    };
    store.save(&device)?;
    println!("imported '{}' as {}", device.name, device.id);
    Ok(())
}

/// Exercises the challenge/verify logic with an ephemeral key, including the case that
/// matters most: a signature over different bytes must be rejected.
pub fn self_test() -> Result<()> {
    use p256::ecdsa::signature::Signer as _;

    // p256 0.13 speaks rand_core 0.6; use the one it re-exports rather than the newer `rand`.
    use p256::elliptic_curve::rand_core::OsRng as P256OsRng;

    let signing = p256::ecdsa::SigningKey::random(&mut P256OsRng);
    let verifying = p256::ecdsa::VerifyingKey::from(&signing);

    let request = VerifyRequest::new(
        1,
        "tester".to_owned(),
        "self-test".to_owned(),
        "ephemeral key".to_owned(),
    )?;
    let line = request.to_line();

    let good: p256::ecdsa::Signature = signing.sign(line.as_bytes());
    let good_der = good.to_der().as_bytes().to_vec();
    verify_assertion(&verifying, &line, &good_der).context("a correct signature failed to verify")?;
    println!("ok: correct signature verifies");

    let tampered = line.replace("\"user\":\"tester\"", "\"user\":\"root\"");
    if verify_assertion(&verifying, &tampered, &good_der).is_ok() {
        bail!("a signature verified over tampered bytes; the binding is broken");
    }
    println!("ok: signature does NOT verify once the user field is altered");

    let other = p256::ecdsa::SigningKey::random(&mut P256OsRng);
    let wrong: p256::ecdsa::Signature = other.sign(line.as_bytes());
    let wrong_der = wrong.to_der().as_bytes().to_vec();
    if verify_assertion(&verifying, &line, &wrong_der).is_ok() {
        bail!("a signature from an unenrolled key verified; device binding is broken");
    }
    println!("ok: a different device's signature is rejected");

    println!("self-test passed");
    Ok(())
}
