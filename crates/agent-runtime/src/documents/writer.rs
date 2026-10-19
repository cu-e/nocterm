//! Explicit document publication handle; admission never re-borrows Runtime.
use super::ownership;
use super::*;
#[derive(Clone)]
pub struct DocumentWriter {
    registry: Rc<RefCell<DocumentRegistry>>,
    writer: Entity<ChatDriver>,
}
impl DocumentWriter {
    /// Intake borrows the real Candidate only for this synchronous publication.
    pub fn save(
        &self,
        candidate: nocterm_ai::history::writing::CandidateOffer<'_>,
        owner: EntityId,
        revision: u64,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        self.admit_candidate(candidate, owner, revision, Admission::Normal, cx)
    }
    fn admit_candidate(
        &self,
        candidate: nocterm_ai::history::writing::CandidateOffer<'_>,
        owner: EntityId,
        revision: u64,
        admission: Admission,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        let key = candidate.id().map_err(StoreError::backend)?;
        let allowed = match admission {
            Admission::Normal => self.registry.borrow().current(&key, owner),
            Admission::Capture => self.registry.borrow().capture_current(&key, owner),
        };
        if !allowed
            || !self
                .registry
                .borrow()
                .accepts_revision(&key, owner, revision)
        {
            return Ok(false);
        }
        let id = key.to_string();
        drop(key);
        let stamp = CandidateStamp {
            owner: Some(owner),
            revision,
            deleted: false,
        };
        self.writer.update(cx, |writer, cx| {
            writer.update_latest_owned(
                id.clone(),
                stamp,
                |previous| {
                    let previous = previous.and_then(|value| match value {
                        ChatIntent::Managed(latest) => Some(latest),
                        _ => None,
                    });
                    candidate
                        .into_latest(previous)
                        .map(ChatIntent::Managed)
                        .map_err(StoreError::backend)
                },
                cx,
            )
        })?;
        self.registry.borrow_mut().accepted(&id, owner, revision);
        self.notify_admission(&id, owner, revision, cx);
        Ok(true)
    }
    pub(super) fn capture_for(&self, owner: EntityId) -> CaptureAdmission {
        CaptureAdmission {
            writer: self.clone(),
            owner,
        }
    }
    pub(super) fn new(
        repository: Arc<dyn nocterm_ai::storage::ChatRepository>,
        registry: Rc<RefCell<DocumentRegistry>>,
        cx: &mut Context<Runtime>,
    ) -> Self {
        let owner = registry.clone();
        let backend = repository.clone();
        let writer = cx.new(|_| {
            ChatDriver::with_owned_values(
                Arc::new(move |id, intent| {
                    let backend = backend.clone();
                    Box::pin(async move {
                        match intent {
                            ownership::ChatOperation::Managed(write) => {
                                backend.save_owned(write).await
                            }
                            ownership::ChatOperation::Delete => backend.delete(id).await,
                        }
                        .map_err(StoreError::backend)
                    })
                }),
                NonZeroUsize::new(32).expect("nonzero constant"),
                ChatValues,
            )
            .with_completion_hook(move |stamp, result, cx| {
                let succeeded = result.is_ok();
                let retained = owner.borrow().target(stamp, succeeded).is_some();
                let registry = owner.clone();
                let stamp = stamp.clone();
                // A client may synchronously reenter Runtime and its writer. Deliver only
                // after the keyed entity borrow ends, and revalidate identity at delivery.
                cx.defer(move |cx| {
                    let target = registry.borrow().target(&stamp, succeeded);
                    if let Some(client) = target
                        && client
                            .state(cx)
                            .is_some_and(|state| state.chat_id == *stamp.key())
                    {
                        let event = match result {
                            Ok(()) => SessionEvent::Saved {
                                revision: stamp.tag().revision,
                            },
                            Err(error) => SessionEvent::SaveFailed {
                                revision: stamp.tag().revision,
                                error: error.into(),
                            },
                        };
                        client.emit(event, cx);
                    }
                });
                if retained {
                    KeyedRetention::Retain
                } else {
                    KeyedRetention::ForgetClean
                }
            })
        });
        Self { registry, writer }
    }
    pub(super) fn entity(&self) -> &Entity<ChatDriver> {
        &self.writer
    }
    pub(super) fn notify_admission(&self, id: &str, owner: EntityId, revision: u64, cx: &mut App) {
        let registry = self.registry.clone();
        let id = id.to_owned();
        cx.defer(move |cx| {
            let target = {
                let registry = registry.borrow();
                registry
                    .capture_current(&id, owner)
                    .then(|| registry.client(owner))
                    .flatten()
            };
            if let Some(client) = target {
                client.emit(SessionEvent::SaveAdmitted { revision }, cx);
            }
        });
    }
    pub(super) fn notify_failed_admission(
        &self,
        id: String,
        owner: EntityId,
        revision: u64,
        error: AgentPersistenceError,
        cx: &mut App,
    ) {
        let registry = self.registry.clone();
        cx.defer(move |cx| {
            let target = {
                let registry = registry.borrow();
                registry
                    .capture_current(&id, owner)
                    .then(|| registry.client(owner))
                    .flatten()
            };
            if let Some(client) = target {
                client.emit(SessionEvent::SaveFailed { revision, error }, cx);
            }
        });
    }
    pub(super) fn submit(
        &self,
        id: String,
        intent: ChatIntent,
        stamp: CandidateStamp,
        cx: &mut App,
    ) -> Result<(), StoreError> {
        self.writer
            .update(cx, |writer, cx| writer.update_latest(id, intent, stamp, cx))
            .map(|_| ())
    }
    pub(super) fn delete(&self, id: String, cx: &mut App) {
        if self.registry.borrow().closing {
            return;
        }
        self.registry.borrow_mut().seal(id.clone());
        if let Err(error) = self.submit(
            id,
            ChatIntent::Delete,
            CandidateStamp {
                owner: None,
                revision: 0,
                deleted: true,
            },
            cx,
        ) {
            tracing::warn!(%error, "agent deletion admission failed");
        }
    }
}

#[derive(Clone, Copy)]
enum Admission {
    Normal,
    Capture,
}
/// Minted only by Runtime's original capture flow. It cannot force a normal
/// save past shutdown, ownership changes or permanent deletion.
pub struct CaptureAdmission {
    writer: DocumentWriter,
    owner: EntityId,
}
impl CaptureAdmission {
    pub fn save(
        &self,
        candidate: nocterm_ai::history::writing::CandidateOffer<'_>,
        revision: u64,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        self.writer
            .admit_candidate(candidate, self.owner, revision, Admission::Capture, cx)
    }
}
