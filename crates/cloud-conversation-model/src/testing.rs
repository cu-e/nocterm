//! An in-memory [`CloudConversation`] and delta helpers for consumers' tests.
use crate::{
    Approval, CloudConversation, CloudConversationError, CloudTranscript, CommandKey, CommandKind,
    CommandState, ConversationAction, ConversationDelta, ConversationFuture, ConversationKey,
    ConversationUpdate, ConversationUpdates, Freshness, IntentKey, ModelName, PendingCommand,
    ProviderOption, Row, RowRole, Run, RunKey, RunStatus, ServerBinding, ServerChoice, ServerKey,
    SessionRow, ToolRow, ToolState, Usage,
};
use futures::FutureExt as _;
use futures::channel::{mpsc, oneshot};
use futures::future::{BoxFuture, Shared};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// A row of `role` with `text`; the index is set when it is pushed.
pub fn row(role: RowRole, run: Option<RunKey>, text: &str) -> Row {
    Row {
        index: 0,
        role,
        run,
        text: text.to_owned(),
        images: 0,
        tool: None,
        session: None,
        at_ms: 1_000,
        updated_at_ms: 1_000,
    }
}

/// A tool row of `intent` calling `tool` in `state`.
pub fn tool_row(run: RunKey, intent: IntentKey, tool: &str, state: ToolState) -> Row {
    Row {
        tool: Some(ToolRow {
            intent,
            title: format!("server · {tool}"),
            tool: tool.to_owned(),
            arguments_json: "{}".to_owned(),
            destination: Some("server".to_owned()),
            mutates: true,
            redacted: false,
            state,
            output_json: None,
            outcome: None,
        }),
        ..row(RowRole::Tool, Some(run), "")
    }
}

/// A session row of `attempt` for `principal`.
pub fn session_row(
    run: RunKey,
    attempt: [u8; 16],
    principal: &str,
    state: crate::SessionState,
) -> Row {
    Row {
        session: Some(SessionRow {
            attempt,
            principal: principal.to_owned(),
            host_key_sha256: [7; 32],
            ssh: true,
            terminal: Some(true),
            state,
        }),
        ..row(RowRole::Session, Some(run), "")
    }
}

/// A run of `status` with the managed model `model-a`.
pub fn run(run: RunKey, status: RunStatus, prompt: CommandKey) -> Run {
    Run {
        run,
        status,
        usage: Usage::default(),
        model: ModelName {
            provider: "managed".to_owned(),
            model: "model-a".to_owned(),
        },
        prompt_command: prompt,
    }
}

#[derive(Default)]
struct State {
    transcript: CloudTranscript,
    next_index: u64,
    freshness: Freshness,
    subscribers: Vec<mpsc::UnboundedSender<ConversationUpdate>>,
    delivered: Vec<ConversationAction>,
    refuse: Option<fn() -> CloudConversationError>,
    settle: Option<CommandState>,
    next_command: u8,
    refreshes: usize,
    wakes: usize,
    choices: Vec<ServerChoice>,
    bindings: Vec<(Vec<ServerKey>, std::time::Duration)>,
    unbinds: usize,
    stale: bool,
    ended: Option<oneshot::Sender<()>>,
}

#[derive(Default)]
struct PromptSetup {
    enabled: bool,
    models: Vec<ModelName>,
    providers: Vec<(ProviderOption, Vec<ModelName>)>,
    selected: Option<String>,
    selecting: bool,
    gate: Option<Shared<oneshot::Receiver<()>>>,
}
struct ProviderAttempt {
    prompt: Arc<Mutex<PromptSetup>>,
    committed: bool,
}
impl ProviderAttempt {
    fn commit(mut self) {
        self.committed = true;
    }
}
impl Drop for ProviderAttempt {
    fn drop(&mut self) {
        if !self.committed {
            self.prompt
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .selecting = false;
        }
    }
}

