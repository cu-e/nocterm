use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::{ClipboardItem, Context, EventEmitter, Subscription, Task};
use nocterm_session::{
    CloseReason, ConnectRequest, ConnectStage, Event, HostKeyDecision, Prompt, PtySize, RemoteFs,
    Secret, Session, SessionError,
};
use nocterm_settings::{CursorShape, TerminalSettings};
use nocterm_ui::{ActiveSettings as _, SettingsStore};
use nocterm_vt::{Effect, Emulator, EmulatorOptions, Palette, Scroll, TermSize};
use nocterm_workspace::{SessionContext, SessionSpec};

use crate::{ActiveTransport, LocalTransportFactory, integration::ShellIntegration};

/// Where a terminal's session stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Connecting(ConnectStage),
    Connected,
    Closed(CloseReason),
}

/// What a [`Terminal`] tells its observers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    /// The status, the pending prompt or the title changed.
    Changed,
    /// The screen changed.
    Output,
    /// The program rang the bell.
    Bell,
}

/// A session and the screen it draws on.
pub struct Terminal {
    codec: crate::codec::TextCodec,
    text_error: Option<String>,
    input_error: RefCell<Option<String>>,
    recording: Option<crate::recording::Recorder>,
    recording_options: nocterm_session::LoggingOptions,
    recording_watch: Option<Task<()>>,
    pub(crate) credentials: crate::credentials::CredentialState,
    spec: SessionSpec,
    local: bool,
    integration: RefCell<ShellIntegration>,
    shell_program: String,
    session: Option<Session>,
    fs: Option<Arc<dyn RemoteFs>>,
    status: Status,
    prompt: Option<Prompt>,
    prompt_epoch: u64,
    emulator: Emulator,
    /// What the running program called its window.
    program_title: Option<String>,
    _pump: Option<Task<()>>,
    sync_timer: Option<Task<()>>,
    _settings: Subscription,
}

impl EventEmitter<TerminalEvent> for Terminal {}

impl Terminal {
    /// A terminal that starts connecting to `spec` at once.
    pub fn new(spec: SessionSpec, cx: &mut Context<Self>) -> Self {
        Self::new_kind(spec, false, cx)
    }

    pub fn new_local(cx: &mut Context<Self>) -> Self {
        Self::new_kind(
            SessionSpec {
                options: Default::default(),
                title: "Local terminal".into(),
                target: nocterm_session::Target::new("local", "localhost", 22),
                auth: nocterm_session::Auth::Auto,
                launch: None,
                credential: None,
            },
            true,
            cx,
        )
    }

    fn new_kind(spec: SessionSpec, local: bool, cx: &mut Context<Self>) -> Self {
        let options = emulator_options(&cx.settings().terminal);
        let mut this = Self {
            codec: crate::codec::TextCodec::new(cx.settings().terminal.charset),
            input_error: RefCell::default(),
            text_error: None,
            recording: None,
            recording_options: Default::default(),
            recording_watch: None,
            credentials: Default::default(),
            spec,
            local,
            integration: RefCell::default(),
            shell_program: String::new(),
            session: None,
            fs: None,
            status: Status::Connecting(ConnectStage::Connecting),
            prompt: None,
            prompt_epoch: 0,
            emulator: Emulator::new(TermSize::default(), options),
            program_title: None,
            _pump: None,
            sync_timer: None,
            _settings: cx.observe_global::<SettingsStore>(|this, cx| {
                let options = emulator_options(&cx.settings().terminal);
                this.emulator.set_options(options);
                cx.emit(TerminalEvent::Output);
            }),
        };
        this.connect(cx);
        this
    }

    pub(crate) fn set_credential_id(&mut self, id: nocterm_session::CredentialId) {
        self.spec.credential = Some(id);
    }

    pub fn spec(&self) -> &SessionSpec {
        &self.spec
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// The question the session is waiting on, if any.
    pub fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }

