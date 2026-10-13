mod input;
mod lifecycle;
mod output;
mod presentation;

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::{Context, EventEmitter, SharedString, Subscription, Task};
use nocterm_session::{
    CloseReason, ConnectRequest, ConnectStage, Event, Prompt, PtySize, SessionError, Transport,
};
use nocterm_session::{LocalShellSettings, SshSettings};
#[cfg(test)]
use nocterm_session::{Secret, Session};
#[cfg(test)]
use nocterm_ui::SettingsStore;
use nocterm_ui::{CursorShape, TerminalSettings};
use nocterm_vt::{
    Effect, Emulator, EmulatorOptions, Palette, Scroll, SearchDirection, SearchOptions,
    SearchPoint, SearchProgress, SearchResult, TermSize,
};
use nocterm_workspace::SessionSpec;

use crate::Installed;

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
    deps: Installed,
    lifecycle: lifecycle::SessionLifecycle,
    input: input::TerminalInput,
    recording: crate::OutputRecording,
    credentials: crate::credentials::CredentialState,
    screen: presentation::TerminalScreen,
    _settings: Subscription,
}

impl EventEmitter<TerminalEvent> for Terminal {}

impl Terminal {
    /// A terminal that starts connecting to `spec` at once.
    pub fn new(deps: Installed, spec: SessionSpec, cx: &mut Context<Self>) -> Self {
        Self::new_kind(deps, spec, false, None, None, cx)
    }

