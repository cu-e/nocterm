//! A chat's SSH binding, certificate requests and per-chat key custody (ADR 0137).
//!
//! Each transition runs on a copy of the chat's access, stores the new journal v8 row,
//! then journals and publishes its events; if publishing fails the previous row is put
//! back. The key seed comes from the coordinator's identity entropy, lives in the sealed
//! row and in memory for sessions, and is dropped when no binding needs it.
use super::{chat::Chat, frame::Frame, services::CoordinatorServices};
use nocterm_cloud_model::{
    authority::{AuthenticationBinding, StreamBinding, TargetBinding},
    coordinator::*,
    identity::{Fingerprint, Timestamp},
    journal::*,
    ssh_access::*,
};
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Chats one runner keeps SSH access for.
const MAX_CHATS: usize = 1024;

/// The runner's reach, its journal rows and the chats' access held in memory.
pub struct CoordinatorSsh {
    keys: Arc<dyn ChatSshKeys>,
    certificates: Arc<dyn SshCertificateReader>,
    reach: SshReach,
    held: Mutex<BTreeMap<StreamBinding, Held>>,
    order: Arc<AsyncMutex<bool>>,
}
#[derive(Clone, Default)]
struct Held {
    access: ChatSshAccess,
    seed: Option<SshSubjectSeed>,
}
impl Held {
    fn stored(&self) -> StoredSshAccess {
        StoredSshAccess {
            seed: self.seed.clone(),
            access: self.access.parts().clone(),
        }
    }
}

/// What the coordinator does with one SSH command after its row is stored.
pub(super) struct SshTransition {
    _order: OwnedMutexGuard<bool>,
    stream: StreamBinding,
    previous: Held,
    next: Held,
    events: Vec<SshAccessEvent>,
}
impl SshTransition {
    pub(super) fn records(
        &self,
    ) -> Result<Vec<(JournalRecord, Option<CoordinatorEvent>)>, CoordinatorError> {
        self.events
            .iter()
            .map(|event| {
                Ok((
                    JournalRecord::new(
                        RecordContext::Chat,
                        JournalRecordKind::SshAccess,
                        Frame::SshAccess.bytes()?,
                    )?,
                    Some(CoordinatorEvent::SshAccess(Box::new(event.clone()))),
                ))
            })
            .collect()
    }
}

