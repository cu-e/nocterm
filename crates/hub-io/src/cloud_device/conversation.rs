//! The `CloudConversation` port over the device core's `OpenChat` flow.
//!
//! One task per open chat runs the flow and the `Deliver` flows of its commands, performs their
//! effects and hands every transcript change and freshness to the subscriptions. It starts when
//! a window subscribes or a command is delivered, and stops (keeping its sealed checkpoint)
//! when nothing subscribes and no delivery is left, or when the account ends. Reading resumes
//! from the checkpoint, so a reopened chat shows its verified prefix at once and replays only
//! what is new. Nothing here polls: the flow probes on open, on watch reconnects, after a
//! receipt of this device's own command and on refresh. A command signed before the application
//! quit is kept in the [`Outbox`] and resumed when its chat is read again: its pending row shows
//! again and the same bytes are published, with a receipt probe.
//!
//! A chat bound to servers (ADR 0147) answers its runner's certificate requests while it is
//! read and the account is unlocked; the core certifies only inside the binding.
use super::account::DeviceAccount;
use super::checkpoints::{AccountPlace, Checkpoints, chat_file};
use super::driver::{self, Control, Effects, now_ms};
use super::mapping;
use super::outbox::Outbox;
use super::server_flows::{BindReply, ServerFlows};
use super::servers::{self, ServerDirectory};
use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use futures::future::{BoxFuture, Shared};
use nocterm_cloud_conversation_model::{
    CloudConversation, CloudConversationError, CommandKey, ConversationAction, ConversationFuture,
    ConversationKey, ConversationUpdate, ConversationUpdates, MAX_BINDING_WINDOW, ServerChoice,
    ServerKey, ServerRefusal,
};
use nocterm_cloud_device::command::{Deliver, DeliveryUpdate, StoredCommand};
use nocterm_cloud_device::context::ChatRef;
use nocterm_cloud_device::flow::{Effect, Flow, Step};
use nocterm_cloud_device::open::{ChatAction, ChatUpdate, ModelChoice, OpenChat};
use nocterm_cloud_device::reader::VerifiedChat;
use nocterm_cloud_device::seat::WakeSeat;
use nocterm_cloud_device::ssh::SshServerChoice;
use nocterm_cloud_device::transcript::CommandState;
use nocterm_cloud_device::{CloudDeviceError, DeviceIdentity};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::runtime::Handle;

/// What a chat needs besides its account.
#[derive(Clone, Debug)]
pub(crate) struct ChatSetup {
    pub(crate) chat: ChatRef,
    /// The model of a first prompt, before any run names one.
    pub(crate) model: Option<ModelChoice>,
    /// The managed agent whose seat serves the chat; its seat is woken before each command.
    pub(crate) wake: Option<String>,
    /// The servers of the account's CA registry, for binding the chat (ADR 0147).
    pub(crate) servers: Option<Arc<dyn ServerDirectory>>,
}
impl std::fmt::Debug for dyn ServerDirectory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ServerDirectory")
    }
}

enum Request {
    Subscribe(mpsc::UnboundedSender<ConversationUpdate>),
    Deliver(
        ConversationAction,
        oneshot::Sender<Result<CommandKey, CloudConversationError>>,
    ),
    Refresh,
    Bind(Vec<SshServerChoice>, i64, BindReply),
    Unbind(oneshot::Sender<Result<(), CloudConversationError>>),
}

type Mailbox = Arc<Mutex<Option<mpsc::UnboundedSender<Request>>>>;

/// One chat of an account, opened for windows.
pub(crate) struct DeviceConversation {
    key: ConversationKey,
    task: Task,
    handle: Handle,
    ended: Shared<BoxFuture<'static, ()>>,
}
impl DeviceConversation {
    pub(crate) fn new(
        account: Arc<DeviceAccount>,
        setup: ChatSetup,
        (checkpoints, outbox): (Arc<Checkpoints>, Arc<Outbox>),
        handle: Handle,
    ) -> Self {
        let ended = account.clone().ended().boxed().shared();
        Self {
            key: ConversationKey::new(setup.chat.to_string()),
            task: Task {
                account,
                setup: Arc::new(setup),
                checkpoints,
                outbox,
                mailbox: Arc::new(Mutex::new(None)),
                generation: Arc::new(AtomicU64::new(1)),
            },
            handle,
            ended,
        }
    }
    fn send(&self, request: Request) -> Result<(), CloudConversationError> {
        send(&self.task, &self.handle, request)
    }
}

