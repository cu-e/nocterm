//! Authenticated control admission and durable one-winner approval publication.
use super::{
    RunCoordinator, chat::Chat, frame::Frame, queue::RunQueue, run::RunRecord,
    services::CoordinatorServices,
};
use nocterm_cloud_model::{
    admission::{QueuedRun, RunAccess},
    authority::RunBinding,
    coordinator::*,
    grant::*,
    identity::*,
    inference::*,
    journal::*,
};
use std::sync::Arc;

/// Marks every run of `chat` that selected `provider` for resumption; the serve
/// loop resumes those that are parked waiting for access.
fn wake_provider_runs(chat: &Chat, provider: &NamespaceId) {
    for run in chat.runs() {
        if run.context().selection().provider() == provider {
            run.wake();
        }
    }
}

impl RunCoordinator {
    /// A provider became usable for `chat` without a command: the host handed the
    /// runner a token. Wakes the chat's runs that selected `provider`, like a
    /// provider grant does. A chat the coordinator has not opened has nothing to wake.
    pub fn wake_provider(&self, chat: CloudChatId, provider: &NamespaceId) {
        let slots = self
            .chats
            .lock()
            .iter()
            .filter(|(stream, _)| stream.chat() == chat)
            .map(|(_, slot)| slot.clone())
            .collect::<Vec<_>>();
        for slot in slots {
            if let Some(chat) = slot.get() {
                wake_provider_runs(chat, provider);
            }
        }
    }
}

