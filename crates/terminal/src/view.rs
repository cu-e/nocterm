use std::{
    ops::Range,
    time::{Duration, Instant},
};

use gpui_kit::{
    AnyElement, App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollWheelEvent, SharedString, Subscription, Task,
    UTF16Selection, Window,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{self as native_input, Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_session::{CloseReason, ConnectStage, HostKeyDecision, Prompt, Secret, SecretRequest};
use nocterm_ui::{ActiveDesign as _, IconName, TerminalStyle};
use nocterm_vt::{
    CellPoint, Frame, KeyPress, Modifiers, MouseEvent, MouseEventKind, Palette, Rgb, Scroll,
    SearchDirection, SelectionKind, encode_focus, encode_key, encode_mouse, encode_paste,
};
use nocterm_workspace::{Item, ItemEvent, SessionContext, SessionSpec, TabState};

use crate::{
    Copy, KEY_CONTEXT, Paste, Reconnect, SCREEN_KEY_CONTEXT, ScrollPageDown, ScrollPageUp,
    ScrollToBottom, ScrollToTop, Status, Terminal, TerminalEvent, ToggleRecording,
    element::{Geometry, TerminalElement},
};

/// How long a blinking cursor stays lit, and dark.
const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// Most wheel reports sent for one scroll event, so a fling cannot flood a
/// slow link.
const MAX_WHEEL_REPORTS: i32 = 10;

mod commands;
mod cursor;
mod input;
mod keyboard;
mod paste;
mod pointer;
mod render;
#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;

/// A terminal tab.
pub struct TerminalView {
    terminal: Entity<Terminal>,
    input: input::ViewInput,
    focus_handle: FocusHandle,
    cache: crate::element::FrameCache,
    cursor: cursor::CursorClock,
    pointer: pointer::PointerState,
    secret: Option<SecretField>,
    find: Option<FindField>,
    notice_messages: [Option<String>; 3],
    _subscriptions: Vec<Subscription>,
    capabilities: nocterm_workspace::TerminalCapabilities,
}

struct SecretField {
    prompt_epoch: u64,
    remember: bool,
    input: Entity<InputState>,
    _subscription: Subscription,
}

struct FindField {
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl EventEmitter<ItemEvent> for TerminalView {}

impl TerminalView {
    pub fn new(
        deps: crate::Installed,
        spec: SessionSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let terminal = cx.new(|cx| Terminal::new(deps, spec, cx));
        Self::with_terminal(terminal, window, cx)
    }

    pub fn new_local(deps: crate::Installed, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let terminal = cx.new(|cx| Terminal::new_local(deps, cx));
        Self::with_terminal(terminal, window, cx)
    }

    pub fn new_local_command(
        deps: crate::Installed,
        launch: nocterm_session::ShellLaunch,
        title: String,
        transport: std::sync::Arc<dyn nocterm_session::Transport>,
        completion: impl FnOnce(nocterm_session::CloseReason, &mut gpui_kit::App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let terminal = cx.new(|cx| {
            Terminal::new_local_command(deps, launch, title.into(), transport, completion, cx)
        });
        Self::with_terminal(terminal, window, cx)
    }

    pub(crate) fn with_terminal(
        terminal: Entity<Terminal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let settings = terminal.read(cx).installed().settings().clone();
        let weak = cx.entity().downgrade();
        let origin = if terminal.read(cx).installed().is_container() {
            crate::access::AccessOrigin::WorkspaceResource
        } else if terminal.read(cx).is_command() {
            crate::access::AccessOrigin::GeneralCommand
        } else {
            crate::access::AccessOrigin::GeneralConnection
        };
        let capabilities = crate::access::Access::capabilities(
            std::rc::Rc::new(crate::access::Access::new(
                cx.weak_entity(),
                terminal.downgrade(),
            )),
            origin,
        );

        let subscriptions = vec![
            cx.subscribe_in(&terminal, window, Self::on_terminal_event),
            cx.observe_keystrokes(Self::on_keyboard_binding),
            settings.observe(cx, move |cx| {
                if let Some(view) = weak.upgrade() {
                    view.update(cx, |this, cx| {
                        if !this
                            .terminal
                            .read(cx)
                            .installed()
                            .settings()
                            .setting::<nocterm_ui::TerminalSettings>(cx)
                            .semantic_highlighting
                        {
                            this.cache.clear_highlights();
                        }
                        cx.notify();
                    });
                }
            }),
            cx.on_focus_in(&focus_handle, window, |this, _, cx| {
                this.focus_changed(true, cx)
            }),
            cx.on_focus_out(&focus_handle, window, |this, _, _, cx| {
                this.focus_changed(false, cx);
            }),
        ];

        Self {
            terminal,
            input: Default::default(),
            focus_handle,
            cache: Default::default(),
            cursor: cursor::CursorClock::new(cx),
            pointer: Default::default(),
            secret: None,
            find: None,
            notice_messages: Default::default(),
            _subscriptions: subscriptions,
            capabilities,
        }
    }

    pub fn terminal(&self) -> &Entity<Terminal> {
        &self.terminal
    }

    pub(crate) fn painted_frame(&self) -> std::cell::Ref<'_, Frame> {
        self.cache.frame()
    }

    #[cfg(test)]
    pub(crate) fn painted_highlights(&self) -> std::cell::Ref<'_, crate::highlighting::Highlights> {
        self.cache.highlights()
    }

    pub(crate) fn painted_geometry(&self) -> Option<Geometry> {
        self.cache.geometry()
    }

    pub(crate) fn terminal_style(&self, cx: &App) -> TerminalStyle {
        TerminalStyle::with_settings(
            self.terminal
                .read(cx)
                .installed()
                .settings()
                .setting::<nocterm_ui::TerminalSettings>(cx),
            cx,
        )
    }

    pub(crate) fn paint_element(
        &self,
        view: Entity<Self>,
        style: TerminalStyle,
        focused: bool,
    ) -> TerminalElement {
        TerminalElement::new(
            view,
            self.terminal.clone(),
            self.focus_handle.clone(),
            style,
            crate::element::PaintState::new(
                focused,
                self.cursor_visible(),
                self.input.composition().map(SharedString::from),
            ),
            self.cache.clone(),
        )
    }

    fn on_terminal_event(
        &mut self,
        _: &Entity<Terminal>,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_keyboard(cx);
        self.sync_pointer(cx);
        match event {
            TerminalEvent::Changed => {
                self.cache.invalidate();
                self.sync_secret_field(window, cx);
                self.sync_operational_notices(window, cx);
                cx.emit(ItemEvent::Changed);
                cx.notify();
            }
            TerminalEvent::Output => {
                self.cache.invalidate();
                self.sync_operational_notices(window, cx);
                cx.notify();
            }
            // A visual or audible bell is a setting still to come.
            TerminalEvent::Bell => {}
            TerminalEvent::ClipboardWrite(text) => {
                if self.focus_handle.is_focused(window)
                    && cx.active_window() == Some(window.window_handle())
                    && self
                        .terminal
                        .read(cx)
                        .installed()
                        .settings()
                        .setting::<nocterm_ui::TerminalSettings>(cx)
                        .clipboard_write
                        == nocterm_ui::ClipboardWritePolicy::FocusedTerminal
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                }
            }
        }
    }

    fn sync_operational_notices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let terminal = self.terminal.read(cx);
        let messages = [
            terminal
                .is_connected()
                .then(|| terminal.credential_message().map(str::to_owned))
                .flatten(),
            terminal.text_error(),
            terminal
                .recording_status()
                .and_then(|status| status.error().map(str::to_owned)),
        ];
        for (index, message) in messages.into_iter().enumerate() {
            if self.notice_messages[index] == message {
                continue;
            }
            self.notice_messages[index] = message.clone();
            let Some(message) = message else { continue };
            match index {
                0 => {
                    let terminal = self.terminal.downgrade();
                    nocterm_ui::notice::warning_action(
                        window,
                        cx,
                        "terminal-credential",
                        "Credential storage",
                        message,
                        "Dismiss",
                        move |_, cx| {
                            if let Some(terminal) = terminal.upgrade() {
                                terminal.update(cx, |terminal, cx| {
                                    terminal.clear_credential_message(cx)
                                });
                            }
                        },
                    );
                }
                1 => nocterm_ui::notice::error(
                    window,
                    cx,
                    "terminal-input",
                    "Terminal input",
                    message,
                ),
                _ => nocterm_ui::notice::error(
                    window,
                    cx,
                    "terminal-recording",
                    "Session recording",
                    message,
                ),
            }
        }
    }

    fn focus_changed(&mut self, focused: bool, cx: &mut Context<Self>) {
        self.input.focus_changed(focused);
        if !focused {
            self.pointer.reset();
        }
        let terminal = self.terminal.read(cx);
        if let Some(report) = encode_focus(focused, terminal.emulator().modes()) {
            terminal.send_protocol(report);
        }
        if focused
            && self
                .secret
                .as_ref()
                .is_some_and(|field| field.input.read(cx).value().is_empty())
            && self.terminal.read(cx).vault_unlocked(cx)
        {
            self.terminal
                .update(cx, |terminal, cx| terminal.retrieve_credential(cx));
        }
        self.wake_cursor(cx);
    }

    pub(crate) fn cursor_visible(&self) -> bool {
        self.cursor.lit()
    }
    fn tick_cursor(&mut self, cx: &mut Context<Self>) -> Duration {
        let blinking = self
            .painted_frame()
            .cursor
            .is_some_and(|cursor| cursor.blinking);
        let (wait, redraw) = self.cursor.tick(
            cx.background_executor().now(),
            self.input.focused(),
            blinking,
        );
        if redraw {
            cx.notify();
        }
        wait
    }
    pub(super) fn wake_cursor(&mut self, cx: &mut Context<Self>) {
        if self.cursor.wake(cx.background_executor().now()) {
            cx.notify();
        }
    }

    fn pointer_host(&self) -> pointer::PointerHost {
        pointer::PointerHost::new(
            self.terminal.clone(),
            self.focus_handle.clone(),
            self.painted_geometry(),
            self.input.focused(),
        )
    }
    pub(super) fn sync_pointer(&mut self, cx: &App) {
        let geometry = self.painted_geometry();
        self.pointer.sync(self.terminal.read(cx), geometry);
    }
    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.pointer_host();
        if self.pointer.mouse_down(event, &host, window, cx) {
            self.paste(&Paste, window, cx);
        }
    }
    pub(crate) fn mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let host = self.pointer_host();
        self.pointer.mouse_move(event, &host, hovered, cx);
    }
    pub(crate) fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let host = self.pointer_host();
        self.pointer.mouse_up(event, &host, cx);
    }
    pub(crate) fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let host = self.pointer_host();
        self.pointer.scroll_wheel(event, &host, cx);
    }
    fn autoscroll_tick(&mut self, cx: &mut Context<Self>) -> bool {
        self.sync_pointer(cx);
        if !self.input.focused() || !self.pointer.dragging() {
            return false;
        }
        let host = self.pointer_host();
        let scrolled = self.pointer.extend_selection(&host, cx);
        if !scrolled {
            self.pointer.stop_autoscroll();
        }
        scrolled
    }
    // ── Keyboard ─────────────────────────────────────────────────────────────

    fn type_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let accepted = self.terminal.update(cx, |terminal, cx| {
            let accepted = terminal.send_text(text, cx);
            if accepted {
                terminal.prepare_input(cx);
            }
            accepted
        });
        self.wake_cursor(cx);
        accepted
    }
    fn toggle_recording(&mut self, _: &ToggleRecording, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.toggle_recording(cx));
    }

    // ── Actions ──────────────────────────────────────────────────────────────

    fn copy_selection(&self, cx: &mut Context<Self>) {
        if let Some(text) = self.terminal.read(cx).emulator().selection_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    fn scroll_page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.scroll(Scroll::PageUp, cx));
    }

    fn scroll_page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.scroll(Scroll::PageDown, cx));
    }

    fn scroll_to_top(&mut self, _: &ScrollToTop, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.scroll(Scroll::Top, cx));
    }

    fn scroll_to_bottom(&mut self, _: &ScrollToBottom, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.scroll(Scroll::Bottom, cx));
    }

    fn reconnect(&mut self, _: &Reconnect, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.reconnect(cx));
        window.focus(&self.focus_handle, cx);
    }

    fn open_find(
        &mut self,
        text: Option<String>,
        from_selection: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in terminal"));
            let subscription = cx.subscribe_in(&input, window, |this, input, event, _, cx| {
                if this.find.as_ref().is_none_or(|field| field.input != *input) {
                    return;
                }
                match event {
                    InputEvent::Change => {
                        let query: String = input
                            .read(cx)
                            .value()
                            .chars()
                            .take(nocterm_vt::MAX_SEARCH_QUERY + 1)
                            .collect();
                        if query != this.terminal.read(cx).find().query() {
                            this.terminal.update(cx, |t, cx| {
                                t.begin_find(query, SearchDirection::Next, None, cx)
                            });
                        }
                    }
                    InputEvent::PressEnter { shift, .. } => {
                        this.terminal.update(cx, |t, cx| t.find_next(*shift, cx))
                    }
                    _ => {}
                }
                cx.notify();
            });
            self.find = Some(FindField {
                input,
                _subscription: subscription,
            });
        }
        let input = self.find.as_ref().unwrap().input.clone();
        if let Some(text) = text {
            let text: String = text
                .chars()
                .take(nocterm_vt::MAX_SEARCH_QUERY + 1)
                .collect();
            input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
            let anchor = from_selection
                .then(|| self.terminal.read(cx).emulator().selection_start())
                .flatten();
            self.terminal.update(cx, |t, cx| {
                t.begin_find(text, SearchDirection::Next, anchor, cx)
            });
        }
        window.focus(&input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    fn hide_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find = None;
        self.terminal.update(cx, |t, cx| t.clear_find(cx));
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn toggle_find_option(
        &mut self,
        edit: impl FnOnce(&mut nocterm_vt::SearchOptions),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut options = self.terminal.read(cx).find().options();
        edit(&mut options);
        self.terminal
            .update(cx, |terminal, cx| terminal.set_find_options(options, cx));
        if let Some(field) = &self.find {
            window.focus(&field.input.read(cx).focus_handle(cx), cx);
        }
    }

    /// Creates the field for a requested secret, or drops it once answered.
    fn sync_secret_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let terminal = self.terminal.read(cx);
        let question = match terminal.prompt() {
            Some(Prompt::Secret { request, .. }) => Some((
                terminal.prompt_epoch(),
                matches!(request, SecretRequest::Interactive { echo: true, .. }),
            )),
            _ => None,
        };
        if self.secret.as_ref().map(|field| field.prompt_epoch) == question.map(|(epoch, _)| epoch)
        {
            return;
        }
        let owns_focus = self.focus_handle.contains_focused(window, cx)
            || self
                .secret
                .as_ref()
                .is_some_and(|field| field.input.read(cx).focus_handle(cx).is_focused(window));
        if let Some(field) = self.secret.take() {
            // set_value also clears the native control's undo history.
            field
                .input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        if let Some((prompt_epoch, echo)) = question {
            let input = cx.new(|cx| InputState::new(window, cx).masked(!echo));
            let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.submit_secret(window, cx);
                }
            });
            if owns_focus {
                input.update(cx, |input, cx| input.focus(window, cx));
            }
            self.secret = Some(SecretField {
                prompt_epoch,
                remember: false,
                input,
                _subscription: subscription,
            });
        } else if owns_focus {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn submit_secret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = &self.secret else {
            return;
        };
        let secret = Secret::new(field.input.read(cx).value().to_string());
        let remember = field.remember;
        field
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.terminal.update(cx, |terminal, cx| {
            terminal.answer_secret_and_remember(secret, remember, cx)
        });
    }

    fn cancel_secret(&mut self, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.answer_secret(None, cx));
    }

    fn answer_host_key(
        &mut self,
        decision: HostKeyDecision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.terminal
            .update(cx, |terminal, cx| terminal.answer_host_key(decision, cx));
        window.focus(&self.focus_handle, cx);
    }

    // ── Rendering ────────────────────────────────────────────────────────────

    /// Tells the emulator the colours it is drawn with, so it can answer
    /// programs that ask.
    pub(super) fn sync_palette(&mut self, style: &TerminalStyle, cx: &mut Context<Self>) {
        let rgb = |color: nocterm_ui::Color| Rgb::new(color.r, color.g, color.b);
        let palette = Palette {
            foreground: rgb(style.foreground),
            background: rgb(style.background),
            cursor: rgb(style.cursor),
            ansi: style.ansi.map(rgb),
        };
        if self.cache.apply_palette(palette) {
            self.terminal
                .update(cx, |terminal, _| terminal.set_palette(palette));
        }
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.secret {
            Some(field) => field.input.read(cx).focus_handle(cx),
            None => self
                .find
                .as_ref()
                .map(|find| find.input.read(cx).focus_handle(cx))
                .unwrap_or_else(|| self.focus_handle.clone()),
        }
    }
}

impl nocterm_workspace::LocalTerminal for TerminalView {
    fn cwd(&self, cx: &App) -> Option<std::path::PathBuf> {
        self.terminal.read(cx).cwd()
    }
    fn change_directory(
        &mut self,
        path: std::path::PathBuf,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), nocterm_workspace::LocalDirectoryError> {
        self.terminal
            .update(cx, |terminal, cx| terminal.change_directory(&path, cx))
    }
}

#[cfg(test)]
#[path = "view_prompt_tests.rs"]
mod prompt_tests;

fn modifiers(modifiers: &gpui_kit::Modifiers) -> Modifiers {
    Modifiers {
        ctrl: modifiers.control,
        alt: modifiers.alt,
        shift: modifiers.shift,
    }
}