    pub(crate) fn prompt_epoch(&self) -> u64 {
        self.prompt_epoch
    }

    pub fn program_title(&self) -> Option<&str> {
        self.program_title.as_deref()
    }

    pub fn emulator(&self) -> &Emulator {
        &self.emulator
    }

    pub fn is_connected(&self) -> bool {
        self.status == Status::Connected
    }

    /// The session, as the workspace shows it to panels.
    pub fn session_context(&self) -> SessionContext {
        SessionContext {
            target: self.spec.target.clone(),
            fs: self.fs.clone(),
            connected: self.is_connected(),
        }
    }

    // ── Session ──────────────────────────────────────────────────────────────

    /// Opens a new session, replacing the current one.
    fn connect(&mut self, cx: &mut Context<Self>) {
        self.clear_credentials();
        self.stop_recording();
        if let Err(error) = self.spec.options.validate() {
            self.set_status(
                Status::Closed(CloseReason::Failed(SessionError::Other(error))),
                cx,
            );
            return;
        }
        let settings = cx.settings();
        self.codec = crate::codec::TextCodec::new(
            self.spec
                .options
                .charset
                .unwrap_or(settings.terminal.charset),
        );
        self.recording_options = self
            .spec
            .options
            .logging
            .clone()
            .unwrap_or_else(|| settings.logging.clone());
        self.text_error = None;
        *self.input_error.borrow_mut() = None;
        let launch = if self.local {
            shell_launch(&settings.local)
        } else {
            self.spec
                .launch
                .clone()
                .unwrap_or_else(|| shell_launch(&settings.ssh.launch))
        };
        if self.local {
            self.shell_program = launch.program.clone().unwrap_or_else(|| {
                std::env::var(if cfg!(windows) { "COMSPEC" } else { "SHELL" }).unwrap_or_default()
            });
            *self.integration.borrow_mut() = ShellIntegration::default();
        }
        let transport = if self.local {
            cx.try_global::<LocalTransportFactory>()
                .map(|factory| (factory.0)(launch.clone()))
        } else {
            cx.try_global::<ActiveTransport>().map(|t| t.0.clone())
        };
        let Some(transport) = transport else {
            self.set_status(
                Status::Closed(CloseReason::Failed(SessionError::Other(
                    "no transport is installed".into(),
                ))),
                cx,
            );
            return;
        };
        let settings = cx.settings();
        let request = ConnectRequest {
            proxy: self
                .spec
                .options
                .proxy
                .clone()
                .unwrap_or_else(|| settings.ssh.proxy.clone()),
            launch,
            target: self.spec.target.clone(),
            auth: self.spec.auth.clone(),
            term: self
                .spec
                .options
                .term
                .clone()
                .unwrap_or_else(|| settings.terminal.term.clone()),
            size: pty_size(self.emulator.size()),
            connect_timeout: Duration::from_secs(settings.ssh.connect_timeout_secs.into()),
            keepalive_interval: Some(settings.ssh.keepalive_interval_secs)
                .filter(|secs| *secs > 0)
                .map(|secs| Duration::from_secs(secs.into())),
        };

        let session = transport.open(request);
        self.fs = session.fs();
        self.session = Some(session.clone());
        self.prompt = None;
        self.set_status(Status::Connecting(ConnectStage::Connecting), cx);

        self._pump = Some(cx.spawn(async move |this, cx| {
            while let Some(event) = session.next_event().await {
                if this
                    .update(cx, |this, cx| this.handle_event(event, cx))
                    .is_err()
                {
                    return;
                }
            }
            // The transport went away without saying why.
            let _ = this.update(cx, |this, cx| {
                if !matches!(this.status, Status::Closed(_)) {
                    let lost = SessionError::ConnectionLost("the transport stopped".into());
                    this.handle_event(Event::Closed(CloseReason::Failed(lost)), cx);
                }
            });
        }));
    }

