//! Answers two questions that cannot be settled by reading code.
//!
//! 1. Can an unprivileged user register a BLE advertisement with BlueZ? If not, the
//!    sandboxed daemon cannot broadcast proximity nonces and the design changes.
//! 2. Is the chosen transmit power the right one? Run with a long duration and walk a phone
//!    around with any BLE scanner: the beacon should be readable at the desk and vanish a
//!    few paces away.

use std::process::ExitCode;
use std::time::Duration;

use kpk_proximity::{Beacon, NONCE_LEN, PROXIMITY_SERVICE_UUID, ProximityNonce};

struct Options {
    adapter: String,
    seconds: u64,
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        adapter: "/org/bluez/hci0".to_owned(),
        seconds: 5,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--adapter" => {
                options.adapter = args.next().ok_or("--adapter needs a D-Bus object path")?;
            }
            "--seconds" => {
                let value = args.next().ok_or("--seconds needs a number")?;
                options.seconds = value.parse().map_err(|_| format!("{value} is not a number"))?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(options)
}

#[tokio::main]
async fn main() -> ExitCode {
    let options = match parse_options() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            eprintln!("usage: kpk-ble-selftest [--adapter <path>] [--seconds <n>]");
            return ExitCode::FAILURE;
        }
    };

    match users_uid() {
        Some(uid) => println!("running as uid {uid}, adapter {}", options.adapter),
        None => println!("running as an unknown uid, adapter {}", options.adapter),
    }

    let nonce = match ProximityNonce::generate() {
        Ok(nonce) => nonce,
        Err(error) => {
            eprintln!("FAIL: {error:#}");
            return ExitCode::FAILURE;
        }
    };

    let beacon = match Beacon::start(&options.adapter, nonce).await {
        Ok(beacon) => beacon,
        Err(error) => {
            eprintln!("FAIL: registration refused: {error:#}");
            return ExitCode::FAILURE;
        }
    };

    println!("OK: advertisement registered");
    println!("     service UUID : {PROXIMITY_SERVICE_UUID}");
    println!("     service data : {NONCE_LEN} random bytes (the proximity nonce)");
    println!("     broadcasting for {}s", options.seconds);
    tokio::time::sleep(Duration::from_secs(options.seconds)).await;

    match beacon.stop().await {
        Ok(()) => {
            println!("OK: advertisement withdrawn");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("FAIL: withdrawal refused: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn users_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata("/proc/self").ok().map(|meta| meta.uid())
}
