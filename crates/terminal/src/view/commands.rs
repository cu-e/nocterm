//! The workspace commands a terminal answers. Each handler is registered only
//! while it can run, so menus read availability from the dispatch tree.
use super::*;
use gpui_kit::Div;
use nocterm_workspace::{
    ClearSelection, DisconnectSession, Find, FindNext, FindNextSelection, FindPrevious,
    ReconnectSession, SessionSettings, StartRecording, StopRecording,
};

impl TerminalView {
    /// Commands for the whole view, so they also reach it from the Find bar.
    pub(super) fn on_commands(&self, element: Div, cx: &mut Context<Self>) -> Div {
        let terminal = self.terminal.read(cx);
        let selected = terminal.emulator().selection_text().is_some();
        let selecting = terminal.emulator().selection_start().is_some();
        let closed = matches!(terminal.status(), Status::Closed(_));
        let command = terminal.is_command();
        let connected = terminal.is_connected();
        let recording = terminal.is_recording();
        let searchable = !terminal.find().query.is_empty() && terminal.find().error.is_none();
        element
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::session_settings))
            .when(selected, |el| {
                el.on_action(cx.listener(Self::find_selection))
            })
            .when(selecting, |el| {
                el.on_action(cx.listener(Self::clear_selection))
            })
            .when(searchable, |el| {
                el.on_action(cx.listener(Self::find_next))
                    .on_action(cx.listener(Self::find_previous))
            })
            .when(!closed, |el| el.on_action(cx.listener(Self::disconnect)))
            .when(!command, |el| {
                el.on_action(cx.listener(|this, _: &ReconnectSession, window, cx| {
                    this.reconnect(&Reconnect, window, cx)
                }))
            })
            .when(!command && connected && !recording, |el| {
                el.on_action(cx.listener(Self::start_recording))
            })
            .when(recording, |el| {
                el.on_action(cx.listener(Self::stop_recording))
            })
    }

    /// The edit commands of the screen, which native text fields also answer.
    pub(super) fn on_edit_commands(&self, element: Div, cx: &mut Context<Self>) -> Div {
        let terminal = self.terminal.read(cx);
        let selected = terminal.emulator().selection_text().is_some();
        let connected = terminal.is_connected();
        element
            .on_action(cx.listener(|this, _: &native_input::SelectAll, _, cx| {
                cx.stop_propagation();
                this.terminal
                    .update(cx, |t, cx| t.update_emulator(cx, |e| e.select_all()));
            }))
            .when(selected, |el| {
                el.on_action(cx.listener(|this, _: &native_input::Copy, _, cx| {
                    cx.stop_propagation();
                    this.copy_selection(cx);
                }))
            })
            .when(connected, |el| {
                el.on_action(cx.listener(|this, _: &native_input::Paste, window, cx| {
                    cx.stop_propagation();
                    this.paste(&Paste, window, cx);
                }))
            })
    }

    /// Menus restore their own focus after they dispatch, so commands that
    /// move focus take it on the next turn.
    pub(super) fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        cx.defer_in(window, |this, window, cx| {
            this.open_find(None, false, window, cx)
        });
    }

    pub(super) fn find_selection(
        &mut self,
        _: &FindNextSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.defer_in(window, |this, window, cx| {
            let text = this.terminal.read(cx).emulator().selection_text();
            if text.is_some() {
                this.open_find(text, true, window, cx);
            }
        });
    }

    fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| t.find_next(false, cx));
    }

    fn find_previous(&mut self, _: &FindPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| t.find_next(true, cx));
    }

    pub(super) fn clear_selection(
        &mut self,
        _: &ClearSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.terminal
            .update(cx, |t, cx| t.update_emulator(cx, |e| e.clear_selection()));
    }

    pub(super) fn disconnect(
        &mut self,
        _: &DisconnectSession,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.terminal.update(cx, |t, cx| t.disconnect(cx));
    }

    fn start_recording(&mut self, _: &StartRecording, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| t.start_recording(cx));
    }

    fn stop_recording(&mut self, _: &StopRecording, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| {
            t.stop_recording();
            cx.emit(TerminalEvent::Changed);
        });
    }

    fn session_settings(
        &mut self,
        _: &SessionSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let terminal = self.terminal.clone();
        window.defer(cx, move |window, cx| {
            crate::session_settings::open(terminal, window, cx)
        });
    }
}

impl Item for TerminalView {
    fn terminal_access(&self) -> Option<std::rc::Rc<dyn nocterm_workspace::TerminalAccess>> {
        Some(std::rc::Rc::new(crate::access::Access(
            self.terminal.downgrade(),
        )))
    }

    fn tab_title(&self, cx: &App) -> SharedString {
        self.terminal.read(cx).spec().title.clone()
    }

    fn tab_state(&self, cx: &App) -> TabState {
        let terminal = self.terminal.read(cx);
        if terminal.prompt().is_some() {
            return TabState::Attention;
        }
        match terminal.status() {
            Status::Connecting(_) => TabState::Busy,
            Status::Connected => TabState::Idle,
            Status::Closed(_) => TabState::Ended,
        }
    }

    fn session(&self, cx: &App) -> Option<SessionContext> {
        let terminal = self.terminal.read(cx);
        (!terminal.is_local()).then(|| terminal.session_context())
    }

    fn session_spec(&self, cx: &App) -> Option<SessionSpec> {
        let terminal = self.terminal.read(cx);
        (!terminal.is_local() && !terminal.is_command()).then(|| terminal.spec().clone())
    }

    fn on_close(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.disconnect(cx));
    }
}