    /// Connects again after the session ended. Does nothing while it is up.
    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        if matches!(self.status, Status::Closed(_)) {
            // Start the new session on a fresh line below the old output.
            self.emulator.advance(b"\r\n");
            self.connect(cx);
        }
    }

    /// Ends the session.
    pub fn close(&mut self) {
        if let Some(session) = &self.session {
            session.close();
        }
    }

    fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Connecting(stage) => self.set_status(Status::Connecting(stage), cx),
            Event::Connected => {
                self.set_status(Status::Connected, cx);
                self.save_authenticated_credential(cx);
                if self.recording_options.auto_start {
                    self.start_recording(cx);
                }
            }
            Event::Output(bytes) => {
                let bytes = self.codec.decode(&bytes, false);
                if let Some(recording) = &mut self.recording {
                    recording.output(&bytes);
                }
                if self.local && self.integration.borrow_mut().advance(&bytes) {
                    cx.emit(TerminalEvent::Changed);
                }
                let effects = self.emulator.advance(&bytes);
                self.apply(effects, cx);
                self.schedule_sync(cx);
                cx.emit(TerminalEvent::Output);
            }
            Event::Prompt(prompt) => {
                // A newer question supersedes an unanswered one, including an
                // identical retry. Views must discard the previous input state.
                self.prompt_epoch = self.prompt_epoch.wrapping_add(1);
                self.prompt = Some(prompt);
                self.retrieve_credential(cx);
                cx.emit(TerminalEvent::Changed);
            }
            Event::Closed(reason) => {
                let tail = self.codec.decode(&[], true);
                if let Some(recording) = &mut self.recording {
                    recording.output(&tail);
                    recording.stop();
                }
                if !tail.is_empty() {
                    let effects = self.emulator.advance(&tail);
                    self.apply(effects, cx);
                }
                self.clear_credentials();
                self.prompt = None;
                self.set_status(Status::Closed(reason), cx);
            }
        }
    }

    fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.emit(TerminalEvent::Changed);
        }
    }

    /// Carries out what the emulator asked of the host.
    fn apply(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::Reply(bytes) => {
                    self.send(bytes);
                }
                Effect::Title(title) => {
                    self.program_title = title;
                    cx.emit(TerminalEvent::Changed);
                }
                Effect::Bell => cx.emit(TerminalEvent::Bell),
                Effect::CopyToClipboard(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
        }
    }

    /// Makes sure a synchronized update the program never ends is ended for it.
    fn schedule_sync(&mut self, cx: &mut Context<Self>) {
        if self.sync_timer.is_some() {
            return;
        }
        let Some(deadline) = self.emulator.sync_deadline() else {
            return;
        };
        let wait = deadline.saturating_duration_since(Instant::now());
        self.sync_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                this.sync_timer = None;
                match this.emulator.sync_deadline() {
                    Some(deadline) if deadline <= Instant::now() => {
                        let effects = this.emulator.finish_sync();
                        this.apply(effects, cx);
                        cx.emit(TerminalEvent::Output);
                    }
                    // The program ended it, or began another one since.
                    _ => this.schedule_sync(cx),
                }
            });
        }));
    }

    /// Whether this model owns the independent local shell.
    pub fn is_local(&self) -> bool {
        self.local
    }

    /// Directory announced by OSC 7, never guessed from prompt text.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.integration.borrow().cwd.clone()
    }

    /// Changes a known idle, empty local prompt. Otherwise returns a prepared command.
    pub fn change_directory(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        if !self.local || !self.is_connected() {
            return Err("Open a connected local terminal first.".into());
        }
        let _ = cx;
        let stem = Path::new(&self.shell_program)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let path = path
            .to_str()
            .ok_or("The shell cannot represent this directory path.")?;
        if path.contains(['\0', '\r', '\n']) {
            return Err(
                "Directory paths with control characters require manual navigation.".into(),
            );
        }
        let command=match stem {
            "bash"|"zsh"|"fish"=>format!("cd -- {}",nocterm_session::quote_posix(path)),
            "pwsh"|"powershell"=>format!("Set-Location -LiteralPath {}",nocterm_session::quote_powershell(path)),
            _=>return Err("Directory synchronization needs Bash, Zsh, fish or PowerShell with shell integration enabled.".into()),
        };
        let integration = self.integration.borrow();
        if !integration.at_prompt || integration.dirty_input || self.emulator.modes().alt_screen {
            return Err(format!(
                "The shell is busy or has unfinished input. Run at an empty prompt: {command}"
            ));
        }
        drop(integration);
        let bytes = self.codec.encode(&format!("{command}\r"))?;
        if !self.send(bytes) {
            return Err(
                "Directory change was not sent because the terminal input queue is full.".into(),
            );
        }
        Ok(())
    }

    // ── Input ────────────────────────────────────────────────────────────────

    /// Encodes only user text. Protocol replies and mouse messages use send unchanged.
    pub fn send_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let previous = self.text_error();
        match self.codec.encode(text) {
            Ok(bytes) => {
                self.text_error = None;
                self.send(bytes);
            }
            Err(error) => self.text_error = Some(error),
        }
        if previous != self.text_error() {
            cx.emit(TerminalEvent::Changed);
        }
    }
    pub fn text_error(&self) -> Option<String> {
        self.text_error
            .clone()
            .or_else(|| self.input_error.borrow().clone())
    }
    pub fn recording_status(&self) -> Option<crate::RecordingStatus> {
        self.recording.as_ref().map(|recording| recording.status())
    }
    pub fn is_recording(&self) -> bool {
        self.recording
            .as_ref()
            .is_some_and(|recording| recording.active())
    }
    pub fn stop_recording(&mut self) {
        if let Some(recording) = &mut self.recording {
            recording.stop();
        }
    }
    pub fn toggle_recording(&mut self, cx: &mut Context<Self>) {
        if self.is_recording() {
            self.stop_recording();
            cx.emit(TerminalEvent::Changed);
        } else {
            self.start_recording(cx);
        }
    }
    fn start_recording(&mut self, cx: &mut Context<Self>) {
        if !self.is_connected() {
            return;
        }
        let directory = self.recording_options.directory.clone().or_else(|| {
            cx.try_global::<crate::RecordingDirectory>()
                .map(|directory| directory.path.clone())
        });
        let Some(directory) = directory else {
            self.text_error =
                Some("Recording is unavailable: no log directory is configured.".into());
            cx.emit(TerminalEvent::Changed);
            return;
        };
        match crate::recording::Recorder::start(directory, self.recording_options.max_file_mib) {
            Ok(recording) => {
                if let Some(directory) = cx.try_global::<crate::RecordingDirectory>() {
                    directory.registry.track(&recording);
                }
                let mut previous = recording.status();
                self.recording = Some(recording);
                self.recording_watch = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(250))
                            .await;
                        let Ok(active) = this.update(cx, |this, cx| {
                            let Some(recording) = &this.recording else {
                                return false;
                            };
                            let status = recording.status();
                            if previous.error != status.error
                                || previous.path != status.path
                                || previous.finished != status.finished
                            {
                                cx.emit(TerminalEvent::Changed);
                            }
                            let active = !status.finished;
                            previous = status;
                            active
                        }) else {
                            break;
                        };
                        if !active {
                            break;
                        }
                    }
                }));
            }
            Err(error) => self.text_error = Some(format!("Could not start recording: {error}")),
        }
        cx.emit(TerminalEvent::Changed);
    }

    /// Delivers bytes to the remote program, if it is running.
    pub fn send(&self, bytes: impl Into<Vec<u8>>) -> bool {
        if let (Some(session), Status::Connected) = (&self.session, &self.status) {
            let bytes = bytes.into();
            let accepted = if self.local {
                let accepted = session.input(bytes.clone());
                if accepted {
                    self.integration.borrow_mut().input(&bytes);
                }
                accepted
            } else {
                session.input(bytes)
            };
            *self.input_error.borrow_mut() = if accepted {
                None
            } else {
                Some("Input was not sent: the connection's input queue is full or the paste exceeds 4 MiB. Wait and retry with a smaller selection.".into())
            };
            accepted
        } else {
            false
        }
    }

    /// Lays the grid out anew, and tells the remote program.
    pub fn resize(&mut self, size: TermSize) {
        if size == self.emulator.size() {
            return;
        }
        self.emulator.resize(size);
        if let Some(session) = &self.session {
            session.resize(pty_size(size));
        }
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.emulator.set_palette(palette);
    }

    /// Gives the emulator to `edit`, for selection and scrolling, and redraws.
    pub fn update_emulator<R>(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Emulator) -> R,
    ) -> R {
        let result = edit(&mut self.emulator);
        cx.emit(TerminalEvent::Output);
        result
    }

    pub fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        self.update_emulator(cx, |emulator| emulator.scroll(scroll));
    }

    // ── Prompts ──────────────────────────────────────────────────────────────

    /// Answers a pending host key question.
    pub fn answer_host_key(&mut self, decision: HostKeyDecision, cx: &mut Context<Self>) {
        match self.prompt.take() {
            Some(Prompt::UnknownHostKey { reply, .. }) => reply.send(decision),
            other => self.prompt = other,
        }
        cx.emit(TerminalEvent::Changed);
    }

    /// Answers a pending request for a secret; `None` declines it.
    pub fn answer_secret(&mut self, secret: Option<Secret>, cx: &mut Context<Self>) {
        if secret.is_none() {
            self.cancel_credential_candidate();
        }
        match self.prompt.take() {
            Some(Prompt::Secret { reply, .. }) => reply.send(secret),
            other => self.prompt = other,
        }
        cx.emit(TerminalEvent::Changed);
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.close();
    }
}

