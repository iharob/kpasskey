//! Paired device records.
//!
//! Spike location is under the user's data dir; the real daemon uses
//! `/var/lib/kpasskey` so the records are readable before any home directory is
//! unlocked. Only public material is stored, so integrity matters and confidentiality does
//! not.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

#[derive(Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: String,
    pub owner: String,
    pub security_level: String,
    pub verified_boot: String,
    pub paired_at: u64,
    pub public_key_pem: String,
}

impl Device {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "model": self.model,
            "owner": self.owner,
            "securityLevel": self.security_level,
            "verifiedBoot": self.verified_boot,
            "pairedAt": self.paired_at,
            "publicKeyPem": self.public_key_pem,
        })
    }

    fn from_json(value: &serde_json::Value) -> Result<Self> {
        let text = |name: &str| -> String {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let id = text("id");
        if id.is_empty() {
            bail!("device record has no id");
        }
        Ok(Self {
            id,
            name: text("name"),
            model: text("model"),
            owner: text("owner"),
            security_level: text("securityLevel"),
            verified_boot: text("verifiedBoot"),
            paired_at: value.get("pairedAt").and_then(serde_json::Value::as_u64).unwrap_or(0),
            public_key_pem: text("publicKeyPem"),
        })
    }
}

pub struct Store {
    directory: PathBuf,
}

impl Store {
    /// Opens (creating if needed) the device directory.
    ///
    /// # Errors
    /// Fails if the directory cannot be created.
    pub fn open(directory: &Path) -> Result<Self> {
        std::fs::create_dir_all(directory)
            .with_context(|| format!("creating {}", directory.display()))?;
        Ok(Self {
            directory: directory.to_path_buf(),
        })
    }

    /// Lists paired devices, skipping records that fail to parse rather than failing the
    /// whole listing — a corrupt file must not make every other device unusable.
    ///
    /// # Errors
    /// Fails if the directory cannot be read.
    pub fn list(&self) -> Result<Vec<Device>> {
        let mut devices = Vec::new();
        let entries = std::fs::read_dir(&self.directory)
            .with_context(|| format!("reading {}", self.directory.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                eprintln!("[store] skipping unparseable record {}", path.display());
                continue;
            };
            match Device::from_json(&value) {
                Ok(device) => devices.push(device),
                Err(error) => eprintln!("[store] skipping {}: {error:#}", path.display()),
            }
        }
        devices.sort_by_key(|device| device.paired_at);
        Ok(devices)
    }

    /// Writes a record atomically: temporary file, rename, then fsync the directory so the
    /// rename itself is durable.
    ///
    /// # Errors
    /// Fails if the record cannot be written or renamed.
    pub fn save(&self, device: &Device) -> Result<()> {
        let final_path = self.path_for(&device.id);
        let temporary = final_path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&device.to_json())?;

        std::fs::write(&temporary, text)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, &final_path)
            .with_context(|| format!("renaming into {}", final_path.display()))?;

        let directory = std::fs::File::open(&self.directory)
            .with_context(|| format!("opening {} to fsync", self.directory.display()))?;
        directory.sync_all().context("fsync on the device directory")?;
        Ok(())
    }

    /// Removes a paired device.
    ///
    /// # Errors
    /// Fails if the record exists but cannot be deleted.
    pub fn remove(&self, id: &str) -> Result<bool> {
        let path = self.path_for(id);
        if !path.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        Ok(true)
    }

    /// Rejects ids that are not plain hex so a crafted id cannot escape the directory.
    fn path_for(&self, id: &str) -> PathBuf {
        let safe: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        self.directory.join(format!("{safe}.json"))
    }
}
