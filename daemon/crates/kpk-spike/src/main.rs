//! Walking skeleton: proves a fingerprint on the phone can authorise a PAM decision on the
//! desktop, end to end, against a real hardware-attested key.
//!
//! Deliberate shortcuts, all of which the real daemon replaces — see `docs/spike.md`:
//!
//! - plain TCP over `adb reverse` instead of TLS 1.3 with SPKI pinning;
//! - newline-delimited JSON instead of CBOR;
//! - a Unix socket instead of the system D-Bus interface;
//! - no BLE proximity nonce, no attestation verification at run time, one device only.
//!
//! What is NOT shortcut: the assertion is a real ECDSA P-256 signature from a TEE-backed,
//! biometric-gated key, and it covers the full request rather than a bare nonce.

mod dbus;
mod fidolink;
mod link;
mod pairing;
mod protocol;
mod server;
mod store;
mod uhid;
mod webauthn;

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};

pub const DEFAULT_PHONE_PORT: u16 = 34719;
const DEFAULT_PAM_SOCKET: &str = "/run/kpasskey/pam.sock";

enum Command {
    Serve {
        devices: PathBuf,
        socket: PathBuf,
        port: u16,
        timeout_secs: u64,
    },
    Import {
        devices: PathBuf,
        pubkey: PathBuf,
        name: String,
        model: String,
    },
    Probe {
        socket: PathBuf,
        action: String,
        detail: String,
    },
    SelfTest,
    Uhid,
}

fn default_device_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map_or_else(
            || PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"),
            PathBuf::from,
        )
        .join("kpasskey/devices")
}

fn usage() -> String {
    format!(
        "usage:\n  \
         kpk-spike serve [--devices <dir>] [--socket <path>] [--port <n>] [--timeout <s>]\n  \
         kpk-spike probe [--socket <path>] [--action <a>] [--detail <d>]\n  \
         kpk-spike self-test\n  \
         kpk-spike uhid            (needs root: /dev/uhid)\n  \
         kpk-spike import --pubkey <pem> --action <name> --detail <model>\n\n\
         defaults: --socket {DEFAULT_PAM_SOCKET} --port {DEFAULT_PHONE_PORT}"
    )
}

fn parse_args() -> Result<Command> {
    let mut args = std::env::args().skip(1);
    let verb = args.next().unwrap_or_default();

    let mut pubkey: Option<PathBuf> = None;
    let mut socket = PathBuf::from(DEFAULT_PAM_SOCKET);
    let mut port = DEFAULT_PHONE_PORT;
    let mut timeout_secs = 45u64;
    let mut action = "polkit".to_owned();
    let mut detail = "manual probe".to_owned();
    let mut directory: Option<PathBuf> = None;

    while let Some(flag) = args.next() {
        let mut value = || -> Result<String> {
            args.next().context("flag needs a value")
        };
        match flag.as_str() {
            "--pubkey" => pubkey = Some(PathBuf::from(value()?)),
            "--socket" => socket = PathBuf::from(value()?),
            "--port" => port = value()?.parse().context("--port must be a number")?,
            "--timeout" => {
                timeout_secs = value()?.parse().context("--timeout must be seconds")?;
            }
            "--action" => action = value()?,
            "--detail" => detail = value()?,
            "--devices" => directory = Some(PathBuf::from(value()?)),
            other => bail!("unknown argument {other}\n\n{}", usage()),
        }
    }

    match verb.as_str() {
        "serve" => Ok(Command::Serve {
            devices: directory.unwrap_or_else(default_device_dir),
            socket,
            port,
            timeout_secs,
        }),
        "import" => Ok(Command::Import {
            devices: directory.unwrap_or_else(default_device_dir),
            pubkey: pubkey.context("import needs --pubkey")?,
            name: action,
            model: detail,
        }),
        "probe" => Ok(Command::Probe {
            socket,
            action,
            detail,
        }),
        "self-test" => Ok(Command::SelfTest),
        "uhid" => Ok(Command::Uhid),

        _ => bail!(usage()),
    }
}

fn main() -> Result<()> {
    let command = parse_args()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the tokio runtime")?;

    runtime.block_on(async {
        match command {
            Command::Serve {
                devices,
                socket,
                port,
                timeout_secs,
            } => server::serve(devices, &socket, port, timeout_secs).await,
            Command::Import {
                devices,
                pubkey,
                name,
                model,
            } => server::import(&devices, &pubkey, &name, &model),
            Command::Probe {
                socket,
                action,
                detail,
            } => server::probe(&socket, &action, &detail).await,
            Command::SelfTest => server::self_test(),
            Command::Uhid => uhid::run(),

        }
    })
}
