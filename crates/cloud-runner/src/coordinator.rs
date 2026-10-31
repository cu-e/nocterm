//! Native host actor, bounded command intake and explicit current authority.
mod authority;
mod chat;
mod commands;
mod control;
mod effects;
mod facades;
mod format;
mod frame;
mod history;
mod intake;
mod io;
mod lifecycle;
mod log;
mod model;
mod publication;
mod queue;
pub use publication::CoordinatorPublication;
mod reconciliation;
mod recovery;
mod replay;
mod run;
mod run_context;
mod run_session;
mod services;
mod session_commands;
pub(crate) mod session_fact;
mod session_publication;
mod session_request;
mod session_services;
pub use session_services::CoordinatorSessions;
mod supervisor;
mod worker;

use chat::Chat;
use commands::CommandHandler;
use futures::{
    FutureExt, StreamExt,
    future::{AbortRegistration, Abortable, BoxFuture},
    stream::FuturesUnordered,
};
use nocterm_cloud_model::{
    admission::AdmissionLimits, authority::StreamBinding, coordinator::*, identity::*, journal::*,
};
use parking_lot::Mutex;
use queue::RunQueue;
pub use services::{CoordinatorExecution, CoordinatorServices, CoordinatorStorage};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::OnceCell;

type ChatSlot = Arc<OnceCell<Arc<Chat>>>;
#[derive(Clone)]
pub struct RunCoordinator {
    services: Arc<CoordinatorServices>,
    queue: Arc<RunQueue>,
    chats: Arc<Mutex<BTreeMap<StreamBinding, ChatSlot>>>,
    intake: intake::Intake,
}
impl RunCoordinator {
    pub fn new(
        services: CoordinatorServices,
        limits: AdmissionLimits,
    ) -> Result<Self, CoordinatorError> {
        let budget = services.publication_work().receive();
        Self::with_receive_budget(services, limits, budget)
    }
    /// The producer receives this same finite owner. No idle worst-case payload
    /// reservation is made until a record's actual advertised length is known.
    pub fn with_receive_budget(
        services: CoordinatorServices,
        limits: AdmissionLimits,
        budget: CommandReceiveBudget,
    ) -> Result<Self, CoordinatorError> {
        services.source().check_current()?;
        if !budget.same_owner(&services.publication_work().receive()) {
            return Err(CoordinatorError::InvalidCommand);
        }
        if services.tasks().workers() < limits.active_global() {
            return Err(CoordinatorError::InvalidCommand);
        }
        let id = SchedulerId::from_bytes(services.identity().fresh()?);
        Ok(Self {
            services: Arc::new(services),
            queue: Arc::new(RunQueue::new(id, limits)),
            chats: Arc::new(Mutex::new(BTreeMap::new())),
            intake: intake::Intake::new(budget),
        })
    }
    /// Accepts only a typed command from the configured trusted producer's scope.
    /// The producer remains responsible for cryptographic verification, never
    /// this constructor or receipt comparison.
    pub async fn accept(
        &self,
        command: ReceivedCommand,
    ) -> Result<CommandReceipt, CoordinatorError> {
        self.accept_resident(self.intake.trusted(command)?).await
    }
    /// Production receive transfers the exact charged generation intact. The
    /// tracked transition retains it even if its caller stops awaiting a reply.
    pub async fn accept_resident(
        &self,
        command: ResidentCommand,
    ) -> Result<CommandReceipt, CoordinatorError> {
        intake::Intake::require_resident(&command)?;
        let class = if command.value().body().high_priority() {
            supervisor::TaskClass::Urgent
        } else {
            supervisor::TaskClass::Command
        };
        if class == supervisor::TaskClass::Command {
            self.services.tasks().check_healthy()?;
        }
        let this = self.clone();
        self.services
            .tasks()
            .submit(
                class,
                async move { this.accept_tracked(command).await }.boxed(),
            )?
            .await
    }
    async fn accept_tracked(
        &self,
        resident: ResidentCommand,
    ) -> Result<CommandReceipt, CoordinatorError> {
        let command = resident.value();
        self.services.source().check_current()?;
        if !self.services.source().binding().contains(command.stream()) {
            return Err(CoordinatorError::StaleSource);
        }
        let stream = command.stream().clone();
        let accepted = command.acceptance().clone();
        let original = self
            .services
            .io()
            .run(move |journal| journal.command_receipt(stream, accepted.id()))
            .await?;
        if let Some(receipt) = original {
            classify_command(
                command.stream(),
                Some(&receipt),
                command.acceptance(),
                receipt.position().control(),
            )?;
            return Ok(receipt);
        }
        let work = self.intake.publication(&resident)?;
        if matches!(command.body(),CommandBody::Approval(answer) if matches!(answer.choice(),ApprovalChoice::ReconcileKnown(_)))
        {
            let stream = command.stream().clone();
            let lookup = stream.clone();
            let head = self
                .services
                .io()
                .run(move |journal| journal.head(lookup))
                .await?
                .ok_or(JournalError::NotFound)?;
            let chat = if head.deleted() {
                None
            } else {
                Some(self.chat(stream).await?)
            };
            let original_run = command.run().cloned();
            let receipt = reconciliation::accept(&self.services, command, &work).await?;
            if let (Some(chat), Some(binding)) = (chat, original_run) {
                let run = chat.run(&binding)?;
                // The durable declaration is a separate fact, never a missing
                // ToolResult or a synthetic successful continuation.
                run.control().fence();
                run.state().lock().await.set_status(RunStatus::Interrupted);
            }
            return Ok(receipt);
        }
        let chat = self.chat(command.stream().clone()).await?;
        let ack = command.acknowledgement();
        let result = CommandHandler::new(self.services.clone(), self.queue.clone(), work)
            .apply(chat, command)
            .await;
        if result.is_err() {
            // Admission may have committed just before a competing retry or a
            // later status/provider failure. Recover its immutable receipt; do
            // not replace an already durable acceptance with an execution error.
            let stream = ack.stream().clone();
            let id = ack.acceptance().id();
            if let Ok(Some(receipt)) = self
                .services
                .io()
                .run(move |journal| journal.command_receipt(stream, id))
                .await
            {
                classify_command(
                    ack.stream(),
                    Some(&receipt),
                    ack.acceptance(),
                    receipt.position().control(),
                )?;
                return Ok(receipt);
            }
        }
        result
    }
    async fn chat(&self, stream: StreamBinding) -> Result<Arc<Chat>, CoordinatorError> {
        self.services.source().check_current()?;
        let slot = {
            let mut chats = self.chats.lock();
            if !chats.contains_key(&stream) && chats.len() >= 4096 {
                return Err(CoordinatorError::Overloaded);
            }
            chats
                .entry(stream.clone())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };
        slot.get_or_try_init(|| async {
            let log = log::ChatLog::claim(
                self.services.io().clone(),
                stream,
                self.services.encoder().clone(),
                self.services.publication_work().clone(),
            )
            .await?;
            let chat = Arc::new(Chat::new(log));
            recovery::restore(&self.services, &self.queue, &chat).await?;
            Ok::<_, CoordinatorError>(chat)
        })
        .await
        .cloned()
    }
    /// Replays the authenticated known catalog with a revision-bound cursor.
    /// A concurrent first chat claim returns Conflict so the caller restarts;
    /// it never silently skips a newly admitted chat.
    pub async fn recover(&self) -> Result<(), CoordinatorError> {
        let mut cursor = None;
        loop {
            let at = cursor.clone();
            let page = self
                .services
                .io()
                .run(move |journal| journal.catalog(at, 64))
                .await?;
            if page.binding() != self.services.source().binding() {
                return Err(CoordinatorError::StaleSource);
            }
            for id in page.chats() {
                let stream = page.binding().stream(*id);
                let lookup = stream.clone();
                let head = self
                    .services
                    .io()
                    .run(move |journal| journal.head(lookup))
                    .await?
                    .ok_or(JournalError::Corrupt)?;
                if head.deleted() {
                    if self.services.io().owned_writer(&stream).as_ref() == Some(head.lease()) {
                        continue;
                    }
                    // Tombstones remain in the authenticated catalog. Startup
                    // acquires only fact-only cleanup authority, never a live chat.
                    let claim = WriterClaim::new(
                        stream,
                        Some(head.lease().epoch()),
                        head.lease().epoch().next()?,
                    )?;
                    let disk = self.services.io().acquire().await?;
                    let cleanup = self
                        .services
                        .io()
                        .cleanup_locked(&disk, move |cleanup| cleanup.claim_cleanup(claim))
                        .await?;
                    self.services
                        .io()
                        .remember_writer(cleanup.writer().clone())?;
                } else {
                    self.chat(stream).await?;
                }
            }
            cursor = page.next_cursor();
            if cursor.is_none() {
                return Ok(());
            }
        }
    }
    pub async fn status(
        &self,
        binding: &nocterm_cloud_model::authority::RunBinding,
    ) -> Result<RunStatus, CoordinatorError> {
        let chat = self.chat(binding.stream()).await?;
        let run = chat.run(binding)?;
        let status = run.state().lock().await.status();
        Ok(status)
    }
    /// Stage a real signed baseline, deliver FIFO, then activate its covered native prefix.
    pub async fn compact(
        &self,
        stream: StreamBinding,
    ) -> Result<ResidentSnapshot, CoordinatorError> {
        self.chat(stream.clone()).await?;
        publication::start(self.services.clone(), stream, true)
            .await?
            .ok_or_else(|| JournalError::Corrupt.into())
    }
    /// Tracked finite delivery batch; retry always reloads the same durable IDs.
    pub async fn flush_outbox(&self, stream: StreamBinding) -> Result<(), CoordinatorError> {
        publication::start(self.services.clone(), stream, false)
            .await
            .map(|_| ())
    }
    pub fn fence(&self) {
        self.services.tasks().begin_drain();
        for binding in self.services.stop_runs() {
            self.queue.cancel(&binding);
        }
    }
    /// Stops fresh effects and waits for actual admitted work within one deadline.
    /// A missing native factory close hook leaves its tracked readers outstanding;
    /// that is a Deadline report, never a fabricated observed exit or clean drain.
    pub async fn shutdown(
        &self,
        deadline: nocterm_core::persist::ShutdownDeadline,
    ) -> Result<DrainReport, CoordinatorError> {
        self.fence();
        let tasks = self.services.tasks().drain(deadline).await;
        if tasks.outstanding > 0 {
            return Ok(DrainReport::Deadline {
                outstanding_tasks: tasks.outstanding,
                in_flight_writes: self.services.io().in_flight_writes(),
                failed_native_tasks: tasks.failed_native_tasks,
            });
        }
        if !self.services.io().freeze_and_drain(deadline).await {
            return Ok(DrainReport::Deadline {
                outstanding_tasks: 0,
                in_flight_writes: self.services.io().in_flight_writes(),
                failed_native_tasks: tasks.failed_native_tasks,
            });
        }
        if tasks.failed_native_tasks > 0 {
            Ok(DrainReport::Failed {
                failed_native_tasks: tasks.failed_native_tasks,
            })
        } else {
            Ok(DrainReport::Drained)
        }
    }
    /// A full ordinary queue returns a bounded refusal; reception continues so
    /// Stop/Revoke in the ordered encrypted inbox can reach reserved urgent slots.
    /// Engine/model futures are never awaited under a journal or actor mutex.
    pub async fn serve(&self, cancellation: AbortRegistration) -> Result<(), CoordinatorError> {
        let _serving = lifecycle::ServingLease::new(self.services.clone(), self.queue.clone());
        let result = Abortable::new(self.serve_inner(), cancellation).await;
        result.map_err(|_| CoordinatorError::Stopped)?
    }
    async fn serve_inner(&self) -> Result<(), CoordinatorError> {
        type Admitted = (
            CommandAcknowledgement,
            CommandReply,
            nocterm_core::allocation::AllocationLease,
        );
        let mut ordinary = FuturesUnordered::<BoxFuture<'static, Admitted>>::new();
        let mut urgent = FuturesUnordered::<BoxFuture<'static, Admitted>>::new();
        let mut deliveries = FuturesUnordered::<BoxFuture<'static, ()>>::new();
        let mut workers =
            FuturesUnordered::<BoxFuture<'static, Result<(), CoordinatorError>>>::new();
        let mut receiving = self.services.source().receive(self.intake.budget());
        let mut tick = self.services.timer().delay(Duration::from_millis(50));
        loop {
            tokio::select! {
                biased;
                admitted=urgent.next(),if !urgent.is_empty()=> {
                    if let Some((command,reply,allocation))=admitted {
                        deliveries.push(intake::deliver(self.services.source().clone(),command,reply,allocation));
                    }
                },
                _=&mut tick=> {
                    self.services.source().check_current()?;
                    if !self.services.tasks().running() { return Err(CoordinatorError::Stopped); }
                    self.expire();
                    tick=self.services.timer().delay(Duration::from_millis(50));
                },
                result=workers.next(),if !workers.is_empty()=> {if let Some(Err(error))=result&& matches!(error,CoordinatorError::Journal(_)) {return Err(error);}},
                admitted=ordinary.next(),if !ordinary.is_empty()=> {
                    if let Some((command,reply,allocation))=admitted {
                        deliveries.push(intake::deliver(self.services.source().clone(),command,reply,allocation));
                    }
                },
                _=deliveries.next(),if !deliveries.is_empty()=> {},
                next=&mut receiving=> {
                    let Some(input)=next? else {return Ok(());};
                    match input {
                        CommandInput::ChatOpened{stream} => {
                            // Claims the chat's writer, head and catalog row; the
                            // memoized slot makes a repeated grant a no-op.
                            let this=self.clone();
                            workers.push(async move { this.chat(stream).await.map(|_| ()) }.boxed());
                        },
                        CommandInput::Deferred{command,reason,allocation} => {
                            deliveries.push(intake::deliver(self.services.source().clone(),*command,CommandReply::RetryLater(reason),allocation));
                        },
                        CommandInput::Ready(command) => {
                            // ACK reservation precedes admission and is independent
                            // from the full body. A missing ordinary slot leaves
                            // the original unacked and cannot consume urgent slots.
                            if let Ok(allocation)=self.intake.acknowledge(command.value()) {
                                let ack=command.value().acknowledgement();
                                let high=command.value().body().high_priority();
                                let this=self.clone();
                                let task=async move {
                                    let result=this.accept_resident(command).await;
                                    (ack,intake::reply(result),allocation)
                                }.boxed();
                                if high {urgent.push(task);} else {ordinary.push(task);}
                            }
                        },
                    }
                    receiving=self.services.source().receive(self.intake.budget());
                },
            }
            self.start_workers(&mut workers).await?;
        }
    }
    fn expire(&self) {
        for lease in self.queue.expired_active(self.services.timer().now()) {
            if let Some(slot) = self
                .chats
                .lock()
                .get(&lease.request().binding().stream())
                .cloned()
                && let Some(chat) = slot.get()
                && let Ok(run) = chat.run(lease.request().binding())
            {
                run.control().stop();
            }
        }
    }
    async fn resume_waiting(&self) -> Result<(), CoordinatorError> {
        let chats = self
            .chats
            .lock()
            .values()
            .filter_map(|slot| slot.get().cloned())
            .collect::<Vec<_>>();
        for chat in chats {
            let runs = chat
                .runs()
                .into_iter()
                .filter(|run| run.wants_resume() && run.control().can_resume())
                .collect::<Vec<_>>();
            if runs.is_empty() || chat.check().is_err() {
                continue;
            }
            let _commands = chat.commands().lock().await;
            chat.log().synchronize().await?;
            for run in runs {
                let status = run.state().lock().await.status();
                if !run.wants_resume()
                    || !matches!(
                        status,
                        RunStatus::NeedsGrant
                            | RunStatus::NeedsUnlock
                            | RunStatus::NeedsProviderAccess
                    )
                {
                    continue;
                }
                if !recovery::resume(&self.services, &chat, &run).await? {
                    run.resumed();
                    continue;
                }
                let now = self.services.timer().now();
                let request = nocterm_cloud_model::admission::QueuedRun::new(
                    run.binding().clone(),
                    run.context().destination().target().clone(),
                    nocterm_cloud_model::admission::RunAccess::Mutating,
                    run.context().allowance().window().expires(),
                );
                // An old foreground may still be retiring. Keep the wake bit
                // until its reservation actually releases; no second worker.
                let Ok(held) = self.queue.prepare(request, now) else {
                    continue;
                };
                held.admitted();
                self.queue.activate(run.binding(), now)?;
                run.resumed();
            }
        }
        Ok(())
    }
    async fn start_workers(
        &self,
        workers: &mut FuturesUnordered<BoxFuture<'static, Result<(), CoordinatorError>>>,
    ) -> Result<(), CoordinatorError> {
        if !self.services.tasks().running() {
            return Ok(());
        }
        self.services.tasks().check_healthy()?;
        self.resume_waiting().await?;
        loop {
            let decision = self.queue.reserve(self.services.timer().now())?;
            for expired in decision.expired {
                let chat = self.chat(expired.binding().stream()).await?;
                if let Ok(run) = chat.run(expired.binding()) {
                    run.control().stop();
                    run.status(chat.log(), RunStatus::Stopped).await?;
                }
            }
            let Some(lease) = decision.lease else {
                break;
            };
            let mut held = lifecycle::WorkerLease::new(
                self.services.clone(),
                self.queue.clone(),
                lease.clone(),
            );
            let chat = self.chat(lease.request().binding().stream()).await?;
            let run = chat.run(lease.request().binding())?;
            held.attach(chat.clone(), run.clone());
            let this = self.clone();
            let work = async move {
                let result = worker::RunWorker::new(this.services.clone(), chat, run)
                    .execute()
                    .await;
                held.finish()?;
                result
            }
            .boxed();
            workers.push(
                self.services
                    .tasks()
                    .submit(supervisor::TaskClass::Worker, work)?,
            );
        }
        Ok(())
    }
    /// Runs ready work for embeddings that own their own outer supervisor.
    pub async fn drive_one(&self) -> Result<bool, CoordinatorError> {
        if !self.services.tasks().running() {
            return Ok(false);
        }
        self.services.tasks().check_healthy()?;
        self.resume_waiting().await?;
        let decision = self.queue.reserve(self.services.timer().now())?;
        let Some(lease) = decision.lease else {
            return Ok(false);
        };
        let mut held =
            lifecycle::WorkerLease::new(self.services.clone(), self.queue.clone(), lease.clone());
        let chat = self.chat(lease.request().binding().stream()).await?;
        let run = chat.run(lease.request().binding())?;
        held.attach(chat.clone(), run.clone());
        let services = self.services.clone();
        let work = async move {
            let result = worker::RunWorker::new(services, chat, run).execute().await;
            held.finish()?;
            result?;
            Ok(true)
        }
        .boxed();
        self.services
            .tasks()
            .track(supervisor::TaskClass::Worker, work)?
            .await
            .into_result()
    }
}
