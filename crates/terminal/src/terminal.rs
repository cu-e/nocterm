mod find;
mod input;
mod output;
mod presentation;

use std::{
    cell::RefCell,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::{Context, EventEmitter, SharedString, Subscription, Task};
use nocterm_session::{
    CloseReason, ConnectRequest, ConnectStage, Event, Prompt, PtySize, RemoteFs, Secret, Session,
    SessionError, Transport,
};
use nocterm_settings::{CursorShape, TerminalSettings};
use nocterm_ui::{ActiveSettings as _, SettingsStore};
use nocterm_vt::{
    Effect, Emulator, EmulatorOptions, Palette, Scroll, SearchDirection, SearchOptions,
    SearchPoint, SearchProgress, SearchResult, TermSize,
};
use nocterm_workspace::SessionSpec;

use crate::{ActiveTransport, LocalTransportFactory, integration::ShellIntegration};

/// Where a terminal's session stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Connecting(ConnectStage),
    Connected,
    Closed(CloseReason),
}

/// What a [`Terminal`] tells its observers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    /// The status, the pending prompt or the title changed.
    Changed,
    /// The screen changed.
    Output,
    /// The program rang the bell.
    Bell,
    /// A program requested a clipboard write; the view applies permission and focus checks.
    ClipboardWrite(String),
}

#[derive(Default)]
pub struct FindState {
    pub query: String,
    pub options: SearchOptions,
    pub searching: bool,
    /// Counts are final when `searching` is false; active may be an early preview.
    pub result: SearchResult,
    pub error: Option<String>,
}

