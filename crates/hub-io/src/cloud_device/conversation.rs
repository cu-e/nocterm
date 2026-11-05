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
use super::deliveries::Deliveries;
use super::driver::{self, Control, Effects, now_ms};
use super::mapping;
use super::outbox::Outbox;
use super::prompt;
use super::providers::{self, PromptModels, ProviderDirectory, ProviderSelection};
use super::requests::{Mailbox, Request};
use super::server_flows::ServerFlows;
use super::servers::{self, ServerDirectory};
use super::subscriptions::{self, Subscriptions, UpdateBudget};
use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::{BoxFuture, Shared};
use nocterm_cloud_conversation_model::{
    CloudConversation, CloudConversationError, CommandKey, ConversationAction, ConversationFuture,
    ConversationKey, ConversationUpdate, ConversationUpdates, MAX_BINDING_WINDOW, ModelName,
    ProviderOption, ServerChoice, ServerKey, ServerRefusal,
};
use nocterm_cloud_device::context::ChatRef;
use nocterm_cloud_device::flow::{Flow, Step};
use nocterm_cloud_device::open::{ChatAction, ChatUpdate, ModelChoice, OpenChat};
use nocterm_cloud_device::reader::VerifiedChat;
use nocterm_cloud_device::seat::WakeSeat;
use nocterm_cloud_device::{CloudDeviceError, DeviceIdentity};
use nocterm_hub_model::cloud::ssh::MAX_BINDING_TARGETS;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::{Notify, mpsc};

/// What a chat needs besides its account.
#[derive(Clone, Debug)]
pub(crate) struct ChatSetup {
    pub(crate) chat: ChatRef,
    /// The model of a first prompt, before any run names one.
    pub(crate) model: Option<ModelChoice>,
    pub(crate) models: Vec<ModelName>,
    /// The managed agent whose seat serves the chat; its seat is woken before each command.
    pub(crate) wake: Option<String>,
    /// The servers of the account's CA registry, for binding the chat (ADR 0147).
    pub(crate) servers: Option<Arc<dyn ServerDirectory>>,
    pub(crate) providers: Option<Arc<dyn ProviderDirectory>>,
}
impl std::fmt::Debug for dyn ServerDirectory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ServerDirectory")
    }
}

/// One chat of an account, opened for windows.
pub(crate) struct DeviceConversation {
    key: ConversationKey,
    task: Task,
    handle: Handle,
    ended: Shared<BoxFuture<'static, ()>>,
    providers: Option<ProviderSelection>,
}
impl DeviceConversation {
    pub(crate) fn new(
        account: Arc<DeviceAccount>,
        mut setup: ChatSetup,
        (checkpoints, outbox): (Arc<Checkpoints>, Arc<Outbox>),
        updates: Arc<UpdateBudget>,
        handle: Handle,
    ) -> Arc<Self> {
        let ended = account.clone().ended().boxed().shared();
        let prompt = Arc::new(PromptModels::new(
            setup.model.take(),
            std::mem::take(&mut setup.models),
        ));
        let provider_directory = setup.providers.take();
        Arc::new_cyclic(|original| Self {
            providers: provider_directory.map(|directory| {
                ProviderSelection::new(
                    account.clone(),
                    directory,
                    original.clone(),
                    prompt.clone(),
                    handle.clone(),
                )
            }),
            key: ConversationKey::new(setup.chat.to_string()),
            task: Task {
                account,
                setup: Arc::new(setup),
                prompt,
                checkpoints,
                outbox,
                mailbox: Arc::new(Mailbox::default()),
                updates,
                subscription_changes: Arc::new(Notify::new()),
            },
            handle,
            ended,
        })
    }
    pub(crate) fn adopt_own_lease(
        &self,
        lease: &nocterm_cloud_device::provider::Lease,
    ) -> Result<(), CloudConversationError> {
        self.task
            .account
            .check()
            .map_err(|_| CloudConversationError::Stale)?;
        if self.task.prompt.adopt_lease(lease)? {
            self.notify_composer_changed();
        }
        Ok(())
    }
    pub(crate) fn refresh_own_lease(
        &self,
        lease: &nocterm_cloud_device::provider::Lease,
    ) -> Result<(), CloudConversationError> {
        self.task
            .account
            .check()
            .map_err(|_| CloudConversationError::Stale)?;
        if self.task.prompt.refresh_lease(lease)? {
            self.notify_composer_changed();
        }
        Ok(())
    }
    pub(super) fn notify_composer_changed(&self) {
        self.task.subscription_changes.notify_one();
    }
    pub(super) fn provider_revision(&self) -> ConversationFuture<u64> {
        let (reply, answer) = oneshot::channel();
        let sent = self.send(Request::ProviderRevision(reply));
        Box::pin(async move {
            sent?;
            answer.await.unwrap_or(Err(CloudConversationError::Ended))
        })
    }
    fn send(&self, request: Request) -> Result<(), CloudConversationError> {
        send(&self.task, &self.handle, request)
    }
}