pub(super) struct CommandHandler {
    services: Arc<CoordinatorServices>,
    queue: Arc<RunQueue>,
    work: super::intake::PublicationWork,
}
impl CommandHandler {
    pub(super) fn new(
        services: Arc<CoordinatorServices>,
        queue: Arc<RunQueue>,
        work: super::intake::PublicationWork,
    ) -> Self {
        Self {
            services,
            queue,
            work,
        }
    }
    pub(super) async fn apply(
        &self,
        chat: Arc<Chat>,
        command: &ReceivedCommand,
    ) -> Result<CommandReceipt, CoordinatorError> {
        self.fence_urgent(&chat, command)?;
        let command_guard = chat.commands().lock().await;
        if let Some(receipt) = chat.log().receipt(command.acceptance().clone()).await? {
            return Ok(receipt);
        }
        let now = self.admit(&chat, command).await?;
        match command.body() {
            CommandBody::Submit(prompt) => self.submit(&chat, command, prompt.clone()).await,
            CommandBody::InferenceGrant(grant) => self.inference_grant(&chat, command, grant).await,
            CommandBody::ToolGrant(grant) => self.tool_grant(&chat, command, grant).await,
            CommandBody::Approval(answer) => self.approval(&chat, command, answer, now).await,
            CommandBody::Stop => self.stop(&chat, command).await,
            CommandBody::SshBinding(_) | CommandBody::SshCertificate(_) => {
                self.ssh(&chat, command, now).await
            }
            CommandBody::RevokeGrant(reference) => {
                self.revoke_grant(&chat, command, *reference, now).await
            }
            CommandBody::RevokeIssuer(revoke) => self.revoke_issuer(&chat, command, revoke).await,
            CommandBody::DeleteChat => self.delete_chat(&chat, command).await,
            CommandBody::ProviderGrant(offer) => {
                let receipt = chat.accept(command, None, vec![], &self.work).await?;
                drop(command_guard);
                self.install_provider(&chat, command, offer).await?;
                Ok(receipt)
            }
            CommandBody::SessionConsent(answer) => {
                if let Ok(setup) = self.services.session_setup() {
                    setup
                        .broker()
                        .check_answer(&command.acknowledgement(), answer)?;
                }
                let receipt = chat.accept(command, None, vec![], &self.work).await?;
                drop(command_guard);
                if !self.services.tasks().running() {
                    return Err(CoordinatorError::Stopped);
                }
                self.services
                    .sessions()
                    .answer(command.acknowledgement(), answer.clone())
                    .await?;
                Ok(receipt)
            }
            CommandBody::BeginSession(request) => {
                super::session_commands::SessionCommands::new(self.services.clone())
                    .begin(&chat, command, request, &self.work)
                    .await
            }
        }
    }
    /// Current authenticated stop/revoke fencing never waits behind disk or
    /// a staged provider/session operation. The immutable receipt lookup in
    /// RunCoordinator precedes this admission, including substitutions.
    fn fence_urgent(&self, chat: &Chat, command: &ReceivedCommand) -> Result<(), CoordinatorError> {
        if !command.body().high_priority()
            || command.acceptance().expected_control() != chat.log().position().control()
        {
            return Ok(());
        }
        self.services.source().check_current()?;
        chat.log().require_current()?;
        match command.body() {
            CommandBody::RevokeGrant(reference) if reference.class() == GrantClass::SshBinding => {}
            CommandBody::Stop | CommandBody::RevokeGrant(_) => {
                let run = chat.run(command.run().ok_or(CoordinatorError::InvalidCommand)?)?;
                run.control().stop();
                run.sessions().cancel_pending();
            }
            CommandBody::RevokeIssuer(_) | CommandBody::DeleteChat => {
                for run in chat.runs() {
                    run.control().stop();
                    run.sessions().cancel_pending();
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// Synchronizes the chat and checks the command is current and unexpired;
    /// returns the admission time.
    async fn admit(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
    ) -> Result<Timestamp, CoordinatorError> {
        chat.log().synchronize().await?;
        self.services.source().check_current()?;
        chat.check()?;
        chat.log().require_current()?;
        if command.acceptance().expected_control() != chat.log().position().control() {
            return Err(CoordinatorError::Conflict);
        }
        let now = self.services.timer().now();
        if now < command.window().issued()
            || now >= command.window().expires()
            || command.hard_expires().is_some_and(|expires| now >= expires)
        {
            return Err(CoordinatorError::Expired);
        }
        Ok(now)
    }
    async fn inference_grant(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
        grant: &InferenceGrant,
    ) -> Result<CommandReceipt, CoordinatorError> {
        let run = chat.run(grant.allowance().run())?;
        let mut grants = run.grants();
        grants.install_inference(grant.clone())?;
        let receipt = chat.accept(command, None, vec![], &self.work).await?;
        run.set_grants(grants);
        run.wake();
        self.queue
            .activate(run.binding(), self.services.timer().now())?;
        Ok(receipt)
    }
    async fn tool_grant(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
        grant: &GrantPolicy,
    ) -> Result<CommandReceipt, CoordinatorError> {
        let run = chat.run(grant.authority().run())?;
        let mut grants = run.grants();
        grants.install_tool(grant.clone())?;
        let receipt = chat.accept(command, None, vec![], &self.work).await?;
        run.set_grants(grants);
        run.wake();
        Ok(receipt)
    }
    async fn approval(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
        answer: &ApprovalAnswer,
        now: Timestamp,
    ) -> Result<CommandReceipt, CoordinatorError> {
        if matches!(answer.choice(), ApprovalChoice::ReconcileKnown(_)) {
            return Err(CoordinatorError::Unsupported);
        }
        let run = chat.run(answer.authority().run())?;
        run.check_approval(answer, now)?;
        let record = JournalRecord::new(
            RecordContext::Run(run.binding().clone()),
            JournalRecordKind::Approval {
                intent: answer.intent(),
                open: false,
            },
            Frame::Approval {
                intent: *answer.intent().as_bytes(),
                expires: now.as_millis(),
            }
            .bytes()?,
        )?;
        let event = CoordinatorEvent::ApprovalResolved {
            command: command.acceptance().id(),
            answer: answer.clone(),
        };
        let receipt = chat
            .accept(command, None, vec![(record, Some(event))], &self.work)
            .await?;
        run.resolve_approval(answer)?;
        Ok(receipt)
    }
    async fn stop(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
    ) -> Result<CommandReceipt, CoordinatorError> {
        let run = chat.run(command.run().ok_or(CoordinatorError::InvalidCommand)?)?;
        run.control().stop();
        self.queue.cancel(run.binding());
        let receipt = chat.accept(command, None, vec![], &self.work).await?;
        run.status_after_command(chat.log(), RunStatus::Stopped, &receipt)
            .await?;
        Ok(receipt)
    }
    async fn revoke_grant(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
        reference: GrantReference,
        now: Timestamp,
    ) -> Result<CommandReceipt, CoordinatorError> {
        // A chat's SSH binding is revoked for the whole chat (ADR 0145).
        if reference.class() == GrantClass::SshBinding {
            return self.ssh(chat, command, now).await;
        }
        let run = chat.run(command.run().ok_or(CoordinatorError::InvalidCommand)?)?;
        run.control().stop();
        self.queue.cancel(run.binding());
        let mut grants = run.grants();
        grants.revoke(reference);
        let receipt = chat.accept(command, None, vec![], &self.work).await?;
        run.set_grants(grants);
        self.services.providers().revoke(reference);
        run.status_after_command(chat.log(), RunStatus::Stopped, &receipt)
            .await?;
        Ok(receipt)
    }
    async fn revoke_issuer(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
        revoke: &IssuerRevocation,
    ) -> Result<CommandReceipt, CoordinatorError> {
        let runs = chat.runs();
        for run in &runs {
            run.control().stop();
            self.queue.cancel(run.binding());
        }
        let receipt = chat.accept(command, None, vec![], &self.work).await?;
        for run in runs {
            let mut grants = run.grants();
            grants.revoke_issuer(revoke);
            run.set_grants(grants);
            run.status_after_command(chat.log(), RunStatus::Stopped, &receipt)
                .await?;
        }
        for reference in revoke.grants() {
            self.services.providers().revoke(*reference);
        }
        Ok(receipt)
    }
    async fn delete_chat(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
    ) -> Result<CommandReceipt, CoordinatorError> {
        chat.delete();
        for run in chat.runs() {
            self.queue.cancel(run.binding());
        }
        if let Ok(setup) = self.services.session_setup() {
            setup.broker().close_chat(chat.log().lease().stream());
        }
        // A deleted chat keeps no SSH key; the row stays only to refuse replays.
        if let Some(ssh) = self.services.ssh() {
            let transition = ssh
                .prepare(&self.services, command, self.services.timer().now())
                .await?;
            ssh.settle(&self.services, transition, true).await?;
        }
        let record = JournalRecord::new(
            RecordContext::Chat,
            JournalRecordKind::Tombstone {
                command: command.acceptance().id(),
            },
            Frame::Status(8).bytes()?,
        )?;
        let receipt = chat
            .accept(
                command,
                None,
                vec![(
                    record,
                    Some(CoordinatorEvent::Deleted {
                        command: command.acceptance().id(),
                    }),
                )],
                &self.work,
            )
            .await?;
        // Only a deletion that is durable in the journal is announced to the hub.
        if let Some(retirement) = self.services.retirement() {
            retirement.retire(ChatRetirementRequest::new(
                chat.log().lease().stream().clone(),
                command.acceptance().issuer().device().clone(),
            ));
        }
        Ok(receipt)
    }
    /// A chat SSH binding, certificate or unbind (ADR 0145): the row is stored first, then
    /// the command and its events are journaled together.
    async fn ssh(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
        now: Timestamp,
    ) -> Result<CommandReceipt, CoordinatorError> {
        let ssh = self.services.ssh().ok_or(CoordinatorError::Unsupported)?;
        let transition = ssh.prepare(&self.services, command, now).await?;
        let records = transition.records()?;
        let accepted = chat.accept(command, None, records, &self.work).await;
        ssh.settle(&self.services, transition, accepted.is_ok())
            .await?;
        accepted
    }
    /// Runs after the command lock is released: provider installation may wait
    /// on external work that must not hold the chat's command order.
    async fn install_provider(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
        offer: &ProviderOffer,
    ) -> Result<(), CoordinatorError> {
        if !self.services.tasks().running() {
            return Err(CoordinatorError::Stopped);
        }
        self.services
            .providers()
            .install(command.acknowledgement(), offer.clone())
            .await?;
        self.services.source().check_current()?;
        wake_provider_runs(chat, offer.provider());
        Ok(())
    }
    async fn submit(
        &self,
        chat: &Chat,
        command: &ReceivedCommand,
        prompt: PromptRequest,
    ) -> Result<CommandReceipt, CoordinatorError> {
        let stream = command.stream();
        let binding = RunBinding::new(
            stream.scope().clone(),
            stream.runner(),
            stream.runner_epoch(),
            stream.chat(),
            RunId::from_bytes(self.services.identity().fresh()?),
            RunEpoch::new(1)?,
        );
        let run = Arc::new(RunRecord::new(binding.clone(), prompt.clone())?);
        let reservation = self.queue.prepare(
            QueuedRun::new(
                binding.clone(),
                prompt.destination().target().clone(),
                RunAccess::Mutating,
                prompt.allowance().window().expires(),
            ),
            self.services.timer().now(),
        )?;
        let record = JournalRecord::new(
            RecordContext::Chat,
            JournalRecordKind::QueueChanged {
                command: command.acceptance().id(),
            },
            Frame::Status(0).bytes()?,
        )?;
        let receipt = chat
            .accept(
                command,
                Some(&binding),
                vec![(
                    record,
                    Some(CoordinatorEvent::Status {
                        run: binding.clone(),
                        status: RunStatus::Queued,
                    }),
                )],
                &self.work,
            )
            .await?;
        self.services.register(&run)?;
        chat.insert(run)?;
        reservation.admitted();
        Ok(receipt)
    }
}