/// A conversation whose transcript a test writes. Clones share one state.
#[derive(Clone)]
pub struct FakeConversation {
    key: ConversationKey,
    prompt: Arc<Mutex<PromptSetup>>,
    wake: bool,
    state: Arc<Mutex<State>>,
    next_receipt: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
    ended: Shared<BoxFuture<'static, ()>>,
}
impl FakeConversation {
    /// A chat that can prompt and wake.
    pub fn new(key: &str) -> Self {
        let (sender, receiver) = oneshot::channel::<()>();
        let state = State {
            ended: Some(sender),
            ..State::default()
        };
        Self {
            key: ConversationKey::new(key),
            prompt: Arc::new(Mutex::new(PromptSetup {
                enabled: true,
                ..PromptSetup::default()
            })),
            wake: true,
            state: Arc::new(Mutex::new(state)),
            next_receipt: Arc::default(),
            ended: receiver.map(drop).boxed().shared(),
        }
    }
    /// The models the agent offers to its composer.
    #[must_use]
    pub fn with_models(self, models: Vec<ModelName>) -> Self {
        self.set_models(models);
        self
    }
    pub fn set_models(&self, models: Vec<ModelName>) {
        let mut prompt = self.prompt.lock().unwrap_or_else(PoisonError::into_inner);
        if prompt.models == models {
            return;
        }
        prompt.models = models;
        drop(prompt);
        self.composer_changed();
    }
    fn composer_changed(&self) {
        self.state().subscribers.retain(|sender| {
            sender
                .unbounded_send(ConversationUpdate::ComposerChanged)
                .is_ok()
        });
    }
    /// An own-runner chat that requires an explicit provider before prompting.
    #[must_use]
    pub fn with_providers(self, providers: Vec<(ProviderOption, Vec<ModelName>)>) -> Self {
        *self.prompt.lock().unwrap_or_else(PoisonError::into_inner) = PromptSetup {
            providers,
            ..PromptSetup::default()
        };
        self
    }
    pub fn selected_provider(&self) -> Option<String> {
        self.prompt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .selected
            .clone()
    }
    pub fn pause_provider_selection(&self) -> oneshot::Sender<()> {
        let (sender, receiver) = oneshot::channel();
        self.prompt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .gate = Some(receiver.shared());
        sender
    }
    /// The same chat without a model to prompt with.
    #[must_use]
    pub fn without_prompt(self) -> Self {
        self.prompt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .enabled = false;
        self
    }
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn publish(state: &mut State, delta: ConversationDelta) {
        state.transcript.apply(delta.clone());
        state.subscribers.retain(|sender| {
            sender
                .unbounded_send(ConversationUpdate::Transcript(delta.clone()))
                .is_ok()
        });
    }
    fn whole(state: &State, reset: bool) -> ConversationDelta {
        let transcript = &state.transcript;
        ConversationDelta {
            reset,
            first_index: transcript.rows().first().map_or(0, |row| row.index),
            changed_from: Some(transcript.rows().first().map_or(0, |row| row.index)),
            rows: transcript.rows().to_vec(),
            runs: Some(transcript.runs().to_vec()),
            approvals: Some(transcript.approvals().to_vec()),
            pending: Some(transcript.pending().to_vec()),
            servers: Some(transcript.servers().clone()),
            control_revision: transcript.control_revision(),
            deleted: transcript.deleted(),
            omitted_earlier: transcript.omitted_earlier(),
        }
    }
    fn change(&self, edit: impl FnOnce(&mut State, &mut ConversationDelta)) {
        let mut state = self.state();
        let mut delta = Self::whole(&state, false);
        delta.changed_from = None;
        delta.rows.clear();
        edit(&mut state, &mut delta);
        Self::publish(&mut state, delta);
    }
    /// Reports a producer's delta, including reset or retention-window changes.
    pub fn apply_delta(&self, delta: ConversationDelta) {
        let mut state = self.state();
        let next = delta
            .rows
            .iter()
            .map(|row| row.index.saturating_add(1))
            .max();
        if delta.reset {
            state.next_index = next.unwrap_or(0);
        } else if let Some(next) = next {
            state.next_index = state.next_index.max(next);
        }
        Self::publish(&mut state, delta);
    }
    /// Appends rows, numbering them after the held ones.
    pub fn push_rows(&self, rows: Vec<Row>) {
        self.change(|state, delta| {
            let from = state.next_index;
            delta.changed_from = Some(from);
            for mut row in rows {
                row.index = state.next_index;
                state.next_index += 1;
                delta.rows.push(row);
            }
        });
    }
    /// Replaces the last row (a streamed text grows, a tool finishes).
    pub fn replace_last(&self, row: Row) {
        self.change(|state, delta| {
            let index = state.next_index.saturating_sub(1);
            delta.changed_from = Some(index);
            delta.rows.push(Row { index, ..row });
        });
    }
    pub fn set_runs(&self, runs: Vec<Run>) {
        self.change(|_, delta| delta.runs = Some(runs));
    }
    pub fn set_approvals(&self, approvals: Vec<Approval>) {
        self.change(|_, delta| delta.approvals = Some(approvals));
    }
    pub fn set_control_revision(&self, revision: u64) {
        self.change(|_, delta| delta.control_revision = revision);
    }
    /// The chat's binding as the verified history now states it.
    pub fn set_servers(&self, servers: ServerBinding) {
        self.change(|_, delta| delta.servers = Some(servers));
    }
    /// The servers [`CloudConversation::servers`] lists.
    pub fn set_server_choices(&self, choices: Vec<ServerChoice>) {
        self.state().choices = choices;
    }
    /// The bindings asked for, in order, with their windows.
    pub fn bindings(&self) -> Vec<(Vec<ServerKey>, std::time::Duration)> {
        self.state().bindings.clone()
    }
    /// How many times the chat was unbound.
    pub fn unbinds(&self) -> usize {
        self.state().unbinds
    }
    pub fn set_deleted(&self) {
        self.change(|_, delta| delta.deleted = true);
    }
    pub fn set_freshness(&self, freshness: Freshness) {
        let mut state = self.state();
        state.freshness = freshness;
        state.subscribers.retain(|sender| {
            sender
                .unbounded_send(ConversationUpdate::Freshness(freshness))
                .is_ok()
        });
    }
    /// Reports a refused read round to every subscription.
    pub fn refuse_round(&self, error: CloudConversationError) {
        let state = self.state();
        if let Some(sender) = state.subscribers.first() {
            let _ = sender.unbounded_send(ConversationUpdate::Refused(error));
        }
    }
    /// Moves a command this device sent to `state`.
    pub fn settle(&self, command: CommandKey, settled: CommandState) {
        self.change(|state, delta| {
            let mut pending = state.transcript.pending().to_vec();
            if let Some(known) = pending.iter_mut().find(|known| known.command == command) {
                known.state = settled;
            }
            delta.pending = Some(pending);
        });
    }
    /// The runner accepted the prompt `command` as `run`: its row and its run appear and the
    /// pending prompt is accepted.
    pub fn accept_prompt(&self, command: CommandKey, run_key: RunKey, status: RunStatus) {
        let text = self
            .state()
            .transcript
            .pending_command(command)
            .map(|known| known.text.clone())
            .unwrap_or_default();
        let mut runs = self.state().transcript.runs().to_vec();
        runs.push(run(run_key, status, command));
        self.set_runs(runs);
        self.push_rows(vec![row(RowRole::User, Some(run_key), &text)]);
        self.settle(command, CommandState::Accepted);
    }
    /// Holds the next delivery's receipt after its command has entered the transcript.
    /// Sending or dropping the returned signal lets that original receipt complete.
    pub fn pause_next_receipt(&self) -> oneshot::Sender<()> {
        let (release, waiting) = oneshot::channel();
        *self
            .next_receipt
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(waiting);
        release
    }
    /// Further delivered commands settle at once in `state` (`None`: they stay `Sending`).
    pub fn settle_deliveries(&self, state: Option<CommandState>) {
        self.state().settle = state;
    }
    /// Further deliveries fail with what `error` makes.
    pub fn refuse_deliveries(&self, error: fn() -> CloudConversationError) {
        self.state().refuse = Some(error);
    }
    /// What was delivered, in order.
    pub fn delivered(&self) -> Vec<ConversationAction> {
        self.state().delivered.clone()
    }
    pub fn refreshes(&self) -> usize {
        self.state().refreshes
    }
    pub fn wakes(&self) -> usize {
        self.state().wakes
    }
    /// How many subscriptions are open.
    pub fn subscribers(&self) -> usize {
        let mut state = self.state();
        state.subscribers.retain(|sender| !sender.is_closed());
        state.subscribers.len()
    }
    /// Ends the current reading streams without retiring the account or its commands.
    pub fn close_reading(&self) {
        self.state().subscribers.clear();
    }
    /// The account ended: the conversation is stale and its subscriptions end.
    pub fn end(&self) {
        let mut state = self.state();
        state.stale = true;
        state.subscribers.clear();
        if let Some(sender) = state.ended.take() {
            let _ = sender.send(());
        }
    }
    /// The transcript the fake holds.
    pub fn transcript(&self) -> CloudTranscript {
        self.state().transcript.clone()
    }
}
impl CloudConversation for FakeConversation {
    fn key(&self) -> &ConversationKey {
        &self.key
    }
    fn can_prompt(&self) -> bool {
        self.prompt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .enabled
    }
    fn models(&self) -> Vec<ModelName> {
        self.prompt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .models
            .clone()
    }
    fn can_choose_provider(&self) -> bool {
        let prompt = self.prompt.lock().unwrap_or_else(PoisonError::into_inner);
        !prompt.providers.is_empty() && prompt.selected.is_none()
    }
    fn provider_options(&self) -> ConversationFuture<Vec<ProviderOption>> {
        let result = self.check_current().map(|()| {
            self.prompt
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .providers
                .iter()
                .map(|(choice, _)| choice.clone())
                .collect()
        });
        Box::pin(async move { result })
    }
    fn select_provider(&self, id: String) -> ConversationFuture<()> {
        if let Err(error) = self.check_current() {
            return Box::pin(async move { Err(error) });
        }
        let mut prompt = self.prompt.lock().unwrap_or_else(PoisonError::into_inner);
        if prompt.selecting {
            return Box::pin(async { Err(CloudConversationError::ProviderSelectionPending) });
        }
        if prompt.selected.is_some() || !prompt.providers.iter().any(|(choice, _)| choice.id == id)
        {
            return Box::pin(async { Err(CloudConversationError::Unsupported) });
        }
        prompt.selecting = true;
        let gate = prompt.gate.take();
        let this = self.clone();
        let attempt = ProviderAttempt {
            prompt: self.prompt.clone(),
            committed: false,
        };
        drop(prompt);
        Box::pin(async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            this.check_current()?;
            let mut prompt = this.prompt.lock().unwrap_or_else(PoisonError::into_inner);
            let models = prompt
                .providers
                .iter()
                .find(|(choice, _)| choice.id == id)
                .map(|(_, models)| models.clone())
                .ok_or(CloudConversationError::Unsupported)?;
            if models.is_empty() {
                return Err(CloudConversationError::NoModel);
            }
            prompt.models = models;
            prompt.selected = Some(id);
            prompt.enabled = !prompt.models.is_empty();
            prompt.selecting = false;
            drop(prompt);
            attempt.commit();
            this.composer_changed();
            Ok(())
        })
    }
    fn can_wake(&self) -> bool {
        self.wake
    }
    fn subscribe(&self) -> Result<ConversationUpdates, CloudConversationError> {
        self.check_current()?;
        let (sender, receiver) = mpsc::unbounded();
        let mut state = self.state();
        let _ = sender.unbounded_send(ConversationUpdate::Transcript(Self::whole(&state, true)));
        let _ = sender.unbounded_send(ConversationUpdate::Freshness(state.freshness));
        state.subscribers.push(sender);
        Ok(Box::pin(receiver))
    }
    fn deliver(&self, action: ConversationAction) -> ConversationFuture<CommandKey> {
        let result = (|| {
            self.check_current()?;
            let mut state = self.state();
            state.delivered.push(action.clone());
            if let Some(error) = state.refuse {
                return Err(error());
            }
            state.next_command = state.next_command.wrapping_add(1);
            let command = CommandKey::from_bytes([state.next_command; 16]);
            let (kind, text, run) = match &action {
                ConversationAction::Prompt { text, .. } => {
                    (CommandKind::Prompt, text.clone(), None)
                }
                ConversationAction::Stop { run } => (CommandKind::Stop, String::new(), *run),
                ConversationAction::Allow { run } => {
                    (CommandKind::Allow, String::new(), Some(*run))
                }
                ConversationAction::Answer { intent, .. } => (
                    CommandKind::Answer,
                    String::new(),
                    state.transcript.approval(*intent).map(|known| known.run),
                ),
            };
            let mut pending = state.transcript.pending().to_vec();
            pending.push(PendingCommand {
                command,
                kind,
                text,
                run,
                state: state.settle.unwrap_or(CommandState::Sending),
                at_ms: 2_000,
            });
            let mut delta = Self::whole(&state, false);
            delta.changed_from = None;
            delta.rows.clear();
            delta.pending = Some(pending);
            Self::publish(&mut state, delta);
            Ok(command)
        })();
        let waiting = self
            .next_receipt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        Box::pin(async move {
            if let Some(waiting) = waiting {
                let _ = waiting.await;
            }
            result
        })
    }
    fn refresh(&self) {
        self.state().refreshes += 1;
    }
    fn wake(&self) -> ConversationFuture<()> {
        let result = self.check_current().map(|()| self.state().wakes += 1);
        Box::pin(async move { result })
    }
    fn servers(&self) -> ConversationFuture<Vec<ServerChoice>> {
        let result = self.check_current().map(|()| self.state().choices.clone());
        Box::pin(async move { result })
    }
    fn bind_servers(
        &self,
        servers: Vec<ServerKey>,
        window: std::time::Duration,
    ) -> ConversationFuture<()> {
        let result = self.check_current().and_then(|()| {
            if servers.is_empty() {
                return Err(CloudConversationError::Servers(
                    crate::ServerRefusal::Servers,
                ));
            }
            if window.is_zero() || window > crate::MAX_BINDING_WINDOW {
                return Err(CloudConversationError::Servers(
                    crate::ServerRefusal::Window,
                ));
            }
            self.state().bindings.push((servers, window));
            Ok(())
        });
        Box::pin(async move { result })
    }
    fn unbind_servers(&self) -> ConversationFuture<()> {
        let result = self.check_current().map(|()| self.state().unbinds += 1);
        Box::pin(async move { result })
    }
    fn check_current(&self) -> Result<(), CloudConversationError> {
        if self.state().stale {
            Err(CloudConversationError::Stale)
        } else {
            Ok(())
        }
    }
    fn ended(&self) -> BoxFuture<'static, ()> {
        self.ended.clone().boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unpolled_provider_choice_stays_busy_until_its_future_is_cancelled() {
        let fake = FakeConversation::new("provider-cancellation").with_providers(vec![(
            ProviderOption {
                id: "personal".into(),
                label: "Personal".into(),
            },
            vec![ModelName {
                provider: "own".into(),
                model: "first".into(),
            }],
        )]);
        let pending = fake.select_provider("personal".into());
        assert!(matches!(
            futures::executor::block_on(fake.select_provider("personal".into())),
            Err(CloudConversationError::ProviderSelectionPending)
        ));
        assert!(!fake.can_prompt());
        drop(pending);
        futures::executor::block_on(fake.select_provider("personal".into())).unwrap();
        assert!(fake.can_prompt());
        assert_eq!(fake.selected_provider().as_deref(), Some("personal"));
    }
}