impl CoordinatorSsh {
    pub fn new(
        keys: Arc<dyn ChatSshKeys>,
        certificates: Arc<dyn SshCertificateReader>,
        reach: SshReach,
    ) -> Self {
        Self {
            keys,
            certificates,
            reach,
            held: Mutex::new(BTreeMap::new()),
            order: Arc::new(AsyncMutex::new(false)),
        }
    }
    async fn held(
        &self,
        services: &CoordinatorServices,
        stream: &StreamBinding,
    ) -> Result<Held, CoordinatorError> {
        if let Some(held) = self.held.lock().get(stream) {
            return Ok(held.clone());
        }
        let keys = self.keys.clone();
        let lookup = stream.clone();
        let stored = services.io().run(move |_| keys.ssh_access(lookup)).await?;
        let held = match stored {
            Some(stored) => Held {
                access: ChatSshAccess::restore(stored.access).map_err(|_| JournalError::Corrupt)?,
                seed: stored.seed,
            },
            None => Held::default(),
        };
        let mut map = self.held.lock();
        if map.len() >= MAX_CHATS && !map.contains_key(stream) {
            return Err(CoordinatorError::Overloaded);
        }
        map.insert(stream.clone(), held.clone());
        Ok(held)
    }
    async fn store(
        &self,
        services: &CoordinatorServices,
        stream: &StreamBinding,
        held: &Held,
    ) -> Result<(), CoordinatorError> {
        let keys = self.keys.clone();
        let (stream, stored) = (stream.clone(), held.stored());
        services
            .io()
            .run(move |_| keys.store_ssh_access(stream, stored))
            .await
    }
    /// Computes and stores the transition of an SSH command; nothing is published yet.
    pub(super) async fn prepare(
        &self,
        services: &CoordinatorServices,
        command: &ReceivedCommand,
        now: Timestamp,
    ) -> Result<SshTransition, CoordinatorError> {
        let order = self.order.clone().lock_owned().await;
        let stream = command.stream().clone();
        let previous = self.held(services, &stream).await?;
        let mut next = previous.clone();
        let change = match command.body() {
            CommandBody::SshBinding(offer) => {
                let seed = match &next.seed {
                    Some(seed) => seed.clone(),
                    None => fresh_seed(services)?,
                };
                let change = next
                    .access
                    .bind(
                        (**offer).clone(),
                        &self.reach,
                        &seed.public_key(),
                        now,
                        request_id(services)?,
                    )
                    .map_err(refusal)?;
                next.seed = Some(seed);
                change
            }
            CommandBody::SshCertificate(offer) => {
                let seed = next.seed.clone().ok_or(CoordinatorError::Conflict)?;
                let ca = *next
                    .access
                    .binding()
                    .ok_or(CoordinatorError::Conflict)?
                    .ca_key();
                let facts = self
                    .certificates
                    .read(offer.certificate().as_bytes(), &ca)
                    .map_err(refusal)?;
                next.access
                    .certify(offer, &facts, &seed.public_key(), now)
                    .map_err(refusal)?
            }
            CommandBody::RevokeGrant(reference) => {
                next.access.revoke(*reference).map_err(refusal)?
            }
            CommandBody::DeleteChat => next.access.forget(),
            _ => return Err(CoordinatorError::InvalidCommand),
        };
        if change.key == KeyChange::Forget {
            next.seed = None;
        }
        // A chat that never had SSH access gets no row.
        if next.access != previous.access || next.seed.is_some() != previous.seed.is_some() {
            self.store(services, &stream, &next).await?;
        }
        Ok(SshTransition {
            _order: order,
            stream,
            previous,
            next,
            events: change.events,
        })
    }
    /// Keeps the transition after its command and events are durable, or puts the
    /// previous row back.
    pub(super) async fn settle(
        &self,
        services: &CoordinatorServices,
        transition: SshTransition,
        durable: bool,
    ) -> Result<(), CoordinatorError> {
        let held = if durable {
            transition.next
        } else {
            self.store(services, &transition.stream, &transition.previous)
                .await?;
            transition.previous
        };
        self.held.lock().insert(transition.stream, held);
        Ok(())
    }
    /// Loads every chat that has a row once, then renews or ends the access of each chat
    /// whose next deadline passed. Returns the chats that published something.
    pub(super) async fn due(
        &self,
        services: &CoordinatorServices,
    ) -> Result<Vec<StreamBinding>, CoordinatorError> {
        let mut loaded = self.order.lock().await;
        if !*loaded {
            let keys = self.keys.clone();
            for stream in services.io().run(move |_| keys.ssh_streams()).await? {
                self.held(services, &stream).await?;
            }
            *loaded = true;
        }
        let now = services.timer().now();
        Ok(self
            .held
            .lock()
            .iter()
            .filter(|(_, held)| held.access.next_due().is_some_and(|due| due <= now))
            .map(|(stream, _)| stream.clone())
            .collect())
    }
    /// One renewal or expiry step of `chat`, journaled like a command's events.
    pub(super) async fn renew(
        &self,
        services: &CoordinatorServices,
        chat: &Chat,
    ) -> Result<(), CoordinatorError> {
        let order = self.order.clone().lock_owned().await;
        let stream = chat.log().lease().stream().clone();
        let previous = self.held(services, &stream).await?;
        let Some(seed) = previous.seed.clone() else {
            return Ok(());
        };
        let mut next = previous.clone();
        let now = services.timer().now();
        let change = next
            .access
            .tick(&self.reach, &seed.public_key(), now, request_id(services)?);
        if change.events.is_empty() {
            return Ok(());
        }
        if change.key == KeyChange::Forget {
            next.seed = None;
        }
        self.store(services, &stream, &next).await?;
        let transition = SshTransition {
            _order: order,
            stream,
            previous,
            next,
            events: change.events,
        };
        let appended = chat.log().append(None, transition.records()?, None).await;
        self.settle(services, transition, appended.is_ok()).await?;
        appended.map(|_| ())
    }
    /// The access of `stream` as it stands, for its summary.
    pub fn report(&self, stream: &StreamBinding) -> Option<SshAccessReport> {
        self.held.lock().get(stream)?.access.report()
    }
}
impl ChatSshCredentials for CoordinatorSsh {
    fn credential(
        &self,
        stream: &StreamBinding,
        endpoint: &SshEndpoint,
        host_key: &Fingerprint,
        now: Timestamp,
    ) -> Option<SshLoginCredential> {
        let held = self.held.lock();
        let held = held.get(stream)?;
        let credential = held.access.credential(endpoint, host_key, now)?;
        Some(SshLoginCredential {
            seed: held.seed.clone()?,
            certificate: credential.certificate.clone(),
            principal: credential.principal.to_owned(),
            valid_before: credential.valid_before,
        })
    }
    fn bound(
        &self,
        stream: &StreamBinding,
        target: &TargetBinding,
        authentication: &AuthenticationBinding,
        now: Timestamp,
    ) -> Option<BoundLogin> {
        let held = self.held.lock();
        let held = held.get(stream)?;
        let selected = held.access.select(target, authentication, now)?;
        let credential = selected.credential.and_then(|credential| {
            Some(SshLoginCredential {
                seed: held.seed.clone()?,
                certificate: credential.certificate.clone(),
                principal: credential.principal.to_owned(),
                valid_before: credential.valid_before,
            })
        });
        Some(BoundLogin {
            endpoint: selected.target.endpoint.clone(),
            host_key: selected.target.host_key,
            credential,
        })
    }
}

fn fresh_seed(services: &CoordinatorServices) -> Result<SshSubjectSeed, CoordinatorError> {
    let mut seed = [0; 32];
    seed[..16].copy_from_slice(&services.identity().fresh()?);
    seed[16..].copy_from_slice(&services.identity().fresh()?);
    Ok(SshSubjectSeed::new(seed))
}
fn request_id(services: &CoordinatorServices) -> Result<[u8; 16], CoordinatorError> {
    services.identity().fresh()
}
fn refusal(error: SshAccessError) -> CoordinatorError {
    match error {
        SshAccessError::Stale | SshAccessError::OtherGrant | SshAccessError::UnknownRequest => {
            CoordinatorError::Conflict
        }
        SshAccessError::NoBinding => CoordinatorError::Conflict,
        SshAccessError::Invalid
        | SshAccessError::SubjectKey
        | SshAccessError::Principals
        | SshAccessError::Validity => CoordinatorError::InvalidCommand,
    }
}