fn pty_size(size: TermSize) -> PtySize {
    PtySize {
        cols: size.cols,
        rows: size.rows,
        pixel_width: size.cols.saturating_mul(size.cell_width),
        pixel_height: size.rows.saturating_mul(size.cell_height),
    }
}

pub(crate) fn emulator_options(settings: &TerminalSettings) -> EmulatorOptions {
    EmulatorOptions {
        scrollback_lines: settings.scrollback_lines as usize,
        cursor_shape: match settings.cursor_shape {
            CursorShape::Block => nocterm_vt::CursorShape::Block,
            CursorShape::Bar => nocterm_vt::CursorShape::Bar,
            CursorShape::Underline => nocterm_vt::CursorShape::Underline,
        },
        cursor_blink: settings.cursor_blink,
    }
}

fn shell_launch(settings: &nocterm_settings::ShellSettings) -> nocterm_session::ShellLaunch {
    nocterm_session::ShellLaunch {
        program: settings.program.clone(),
        args: settings.args.clone(),
        cwd: settings.cwd.clone(),
        env: settings.env.clone(),
        integration: settings.integration,
    }
}

#[cfg(test)]
mod local_tests {
    use super::*;
    use gpui_kit::{AppContext as _, TestAppContext};
    use std::sync::Mutex;