/// Hands `request` to the running task of `task`'s chat, starting one when none runs.
fn send(task: &Task, handle: &Handle, request: Request) -> Result<(), CloudConversationError> {
    task.account
        .check()
        .map_err(|_| CloudConversationError::Stale)?;
    let mut mailbox = task.mailbox.lock().unwrap_or_else(PoisonError::into_inner);
    let request = match mailbox.as_ref() {
        Some(sender) => match sender.unbounded_send(request) {
            Ok(()) => return Ok(()),
            Err(failed) => failed.into_inner(),
        },
        None => request,
    };
    let (sender, receiver) = mpsc::unbounded();
    sender
        .unbounded_send(request)
        .map_err(|_| CloudConversationError::Ended)?;
    *mailbox = Some(sender);
    handle.spawn(task.clone().run(receiver));
    Ok(())
}

impl CloudConversation for DeviceConversation {
    fn key(&self) -> &ConversationKey {
        &self.key
    }
    fn can_prompt(&self) -> bool {
        self.task.setup.model.is_some()
    }
    fn can_wake(&self) -> bool {
        self.task.setup.wake.is_some()
    }
    fn subscribe(&self) -> Result<ConversationUpdates, CloudConversationError> {
        let (sender, receiver) = mpsc::unbounded();
        self.send(Request::Subscribe(sender))?;
        Ok(receiver.boxed())
    }
    fn deliver(&self, action: ConversationAction) -> ConversationFuture<CommandKey> {
        let (reply, answer) = oneshot::channel();
        let sent = self.send(Request::Deliver(action, reply));
        Box::pin(async move {
            sent?;
            answer.await.unwrap_or(Err(CloudConversationError::Ended))
        })
    }
    fn refresh(&self) {
        let _ = self.send(Request::Refresh);
    }
    /// Wakes the managed seat of the chat with the core's `WakeSeat` (its enrolled seat,
    /// through its own pool); an agent without one is refused, never provisioned.
    fn wake(&self) -> ConversationFuture<()> {
        let Some(agent_id) = self.task.setup.wake.clone() else {
            return Box::pin(async { Err(CloudConversationError::Unsupported) });
        };
        let mut flow = WakeSeat::new(agent_id, Some(self.task.setup.chat.chat_id));
        let effects = self.task.account.effects().clone();
        let started = self.handle.spawn(async move {
            let mut woken = false;
            driver::run(&mut flow, effects, |_, _| {
                woken = true;
                Control::Stop
            })
            .await
            .map(|()| woken)
        });
        let refresh = self.task.mailbox.clone();
        Box::pin(async move {
            match started.await.map_err(|_| CloudConversationError::Ended)? {
                Ok(true) => {
                    if let Some(sender) = refresh
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .as_ref()
                    {
                        let _ = sender.unbounded_send(Request::Refresh);
                    }
                    Ok(())
                }
                Ok(false) => Err(CloudConversationError::Unavailable),
                Err(error) => Err(mapping::error(error)),
            }
        })
    }
    fn servers(&self) -> ConversationFuture<Vec<ServerChoice>> {
        match &self.task.setup.servers {
            Some(directory) => directory.choices(),
            None => Box::pin(async { Err(CloudConversationError::Unsupported) }),
        }
    }
    fn bind_servers(
        &self,
        keys: Vec<ServerKey>,
        window: std::time::Duration,
    ) -> ConversationFuture<()> {
        let Some(directory) = self.task.setup.servers.clone() else {
            return Box::pin(async { Err(CloudConversationError::Unsupported) });
        };
        if window.is_zero() || window > MAX_BINDING_WINDOW {
            return Box::pin(async { Err(CloudConversationError::Servers(ServerRefusal::Window)) });
        }
        let window_ms = i64::try_from(window.as_millis()).unwrap_or(i64::MAX);
        let (task, handle) = (self.task.clone(), self.handle.clone());
        Box::pin(async move {
            let chosen = servers::chosen(&keys, &directory.choices().await?)?;
            let (reply, answer) = oneshot::channel();
            send(&task, &handle, Request::Bind(chosen, window_ms, reply))?;
            answer.await.unwrap_or(Err(CloudConversationError::Ended))
        })
    }
    fn unbind_servers(&self) -> ConversationFuture<()> {
        let (reply, answer) = oneshot::channel();
        let sent = self.send(Request::Unbind(reply));
        Box::pin(async move {
            sent?;
            answer.await.unwrap_or(Err(CloudConversationError::Ended))
        })
    }
    fn check_current(&self) -> Result<(), CloudConversationError> {
        self.task
            .account
            .check()
            .map_err(|_| CloudConversationError::Stale)
    }
    fn ended(&self) -> BoxFuture<'static, ()> {
        self.ended.clone().boxed()
    }
}