    pub fn new_local(deps: Installed, cx: &mut Context<Self>) -> Self {
        Self::new_kind(
            deps,
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
        deps: Installed,
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
        Self::new_kind(
            deps,
            spec,
            true,
            Some(transport),
            Some(Box::new(completion)),
            cx,
        )
    }

    fn new_kind(
        deps: Installed,
        spec: SessionSpec,
        local: bool,
        local_transport: Option<Arc<dyn Transport>>,
        command_completion: Option<CommandCompletion>,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = emulator_options(deps.settings().setting::<nocterm_ui::TerminalSettings>(cx));
        let weak = cx.entity().downgrade();
        let mut this = Self {
            deps: deps.clone(),
            lifecycle: lifecycle::SessionLifecycle::new(
                spec,
                local,
                local_transport,
                command_completion,
            ),
            input: input::TerminalInput::new(
                deps.settings()
                    .setting::<nocterm_ui::TerminalSettings>(cx)
                    .charset,
            ),
            recording: crate::OutputRecording::default(),
            credentials: crate::credentials::CredentialState::default(),
            screen: presentation::TerminalScreen::new(options),
            _settings: deps.settings().observe(cx, move |cx| {
                if let Some(terminal) = weak.upgrade() {
                    terminal.update(cx, |this, cx| {
                        let options = emulator_options(
                            this.deps
                                .settings()
                                .setting::<nocterm_ui::TerminalSettings>(cx),
                        );
                        this.screen.apply_options(options);
                        this.refresh_find(cx);
                        this.emit_output(cx);
                    });
                }
            }),
        };
        this.connect(cx);
        this
    }

    pub(crate) fn installed(&self) -> &Installed {
        &self.deps
    }

    pub(crate) fn set_credential_id(&mut self, id: nocterm_session::CredentialId) {
        self.lifecycle.set_credential(id);
    }

    pub fn spec(&self) -> &SessionSpec {
        self.lifecycle.spec()
    }

    /// Changes this tab's next-launch options without changing a saved profile.
    pub fn set_session_options(
        &mut self,
        options: nocterm_session::SessionOptions,
        cx: &mut Context<Self>,
    ) -> Result<(), nocterm_session::SessionOptionsError> {
        self.lifecycle.apply_options(options)?;
        cx.emit(TerminalEvent::Changed);
        Ok(())
    }

    pub fn status(&self) -> &Status {
        self.lifecycle.status()
    }

    /// The question the session is waiting on, if any.
    pub fn prompt(&self) -> Option<&Prompt> {
        self.lifecycle.prompt()
    }

    pub(crate) fn connection_epoch(&self) -> u64 {
        self.lifecycle.epoch()
    }

    pub(crate) fn prompt_epoch(&self) -> u64 {
        self.lifecycle.prompt_epoch()
    }

    pub fn program_title(&self) -> Option<&str> {
        self.screen.title()
    }

    pub fn emulator(&self) -> &Emulator {
        self.screen.emulator()
    }

    pub fn is_connected(&self) -> bool {
        self.status() == &Status::Connected
    }

    // ── Session ──────────────────────────────────────────────────────────────

    /// Opens a new session, replacing the current one.
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn connect(&mut self, cx: &mut Context<Self>) {
        self.close(cx);
        if let Err(error) = self.lifecycle.spec().options.validate() {
            self.set_status(
                Status::Closed(CloseReason::Failed(SessionError::InvalidOptions(error))),
                cx,
            );
            return;
        }
        self.input.reset_charset(
            self.lifecycle
                .spec()
                .options
                .charset
                .unwrap_or(self.deps.settings().setting::<TerminalSettings>(cx).charset),
        );
        let mut recording_options = self
            .lifecycle
            .spec()
            .options
            .logging
            .clone()
            .unwrap_or_else(|| {
                self.deps
                    .settings()
                    .setting::<nocterm_session::LoggingOptions>(cx)
                    .clone()
            });
        if self.lifecycle.is_local() {
            recording_options.auto_start = false;
        }
        self.recording.set_options(recording_options);
        self.input.clear_errors();
        let launch = if self.lifecycle.is_local() {
            self.lifecycle.spec().launch.clone().unwrap_or_else(|| {
                shell_launch(self.deps.settings().setting::<LocalShellSettings>(cx))
            })
        } else {
            self.lifecycle.spec().launch.clone().unwrap_or_else(|| {
                shell_launch(&self.deps.settings().setting::<SshSettings>(cx).launch)
            })
        };
        let shell_program = launch.program.clone().unwrap_or_else(|| {
            if self.lifecycle.is_local() {
                self.deps.default_program().to_owned()
            } else {
                String::new()
            }
        });
        self.input
            .set_shell(shell_program, self.lifecycle.is_local());
        let transport = match self.lifecycle.transport() {
            Some(transport) => Some(transport),
            None if self.lifecycle.is_local() => self.deps.local_transport(launch.clone()),
            None => Some(self.deps.remote()),
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
        let ssh = self.deps.settings().setting::<SshSettings>(cx).clone();
        let request = ConnectRequest {
            proxy: self
                .lifecycle
                .spec()
                .options
                .proxy
                .clone()
                .unwrap_or_else(|| ssh.proxy.clone()),
            launch,
            target: self.lifecycle.spec().target.clone(),
            auth: self.lifecycle.spec().auth.clone(),
            term: self
                .lifecycle
                .spec()
                .options
                .term
                .clone()
                .unwrap_or_else(|| {
                    self.deps
                        .settings()
                        .setting::<TerminalSettings>(cx)
                        .term
                        .clone()
                }),
            size: pty_size(self.emulator().size()),
            connect_timeout: Duration::from_secs(ssh.connect_timeout_secs.into()),
            keepalive_interval: Some(ssh.keepalive_interval_secs)
                .filter(|secs| *secs > 0)
                .map(|secs| Duration::from_secs(secs.into())),
        };

        let session = transport.open(request);
        self.lifecycle.accept_session(session.clone());
        self.set_status(Status::Connecting(ConnectStage::Connecting), cx);
        cx.emit(TerminalEvent::Changed);

        let pump = pump::spawn(session, self.connection_epoch(), cx);
        self.lifecycle.install_pump(pump);
    }

    /// Replaces a live connection as well as reconnecting a closed one.
    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        if self.is_command() {
            return;
        }
        self.screen.advance(b"\r\n");
        self.connect(cx);
        self.refresh_find(cx);
    }

    fn close_session(&mut self) {
        if let Some(session) = self.lifecycle.retire_session() {
            session.close();
        }
        self.clear_credentials();
    }

    /// Ends the session, invalidating authority before external cleanup.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        let session = self.lifecycle.close();
        self.input.revoke();
        self.screen.stop_sync();
        self.clear_credentials();
        if let Some(session) = session {
            session.close();
        }
        self.stop_recording(cx);
    }