type CommandCompletion = Box<dyn FnOnce(CloseReason, &mut gpui_kit::App)>;

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
    local_transport: Option<Arc<dyn Transport>>,
    command_completion: Option<CommandCompletion>,
    integration: RefCell<ShellIntegration>,
    agent_input: RefCell<input::AgentInput>,
    shell_program: String,
    shell_syntax: Option<nocterm_workspace::ShellSyntax>,
    session: Option<Session>,
    fs: Option<Arc<dyn RemoteFs>>,
    status: Status,
    prompt: Option<Prompt>,
    prompt_epoch: u64,
    connection_epoch: u64,
    emulator: Emulator,
    display_revision: u64,
    find: FindState,
    find_task: Option<Task<()>>,
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
        Self::new_kind(spec, false, None, None, cx)
    }

    pub fn new_local(cx: &mut Context<Self>) -> Self {
        Self::new_kind(
            SessionSpec {
                profile: None,
                options: Default::default(),
                title: "Local terminal".into(),
                target: nocterm_session::Target::new("local", "localhost", 22),
                auth: nocterm_session::Auth::Auto,
                launch: None,
                credential: None,
            },
            true,
            None,
            None,
            cx,
        )
    }

    pub fn new_local_command(
        launch: nocterm_session::ShellLaunch,
        title: SharedString,
        transport: Arc<dyn Transport>,
        completion: impl FnOnce(CloseReason, &mut gpui_kit::App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let spec = SessionSpec {
            profile: None,
            options: Default::default(),
            title,
            target: nocterm_session::Target::new("local", "localhost", 22),
            auth: nocterm_session::Auth::Auto,
            launch: Some(launch),
            credential: None,
        };
        Self::new_kind(spec, true, Some(transport), Some(Box::new(completion)), cx)
    }

    fn new_kind(
        spec: SessionSpec,
        local: bool,
        local_transport: Option<Arc<dyn Transport>>,
        command_completion: Option<CommandCompletion>,
        cx: &mut Context<Self>,
    ) -> Self {
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
            local_transport,
            command_completion,
            integration: RefCell::default(),
            agent_input: RefCell::default(),
            shell_program: String::new(),
            shell_syntax: None,
            session: None,
            fs: None,
            status: Status::Connecting(ConnectStage::Connecting),
            prompt: None,
            prompt_epoch: 0,
            connection_epoch: 0,
            emulator: Emulator::new(TermSize::default(), options),
            display_revision: 0,
            find: FindState::default(),
            find_task: None,
            program_title: None,
            _pump: None,
            sync_timer: None,
            _settings: cx.observe_global::<SettingsStore>(|this, cx| {
                let options = emulator_options(&cx.settings().terminal);
                this.emulator.set_options(options);
                this.refresh_find(cx);
                this.emit_output(cx);
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

    /// Changes this tab's next-launch options without changing a saved profile.
    pub fn set_session_options(
        &mut self,
        options: nocterm_session::SessionOptions,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        options.validate()?;
        self.spec.options = options;
        cx.emit(TerminalEvent::Changed);
        Ok(())
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

    // ── Session ──────────────────────────────────────────────────────────────

    /// Opens a new session, replacing the current one.
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn connect(&mut self, cx: &mut Context<Self>) {
        self.close();
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
        if self.local_transport.is_some() {
            self.recording_options.auto_start = false;
        }
        self.text_error = None;
        *self.input_error.borrow_mut() = None;
        let launch = if self.local {
            self.spec
                .launch
                .clone()
                .unwrap_or_else(|| shell_launch(&settings.local))
        } else {
            self.spec
                .launch
                .clone()
                .unwrap_or_else(|| shell_launch(&settings.ssh.launch))
        };
        self.shell_program = launch.program.clone().unwrap_or_else(|| {
            if self.local {
                std::env::var(if cfg!(windows) { "COMSPEC" } else { "SHELL" }).unwrap_or_default()
            } else {
                String::new()
            }
        });
        if self.local {
            *self.integration.borrow_mut() = ShellIntegration::default();
        }
        let transport = match &self.local_transport {
            Some(transport) => Some(transport.clone()),
            None if self.local => cx
                .try_global::<LocalTransportFactory>()
                .map(|factory| (factory.0)(launch.clone())),
            None => cx.try_global::<ActiveTransport>().map(|t| t.0.clone()),
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
        cx.emit(TerminalEvent::Changed);

        self.spawn_pump(session, cx);
    }

    /// Replaces a live connection as well as reconnecting a closed one.
    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        if self.is_command() {
            return;
        }
        self.emulator.advance(b"\r\n");
        self.connect(cx);
        self.refresh_find(cx);
    }

    /// Ends the session.
    pub fn close(&mut self) {
        self.agent_input.borrow_mut().revoke();
        *self.integration.borrow_mut() = ShellIntegration::default();
        self.connection_epoch = self.connection_epoch.wrapping_add(1);
        self._pump = None;
        self.sync_timer = None;
        if let Some(session) = self.session.take() {
            session.close();
        }
        self.fs = None;
        self.prompt = None;
        self.prompt_epoch = self.prompt_epoch.wrapping_add(1);
        self.clear_credentials();
        self.stop_recording();
    }

    /// Immediately publishes disconnection, including a cancelled handshake.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.close();
        self.set_status(Status::Closed(CloseReason::ClosedByUser), cx);
        cx.emit(TerminalEvent::Changed);
    }

    pub(crate) fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Connecting(stage) => self.set_status(Status::Connecting(stage), cx),
            Event::Connected => {
                self.set_status(Status::Connected, cx);
                self.save_authenticated_credential(cx);
                if self.recording_options.auto_start {
                    self.start_recording(cx);
                }
            }
            Event::Output(bytes) => self.advance_output(&bytes, cx),
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
                self.fs = None;
                self.refresh_find(cx);
                self.set_status(Status::Closed(reason), cx);
            }
        }
    }

    fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        if let Status::Closed(reason) = &status
            && let Some(completion) = self.command_completion.take()
        {
            completion(reason.clone(), cx);
        }
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
                    self.send_protocol(bytes);
                }
                Effect::Title(title) => {
                    if self.program_title != title {
                        self.program_title = title;
                        cx.emit(TerminalEvent::Changed);
                    }
                }
                Effect::Bell => cx.emit(TerminalEvent::Bell),
                Effect::CopyToClipboard(text) => {
                    if text.len() <= 1024 * 1024
                        && cx.settings().terminal.clipboard_write
                            == nocterm_settings::ClipboardWritePolicy::FocusedTerminal
                    {
                        cx.emit(TerminalEvent::ClipboardWrite(text));
                    }
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
                        this.refresh_find(cx);
                        this.emit_output(cx);
                    }
                    // The program ended it, or began another one since.
                    _ => this.schedule_sync(cx),
                }
            });
        }));
    }

    /// Whether this model owns a local PTY.
    pub fn is_local(&self) -> bool {
        self.local
    }

    /// Directory announced by OSC 7, never guessed from prompt text.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.integration.borrow().cwd.clone()
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
    pub fn start_recording(&mut self, cx: &mut Context<Self>) {
        if self.local_transport.is_some() || self.is_recording() {
            return;
        }
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

    /// Lays the grid out anew, and tells the remote program.
    pub fn resize(&mut self, size: TermSize, cx: &mut Context<Self>) {
        if size == self.emulator.size() {
            return;
        }
        self.emulator.resize(size);
        self.refresh_find(cx);
        if let Some(session) = &self.session {
            session.resize(pty_size(size));
        }
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.emulator.set_palette(palette);
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

mod program;
mod pump;

#[cfg(test)]
mod credential_tests;
#[cfg(test)]
mod local_tests;
#[cfg(test)]
mod recording_tests;

mod prompts;