/// What starts a reading task.
#[derive(Clone)]
struct Task {
    account: Arc<DeviceAccount>,
    setup: Arc<ChatSetup>,
    checkpoints: Arc<Checkpoints>,
    outbox: Arc<Outbox>,
    mailbox: Mailbox,
    generation: Arc<AtomicU64>,
}
impl Task {
    async fn run(self, requests: mpsc::UnboundedReceiver<Request>) {
        let place = AccountPlace::of(self.account.session().scope());
        let file = chat_file(&self.setup.chat);
        let checkpoints = self.checkpoints.clone();
        let (load_place, load_file) = (place.clone(), file.clone());
        let stored = tokio::task::spawn_blocking(move || checkpoints.load(&load_place, &load_file))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
            .map(VerifiedChat::from_bytes);
        let opened = self
            .account
            .device()
            .map_err(|_| CloudDeviceError::Ended)
            .and_then(|device| {
                OpenChat::new(
                    self.account.session().clone(),
                    device.clone(),
                    &self.setup.chat,
                    stored.as_ref(),
                )
                .map(|chat| (chat, device))
            });
        let reading = match opened {
            Ok((chat, device)) => Reading {
                chat,
                device,
                deliveries: Vec::new(),
                servers: ServerFlows::default(),
                next_slot: 1,
                effects: Effects::new(self.account.effects().clone()),
                subscribers: Vec::new(),
                task: self,
                place,
                file,
                kept: Vec::new(),
            },
            Err(error) => {
                self.refuse_all(requests, &mapping::error(error));
                return;
            }
        };
        let mut reading = reading;
        reading.resume_left();
        reading.run(requests).await;
    }
    /// Answers every waiting request with `error` and lets the next request start afresh.
    fn refuse_all(
        &self,
        mut requests: mpsc::UnboundedReceiver<Request>,
        error: &CloudConversationError,
    ) {
        let mut mailbox = self.mailbox.lock().unwrap_or_else(PoisonError::into_inner);
        while let Ok(request) = requests.try_recv() {
            if let Request::Deliver(_, reply) = request {
                let _ = reply.send(Err(error.clone()));
            }
        }
        *mailbox = None;
    }
}

/// A running chat: its flow, its commands' flows and its subscriptions.
struct Reading {
    chat: OpenChat,
    device: DeviceIdentity,
    deliveries: Vec<(u32, Deliver)>,
    /// Binding and certificate flows (ADR 0147).
    servers: ServerFlows,
    next_slot: u32,
    effects: Effects<u32>,
    subscribers: Vec<mpsc::UnboundedSender<ConversationUpdate>>,
    task: Task,
    place: AccountPlace,
    file: String,
    /// The command each delivery slot keeps in the outbox.
    kept: Vec<(u32, [u8; 16])>,
}
/// The tag of the chat flow's own effects; deliveries take the slots from 1 on.
const CHAT_SLOT: u32 = 0;