    struct FakeTransport(Arc<Mutex<Option<nocterm_session::SessionDriver>>>);
    impl nocterm_session::Transport for FakeTransport {
        fn open(&self, _: ConnectRequest) -> Session {
            let (session, driver) = nocterm_session::channel(None);
            *self.0.lock().unwrap() = Some(driver);
            session
        }
    }

    #[gpui_kit::test]
    fn directory_bridge_rejects_busy_input_and_uses_running_shell_snapshot(
        cx: &mut TestAppContext,
    ) {
        let driver = Arc::new(Mutex::new(None));
        let captured = driver.clone();
        let terminal = cx.update(|cx| {
            gpui_kit::init(cx);
            let mut settings = nocterm_settings::Settings::default();
            settings.local.program = Some("/bin/bash".into());
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::in_memory(settings),
                cx,
            );
            crate::init_local(
                Arc::new(move |_| Arc::new(FakeTransport(captured.clone()))),
                cx,
            );
            cx.new(Terminal::new_local)
        });
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.handle_event(Event::Connected, cx);
                assert!(
                    terminal
                        .change_directory(Path::new("/tmp/a"), cx)
                        .unwrap_err()
                        .contains("busy")
                );
                terminal.handle_event(
                    Event::Output(b"\x1b]7;file://localhost/home/egor\x07\x1b]133;A\x07".to_vec()),
                    cx,
                );
                assert_eq!(terminal.cwd(), Some(PathBuf::from("/home/egor")));
                nocterm_ui::update_settings(cx, |s| s.local.program = Some("pwsh".into())).unwrap();
                terminal
                    .change_directory(Path::new("/tmp/a' $(id)"), cx)
                    .unwrap();
                assert!(
                    terminal.change_directory(Path::new("/tmp/b"), cx).is_err(),
                    "must wait for the next prompt"
                );
            })
        });
        let driver = driver.lock().unwrap().take().unwrap();
        assert_eq!(
            futures::executor::block_on(driver.next_command()),
            Some(nocterm_session::Command::Input(
                b"cd -- '/tmp/a'\\'' $(id)'\r".to_vec()
            ))
        );
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.handle_event(Event::Output(b"\x1b]133;A\x07".to_vec()), cx);
                terminal.send(b"unfinished".to_vec());
                assert!(
                    terminal
                        .change_directory(Path::new("/tmp/c"), cx)
                        .unwrap_err()
                        .contains("unfinished")
                );
            })
        });
    }
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    use futures::executor::block_on;
    use gpui_kit::{AppContext as _, TestAppContext};
    use nocterm_session::{Auth, Reply, SecretRequest, Target};
    use nocterm_vault::{CredentialBinding, VaultService};
    use std::{cell::RefCell, rc::Rc};

    fn prompt(request: SecretRequest) -> Prompt {
        let (reply, _answer) = Reply::channel();
        Prompt::Secret { request, reply }
    }
    #[gpui_kit::test]
    fn retrieves_only_matching_non_retry_credentials_and_ignores_stale_requests(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let service = Arc::new(
            VaultService::new(directory.path().join("vault"), Duration::from_secs(60)).unwrap(),
        );
        block_on(service.create(Secret::new("test unique master passphrase"))).unwrap();
        let target = Target::new("test", "test-host", 22);
        let id = block_on(service.put(
            None,
            "test".into(),
            CredentialBinding::Password {
                target: target.clone(),
            },
            Secret::new("stored account password"),
        ))
        .unwrap();
        let terminal = cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::in_memory(Default::default()),
                cx,
            );
            crate::init_credentials(service.clone(), |_, _, _| Ok(()), cx);
            cx.new(|cx| {
                Terminal::new(
                    SessionSpec {
                        options: Default::default(),
                        title: "test".into(),
                        target: target.clone(),
                        auth: Auth::Password,
                        launch: None,
                        credential: Some(id),
                    },
                    cx,
                )
            })
        });
        let (reply, answer) = Reply::channel();
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.handle_event(
                    Event::Prompt(Prompt::Secret {
                        request: SecretRequest::Password {
                            target: target.clone(),
                            retry: false,
                        },
                        reply,
                    }),
                    cx,
                )
            })
        });
        block_on(service.list()).unwrap();
        cx.run_until_parked();
        assert_eq!(
            block_on(answer).unwrap().unwrap().expose(),
            "stored account password"
        );
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.handle_event(
                    Event::Prompt(prompt(SecretRequest::Password {
                        target: target.clone(),
                        retry: true,
                    })),
                    cx,
                )
            })
        });
        block_on(service.list()).unwrap();
        cx.run_until_parked();
        assert!(
            cx.update(|cx| terminal.read(cx).prompt().is_some()),
            "a rejected credential must not loop automatically"
        );
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.handle_event(
                    Event::Prompt(prompt(SecretRequest::Password {
                        target: Target::new("other", "other-host", 22),
                        retry: false,
                    })),
                    cx,
                )
            })
        });
        block_on(service.list()).unwrap();
        cx.run_until_parked();
        assert!(cx.update(|cx| {
            terminal
                .read(cx)
                .credential_message()
                .unwrap()
                .contains("does not match")
        }));
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.handle_event(
                    Event::Prompt(prompt(SecretRequest::Password {
                        target,
                        retry: false,
                    })),
                    cx,
                );
                terminal.handle_event(
                    Event::Prompt(prompt(SecretRequest::Interactive {
                        prompt: "MFA".into(),
                        echo: false,
                    })),
                    cx,
                );
            })
        });
        block_on(service.list()).unwrap();
        cx.run_until_parked();
        assert!(
            cx.update(|cx| matches!(
                terminal.read(cx).prompt(),
                Some(Prompt::Secret {
                    request: SecretRequest::Interactive { .. },
                    ..
                })
            )),
            "late vault lookup cannot answer a newer MFA prompt"
        );
    }

    #[gpui_kit::test]
    fn remembers_password_only_after_success_and_never_remembers_mfa(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let service = Arc::new(
            VaultService::new(directory.path().join("vault"), Duration::from_secs(60)).unwrap(),
        );
        block_on(service.create(Secret::new("test unique master passphrase"))).unwrap();
        let saved = Rc::new(RefCell::new(Vec::new()));
        let sink = saved.clone();
        let target = Target::new("test", "test-host", 22);
        let terminal = cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::in_memory(Default::default()),
                cx,
            );
            crate::init_credentials(
                service.clone(),
                move |_, id, _| {
                    sink.borrow_mut().push(id);
                    Ok(())
                },
                cx,
            );
            cx.new(|cx| {
                Terminal::new(
                    SessionSpec {
                        options: Default::default(),
                        title: "test".into(),
                        target: target.clone(),
                        auth: Auth::Password,
                        launch: None,
                        credential: None,
                    },
                    cx,
                )
            })
        });
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.handle_event(
                    Event::Prompt(prompt(SecretRequest::Password {
                        target: target.clone(),
                        retry: false,
                    })),
                    cx,
                );
                terminal.answer_secret_and_remember(
                    Secret::new("saved account password"),
                    true,
                    cx,
                );
            })
        });
        assert!(
            block_on(service.list()).unwrap().is_empty(),
            "answering a prompt is not successful authentication"
        );
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.handle_event(
                    Event::Prompt(prompt(SecretRequest::Interactive {
                        prompt: "MFA".into(),
                        echo: false,
                    })),
                    cx,
                );
                terminal.answer_secret_and_remember(Secret::new("123456"), true, cx);
                terminal.handle_event(Event::Connected, cx);
            })
        });
        let records = block_on(service.list()).unwrap();
        cx.run_until_parked();
        assert_eq!(records.len(), 1);
        assert_eq!(saved.borrow().len(), 1);
        let binding = CredentialBinding::Password {
            target: target.clone(),
        };
        assert_eq!(
            block_on(service.get(records[0].id, binding))
                .unwrap()
                .expose(),
            "saved account password"
        );
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.handle_event(
                    Event::Prompt(prompt(SecretRequest::Password {
                        target,
                        retry: false,
                    })),
                    cx,
                );
                terminal.answer_secret_and_remember(Secret::new("wrong-password"), true, cx);
                terminal.handle_event(Event::Closed(CloseReason::ClosedByUser), cx);
                terminal.handle_event(Event::Connected, cx);
            })
        });
        assert_eq!(
            block_on(service.list()).unwrap().len(),
            1,
            "failed attempts must never be saved"
        );
    }
}

