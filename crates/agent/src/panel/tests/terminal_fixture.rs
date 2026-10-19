//! Existing scripted terminal fixture implements the independent workspace ports.
use super::*;
pub(super) struct Access {
    item: std::cell::Cell<Option<gpui_kit::EntityId>>,
    executor: std::cell::RefCell<Option<Arc<dyn nocterm_session::HostExec>>>,
    lease: std::cell::RefCell<Option<Arc<AtomicBool>>>,
    at_prompt: std::cell::Cell<bool>,
    sent: std::cell::RefCell<Vec<String>>,
    profile: std::cell::RefCell<Option<gpui_kit::SharedString>>,
    /// Shared by the sessions of a test.
    sign_in: Rc<SignIn>,
    link: Link,
}
/// The connection of a remote session, which may drop.
#[derive(Default)]
pub(super) struct Link {
    epoch: std::cell::Cell<u64>,
    remote: std::cell::Cell<bool>,
    closed: std::cell::Cell<bool>,
    /// Reconnecting fails, leaving it closed.
    unreachable: std::cell::Cell<bool>,
    reconnects: std::cell::Cell<usize>,
}
/// What the sessions of a test ask before they connect.
#[derive(Default)]
pub(super) struct SignIn {
    epoch: std::cell::Cell<u64>,
    /// Their saved secret needs the vault.
    vault_locked: std::cell::Cell<bool>,
    /// They ask for a password nothing saved answers.
    asks: std::cell::Cell<bool>,
    answers: std::cell::RefCell<Vec<String>>,
}
impl Access {
    pub(super) fn new(profile: Option<gpui_kit::SharedString>, sign_in: Rc<SignIn>) -> Rc<Self> {
        Rc::new(Self {
            item: Default::default(),
            executor: Default::default(),
            lease: Default::default(),
            at_prompt: std::cell::Cell::new(true),
            sent: Default::default(),
            profile: std::cell::RefCell::new(profile),
            sign_in,
            link: Default::default(),
        })
    }

