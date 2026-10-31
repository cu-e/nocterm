//! Intent -> durable dispatch -> native effect -> durable observation.
use super::{
    chat::Chat,
    frame::{Call, Frame, Grant},
    run::{PendingApproval, RunRecord},
    services::CoordinatorServices,
};
use futures::{
    channel::oneshot,
    future::{AbortHandle, Abortable},
};
use nocterm_ai::{TerminalCall, tool_display::ToolDisplay};
use nocterm_cloud_model::{
    coordinator::*, engine::*, grant::GrantState, identity::*, journal::*, payload::BoundedJson,
    run::*, tool::*,
};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    time::Duration,
};

struct RunAuthority {
    services: Arc<CoordinatorServices>,
    chat: Weak<Chat>,
    run: Weak<RunRecord>,
    allowed: Mutex<BTreeMap<IntentId, ScopedToolRequest>>,
}
impl RunAuthority {
    fn allow(&self, request: ScopedToolRequest) -> Result<(), CoordinatorError> {
        let mut allowed = self.allowed.lock();
        let id = request.permit().intent().id();
        if allowed.contains_key(&id) {
            return Err(CoordinatorError::Conflict);
        }
        allowed.insert(id, request);
        Ok(())
    }
    fn validate(&self, request: &ScopedToolRequest) -> Result<(), CoordinatorError> {
        self.services.tasks().check_healthy()?;
        if !self.services.tasks().running() {
            return Err(CoordinatorError::Stopped);
        }
        let run = self.run.upgrade().ok_or(CoordinatorError::Stopped)?;
        let chat = self.chat.upgrade().ok_or(CoordinatorError::Stopped)?;
        if run.selected_session().as_ref() != Some(request.permit().intent().authority().session())
        {
            return Err(CoordinatorError::NeedsUnlock);
        }
        run.control().check(self.services.source().as_ref())?;
        chat.check()?;
        chat.log().require_current()?;
        let allowed = self.allowed.lock();
        let original = allowed
            .get(&request.permit().intent().id())
            .ok_or(CoordinatorError::Conflict)?;
        if original.permit() != request.permit()
            || original.call().name() != request.call().name()
            || original.call().arguments().map_err(EngineError::from)?
                != request.call().arguments().map_err(EngineError::from)?
            || self.services.arguments().fingerprint(request.call())?
                != request.permit().intent().arguments()
        {
            return Err(CoordinatorError::Conflict);
        }
        let mut grant = run
            .grants()
            .tool(request.permit().intent().authority().grant())
            .cloned()
            .ok_or(CoordinatorError::NeedsGrant)?;
        run.control()
            .check_window(self.services.timer().now(), grant.policy().window())?;
        grant.revalidate_reserved(
            request.permit().intent().authority(),
            request.call(),
            self.services.timer().now(),
        )?;
        Ok(())
    }
}
impl ToolAuthority for RunAuthority {
    fn check(&self, request: &ScopedToolRequest) -> Result<(), ToolExecutionError> {
        self.validate(request)
            .map_err(|cause| ToolExecutionError::Execution(nocterm_core::SharedCause::new(cause)))
    }
}

