use super::*;

impl Item for TerminalView {
    fn terminal_access(&self) -> Option<std::rc::Rc<dyn nocterm_workspace::TerminalAccess>> {
        Some(std::rc::Rc::new(crate::access::Access(
            self.terminal.downgrade(),
        )))
    }
    fn command_enabled(&self, command: ItemCommand, cx: &App) -> bool {
        let terminal = self.terminal.read(cx);
        match command {
            ItemCommand::Copy | ItemCommand::FindNextSelection => {
                terminal.emulator().selection_text().is_some()
            }
            ItemCommand::ClearSelection => terminal.emulator().selection_start().is_some(),
            ItemCommand::Paste => terminal.is_connected(),
            ItemCommand::Disconnect => !matches!(terminal.status(), Status::Closed(_)),
            ItemCommand::StartRecording => {
                !terminal.is_command() && terminal.is_connected() && !terminal.is_recording()
            }
            ItemCommand::Reconnect => !terminal.is_command(),
            ItemCommand::StopRecording => terminal.is_recording(),
            ItemCommand::FindNext | ItemCommand::FindPrevious => {
                !terminal.find().query.is_empty() && terminal.find().error.is_none()
            }
            ItemCommand::SelectAll | ItemCommand::Find | ItemCommand::SessionSettings => true,
        }
    }

    fn execute(&mut self, command: ItemCommand, window: &mut Window, cx: &mut Context<Self>) {
        if !self.command_enabled(command, cx) {
            return;
        }
        match command {
            ItemCommand::Copy => self.copy_selection(cx),
            ItemCommand::Paste => self.paste(&Paste, window, cx),
            ItemCommand::SelectAll => self
                .terminal
                .update(cx, |t, cx| t.update_emulator(cx, |e| e.select_all())),
            ItemCommand::ClearSelection => self
                .terminal
                .update(cx, |t, cx| t.update_emulator(cx, |e| e.clear_selection())),
            ItemCommand::Find => self.open_find(None, false, window, cx),
            ItemCommand::FindNextSelection => {
                let text = self.terminal.read(cx).emulator().selection_text();
                self.open_find(text, true, window, cx);
            }
            ItemCommand::FindNext | ItemCommand::FindPrevious => {
                self.terminal.update(cx, |t, cx| {
                    t.find_next(command == ItemCommand::FindPrevious, cx)
                })
            }
            ItemCommand::Disconnect => self.terminal.update(cx, |t, cx| t.disconnect(cx)),
            ItemCommand::Reconnect => self.reconnect(&Reconnect, window, cx),
            ItemCommand::StartRecording => self.terminal.update(cx, |t, cx| t.start_recording(cx)),
            ItemCommand::StopRecording => self.terminal.update(cx, |t, cx| {
                t.stop_recording();
                cx.emit(TerminalEvent::Changed);
            }),
            ItemCommand::SessionSettings => {
                crate::session_settings::open(self.terminal.clone(), window, cx)
            }
        }
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
