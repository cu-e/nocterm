//! The objects of the `agent_ssh_trust` collection: the CA document and one trust record per
//! server, read through the change stream and written with the version they were read at.

use std::collections::BTreeMap;

use nocterm_hubs_model::AgentAccessError;
use nocterm_keys::agents::SshTrustRecord;
use sha2::{Digest, Sha256};

use super::document::{CA_OBJECT, CaDocument};
use crate::store::{Located, OpError, Store};

/// The kind of the personal collection that holds the CA and the registry.
pub(super) const KIND: &str = "agent_ssh_trust";

/// A value with the version of the object it was read from; 0 when it was never stored.
pub(super) struct Versioned<T> {
    pub(super) value: T,
    pub(super) version: u64,
}

/// Everything the collection holds.
pub(super) struct Registry {
    pub(super) at: Located,
    pub(super) ca: Option<Versioned<CaDocument>>,
    pub(super) servers: BTreeMap<String, Versioned<SshTrustRecord>>,
}

pub(super) fn records(error: impl std::error::Error + Send + Sync + 'static) -> AgentAccessError {
    AgentAccessError::Records {
        cause: nocterm_core::error::SharedCause::new(error),
    }
}

pub(super) fn store_error(error: OpError) -> AgentAccessError {
    match error {
        OpError::Hub(error) => AgentAccessError::Hub(error),
        other => records(other),
    }
}

/// The object id of the server reached through `connection`.
pub(super) fn server_object(connection: &str) -> String {
    let digest = Sha256::digest(connection.as_bytes());
    let mut id = String::from("srv-");
    id.push_str(&hex::encode(&digest[..16]));
    id
}

impl Registry {
    /// The collection and what it holds. `create` makes the collection when the account has none.
    pub(super) async fn load(
        store: &Store,
        create: bool,
    ) -> Result<Option<Self>, AgentAccessError> {
        let at = if create {
            store.ensure(KIND).await.map_err(store_error)?
        } else {
            match store.find(KIND).await.map_err(store_error)? {
                Some(at) => at,
                None => return Ok(None),
            }
        };
        let mut registry = Self {
            at,
            ca: None,
            servers: BTreeMap::new(),
        };
        let mut after = 0;
        loop {
            let batch = store
                .changes(&registry.at, after)
                .await
                .map_err(store_error)?;
            for object in batch.changes {
                after = after.max(object.stream_version);
                registry.absorb(&object)?;
            }
            if !batch.more {
                return Ok(Some(registry));
            }
        }
    }

    fn absorb(
        &mut self,
        object: &nocterm_hub_proto::v1::StoredObject,
    ) -> Result<(), AgentAccessError> {
        if object.kind != KIND {
            return Ok(());
        }
        if object.deleted {
            self.servers
                .retain(|connection, _| server_object(connection) != object.object_id);
            if object.object_id == CA_OBJECT {
                self.ca = None;
            }
            return Ok(());
        }
        let plain = self
            .at
            .key()
            .open_object(self.at.id(), &object.object_id, KIND, &object.ciphertext)
            .map_err(records)?;
        if object.object_id == CA_OBJECT {
            let value = CaDocument::decode(&plain).map_err(records)?;
            self.ca = Some(Versioned {
                value,
                version: object.version,
            });
        } else if let Ok(record) = SshTrustRecord::decode(&plain) {
            self.servers.insert(
                record.profile_id.clone(),
                Versioned {
                    value: record,
                    version: object.version,
                },
            );
        }
        Ok(())
    }

    pub(super) async fn save_ca(
        &mut self,
        store: &Store,
        document: CaDocument,
    ) -> Result<(), AgentAccessError> {
        let expected = self.ca.as_ref().map_or(0, |ca| ca.version);
        let plain = document.encode().map_err(records)?;
        let written = store
            .put(&self.at, KIND, CA_OBJECT, &plain, expected)
            .await
            .map_err(store_error)?;
        self.ca = Some(Versioned {
            value: document,
            version: written.version,
        });
        Ok(())
    }

    pub(super) async fn save_server(
        &mut self,
        store: &Store,
        record: SshTrustRecord,
    ) -> Result<(), AgentAccessError> {
        let connection = record.profile_id.clone();
        let expected = self
            .servers
            .get(&connection)
            .map_or(0, |known| known.version);
        let plain = record.encode().map_err(records)?;
        let written = store
            .put(
                &self.at,
                KIND,
                &server_object(&connection),
                &plain,
                expected,
            )
            .await
            .map_err(store_error)?;
        self.servers.insert(
            connection,
            Versioned {
                value: record,
                version: written.version,
            },
        );
        Ok(())
    }
}