    pub(super) fn replace_connection(&self) {
        self.link.epoch.set(self.link.epoch.get() + 1);
        self.link.closed.set(false);
        if let Some(lease) = self.lease.borrow_mut().take() {
            lease.store(false, Ordering::Release);
        }
    }
    pub(super) fn replace_prompt(&self) {
        self.sign_in.epoch.set(self.sign_in.epoch.get() + 1);
        self.sign_in.asks.set(true);
    }
    fn check(&self, expected: ExpectedSession) -> Result<(), nocterm_workspace::TerminalError> {
        if self.item.get() != Some(expected.item()) || self.link.epoch.get() != expected.epoch() {
            return Err(nocterm_workspace::TerminalError::StaleSession);
        }
        Ok(())
    }
}
impl TerminalState for Access {
    fn info(&self, _: &App) -> Option<TerminalInfo> {
        Some(TerminalInfo {
            expected: ExpectedSession::new(self.item.get()?, self.link.epoch.get()),
            title: "Terminal".into(),
            local: !self.link.remote.get(),
            target: None,
            profile: self.profile.borrow().clone(),
            status: if self.link.closed.get() {
                TerminalStatus::Closed
            } else if self.sign_in.vault_locked.get() {
                TerminalStatus::AwaitingVault
            } else if self.sign_in.asks.get() {
                TerminalStatus::AwaitingUser
            } else {
                TerminalStatus::Connected
            },
            cwd: None,
            at_prompt: Some(self.at_prompt.get()),
            dirty_input: false,
            alt_screen: false,
            generation: 1,
            sign_in: (self.sign_in.vault_locked.get() || self.sign_in.asks.get()).then(|| {
                nocterm_workspace::SignInPrompt {
                    token: UserPromptToken::new(
                        ExpectedSession::new(self.item.get().unwrap(), self.link.epoch.get()),
                        self.sign_in.epoch.get(),
                    ),
                    label: "Password for user@example.test".into(),
                    retry: false,
                    masked: true,
                }
            }),
        })
    }
}
impl TerminalTextSource for Access {
    fn read(
        &self,
        expected: ExpectedSession,
        _: TextRequest,
        _: &App,
    ) -> Result<TerminalText, nocterm_workspace::TerminalError> {
        self.check(expected)?;
        Ok(TerminalText {
            text: "output".into(),
            first_line: 1,
            next_line: 1,
            truncated: false,
            alt_screen: false,
        })
    }
}
impl TerminalInput for Access {
    fn send_text(
        &self,
        expected: ExpectedSession,
        text: &str,
        _: &mut App,
    ) -> Result<(), nocterm_workspace::TerminalError> {
        self.check(expected)?;
        self.sent.borrow_mut().push(text.into());
        Ok(())
    }
}
impl TerminalExecution for Access {
    fn executor(
        &self,
        expected: ExpectedSession,
        _cx: &App,
    ) -> Result<Arc<dyn nocterm_session::HostExec>, nocterm_workspace::TerminalError> {
        self.check(expected)?;
        if self.sign_in.asks.get() || self.sign_in.vault_locked.get() {
            return Err(nocterm_workspace::TerminalError::Unavailable);
        }
        self.executor
            .borrow()
            .clone()
            .ok_or(nocterm_workspace::TerminalError::Unsupported(
                nocterm_workspace::TerminalCapability::StructuredExecution,
            ))
    }
}
impl LiveCommands for Access {
    fn begin(
        &self,
        expected: ExpectedSession,
        command: &str,
        cx: &mut App,
    ) -> Result<nocterm_workspace::LiveCommandLease, nocterm_workspace::TerminalError> {
        self.check(expected)?;
        if self
            .lease
            .borrow()
            .as_ref()
            .is_some_and(|lease| lease.load(Ordering::Acquire))
        {
            return Err(nocterm_workspace::TerminalError::LiveCommandOwned);
        }
        self.send_text(expected, command, cx)?;
        let lease = Arc::new(AtomicBool::new(true));
        *self.lease.borrow_mut() = Some(lease.clone());
        Ok(nocterm_workspace::LiveCommandLease::new(lease, None))
    }
}
impl SignInPrompts for Access {
    fn pending(&self, cx: &App) -> Option<nocterm_workspace::SignInPrompt> {
        self.info(cx)?.sign_in
    }
    fn answer(
        &self,
        token: UserPromptToken,
        answer: nocterm_session::Secret,
        _: &mut App,
    ) -> Result<(), nocterm_workspace::TerminalError> {
        self.check(token.expected())?;
        if token.prompt_epoch() != self.sign_in.epoch.get() {
            return Err(nocterm_workspace::TerminalError::StalePrompt);
        }
        if !self.sign_in.asks.get() && !self.sign_in.vault_locked.get() {
            return Err(nocterm_workspace::TerminalError::SignInNoLongerRequested);
        }
        self.sign_in
            .answers
            .borrow_mut()
            .push(answer.expose().to_owned());
        self.sign_in.epoch.set(self.sign_in.epoch.get() + 1);
        self.sign_in.vault_locked.set(false);
        self.sign_in.asks.set(false);
        Ok(())
    }
}
impl RemoteReconnect for Access {
    fn reconnect(
        &self,
        expected: ExpectedSession,
        _: &mut App,
    ) -> Result<ExpectedSession, nocterm_workspace::TerminalError> {
        self.check(expected)?;
        if !self.link.remote.get() {
            return Err(nocterm_workspace::TerminalError::OnlyRemoteReconnect);
        }
        if self.link.closed.get() {
            self.link.epoch.set(self.link.epoch.get() + 1);
            self.link.reconnects.set(self.link.reconnects.get() + 1);
            self.link.closed.set(self.link.unreachable.get());
        }
        Ok(ExpectedSession::new(expected.item(), self.link.epoch.get()))
    }
}

