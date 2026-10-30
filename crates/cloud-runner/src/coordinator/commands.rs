//! Authenticated control admission and durable one-winner approval publication.
use super::{
    chat::Chat, frame::Frame, queue::RunQueue, run::RunRecord, services::CoordinatorServices,
};
use nocterm_cloud_model::{
    admission::{QueuedRun, RunAccess},
    authority::RunBinding,
    coordinator::*,
    identity::*,
    journal::*,
};
use std::sync::Arc;

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
        // Current authenticated stop/revoke fencing never waits behind disk or
        // a staged provider/session operation. The immutable receipt lookup in
        // RunCoordinator precedes this admission, including substitutions.
        if command.body().high_priority()
            && command.acceptance().expected_control() == chat.log().position().control()
        {
            self.services.source().check_current()?;
            chat.log().require_current()?;
            match command.body() {
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
        }
        let command_guard = chat.commands().lock().await;
        if let Some(receipt) = chat.log().receipt(command.acceptance().clone()).await? {
            return Ok(receipt);
        }
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
        match command.body() {
            CommandBody::Submit(prompt) => self.submit(&chat, command, prompt.clone()).await,
            CommandBody::InferenceGrant(grant) => {
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
            CommandBody::ToolGrant(grant) => {
                let run = chat.run(grant.authority().run())?;
                let mut grants = run.grants();
                grants.install_tool(grant.clone())?;
                let receipt = chat.accept(command, None, vec![], &self.work).await?;
                run.set_grants(grants);
                run.wake();
                Ok(receipt)
            }
            CommandBody::Approval(answer) => {
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
            CommandBody::Stop => {
                let run = chat.run(command.run().ok_or(CoordinatorError::InvalidCommand)?)?;
                run.control().stop();
                self.queue.cancel(run.binding());
                let receipt = chat.accept(command, None, vec![], &self.work).await?;
                run.status_after_command(chat.log(), RunStatus::Stopped, &receipt)
                    .await?;
                Ok(receipt)
            }
            CommandBody::RevokeGrant(reference) => {
                let run = chat.run(command.run().ok_or(CoordinatorError::InvalidCommand)?)?;
                run.control().stop();
                self.queue.cancel(run.binding());
                let mut grants = run.grants();
                grants.revoke(*reference);
                let receipt = chat.accept(command, None, vec![], &self.work).await?;
                run.set_grants(grants);
                self.services.providers().revoke(*reference);
                run.status_after_command(chat.log(), RunStatus::Stopped, &receipt)
                    .await?;
                Ok(receipt)
            }
            CommandBody::RevokeIssuer(revoke) => {
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
            CommandBody::DeleteChat => {
                chat.delete();
                for run in chat.runs() {
                    self.queue.cancel(run.binding());
                }
                if let Ok(setup) = self.services.session_setup() {
                    setup.broker().close_chat(chat.log().lease().stream());
                }
                let record = JournalRecord::new(
                    RecordContext::Chat,
                    JournalRecordKind::Tombstone {
                        command: command.acceptance().id(),
                    },
                    Frame::Status(8).bytes()?,
                )?;
                chat.accept(
                    &command,
                    None,
                    vec![(
                        record,
                        Some(CoordinatorEvent::Deleted {
                            command: command.acceptance().id(),
                        }),
                    )],
                    &self.work,
                )
                .await
            }
            CommandBody::ProviderGrant(offer) => {
                let receipt = chat.accept(command, None, vec![], &self.work).await?;
                drop(command_guard);
                if !self.services.tasks().running() {
                    return Err(CoordinatorError::Stopped);
                }
                self.services
                    .providers()
                    .install(command.acknowledgement(), offer.clone())
                    .await?;
                self.services.source().check_current()?;
                for run in chat.runs() {
                    if run.context().selection().provider() == offer.provider() {
                        run.wake();
                    }
                }
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