impl Reading {
    /// Resumes the commands an earlier run of this chat kept; one with nothing left to deliver,
    /// or that does not open for this chat, is forgotten.
    fn resume_left(&mut self) {
        let chat = self.task.setup.chat;
        for (command_id, stored) in self.task.outbox.left(&self.place, &chat) {
            let wake = self.task.setup.wake.clone();
            match self.chat.resume(&stored, wake, now_ms()) {
                Ok(Some(deliver)) => {
                    let slot = self.slot();
                    self.deliveries.push((slot, deliver));
                    self.kept.push((slot, command_id));
                }
                Ok(None) => self.task.outbox.settled(&self.place, &chat, &command_id),
                Err(error) => {
                    tracing::debug!(error = %nocterm_core::error::chain(&error), "a kept Cloud command was not resumed");
                    self.task.outbox.settled(&self.place, &chat, &command_id);
                }
            }
        }
    }
    fn slot(&mut self) -> u32 {
        let slot = self.next_slot;
        self.next_slot = self.next_slot.wrapping_add(1).max(1);
        slot
    }
    async fn run(mut self, mut requests: mpsc::UnboundedReceiver<Request>) {
        let ended = self.task.account.clone().ended();
        let mut ended = std::pin::pin!(ended);
        loop {
            let until = match self.pump() {
                Ok(until) => until,
                Err(error) => {
                    self.broadcast(ConversationUpdate::Refused(mapping::error(error)));
                    break;
                }
            };
            if self.finished(&mut requests) {
                return;
            }
            tokio::select! {
                request = requests.next() => match request {
                    Some(request) => self.request(request),
                    None => break,
                },
                (slot, id, event) = self.effects.next() => {
                    if slot == CHAT_SLOT {
                        driver::answer(&mut self.chat, id, event);
                    } else if let Some((slot, id, event)) = self.servers_resolve(slot, id, event)
                        && let Some((_, deliver)) =
                        self.deliveries.iter_mut().find(|(known, _)| *known == slot)
                    {
                        driver::answer(deliver, id, event);
                    }
                },
                () = driver::sleep_until(until) => {}
                () = &mut ended => break,
            }
        }
        self.task
            .refuse_all(requests, &CloudConversationError::Ended);
    }
    /// Ends the task when nothing subscribes and no command is being delivered. Holding the
    /// mailbox while deciding means no request is lost to a task that is about to end.
    fn finished(&mut self, requests: &mut mpsc::UnboundedReceiver<Request>) -> bool {
        self.subscribers.retain(|sender| !sender.is_closed());
        if !self.subscribers.is_empty() || !self.deliveries.is_empty() || !self.servers.is_empty() {
            return false;
        }
        let held = self.task.mailbox.clone();
        let mut mailbox = held.lock().unwrap_or_else(PoisonError::into_inner);
        match requests.try_recv() {
            Ok(request) => {
                drop(mailbox);
                self.request(request);
                false
            }
            _ => {
                *mailbox = None;
                true
            }
        }
    }
    fn broadcast(&mut self, update: ConversationUpdate) {
        self.subscribers
            .retain(|sender| sender.unbounded_send(update.clone()).is_ok());
    }
    fn request(&mut self, request: Request) {
        match request {
            Request::Subscribe(sender) => {
                let whole = mapping::delta(self.chat.transcript().full_delta(), now_ms());
                let freshness = mapping::freshness(self.chat.freshness());
                let sent = sender
                    .unbounded_send(ConversationUpdate::Transcript(whole))
                    .is_ok()
                    && sender
                        .unbounded_send(ConversationUpdate::Freshness(freshness))
                        .is_ok();
                if sent {
                    self.subscribers.push(sender);
                }
            }
            Request::Deliver(action, reply) => {
                let _ = reply.send(self.deliver(&action));
            }
            Request::Refresh => {
                self.refresh_device();
                self.chat.refresh();
            }
            Request::Bind(choices, window_ms, reply) => {
                let wake = self.task.setup.wake.clone();
                match self
                    .chat
                    .bind_servers(&choices, Some(window_ms), wake, now_ms())
                {
                    Ok(flow) => {
                        let slot = self.slot();
                        self.servers.bind(slot, flow, reply);
                    }
                    Err(error) => {
                        let _ = reply.send(Err(mapping::error(error)));
                    }
                }
            }
            Request::Unbind(reply) => {
                let wake = self.task.setup.wake.clone();
                let result = self
                    .chat
                    .unbind_servers(wake, now_ms())
                    .map(|deliver| {
                        let slot = self.slot();
                        self.deliveries.push((slot, deliver));
                    })
                    .map_err(mapping::error);
                let _ = reply.send(result);
            }
        }
    }
    /// Hands an effect's event to its SSH flow; returns it when no SSH flow has that slot.
    fn servers_resolve(
        &mut self,
        slot: u32,
        id: nocterm_cloud_device::flow::EffectId,
        event: super::effects::Event,
    ) -> Option<(
        u32,
        nocterm_cloud_device::flow::EffectId,
        super::effects::Event,
    )> {
        if self.servers.owns(slot) {
            self.servers.resolve(slot, id, event);
            None
        } else {
            Some((slot, id, event))
        }
    }
    /// After a token refresh the flow names the device with its new token.
    fn refresh_device(&mut self) {
        if let Ok(device) = self.task.account.device()
            && device != self.device
        {
            self.chat.set_device(device.clone());
            self.device = device;
        }
    }
    fn deliver(
        &mut self,
        action: &ConversationAction,
    ) -> Result<CommandKey, CloudConversationError> {
        self.refresh_device();
        let transcript = self.chat.transcript();
        if transcript.deleted() {
            return Err(CloudConversationError::Deleted);
        }
        let action = match action {
            ConversationAction::Prompt { text, server } => ChatAction::Prompt {
                text: text.clone(),
                // Before the first run the chat's own model names the prompt; afterwards the
                // latest run's model does.
                model: if transcript.runs().is_empty() {
                    Some(
                        self.task
                            .setup
                            .model
                            .clone()
                            .ok_or(CloudConversationError::NoModel)?,
                    )
                } else {
                    None
                },
                server: server.as_ref().map(servers::server_id),
            },
            ConversationAction::Stop { run } => ChatAction::Stop {
                run: run.map(|run| *run.as_bytes()),
            },
            ConversationAction::Allow { run } => ChatAction::Allow {
                run: *run.as_bytes(),
            },
            ConversationAction::Answer { intent, approve } => ChatAction::Answer {
                intent: *intent.as_bytes(),
                approve: *approve,
            },
        };
        let deliver = self
            .chat
            .deliver(&action, self.task.setup.wake.clone(), now_ms())
            .map_err(mapping::error)?;
        let command = CommandKey::from_bytes(deliver.command_id());
        let slot = self.slot();
        self.deliveries.push((slot, deliver));
        Ok(command)
    }
    /// Runs every flow until each waits; returns when the earliest of them wants to be called
    /// again. Fails when the chat flow itself ended.
    fn pump(&mut self) -> Result<Option<i64>, CloudDeviceError> {
        loop {
            let chat_until = self.pump_chat()?;
            let (progressed, delivery_until) = self.pump_deliveries();
            let (moved, servers_until) =
                self.servers
                    .pump(&mut self.chat, &mut self.effects, now_ms());
            if !progressed && !moved {
                return Ok([chat_until, delivery_until, servers_until]
                    .into_iter()
                    .flatten()
                    .min());
            }
        }
    }
    fn pump_chat(&mut self) -> Result<Option<i64>, CloudDeviceError> {
        loop {
            match self.chat.next(now_ms()) {
                Step::Effect(id, effect) => self.effects.start(CHAT_SLOT, id, effect),
                Step::Output(update) => self.chat_update(update),
                Step::Idle { until_ms } => return Ok(until_ms),
                Step::Done => return Err(CloudDeviceError::Ended),
                Step::Failed(error) => return Err(error),
            }
        }
    }
    fn pump_deliveries(&mut self) -> (bool, Option<i64>) {
        let mut progressed = false;
        let mut until = None::<i64>;
        let mut index = 0;
        while index < self.deliveries.len() {
            let (slot, deliver) = &mut self.deliveries[index];
            let slot = *slot;
            match deliver.next(now_ms()) {
                Step::Effect(id, effect) => {
                    // Kept once signed, before it is first published; retries publish the
                    // same bytes and need nothing new.
                    let keep = matches!(effect, Effect::Publish(_))
                        && !self.kept.iter().any(|(known, _)| *known == slot);
                    let stored = keep.then(|| (deliver.command_id(), self.chat.stored(deliver)));
                    if let Some((command_id, stored)) = stored {
                        self.keep(slot, command_id, stored);
                    }
                    self.effects.start(slot, id, effect);
                    progressed = true;
                }
                Step::Output(update) => {
                    // A resumed delivery may still probe for its receipt after it reported
                    // `Delivered`: its outbox copy goes once the flow is done.
                    self.chat.delivery(&update);
                    progressed = true;
                }
                Step::Idle { until_ms } => {
                    until = [until, until_ms].into_iter().flatten().min();
                    index += 1;
                }
                Step::Done => {
                    self.deliveries.remove(index);
                    self.forget(slot);
                    progressed = true;
                }
                Step::Failed(error) => {
                    tracing::warn!(error = %nocterm_core::error::chain(&error), "a Cloud command failed");
                    let command_id = deliver.command_id();
                    self.chat.delivery(&DeliveryUpdate {
                        command_id,
                        state: CommandState::Refused,
                        cursor: None,
                    });
                    self.deliveries.remove(index);
                    self.forget(slot);
                    progressed = true;
                }
            }
        }
        (progressed, until)
    }
    fn keep(
        &mut self,
        slot: u32,
        command_id: [u8; 16],
        stored: Result<Option<StoredCommand>, CloudDeviceError>,
    ) {
        match stored {
            Ok(Some(stored)) => {
                let chat = &self.task.setup.chat;
                if let Err(error) = self
                    .task
                    .outbox
                    .keep(&self.place, chat, &command_id, &stored)
                {
                    tracing::warn!(%error, "a Cloud command was sent without its outbox copy");
                }
            }
            // Not signed yet: kept at its next publication.
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(error = %nocterm_core::error::chain(&error), "a Cloud command was sent without its outbox copy");
            }
        }
        self.kept.push((slot, command_id));
    }
    fn forget(&mut self, slot: u32) {
        if let Some(index) = self.kept.iter().position(|(known, _)| *known == slot) {
            let (_, command_id) = self.kept.remove(index);
            self.task
                .outbox
                .settled(&self.place, &self.task.setup.chat, &command_id);
        }
    }
    fn chat_update(&mut self, update: ChatUpdate) {
        match update {
            ChatUpdate::Transcript(delta) => {
                let delta = mapping::delta(delta, now_ms());
                self.broadcast(ConversationUpdate::Transcript(delta));
            }
            ChatUpdate::Freshness(freshness) => {
                self.broadcast(ConversationUpdate::Freshness(mapping::freshness(freshness)));
            }
            ChatUpdate::Checkpoint(checkpoint) => self.save(checkpoint),
            // The person's own accepted prompt waits for its model grant: allow it once,
            // within the chat's bounds (ADR 0129). A refusal leaves the explicit Allow.
            ChatUpdate::AllowRun { run } => {
                let allow = ConversationAction::Allow {
                    run: nocterm_cloud_conversation_model::RunKey::from_bytes(run.id),
                };
                if let Err(error) = self.deliver(&allow) {
                    tracing::debug!(%error, "the run's model grant was not sent");
                }
            }
            // The account is unlocked while the chat is read: answer inside the binding the
            // person signed (the core refuses anything else).
            ChatUpdate::SshRequest { request } => {
                let slot = self.slot();
                self.servers
                    .answer(&mut self.chat, slot, &request, now_ms());
            }
            ChatUpdate::Refused(error) => {
                tracing::debug!(error = %nocterm_core::error::chain(&error), "a Cloud chat round was refused");
                self.broadcast(ConversationUpdate::Refused(mapping::error(error)));
            }
        }
    }
    fn save(&self, checkpoint: VerifiedChat) {
        let generation = self.task.generation.fetch_add(1, Ordering::AcqRel);
        let checkpoints = self.task.checkpoints.clone();
        let (place, file) = (self.place.clone(), self.file.clone());
        let bytes = checkpoint.into_bytes();
        tokio::task::spawn_blocking(move || {
            if let Err(error) = checkpoints.store(&place, &file, generation, &bytes) {
                tracing::warn!(error = %nocterm_core::error::chain(&error), "a Cloud chat checkpoint was not saved");
            }
        });
    }
}
