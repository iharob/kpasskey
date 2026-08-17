//! BLE proximity beacon: carries a per-verification nonce over a minimum-power radio link
//! so that a network-only attacker cannot produce it.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result};
use rand::TryRngCore as _;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy, interface};

/// Service UUID under which the proximity nonce is advertised as GATT service data.
pub const PROXIMITY_SERVICE_UUID: &str = "6f1d2b1a-9c4e-4f57-a1d3-7b8e2c05f9a4";

/// Advertised nonce length. 16 bytes fits extended advertising; legacy 31-byte advertising
/// forces a shorter value, still ample for a single-use token with a 30 s life.
pub const NONCE_LEN: usize = 16;

/// Minimum transmit power in dBm. The controller clamps this to its own floor; the intent
/// is a beacon readable at roughly arm's length rather than across a building.
pub const MIN_TX_POWER_DBM: i16 = -34;

const ADVERTISEMENT_PATH: &str = "/org/kpasskey/advertisement/0";

/// A nonce that has been broadcast but not yet echoed back inside a signed assertion.
#[derive(Clone, PartialEq, Eq)]
pub struct ProximityNonce([u8; NONCE_LEN]);

impl ProximityNonce {
    /// Draws a fresh nonce from the OS CSPRNG.
    ///
    /// # Errors
    /// Fails if the operating system entropy source is unavailable.
    pub fn generate() -> Result<Self> {
        let mut bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .context("drawing a proximity nonce from the OS CSPRNG")?;
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; NONCE_LEN] {
        &self.0
    }
}

/// Deliberately opaque: a nonce must never reach a log, and `Debug` is how it would.
impl std::fmt::Debug for ProximityNonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProximityNonce(<redacted>)")
    }
}

struct Advertisement {
    nonce: ProximityNonce,
    kind: String,
    uuids: Vec<String>,
    tx_power: i16,
    released: Arc<AtomicBool>,
}

#[interface(name = "org.bluez.LEAdvertisement1")]
impl Advertisement {
    /// BlueZ calls this when it has dropped the advertisement of its own accord. Recording
    /// it keeps [`Beacon::stop`] from unregistering something that no longer exists, which
    /// BlueZ answers with `DoesNotExist`.
    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }

    #[zbus(property, name = "Type")]
    fn advertisement_type(&self) -> String {
        self.kind.clone()
    }

    #[zbus(property, name = "ServiceUUIDs")]
    fn service_uuids(&self) -> Vec<String> {
        self.uuids.clone()
    }

    #[zbus(property, name = "ServiceData")]
    fn service_data(&self) -> HashMap<String, OwnedValue> {
        let payload = Value::from(self.nonce.as_bytes().to_vec());
        let mut data = HashMap::new();
        if let Ok(owned) = OwnedValue::try_from(payload) {
            data.insert(PROXIMITY_SERVICE_UUID.to_owned(), owned);
        }
        data
    }

    #[zbus(property, name = "TxPower")]
    fn tx_power(&self) -> i16 {
        self.tx_power
    }
}

/// A registered advertisement. Dropping it does not unregister — call [`Beacon::stop`] so
/// failures on the way down are visible rather than swallowed in a destructor.
pub struct Beacon {
    connection: Connection,
    adapter: OwnedObjectPath,
    path: OwnedObjectPath,
    released: Arc<AtomicBool>,
}

impl Beacon {
    /// Registers a proximity advertisement carrying `nonce` with BlueZ.
    ///
    /// # Errors
    /// Fails if the system bus is unreachable, the object cannot be served, or BlueZ
    /// refuses the registration (no adapter, powered off, or policy denial).
    pub async fn start(adapter: &str, nonce: ProximityNonce) -> Result<Self> {
        let path = OwnedObjectPath::try_from(ADVERTISEMENT_PATH)
            .context("building the advertisement object path")?;
        let adapter = OwnedObjectPath::try_from(adapter)
            .with_context(|| format!("{adapter} is not a valid adapter object path"))?;

        let released = Arc::new(AtomicBool::new(false));
        let advertisement = Advertisement {
            nonce,
            kind: "peripheral".to_owned(),
            uuids: vec![PROXIMITY_SERVICE_UUID.to_owned()],
            tx_power: MIN_TX_POWER_DBM,
            released: Arc::clone(&released),
        };

        let connection = zbus::connection::Builder::system()
            .context("connecting to the system bus")?
            .serve_at(&path, advertisement)
            .context("serving the advertisement object")?
            .build()
            .await
            .context("starting the system bus connection")?;

        let beacon = Self {
            connection,
            adapter,
            path,
            released,
        };
        beacon.register().await?;
        Ok(beacon)
    }

    async fn register(&self) -> Result<()> {
        let manager = self.manager().await?;
        let options: HashMap<&str, Value<'_>> = HashMap::new();
        manager
            .call_method("RegisterAdvertisement", &(&self.path, options))
            .await
            .context("RegisterAdvertisement rejected by BlueZ")?;
        Ok(())
    }

    /// Withdraws the advertisement, unless BlueZ already released it.
    ///
    /// # Errors
    /// Fails if BlueZ rejects the unregistration; the caller should still treat the
    /// verification as finished.
    pub async fn stop(self) -> Result<()> {
        if self.released.load(Ordering::SeqCst) {
            return Ok(());
        }
        let manager = self.manager().await?;
        manager
            .call_method("UnregisterAdvertisement", &(&self.path,))
            .await
            .context("UnregisterAdvertisement rejected by BlueZ")?;
        Ok(())
    }

    async fn manager(&self) -> Result<Proxy<'_>> {
        let adapter: &ObjectPath<'_> = &self.adapter;
        Proxy::new(
            &self.connection,
            "org.bluez",
            adapter.clone(),
            "org.bluez.LEAdvertisingManager1",
        )
        .await
        .context("reaching org.bluez.LEAdvertisingManager1")
    }
}