pub(super) struct FakeTerminal {
    access: Rc<Access>,
    focus: FocusHandle,
}
impl EventEmitter<ItemEvent> for FakeTerminal {}
impl Focusable for FakeTerminal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for FakeTerminal {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::div().size_full().track_focus(&self.focus)
    }
}
impl Item for FakeTerminal {
    fn tab_title(&self, _: &App) -> gpui_kit::SharedString {
        "Terminal".into()
    }
    fn terminal_capabilities(
        &self,
        owner: gpui_kit::WeakEntity<Self>,
    ) -> Option<TerminalCapabilities> {
        self.access.item.set(Some(owner.entity_id()));
        Some(
            TerminalCapabilities::new(self.access.clone())
                .with_text(self.access.clone())
                .with_input(self.access.clone())
                .with_live(self.access.clone())
                .with_execution(self.access.clone())
                .with_prompts(self.access.clone())
                .with_reconnect(self.access.clone()),
        )
    }
}

impl Access {
    pub(super) fn executor(
        &self,
    ) -> &std::cell::RefCell<Option<Arc<dyn nocterm_session::HostExec>>> {
        &self.executor
    }
    pub(super) fn lease(&self) -> &std::cell::RefCell<Option<Arc<AtomicBool>>> {
        &self.lease
    }
    pub(super) fn at_prompt(&self) -> &std::cell::Cell<bool> {
        &self.at_prompt
    }
    pub(super) fn sent(&self) -> &std::cell::RefCell<Vec<String>> {
        &self.sent
    }
    pub(super) fn profile(&self) -> &std::cell::RefCell<Option<gpui_kit::SharedString>> {
        &self.profile
    }
    pub(super) fn sign_in(&self) -> &Rc<SignIn> {
        &self.sign_in
    }
    pub(super) fn link(&self) -> &Link {
        &self.link
    }
}

impl SignIn {
    pub(super) fn vault_locked(&self) -> &std::cell::Cell<bool> {
        &self.vault_locked
    }
    pub(super) fn asks(&self) -> &std::cell::Cell<bool> {
        &self.asks
    }
    pub(super) fn answers(&self) -> &std::cell::RefCell<Vec<String>> {
        &self.answers
    }
}

impl Link {
    pub(super) fn remote(&self) -> &std::cell::Cell<bool> {
        &self.remote
    }
    pub(super) fn closed(&self) -> &std::cell::Cell<bool> {
        &self.closed
    }
    pub(super) fn unreachable(&self) -> &std::cell::Cell<bool> {
        &self.unreachable
    }
    pub(super) fn reconnects(&self) -> &std::cell::Cell<usize> {
        &self.reconnects
    }
}

impl FakeTerminal {
    pub(super) fn new(access: Rc<Access>, cx: &mut Context<Self>) -> Self {
        Self {
            access,
            focus: cx.focus_handle(),
        }
    }
}

/// A remote view of the scripted terminal whose connection target can be edited.
pub(super) struct ConnectedAccess {
    pub(super) base: Rc<Access>,
    pub(super) target: std::cell::RefCell<nocterm_session::Target>,
}
impl TerminalState for ConnectedAccess {
    fn info(&self, cx: &App) -> Option<TerminalInfo> {
        let mut info = self.base.info(cx)?;
        info.local = false;
        info.profile = Some("edited-profile".into());
        info.target = Some(self.target.borrow().clone());
        Some(info)
    }
}
impl TerminalTextSource for ConnectedAccess {
    fn read(
        &self,
        expected: ExpectedSession,
        request: TextRequest,
        cx: &App,
    ) -> Result<TerminalText, nocterm_workspace::TerminalError> {
        self.base.read(expected, request, cx)
    }
}
impl TerminalInput for ConnectedAccess {
    fn send_text(
        &self,
        expected: ExpectedSession,
        text: &str,
        cx: &mut App,
    ) -> Result<(), nocterm_workspace::TerminalError> {
        self.base.send_text(expected, text, cx)
    }
}
impl SignInPrompts for ConnectedAccess {
    fn pending(&self, cx: &App) -> Option<nocterm_workspace::SignInPrompt> {
        self.info(cx)?.sign_in
    }
    fn answer(
        &self,
        token: UserPromptToken,
        answer: nocterm_session::Secret,
        cx: &mut App,
    ) -> Result<(), nocterm_workspace::TerminalError> {
        self.base.answer(token, answer, cx)
    }
}