pub(super) struct EffectHost {
    services: Arc<CoordinatorServices>,
    chat: Arc<Chat>,
    run: Arc<RunRecord>,
    state: RunState,
    authority: Arc<RunAuthority>,
    runtime: Option<Arc<dyn RunToolRuntime>>,
}
impl EffectHost {
    pub(super) fn new(
        services: Arc<CoordinatorServices>,
        chat: Arc<Chat>,
        run: Arc<RunRecord>,
        state: RunState,
    ) -> Self {
        let authority = Arc::new(RunAuthority {
            services: services.clone(),
            chat: Arc::downgrade(&chat),
            run: Arc::downgrade(&run),
            allowed: Mutex::new(BTreeMap::new()),
        });
        Self {
            services,
            chat,
            run,
            state,
            authority,
            runtime: None,
        }
    }
    pub(super) async fn execute(
        &mut self,
        id: IntentId,
        call: EngineToolCall,
    ) -> Result<EngineToolResult, CoordinatorError> {
        self.run.control().check(self.services.source().as_ref())?;
        self.attach().await?;
        let session = self
            .run
            .selected_session()
            .ok_or(CoordinatorError::NeedsUnlock)?;
        let mut grant = self
            .run
            .grants()
            .select_tool(&session, call.call(), self.services.timer().now())
            .ok_or(CoordinatorError::NeedsGrant)?;
        let original_authority = grant.authority().clone();
        grant.authorize(
            &original_authority,
            call.call(),
            self.services.timer().now(),
        )?;
        let digest = self.services.arguments().fingerprint(call.call())?;
        let intent = ToolIntent::new(
            id,
            call.slot().step(),
            grant.authority().clone(),
            digest,
            call.call(),
        )?;
        self.state
            .prepare_intent(intent.clone(), self.state.revision())?;
        let stamp = self.next_stamp()?;
        let record = JournalRecord::new(
            RecordContext::Transition(stamp.clone()),
            JournalRecordKind::Intent(Box::new(intent.clone())),
            Frame::Intent {
                intent: (&intent).into(),
                call: Call::try_from(call.call())?,
                grant: Grant::from(&grant),
            }
            .bytes()?,
        )?;
        let mut records = vec![(
            record,
            Some(CoordinatorEvent::ToolIntent {
                intent: intent.clone(),
                call: call.call().clone(),
                display: ToolDisplay::new(call.call(), None),
            }),
        )];
        let wait = if self.needs_person(&intent, call.call()) {
            let expires = grant.expires();
            let (record, event, receiver) = self.pending_approval(&intent, call.call(), expires)?;
            records.push((record, Some(event)));
            Some((receiver, expires))
        } else {
            None
        };
        let committed = self.commit(records).await?;
        self.state.intent_committed(id, committed)?;
        let mut grants = self.run.grants();
        grants.replace_tool(grant.clone());
        self.run.set_grants(grants);
        self.run.arm_approval();
        if let Some((answer, expires)) = wait {
            self.run
                .status(self.chat.log(), RunStatus::WaitingApproval)
                .await?;
            let choice = match self.approval(answer, expires).await {
                Ok(choice) => choice,
                Err(error) => return self.decline(&intent, &call, &error.to_string()).await,
            };
            if choice != ApprovalChoice::Approve {
                return self
                    .decline(&intent, &call, "The operation was declined.")
                    .await;
            }
        }
        self.release(intent, call, grant).await
    }
    /// A mutating call waits for a person unless the runner's policy relaxes it
    /// for the contained target the call is bound to.
    fn needs_person(&self, intent: &ToolIntent, call: &TerminalCall) -> bool {
        intent.mutates()
            && self.services.approval().is_none_or(|policy| {
                policy.requires_approval(call, intent.authority().session().target())
            })
    }
    fn pending_approval(
        &self,
        intent: &ToolIntent,
        call: &TerminalCall,
        expires: Timestamp,
    ) -> Result<
        (
            JournalRecord,
            CoordinatorEvent,
            oneshot::Receiver<ApprovalChoice>,
        ),
        CoordinatorError,
    > {
        let (sender, receiver) = oneshot::channel();
        self.run
            .install_approval(PendingApproval::new(intent.clone(), expires, sender))?;
        let record = JournalRecord::new(
            RecordContext::Run(self.run.binding().clone()),
            JournalRecordKind::Approval {
                intent: intent.id(),
                open: true,
            },
            Frame::Approval {
                intent: *intent.id().as_bytes(),
                expires: expires.as_millis(),
            }
            .bytes()?,
        )?;
        let event = CoordinatorEvent::Approval {
            intent: intent.clone(),
            call: call.clone(),
            display: ToolDisplay::new(call, None),
            expires,
        };
        Ok((record, event, receiver))
    }
    /// Reuses the original durable intent and its already consumed grant. No
    /// synthetic dispatch permit or second reservation is created on recovery.
    pub(super) async fn undispatched(
        &mut self,
        intent: ToolIntent,
        call: EngineToolCall,
        choice: Option<ApprovalChoice>,
    ) -> Result<EngineToolResult, CoordinatorError> {
        if self.state.pending_stage() != Some(EffectStage::IntentCommitted)
            || intent.authority().run() != self.run.binding()
            || intent.step() != call.slot().step()
            || intent.arguments() != self.services.arguments().fingerprint(call.call())?
            || intent.mutates() != call.call().writes()
        {
            return Err(JournalError::Corrupt.into());
        }
        let grant = self
            .run
            .grants()
            .tool(intent.authority().grant())
            .cloned()
            .ok_or(CoordinatorError::NeedsGrant)?;
        if self.needs_person(&intent, call.call()) {
            let answer = if let Some(choice) = choice {
                choice
            } else {
                let (sender, receiver) = oneshot::channel();
                self.run.install_approval(PendingApproval::new(
                    intent.clone(),
                    grant.expires(),
                    sender,
                ))?;
                self.run.arm_approval();
                self.run
                    .status(self.chat.log(), RunStatus::WaitingApproval)
                    .await?;
                match self.approval(receiver, grant.expires()).await {
                    Ok(choice) => choice,
                    Err(error) => return self.decline(&intent, &call, &error.to_string()).await,
                }
            };
            if answer != ApprovalChoice::Approve {
                return self
                    .decline(&intent, &call, "The operation was declined.")
                    .await;
            }
        }
        if let Err(error) = self.attach().await {
            return self.decline(&intent, &call, &error.to_string()).await;
        }
        self.release(intent, call, grant).await
    }
    async fn release(
        &mut self,
        intent: ToolIntent,
        call: EngineToolCall,
        mut grant: GrantState,
    ) -> Result<EngineToolResult, CoordinatorError> {
        let id = intent.id();
        let digest = intent.arguments();
        if let Err(error) = self.pre_dispatch(&intent, call.call(), &mut grant).await {
            return self.decline(&intent, &call, &error.to_string()).await;
        }
        let stamp = self.next_stamp()?;
        let record = JournalRecord::new(
            RecordContext::Transition(stamp),
            JournalRecordKind::Dispatched { intent: id },
            Frame::Dispatch {
                intent: *id.as_bytes(),
                grant: Grant::from(&grant),
            }
            .bytes()?,
        )?;
        let committed = self
            .commit(vec![(
                record,
                Some(CoordinatorEvent::Status {
                    run: self.run.binding().clone(),
                    status: RunStatus::Running,
                }),
            )])
            .await?;
        let permit = self.state.dispatch_committed(id, committed)?;
        let request = ScopedToolRequest::new(permit, call.call().clone(), digest)?;
        self.authority.allow(request.clone())?;
        let runtime = self.runtime.as_ref().ok_or(CoordinatorError::NeedsUnlock)?;
        let result = runtime.execute(request.clone()).await;
        self.observe(&intent, &call, result).await
    }
    async fn attach(&mut self) -> Result<(), CoordinatorError> {
        if self.runtime.is_some() {
            return Ok(());
        }
        let session = self
            .run
            .selected_session()
            .ok_or(CoordinatorError::NeedsUnlock)?;
        let runtime = self
            .services
            .tools()
            .attach(
                self.run.binding().clone(),
                session.clone(),
                self.authority.clone(),
            )
            .await?;
        if runtime.run() != self.run.binding() || runtime.receipt() != &session {
            return Err(CoordinatorError::Conflict);
        }
        runtime.check_current()?;
        self.run.control().install_runtime(runtime.clone())?;
        self.runtime = Some(runtime);
        Ok(())
    }
    async fn approval(
        &self,
        answer: oneshot::Receiver<ApprovalChoice>,
        expires: Timestamp,
    ) -> Result<ApprovalChoice, CoordinatorError> {
        let (abort, cancel) = AbortHandle::new_pair();
        self.run.control().install_abort(abort);
        let remaining = expires
            .elapsed_since(self.services.timer().now())
            .ok_or(CoordinatorError::Expired)?;
        let answer = Abortable::new(answer, cancel);
        tokio::select! {answer=answer=>answer.map_err(|_|CoordinatorError::Stopped)?.map_err(|_|CoordinatorError::Stopped),_=self.services.timer().delay(Duration::from_millis(remaining))=>Err(CoordinatorError::Expired)}
    }
    async fn pre_dispatch(
        &self,
        intent: &ToolIntent,
        call: &TerminalCall,
        grant: &mut GrantState,
    ) -> Result<(), CoordinatorError> {
        if self.run.selected_session().as_ref() != Some(intent.authority().session()) {
            return Err(CoordinatorError::NeedsUnlock);
        }
        self.services.tasks().check_healthy()?;
        if !self.services.tasks().running() {
            return Err(CoordinatorError::Stopped);
        }
        self.run.control().check(self.services.source().as_ref())?;
        self.chat.check()?;
        self.chat.log().require_current()?;
        let current = self
            .run
            .grants()
            .tool(grant.authority().grant())
            .cloned()
            .ok_or(CoordinatorError::NeedsGrant)?;
        *grant = current;
        grant.revalidate_reserved(intent.authority(), call, self.services.timer().now())?;
        if intent.mutates() {
            let scope = intent.authority().run().scope().clone();
            let target = intent.authority().session().target().clone();
            let pending = self
                .services
                .io()
                .run(move |journal| journal.unresolved_target(scope, target, 256))
                .await?;
            if !pending.is_empty() {
                return Err(ToolExecutionError::OutcomeUnknown.into());
            }
        }
        self.run.control().check(self.services.source().as_ref())
    }
    async fn decline(
        &mut self,
        intent: &ToolIntent,
        call: &EngineToolCall,
        message: &str,
    ) -> Result<EngineToolResult, CoordinatorError> {
        let output = BoundedJson::new(serde_json::json!({"error":message}))?;
        let mut display = ToolDisplay::new(call.call(), None);
        display.finish(Err(nocterm_ai::ToolFailure::from(message)));
        let stamp = self.next_stamp()?;
        let record = JournalRecord::new(
            RecordContext::Transition(stamp),
            JournalRecordKind::Declined {
                intent: intent.id(),
            },
            Frame::result(intent.id(), &output, display.clone(), true, false).bytes()?,
        )?;
        let observation = ToolObservation::failed(
            intent.id(),
            output.clone(),
            display,
            ToolEffectStatus::Observed,
        );
        let committed = self
            .commit(vec![(
                record,
                Some(CoordinatorEvent::ToolResult {
                    run: self.run.binding().clone(),
                    observation,
                }),
            )])
            .await?;
        self.state.decline_committed(intent.id(), committed)?;
        Ok(EngineToolResult::new(call.slot(), output, true))
    }
    async fn observe(
        &mut self,
        intent: &ToolIntent,
        call: &EngineToolCall,
        result: Result<ToolExecutionResult, ToolExecutionError>,
    ) -> Result<EngineToolResult, CoordinatorError> {
        let observation = match result {
            Ok(result) if result.intent() == intent.id() => ToolObservation::from_result(result),
            Ok(_) => return Err(CoordinatorError::Conflict),
            Err(error) => {
                let output = BoundedJson::new(serde_json::json!({"error":error.to_string()}))?;
                let mut display = ToolDisplay::new(call.call(), None);
                display.finish(Err(nocterm_ai::ToolFailure::from(error.to_string())));
                ToolObservation::failed(intent.id(), output, display, ToolEffectStatus::Unknown)
            }
        };
        let unknown = matches!(
            observation.effect(),
            nocterm_cloud_model::coordinator::ObservedEffect::Unknown
        ) && intent.mutates();
        let digest = Sha256::digest(
            serde_json::to_vec(observation.output().value()).map_err(|_| JournalError::Corrupt)?,
        );
        let outcome = if unknown {
            EffectOutcome::Unknown
        } else {
            EffectOutcome::Known(Fingerprint::from_bytes(digest.into()))
        };
        let stamp = self.next_stamp()?;
        let record = JournalRecord::new(
            RecordContext::Transition(stamp),
            JournalRecordKind::Outcome {
                intent: intent.id(),
                outcome,
            },
            Frame::result(
                intent.id(),
                observation.output(),
                observation.display().clone(),
                observation.is_failed(),
                unknown,
            )
            .bytes()?,
        )?;
        let committed = self
            .commit(vec![(
                record,
                Some(CoordinatorEvent::ToolResult {
                    run: self.run.binding().clone(),
                    observation: observation.clone(),
                }),
            )])
            .await?;
        self.state
            .outcome_committed(intent.id(), committed, outcome)?;
        if unknown {
            return Err(ToolExecutionError::OutcomeUnknown.into());
        }
        Ok(EngineToolResult::new(
            call.slot(),
            observation.output().clone(),
            observation.is_failed(),
        ))
    }
    fn next_stamp(&self) -> Result<CommitStamp, CoordinatorError> {
        Ok(CommitStamp::new(
            self.run.binding().clone(),
            self.chat.log().lease().epoch(),
            self.state.revision().next()?,
        ))
    }
    async fn commit(
        &self,
        records: Vec<(JournalRecord, Option<CoordinatorEvent>)>,
    ) -> Result<CommitStamp, CoordinatorError> {
        let expected = self.next_stamp()?;
        match self.chat.log().append(None, records, None).await? {
            CommitOutcome::Committed(receipt) => receipt
                .run_commits()
                .iter()
                .find(|stamp| **stamp == expected)
                .cloned()
                .ok_or_else(|| JournalError::Corrupt.into()),
            CommitOutcome::Duplicate(_) => Err(JournalError::Conflict.into()),
        }
    }
}