#[cfg(test)]
mod recording_tests {
    use super::*;
    use gpui_kit::{AppContext as _, TestAppContext};
    use nocterm_session::{Auth, Command, Reply, SecretRequest, Target};
    #[gpui_kit::test]
    fn oversized_bracketed_paste_is_rejected_without_a_partial_protocol_sequence(
        cx: &mut TestAppContext,
    ) {
        use futures::FutureExt as _;
        let (session, driver) = nocterm_session::channel(None);
        let terminal = cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::in_memory(Default::default()),
                cx,
            );
            cx.new(|cx| {
                let mut terminal = Terminal::new(
                    SessionSpec {
                        options: Default::default(),
                        title: "test".into(),
                        target: Target::new("me", "host", 22),
                        auth: Auth::Password,
                        launch: None,
                        credential: None,
                    },
                    cx,
                );
                terminal.session = Some(session);
                terminal.status = Status::Connected;
                terminal
            })
        });
        let paste = format!("\x1b[200~{}\x1b[201~", "x".repeat(4 * 1024 * 1024));
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.send_text(&paste, cx);
                assert!(terminal.text_error().unwrap().contains("4 MiB"));
            });
        });
        assert!(driver.next_command().now_or_never().is_none());
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.send_text("\x1b[200~small\x1b[201~", cx);
                assert!(terminal.text_error().is_none());
            })
        });
        assert_eq!(
            futures::executor::block_on(driver.next_command()),
            Some(Command::Input(b"\x1b[200~small\x1b[201~".to_vec()))
        );
    }
    #[gpui_kit::test]
    fn automatic_logs_exclude_authentication_and_input_and_drain_on_disconnect(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (session, driver) = nocterm_session::channel(None);
        let terminal = cx.update(|cx| {
            gpui_kit::init(cx);
            let mut settings = nocterm_settings::Settings::default();
            settings.logging.auto_start = true;
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::in_memory(settings),
                cx,
            );
            crate::init_recording(directory.path().into(), cx);
            cx.new(|cx| {
                Terminal::new(
                    SessionSpec {
                        options: Default::default(),
                        title: "test".into(),
                        target: Target::new("me", "host", 22),
                        auth: Auth::Password,
                        launch: None,
                        credential: None,
                    },
                    cx,
                )
            })
        });
        let (reply, answer) = Reply::channel();
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.handle_event(
                    Event::Prompt(Prompt::Secret {
                        request: SecretRequest::Password {
                            target: terminal.spec.target.clone(),
                            retry: false,
                        },
                        reply,
                    }),
                    cx,
                );
                assert!(
                    terminal.recording_status().is_none(),
                    "do not open logs during authentication"
                );
                terminal.answer_secret(Some(Secret::new("never-log-auth-password")), cx);
                terminal.session = Some(session);
                terminal.handle_event(Event::Connected, cx);
                terminal.send_text("never-log-input", cx);
                terminal.handle_event(Event::Output("Привет output\n".as_bytes().to_vec()), cx);
                terminal.handle_event(Event::Closed(CloseReason::ClosedByUser), cx);
            })
        });
        assert_eq!(
            futures::executor::block_on(answer)
                .unwrap()
                .unwrap()
                .expose(),
            "never-log-auth-password"
        );
        assert_eq!(
            futures::executor::block_on(driver.next_command()),
            Some(Command::Input(b"never-log-input".to_vec()))
        );
        for _ in 0..200 {
            if cx.update(|cx| terminal.read(cx).recording_status().unwrap().finished) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let status = cx.update(|cx| terminal.read(cx).recording_status().unwrap());
        assert!(status.finished);
        assert!(status.error.is_none());
        assert_eq!(
            std::fs::read(status.path.unwrap()).unwrap(),
            "Привет output\n".as_bytes()
        );
    }
}
