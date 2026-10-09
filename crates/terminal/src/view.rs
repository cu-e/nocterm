use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
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
use nocterm_ui::{ActiveDesign as _, IconName, SettingsExt as _, TerminalStyle};
use nocterm_vt::{
    CellPoint, Frame, KeyPress, Modifiers, MouseEvent, MouseEventKind, Palette, Rgb, Scroll,
    SearchDirection, SelectionKind, encode_focus, encode_key, encode_mouse, encode_paste,
};
use nocterm_workspace::{Item, ItemCommand, ItemEvent, SessionContext, SessionSpec, TabState};

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
mod host_key;
mod input;
mod keyboard;
mod paste;
mod pointer;
mod render;
#[path = "view_secret.rs"]
mod secret;
#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;

/// A terminal tab.
pub struct TerminalView {
    terminal: Entity<Terminal>,
    keyboard: keyboard::KeyboardInput,
    focus_handle: FocusHandle,
    focused: bool,
    /// Shared with the element, which fills them in while painting.
    frame: Rc<RefCell<Frame>>,
    frame_dirty: Rc<Cell<bool>>,
    frame_version: Rc<Cell<Option<(u64, u64)>>>,
    highlights: Rc<RefCell<crate::highlighting::Highlights>>,
    geometry: Rc<Cell<Option<Geometry>>>,
    /// The palette last handed to the emulator.
    palette: Option<Palette>,
    cursor_lit: bool,
    next_blink: Instant,
    _blink: Task<()>,
    /// Text an input method is composing.
    marked_text: Option<String>,
    pointer: pointer::PointerState,
    secret: Option<SecretField>,
    find: Option<FindField>,
    notice_messages: [Option<String>; 3],
    _subscriptions: Vec<Subscription>,
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
    pub fn new(spec: SessionSpec, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let terminal = cx.new(|cx| Terminal::new(spec, cx));
        Self::with_terminal(terminal, window, cx)
    }

    pub fn new_local(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let terminal = cx.new(Terminal::new_local);
        Self::with_terminal(terminal, window, cx)
    }

    pub fn new_local_command(
        launch: nocterm_session::ShellLaunch,
        title: String,
        transport: std::sync::Arc<dyn nocterm_session::Transport>,
        completion: impl FnOnce(nocterm_session::CloseReason, &mut gpui_kit::App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let terminal = cx
            .new(|cx| Terminal::new_local_command(launch, title.into(), transport, completion, cx));
        Self::with_terminal(terminal, window, cx)
    }

    pub(crate) fn with_terminal(
        terminal: Entity<Terminal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let subscriptions = vec![
            cx.subscribe_in(&terminal, window, Self::on_terminal_event),
            cx.observe_keystrokes(Self::on_keyboard_binding),
            cx.observe_global::<nocterm_ui::SettingsStore>(|this, cx| {
                if !cx
                    .setting::<nocterm_ui::TerminalSettings>()
                    .semantic_highlighting
                {
                    this.highlights.borrow_mut().clear();
                }
                cx.notify();
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
            keyboard: Default::default(),
            focus_handle,
            focused: false,
            frame: Rc::default(),
            frame_dirty: Rc::new(Cell::new(true)),
            frame_version: Rc::default(),
            highlights: Rc::default(),
            geometry: Rc::default(),
            palette: None,
            cursor_lit: true,
            next_blink: cx.background_executor().now() + CURSOR_BLINK_INTERVAL,
            _blink: Self::blink(cx),
            marked_text: None,
            pointer: Default::default(),
            secret: None,
            find: None,
            notice_messages: Default::default(),
            _subscriptions: subscriptions,
        }
    }

    pub fn terminal(&self) -> &Entity<Terminal> {
        &self.terminal
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
                self.frame_dirty.set(true);
                self.sync_secret_field(window, cx);
                self.sync_operational_notices(window, cx);
                cx.emit(ItemEvent::Changed);
                cx.notify();
            }
            TerminalEvent::Output => {
                self.frame_dirty.set(true);
                self.sync_operational_notices(window, cx);
                cx.notify();
            }
            // A visual or audible bell is a setting still to come.
            TerminalEvent::Bell => {}
            TerminalEvent::ClipboardWrite(text) => {
                if self.focus_handle.is_focused(window)
                    && cx.active_window() == Some(window.window_handle())
                    && cx.setting::<nocterm_ui::TerminalSettings>().clipboard_write
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
            terminal.recording_status().and_then(|status| status.error),
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
        self.focused = focused;
        if !focused {
            self.pointer = Default::default();
            self.keyboard.clear();
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

    // ── Keyboard ─────────────────────────────────────────────────────────────

    fn type_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let accepted = self.terminal.update(cx, |terminal, cx| {
            terminal.prepare_input(cx);
            terminal.send_text(text, cx)
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
                        if query != this.terminal.read(cx).find().query {
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
        let mut options = self.terminal.read(cx).find().options;
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
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            self.frame_dirty.set(true);
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
    ) -> Result<(), String> {
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
