//! Lazy per-chat retained owners and tracked original-run login work.
use super::{
    login::SessionProofExchange,
    publication::{CancelPending, PendingConnection},
    resources::{ChatCapacity, SessionResources},
    setup::SessionBrokerSetup,
    targets::ConfiguredSessionTargets,
    task::{SessionAttemptWork, SessionTask},
};
use nocterm_agent_tools::headless::{RetainedTerminals, RetainedToolsFactory};
use nocterm_cloud_model::{
    authority::{RunBinding, SessionReceipt, StreamBinding},
    coordinator::*,
    tool::{RunToolRuntime, RunToolRuntimeFactory, ToolAuthority, ToolExecutionError},
};
use nocterm_core::{SharedCause, allocation::Allocated, persist::ShutdownDeadline};
use nocterm_transfer_runtime::TransferRuntime;
use nocterm_transfers::{TransferRequests, TransferService};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

struct ChatSessions {
    retained: RetainedTerminals,
    tools: RetainedToolsFactory,
    capacity: ChatCapacity,
}
impl ChatSessions {
    /// Ends the chat's retained terminals and cancels the transfers it started.
    fn close(&self) {
        self.tools.cancel_transfers();
        self.retained.close();
    }
}
#[derive(Default)]
struct Registry {
    chats: BTreeMap<StreamBinding, Arc<ChatSessions>>,
    proofs: BTreeMap<RunBinding, Weak<SessionProofExchange>>,
    closed: bool,
}
pub(crate) struct NativeSessionBroker {
    setup: Arc<SessionBrokerSetup>,
    targets: Arc<ConfiguredSessionTargets>,
    resources: SessionResources,
    transfers: Arc<TransferRuntime>,
    registry: Mutex<Registry>,
}
impl NativeSessionBroker {
    pub(crate) fn new(
        setup: SessionBrokerSetup,
        targets: Arc<ConfiguredSessionTargets>,
        resources: SessionResources,
        transfers: Arc<TransferRuntime>,
    ) -> Result<Arc<Self>, CoordinatorError> {
        if setup.connection().binding() != targets.binding()
            || setup.source().binding() != targets.binding()
        {
            return Err(CoordinatorError::StaleSource);
        }
        Ok(Arc::new(Self {
            setup: Arc::new(setup),
            targets,
            resources,
            transfers,
            registry: Mutex::new(Registry::default()),
        }))
    }
    fn chat(&self, run: &RunBinding) -> Result<Arc<ChatSessions>, CoordinatorError> {
        self.setup.source().check_current()?;
        if !self.targets.binding().contains(&run.stream()) {
            return Err(CoordinatorError::StaleSource);
        }
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        if registry.closed {
            return Err(CoordinatorError::Stopped);
        }
        if let Some(chat) = registry.chats.get(&run.stream()) {
            return Ok(chat.clone());
        }
        if registry.chats.len() >= 128 {
            return Err(CoordinatorError::Overloaded);
        }
        let retained = RetainedTerminals::new(
            run,
            self.setup.authority().clone(),
            self.setup.cleanup().clone(),
        );
        let tools = RetainedToolsFactory::existing_only(
            retained.clone(),
            self.setup.timer().clone(),
            self.setup.journal().clone(),
            self.setup.tasks().clone(),
            self.setup.handles(),
        )
        .with_transfers(self.transfers.clone() as Arc<dyn TransferRequests>);
        let chat = Arc::new(ChatSessions {
            retained,
            tools,
            capacity: ChatCapacity::new(),
        });
        registry.chats.insert(run.stream(), chat.clone());
        Ok(chat)
    }
    fn proof(&self, run: &RunBinding) -> Result<Arc<SessionProofExchange>, CoordinatorError> {
        self.registry
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?
            .proofs
            .get(run)
            .and_then(Weak::upgrade)
            .ok_or(CoordinatorError::Conflict)
    }
    pub(crate) fn close(&self) {
        let chats = match self.registry.lock() {
            Ok(mut registry) => {
                registry.closed = true;
                registry.proofs.clear();
                std::mem::take(&mut registry.chats)
            }
            Err(_) => return,
        };
        for chat in chats.into_values() {
            chat.close();
        }
    }
    pub(crate) async fn drain(&self, deadline: ShutdownDeadline) -> bool {
        self.close();
        let transfers = self.transfers.freeze_and_drain();
        let transfers = matches!(
            tokio::time::timeout(deadline.remaining(), transfers).await,
            Ok(Ok(()))
        );
        self.resources.drain(deadline).await && transfers
    }
}
impl RunSessionBroker for NativeSessionBroker {
    fn check_answer(
        &self,
        command: &CommandAcknowledgement,
        answer: &SessionConsent,
    ) -> Result<(), CoordinatorError> {
        self.proof(answer.run())?.check_answer(command, answer)
    }
    fn begin(
        &self,
        command: CommandAcknowledgement,
        request: Arc<Allocated<SessionRequest>>,
        facts: Arc<dyn SessionFactSink>,
    ) -> Result<Box<dyn SessionAttemptCancellation>, CoordinatorError> {
        self.setup.source().check_current()?;
        facts.check_current()?;
        let chat = self.chat(request.value().binding().run())?;
        let admission = self.resources.reserve(&chat.capacity)?;
        let proof = if request.value().ssh_consent().is_some() {
            Some(Arc::new(SessionProofExchange::new(
                command,
                request.clone(),
                facts.clone(),
                self.setup.attachments().clone(),
                self.setup.timer().clone(),
                self.setup.work().clone(),
            )?))
        } else {
            None
        };
        if let Some(proof) = &proof {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| CoordinatorError::NativeTaskFailed)?;
            registry.proofs.retain(|_, value| value.strong_count() > 0);
            if registry.closed || registry.proofs.len() >= 256 {
                return Err(CoordinatorError::Overloaded);
            }
            registry.proofs.insert(
                request.value().binding().run().clone(),
                Arc::downgrade(proof),
            );
        }
        let pending = PendingConnection::new();
        let task = SessionTask::new(
            self.setup.clone(),
            self.targets.clone(),
            chat.retained.clone(),
            SessionAttemptWork::new(request, facts, proof, pending.clone(), admission),
        );
        self.setup.tasks().spawn(Box::pin(async move {
            task.run()
                .await
                .map_err(|error| ToolExecutionError::Execution(SharedCause::new(error)))
        }))?;
        Ok(Box::new(CancelPending::new(pending)))
    }
    fn close_chat(&self, stream: &StreamBinding) {
        let chat = self.registry.lock().ok().and_then(|mut registry| {
            registry.proofs.retain(|run, _| &run.stream() != stream);
            registry.chats.remove(stream)
        });
        if let Some(chat) = chat {
            chat.close();
        }
    }
}
impl SessionConsentConsumer for NativeSessionBroker {
    fn answer(
        &self,
        command: CommandAcknowledgement,
        answer: SessionConsent,
    ) -> futures::future::BoxFuture<'static, Result<Option<SessionReceipt>, CoordinatorError>> {
        let result = self
            .proof(answer.run())
            .and_then(|proof| proof.deliver(&command, &answer))
            .map(|_| None);
        Box::pin(async move { result })
    }
}
impl RunToolRuntimeFactory for NativeSessionBroker {
    fn attach(
        &self,
        run: RunBinding,
        receipt: SessionReceipt,
        authority: Arc<dyn ToolAuthority>,
    ) -> futures::future::BoxFuture<'static, Result<Arc<dyn RunToolRuntime>, ToolExecutionError>>
    {
        match self.chat(&run) {
            Ok(chat) => chat.tools.attach(run, receipt, authority),
            Err(error) => {
                Box::pin(async move { Err(ToolExecutionError::Execution(SharedCause::new(error))) })
            }
        }
    }
}
