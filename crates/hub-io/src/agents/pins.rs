//! The pool keys this device pinned, one private file per hub.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use nocterm_cloud_agents_model::CloudAgentError;
use nocterm_core::SharedCause;
use nocterm_hub_model::agents::PoolKeys;
use nocterm_hubs_model::HubId;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const MAX_FILE_BYTES: usize = 256 * 1024;

#[derive(Default, Serialize, Deserialize)]
struct File {
    pools: BTreeMap<String, Pin>,
}

#[derive(Serialize, Deserialize)]
struct Pin {
    encryption: [u8; 32],
    signing: [u8; 32],
}

/// Reads and writes the pins. Pinning is the only way a pool key becomes trusted here.
pub(super) struct PoolPins {
    directory: PathBuf,
    writing: Mutex<()>,
}

impl PoolPins {
    pub(super) fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            writing: Mutex::new(()),
        }
    }

    fn path(&self, hub: &HubId) -> PathBuf {
        let digest = Sha256::digest(hub.address().to_string().as_bytes());
        let name: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
        self.directory.join(format!("{name}.json"))
    }

    fn read(&self, hub: &HubId) -> Result<File, CloudAgentError> {
        let path = self.path(hub);
        if !path.exists() {
            return Ok(File::default());
        }
        let bytes =
            nocterm_core::fs::read_regular_bounded(&path, MAX_FILE_BYTES).map_err(storage)?;
        serde_json::from_slice(&bytes).map_err(storage)
    }

    /// The keys pinned for `pool` on `hub`.
    pub(super) fn get(&self, hub: &HubId, pool: &str) -> Result<Option<PoolKeys>, CloudAgentError> {
        let _guard = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(self.read(hub)?.pools.get(pool).map(|pin| PoolKeys {
            encryption: pin.encryption,
            signing: pin.signing,
        }))
    }

    /// Pins `keys` for `pool`, replacing nothing: an existing different pin is refused by the
    /// caller before this is reached.
    pub(super) fn pin(
        &self,
        hub: &HubId,
        pool: &str,
        keys: &PoolKeys,
    ) -> Result<(), CloudAgentError> {
        let _guard = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let mut file = self.read(hub)?;
        file.pools.insert(
            pool.to_owned(),
            Pin {
                encryption: keys.encryption,
                signing: keys.signing,
            },
        );
        let bytes = serde_json::to_vec(&file).map_err(storage)?;
        nocterm_core::fs::write_private_atomic(&self.path(hub), &bytes).map_err(storage)
    }
}

fn storage(error: impl std::error::Error + Send + Sync + 'static) -> CloudAgentError {
    CloudAgentError::Registration(SharedCause::new(error))
}

#[cfg(test)]
mod tests {
    use nocterm_hubs_model::HubAddress;

    use super::*;

    fn hub(host: &str) -> HubId {
        HubId::from(HubAddress::parse(host).expect("address"))
    }

    fn keys(byte: u8) -> PoolKeys {
        PoolKeys {
            encryption: [byte; 32],
            signing: [byte + 1; 32],
        }
    }

    #[test]
    fn a_pin_survives_a_restart_and_belongs_to_one_hub_and_pool() {
        let dir = tempfile::tempdir().expect("dir");
        let pins = PoolPins::new(dir.path().join("pins"));
        assert!(
            pins.get(&hub("a.example.org"), "p1")
                .expect("get")
                .is_none()
        );
        pins.pin(&hub("a.example.org"), "p1", &keys(1))
            .expect("pin");

        let again = PoolPins::new(dir.path().join("pins"));
        assert_eq!(
            again.get(&hub("a.example.org"), "p1").expect("get"),
            Some(keys(1))
        );
        assert!(
            again
                .get(&hub("a.example.org"), "p2")
                .expect("get")
                .is_none()
        );
        assert!(
            again
                .get(&hub("b.example.org"), "p1")
                .expect("get")
                .is_none()
        );
    }

    #[test]
    fn a_damaged_pin_file_is_an_error_not_an_unpinned_pool() {
        let dir = tempfile::tempdir().expect("dir");
        let pins = PoolPins::new(dir.path().to_owned());
        pins.pin(&hub("a.example.org"), "p1", &keys(1))
            .expect("pin");
        std::fs::write(pins.path(&hub("a.example.org")), b"{").expect("damage");
        assert!(pins.get(&hub("a.example.org"), "p1").is_err());
    }
}