/// Admission keeps the captured account check outside the mailbox's channel ownership.
fn send(task: &Task, handle: &Handle, request: Request) -> Result<(), CloudConversationError> {
    task.account
        .check()
        .map_err(|_| CloudConversationError::Stale)?;
    task.mailbox.send(request, |receiver| {
        handle.spawn(task.clone().run(receiver));
    })
}

impl CloudConversation for DeviceConversation {
    fn key(&self) -> &ConversationKey {
        &self.key
    }
    fn can_prompt(&self) -> bool {
        self.task.prompt.can_prompt()
    }
    fn default_model(&self) -> Option<ModelName> {
        self.task.prompt.default().map(|model| ModelName {
            provider: model.provider.clone(),
            model: model.model.clone(),
        })
    }
    fn models(&self) -> Vec<ModelName> {
        self.task.prompt.models()
    }
    fn can_choose_provider(&self) -> bool {
        self.providers
            .as_ref()
            .is_some_and(ProviderSelection::can_choose)
    }
    fn provider_options(&self) -> ConversationFuture<Vec<ProviderOption>> {
        self.providers.as_ref().map_or_else(
            || {
                Box::pin(async { Err(CloudConversationError::Unsupported) })
                    as ConversationFuture<_>
            },
            ProviderSelection::options,
        )
    }
    fn select_provider(&self, id: String) -> ConversationFuture<()> {
        self.providers.as_ref().map_or_else(
            || {
                Box::pin(async { Err(CloudConversationError::Unsupported) })
                    as ConversationFuture<_>
            },
            |selection| selection.select(id),
        )
    }
    fn can_wake(&self) -> bool {
        self.task.setup.wake.is_some()
    }
    fn subscribe(&self) -> Result<ConversationUpdates, CloudConversationError> {
        let (subscriber, updates) = subscriptions::channel(self.task.subscription_changes.clone());
        self.send(Request::Subscribe(subscriber))?;
        Ok(updates)
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
                    let _ = refresh.send_existing(Request::Refresh);
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
        mut keys: Vec<ServerKey>,
        window: std::time::Duration,
    ) -> ConversationFuture<()> {
        let Some(directory) = self.task.setup.servers.clone() else {
            return Box::pin(async { Err(CloudConversationError::Unsupported) });
        };
        if keys.len() > MAX_BINDING_TARGETS {
            return Box::pin(async { Err(CloudConversationError::Capacity) });
        }
        if keys.capacity() > MAX_BINDING_TARGETS {
            keys = keys.into_boxed_slice().into_vec();
        }
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
    prompt: Arc<PromptModels>,
    checkpoints: Arc<Checkpoints>,
    outbox: Arc<Outbox>,
    mailbox: Arc<Mailbox>,
    updates: Arc<UpdateBudget>,
    subscription_changes: Arc<Notify>,
}
impl Task {
    async fn run(self, requests: mpsc::Receiver<Request>) {
        let place = AccountPlace::of(self.account.session().scope());
        let file = chat_file(&self.setup.chat);
        let ended = self.account.clone().ended();
        tokio::pin!(ended);
        let stored = tokio::select! {
            biased;
            () = &mut ended => {
                self.refuse_all(requests, &CloudConversationError::Stale);
                return;
            }
            stored = self.checkpoints.load_async(&place, &file) => stored.ok().flatten().map(VerifiedChat::from_bytes),
        };
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
                deliveries: Deliveries::new(
                    self.account.clone(),
                    self.outbox.clone(),
                    place.clone(),
                    self.setup.chat,
                ),
                servers: ServerFlows::default(),
                next_slot: 1,
                effects: Effects::new(self.account.effects().clone()),
                chat_effects: Effects::new(self.account.effects().clone()),
                subscribers: Subscriptions::new(self.updates.clone()),
                task: self,
                place,
                file,
                models_revision: 0,
            },
            Err(error) => {
                self.refuse_all(requests, &mapping::error(error));
                return;
            }
        };
        let mut reading = reading;
        let resumed = tokio::select! {
            biased;
            () = &mut ended => Err(CloudConversationError::Stale),
            result = reading.resume_left() => result,
        };
        if let Err(error) = resumed {
            reading.task.refuse_all(requests, &error);
            return;
        }
        reading.run(requests).await;
    }
    /// Answers every waiting request with `error` and lets the next request start afresh.
    fn refuse_all(&self, requests: mpsc::Receiver<Request>, error: &CloudConversationError) {
        self.mailbox.refuse_all(requests, error);
    }
}

