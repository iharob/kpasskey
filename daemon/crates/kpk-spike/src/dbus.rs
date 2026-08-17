//! The interface System Settings talks to.
//!
//! Spike runs on the **session** bus so the KCM works without a policy file or a privileged
//! daemon. The real daemon owns `org.kpasskey` on the *system* bus, because PAM needs it
//! before any user session exists — see the plan. The interface shape is the same either
//! way, so the KCM does not change when the bus does.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use tokio::sync::Mutex;
use zbus::object_server::SignalEmitter;
use zbus::{interface, zvariant::Type};

use crate::pairing::{PairingSession, Pairings};
use crate::store::Store;

pub const BUS_NAME: &str = "org.kpasskey";
pub const OBJECT_PATH: &str = "/org/kpasskey/Manager1";

/// One row in the System Settings device list.
#[derive(serde::Serialize, serde::Deserialize, Type)]
pub struct DeviceEntry {
    pub id: String,
    pub name: String,
    pub model: String,
    pub owner: String,
    pub security_level: String,
    pub verified_boot: String,
    pub paired_at: u64,
    /// Shown beside the row so the user can match it against the same value on the phone.
    pub fingerprint: String,
}

pub struct Manager {
    store: Arc<Store>,
    pairings: Arc<Mutex<Pairings>>,
}

impl Manager {
    pub fn new(store: Arc<Store>, pairings: Arc<Mutex<Pairings>>) -> Self {
        Self { store, pairings }
    }
}

#[interface(name = "org.kpasskey.Manager1")]
impl Manager {
    /// Version plus how many devices are paired, so the KCM header needs one round trip.
    #[zbus(property)]
    fn version(&self) -> String {
        let count = self.store.list().map_or(0, |devices| devices.len());
        format!("{} ({count} paired)", env!("CARGO_PKG_VERSION"))
    }

    /// Paired devices, newest last.
    fn list_devices(&self) -> zbus::fdo::Result<Vec<DeviceEntry>> {
        let devices = self
            .store
            .list()
            .map_err(|error| zbus::fdo::Error::Failed(format!("{error:#}")))?;
        Ok(devices
            .into_iter()
            .map(|device| DeviceEntry {
                fingerprint: crate::link::key_fingerprint(&device.public_key_pem)
                    .unwrap_or_else(|_| "unreadable key".to_owned()),
                id: device.id,
                name: device.name,
                model: device.model,
                owner: device.owner,
                security_level: device.security_level,
                verified_boot: device.verified_boot,
                paired_at: device.paired_at,
            })
            .collect())
    }

    /// Forgets a device. Returns false when the id was already gone, so the UI can refresh
    /// without treating a double-click as an error.
    async fn remove_device(
        &self,
        id: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<bool> {
        let removed = self
            .store
            .remove(&id)
            .map_err(|error| zbus::fdo::Error::Failed(format!("{error:#}")))?;
        if removed {
            Self::devices_changed(&emitter).await.ok();
        }
        Ok(removed)
    }

    /// Opens a pairing window and returns the URI to render as a QR code.
    async fn begin_pairing(&self, label: String) -> zbus::fdo::Result<(String, String)> {
        let mut pairings = self.pairings.lock().await;
        let session: &PairingSession = pairings
            .begin(label)
            .map_err(|error| zbus::fdo::Error::Failed(format!("{error:#}")))?;
        Ok((session.uri.clone(), session.short_code.clone()))
    }

    async fn cancel_pairing(&self) -> zbus::fdo::Result<()> {
        self.pairings.lock().await.cancel();
        Ok(())
    }

    /// Seconds remaining in the current pairing window, or 0 when none is open.
    async fn pairing_seconds_left(&self) -> zbus::fdo::Result<u32> {
        Ok(self.pairings.lock().await.seconds_left())
    }

    #[zbus(signal)]
    async fn devices_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Publishes the manager on the session bus and keeps it alive.
///
/// # Errors
/// Fails if the bus is unreachable or the name is already owned.
pub async fn serve(
    store: Arc<Store>,
    pairings: Arc<Mutex<Pairings>>,
) -> Result<zbus::Connection> {
    let manager = Manager::new(store, pairings);

    let connection = zbus::connection::Builder::session()
        .context("connecting to the session bus")?
        .name(BUS_NAME)
        .context("requesting the bus name")?
        .serve_at(OBJECT_PATH, manager)
        .context("exporting the manager object")?
        .build()
        .await
        .context("starting the bus connection")?;

    println!("D-Bus: {BUS_NAME} at {OBJECT_PATH} (session bus)");
    Ok(connection)
}