    /// Immediately publishes disconnection, including a cancelled handshake.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.close(cx);
        self.set_status(Status::Closed(CloseReason::ClosedByUser), cx);
        cx.emit(TerminalEvent::Changed);
    }

    pub(crate) fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Connecting(stage) => self.set_status(Status::Connecting(stage), cx),
            Event::Connected => {
                self.set_status(Status::Connected, cx);
                self.save_authenticated_credential(cx);
                if self.recording.options().auto_start {
                    self.start_recording(cx);
                }
            }
            Event::Output(bytes) => self.advance_output(&bytes, cx),
            Event::Prompt(prompt) => {
                // A newer question supersedes an unanswered one, including an
                // identical retry. Views must discard the previous input state.
                self.lifecycle.accept_prompt(prompt);
                self.retrieve_credential(cx);
                cx.emit(TerminalEvent::Changed);
            }
            Event::Closed(reason) => {
                let tail = self.input.decode(&[], true);
                self.recording.output(&tail, cx);
                self.recording.stop(cx);
                if !tail.is_empty() {
                    let effects = self.screen.advance(&tail);
                    self.apply(effects, cx);
                }
                self.clear_credentials();
                self.lifecycle.finish();
                self.refresh_find(cx);
                self.set_status(Status::Closed(reason), cx);
            }
        }
    }

    fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        let (changed, completion) = self.lifecycle.set_status(status);
        if let Some((callback, reason)) = completion {
            callback(reason, cx);
        }
        if changed {
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
                    if self.screen.set_title(title) {
                        cx.emit(TerminalEvent::Changed);
                    }
                }
                Effect::Bell => cx.emit(TerminalEvent::Bell),
                Effect::CopyToClipboard(text) => {
                    if text.len() <= 1024 * 1024
                        && self
                            .deps
                            .settings()
                            .setting::<nocterm_ui::TerminalSettings>(cx)
                            .clipboard_write
                            == nocterm_ui::ClipboardWritePolicy::FocusedTerminal
                    {
                        cx.emit(TerminalEvent::ClipboardWrite(text));
                    }
                }
            }
        }
    }

    fn schedule_sync(&mut self, cx: &mut Context<Self>) {
        self.screen.schedule_sync(cx);
    }
    fn finish_sync(&mut self, cx: &mut Context<Self>) {
        if let Some(effects) = self.screen.finish_sync() {
            self.apply(effects, cx);
            self.refresh_find(cx);
            self.emit_output(cx);
        } else {
            self.schedule_sync(cx);
        }
    }

    /// Whether this model owns a local PTY.
    pub fn is_local(&self) -> bool {
        self.lifecycle.is_local()
    }

    /// Directory announced by OSC 7, never guessed from prompt text.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.input.cwd()
    }

    pub fn text_error(&self) -> Option<String> {
        self.input.text_error()
    }
    pub fn recording_status(&self) -> Option<crate::RecordingStatus> {
        self.recording.status()
    }
    pub fn is_recording(&self) -> bool {
        self.recording.is_active()
    }
    pub fn stop_recording(&mut self, cx: &mut Context<Self>) {
        self.recording.stop(cx);
        cx.emit(TerminalEvent::Changed);
    }
    pub fn toggle_recording(&mut self, cx: &mut Context<Self>) {
        if self.is_recording() {
            self.stop_recording(cx);
        } else {
            self.start_recording(cx);
        }
    }
    pub fn start_recording(&mut self, cx: &mut Context<Self>) {
        if self.lifecycle.is_command() || self.is_recording() {
            return;
        }
        if !self.is_connected() {
            return;
        }
        if let Some(registry) = self.deps.recording() {
            self.recording.set_registry(registry);
        }
        if let Err(error) = self.recording.start(cx) {
            self.input
                .set_text_error(Some(format!("Could not start recording: {error}")));
            cx.emit(TerminalEvent::Changed);
        } else {
            cx.emit(TerminalEvent::Changed);
        }
    }

    /// Lays the grid out anew, and tells the remote program.
    pub fn resize(&mut self, size: TermSize, cx: &mut Context<Self>) {
        if !self.screen.resize(size) {
            return;
        }
        self.refresh_find(cx);
        if let Some(session) = self.lifecycle.session() {
            session.resize(pty_size(size));
        }
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.screen.set_palette(palette);
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.close_session();
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

fn shell_launch(settings: &nocterm_session::ShellSettings) -> nocterm_session::ShellLaunch {
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