/// A running chat: its flow, its commands' flows and its subscriptions.
struct Reading {
    chat: OpenChat,
    device: DeviceIdentity,
    deliveries: Deliveries,
    /// Binding and certificate flows (ADR 0147).
    servers: ServerFlows,
    next_slot: u32,
    effects: Effects<u32>,
    chat_effects: Effects<()>,
    subscribers: Subscriptions,
    task: Task,
    place: AccountPlace,
    file: String,
    models_revision: u64,
}
const MAX_CONTROL_FLOWS: usize = 32;

impl Reading {
    async fn resume_left(&mut self) -> Result<(), CloudConversationError> {
        let restored = self
            .deliveries
            .restore(&mut self.chat, self.task.setup.wake.clone())
            .await?;
        for flow in restored {
            let slot = self.slot();
            self.deliveries.insert(slot, flow, true);
        }
        Ok(())
    }
    fn slot(&mut self) -> u32 {
        let slot = self.next_slot;
        self.next_slot = self.next_slot.wrapping_add(1).max(1);
        slot
    }
    async fn run(mut self, mut requests: mpsc::Receiver<Request>) {
        let ended = self.task.account.clone().ended();
        tokio::pin!(ended);
        let failed = loop {
            let pumped = tokio::select! {
                biased;
                () = &mut ended => break CloudConversationError::Stale,
                result = self.pump() => result,
            };
            let until = match pumped {
                Ok(until) => until,
                Err(error) => break error,
            };
            if self.finished(&mut requests) {
                return;
            }
            match self.hear(&mut requests, until).await {
                Ok(true) => {}
                Ok(false) => break CloudConversationError::Ended,
                Err(error) => break error,
            }
        };
        self.task.refuse_all(requests, &failed);
        self.subscribers
            .finish(failed, self.task.account.clone().ended().boxed().shared())
            .await;
    }
    async fn hear(
        &mut self,
        requests: &mut mpsc::Receiver<Request>,
        until: Option<i64>,
    ) -> Result<bool, CloudConversationError> {
        tokio::select! {
            () = self.task.account.clone().ended() => return Err(CloudConversationError::Stale),
            result = self.subscribers.advance() => result?,
            request = requests.recv() => {
                let Some(request) = request else { return Ok(false); };
                self.request(request);
            },
            (slot, id, event) = self.effects.next() => {
                if let Some((slot, id, event)) = self.servers_resolve(slot, id, event) {
                    self.deliveries.resolve(slot, id, event);
                }
            },
            ((), id, event) = self.chat_effects.next(), if !self.subscribers.busy() => {
                driver::answer(&mut self.chat, id, event);
            },
            () = self.task.subscription_changes.notified() => {},
            () = driver::sleep_until(until) => {},
        }
        Ok(true)
    }
    /// Checks subscription close after its drop wake, while holding the mailbox
    /// across the final stop decision so a concurrent request is never lost.
    fn finished(&mut self, requests: &mut mpsc::Receiver<Request>) -> bool {
        if !self.subscribers.is_empty() || !self.deliveries.is_empty() || !self.servers.is_empty() {
            return false;
        }
        let Some(request) = self.task.mailbox.finish(requests) else {
            return true;
        };
        self.request(request);
        false
    }
    fn broadcast(&mut self, update: ConversationUpdate) {
        self.subscribers
            .begin_publish(update, self.task.account.clone().ended().boxed().shared());
    }
    fn request(&mut self, request: Request) {
        match request {
            Request::Subscribe(subscriber) => self.subscribers.defer(subscriber),
            Request::Deliver(action, reply) => {
                let _ = reply.send(self.deliver(&action));
            }
            Request::ProviderRevision(reply) => {
                let _ = reply.send(providers::admission_revision(&self.chat));
            }
            Request::Refresh => {
                self.refresh_device();
                self.chat.refresh();
            }
            Request::Bind(choices, window_ms, reply) => {
                if let Err(error) = self.admit() {
                    let _ = reply.send(Err(error));
                    return;
                }
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
                if let Err(error) = self.admit() {
                    let _ = reply.send(Err(error));
                    return;
                }
                let wake = self.task.setup.wake.clone();
                let result = self
                    .chat
                    .unbind_servers(wake, now_ms())
                    .map(|deliver| {
                        let slot = self.slot();
                        self.deliveries.insert(slot, deliver, false);
                    })
                    .map_err(mapping::error);
                let _ = reply.send(result);
            }
        }
    }
    fn admit(&self) -> Result<(), CloudConversationError> {
        if self.deliveries.len() + self.servers.len() >= MAX_CONTROL_FLOWS {
            Err(CloudConversationError::Capacity)
        } else {
            Ok(())
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
        self.admit()?;
        self.refresh_device();
        let transcript = self.chat.transcript();
        if transcript.deleted() {
            return Err(CloudConversationError::Deleted);
        }
        let action = match action {
            ConversationAction::Prompt { model, .. } => {
                if !self.task.prompt.can_prompt()
                    || model
                        .as_ref()
                        .is_some_and(|model| !self.task.prompt.allows(model))
                {
                    return Err(CloudConversationError::NoModel);
                }
                let latest_allowed = transcript.runs().last().is_some_and(|run| {
                    self.task.prompt.allows(&ModelName {
                        provider: run.model.provider.as_str().to_owned(),
                        model: run.model.model.as_str().to_owned(),
                    })
                });
                prompt::prompt(action, self.task.prompt.default().as_ref(), !latest_allowed)?
            }
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
        self.deliveries.insert(slot, deliver, false);
        Ok(command)
    }
    /// Runs every flow until each waits; returns when the earliest of them wants to be called
    /// again. Fails when the chat flow itself ended.
    async fn pump(&mut self) -> Result<Option<i64>, CloudConversationError> {
        loop {
            let chat_until = self.pump_chat()?;
            let (progressed, delivery_until) = self
                .deliveries
                .pump(&mut self.chat, &mut self.effects)
                .await?;
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
    fn pump_chat(&mut self) -> Result<Option<i64>, CloudConversationError> {
        if let Some(subscriber) = self.subscribers.waiting() {
            let whole = mapping::delta(self.chat.transcript().full_delta(), now_ms());
            let freshness = mapping::freshness(self.chat.freshness());
            self.subscribers.begin_subscribe(
                subscriber,
                [
                    ConversationUpdate::Transcript(whole),
                    ConversationUpdate::Freshness(freshness),
                ],
                self.task.account.clone().ended().boxed().shared(),
            );
        }
        while !self.subscribers.busy() {
            let revision = self.task.prompt.revision();
            if revision != self.models_revision {
                self.models_revision = revision;
                self.broadcast(ConversationUpdate::ComposerChanged);
                continue;
            }
            match self.chat.next(now_ms()) {
                Step::Effect(id, effect) => self.chat_effects.start((), id, effect),
                Step::Output(update) => self.chat_update(update),
                Step::Idle { until_ms } => {
                    if let Some(error) = self.deliveries.take_failure() {
                        self.broadcast(ConversationUpdate::Refused(error));
                    }
                    return Ok(until_ms);
                }
                Step::Done => return Err(mapping::error(CloudDeviceError::Ended)),
                Step::Failed(error) => return Err(mapping::error(error)),
            }
        }
        Ok(None)
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
            ChatUpdate::AllowRun { run } => self.allow_run(run.id),
            ChatUpdate::SshRequest { request } => self.answer_ssh(&request),
            ChatUpdate::Refused(error) => {
                tracing::debug!(error = %nocterm_core::error::chain(&error), "a Cloud chat round was refused");
                self.broadcast(ConversationUpdate::Refused(mapping::error(error)));
            }
        }
    }
    /// The person's accepted prompt waits for its model grant (ADR 0129).
    /// A refused automatic grant leaves the explicit Allow action available.
    fn allow_run(&mut self, run: [u8; 16]) {
        let allow = ConversationAction::Allow {
            run: nocterm_cloud_conversation_model::RunKey::from_bytes(run),
        };
        if let Err(error) = self.deliver(&allow) {
            tracing::debug!(%error, "the run's model grant was not sent");
        }
    }
    /// Answer only inside the person's signed binding, while account lifetime
    /// and active flow admission still permit it; the core verifies that binding.
    fn answer_ssh(&mut self, request: &[u8; 16]) {
        if self.admit().is_err() {
            tracing::debug!("a Cloud certificate request exceeded active flow capacity");
            return;
        }
        let slot = self.slot();
        self.servers.answer(&mut self.chat, slot, request, now_ms());
    }
    fn save(&self, checkpoint: VerifiedChat) {
        if let Err(error) =
            self.task
                .checkpoints
                .offer(&self.place, &self.file, checkpoint.into_bytes())
        {
            tracing::warn!(error = %nocterm_core::error::chain(&error), "a Cloud chat checkpoint was not saved");
        }
    }
}
