use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
    time::Duration,
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
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_session::{CloseReason, ConnectStage, HostKeyDecision, Prompt, Secret, SecretRequest};
use nocterm_ui::{ActiveDesign as _, ActiveSettings as _, IconName, TerminalStyle};
use nocterm_vt::{
    CellPoint, Frame, KeyPress, Modifiers, MouseEvent, MouseEventKind, Palette, Rgb, Scroll,
    SelectionKind, encode_focus, encode_key, encode_mouse, encode_paste,
};
use nocterm_workspace::{Item, ItemEvent, OpenVault, SessionContext, SessionSpec, TabState};

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

/// A terminal tab.
pub struct TerminalView {
    terminal: Entity<Terminal>,
    focus_handle: FocusHandle,
    focused: bool,
    /// Shared with the element, which fills them in while painting.
    frame: Rc<RefCell<Frame>>,
    geometry: Rc<Cell<Option<Geometry>>>,
    /// The palette last handed to the emulator.
    palette: Option<Palette>,
    cursor_lit: bool,
    _blink: Task<()>,
    /// Text an input method is composing.
    marked_text: Option<String>,
    /// A drag is extending the selection.
    selecting: bool,
    /// A button press the running program was told about.
    reported_button: Option<nocterm_vt::MouseButton>,
    last_reported_cell: Option<CellPoint>,
    /// Wheel movement too small to make a whole line yet.
    scroll_remainder: f32,
    secret: Option<SecretField>,
    _subscriptions: Vec<Subscription>,
}

struct SecretField {
    prompt_epoch: u64,
    remember: bool,
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

    fn with_terminal(
        terminal: Entity<Terminal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let subscriptions = vec![
            cx.subscribe_in(&terminal, window, Self::on_terminal_event),
            cx.on_focus_in(&focus_handle, window, |this, _, cx| {
                this.focus_changed(true, cx)
            }),
            cx.on_focus_out(&focus_handle, window, |this, _, _, cx| {
                this.focus_changed(false, cx);
            }),
        ];

        Self {
            terminal,
            focus_handle,
            focused: false,
            frame: Rc::default(),
            geometry: Rc::default(),
            palette: None,
            cursor_lit: true,
            _blink: Self::blink(cx),
            marked_text: None,
            selecting: false,
            reported_button: None,
            last_reported_cell: None,
            scroll_remainder: 0.0,
            secret: None,
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
        match event {
            TerminalEvent::Changed => {
                self.sync_secret_field(window, cx);
                cx.emit(ItemEvent::Changed);
                cx.notify();
            }
            TerminalEvent::Output => cx.notify(),
            // A visual or audible bell is a setting still to come.
            TerminalEvent::Bell => {}
        }
    }

    fn focus_changed(&mut self, focused: bool, cx: &mut Context<Self>) {
        self.focused = focused;
        let terminal = self.terminal.read(cx);
        if let Some(report) = encode_focus(focused, terminal.emulator().modes()) {
            terminal.send(report);
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

    // ── Cursor ───────────────────────────────────────────────────────────────

    fn blink(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CURSOR_BLINK_INTERVAL).await;
                let alive = this.update(cx, |this, cx| {
                    let blinking = this.frame.borrow().cursor.is_some_and(|c| c.blinking);
                    if this.focused && blinking {
                        this.cursor_lit = !this.cursor_lit;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
    }

    /// Lights the cursor and restarts its blink, as after typing.
    fn wake_cursor(&mut self, cx: &mut Context<Self>) {
        self.cursor_lit = true;
        self._blink = Self::blink(cx);
        cx.notify();
    }

    // ── Keyboard ─────────────────────────────────────────────────────────────

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Keys typed into a prompt bubble up through here too.
        if !self.focus_handle.is_focused(window) {
            return;
        }
        let keystroke = &event.keystroke;
        // The platform key (cmd, super) belongs to the application's bindings.
        if keystroke.modifiers.platform {
            return;
        }
        let press = KeyPress {
            key: &keystroke.key,
            modifiers: modifiers(&keystroke.modifiers),
            text: keystroke.key_char.as_deref(),
        };
        let modes = self.terminal.read(cx).emulator().modes();
        if let Some(bytes) = encode_key(&press, modes) {
            cx.stop_propagation();
            self.type_text(
                std::str::from_utf8(&bytes)
                    .expect("keyboard encoder produces UTF-8 text and ASCII controls"),
                cx,
            );
        }
    }

    fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.terminal.update(cx, |terminal, cx| {
            terminal.update_emulator(cx, |emulator| {
                emulator.scroll(Scroll::Bottom);
                emulator.clear_selection();
            });
            terminal.send_text(text, cx);
        });
        self.wake_cursor(cx);
    }
    fn toggle_recording(&mut self, _: &ToggleRecording, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.toggle_recording(cx));
    }

    // ── Pointer ──────────────────────────────────────────────────────────────

    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(geometry) = self.geometry.get() else {
            return;
        };
        let (at, side) = geometry.cell_at(event.position);
        let modes = self.terminal.read(cx).emulator().modes();

        // Shift reaches past a program's mouse handling to the selection.
        if modes.reports_mouse() && !event.modifiers.shift {
            if let Some(button) = vt_button(event.button) {
                self.reported_button = Some(button);
                self.report_mouse(MouseEventKind::Press(button), at, &event.modifiers, cx);
            }
            return;
        }

        match event.button {
            MouseButton::Left => {
                let extend = event.modifiers.shift
                    && event.click_count <= 1
                    && self.terminal.read(cx).emulator().selection_text().is_some();
                let kind = match event.click_count {
                    0 | 1 => SelectionKind::Cells,
                    2 => SelectionKind::Words,
                    _ => SelectionKind::Lines,
                };
                self.terminal.update(cx, |terminal, cx| {
                    terminal.update_emulator(cx, |emulator| {
                        if extend {
                            emulator.update_selection(at, side);
                        } else {
                            emulator.start_selection(kind, at, side);
                        }
                    });
                });
                self.selecting = true;
            }
            MouseButton::Middle => self.paste(&Paste, window, cx),
            _ => {}
        }
    }

    pub(crate) fn mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(geometry) = self.geometry.get() else {
            return;
        };
        let (at, side) = geometry.cell_at(event.position);

        if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            self.terminal.update(cx, |terminal, cx| {
                terminal.update_emulator(cx, |emulator| emulator.update_selection(at, side));
            });
            return;
        }

        let held = self
            .reported_button
            .filter(|_| event.pressed_button.is_some());
        let modes = self.terminal.read(cx).emulator().modes();
        let wanted = held.is_some() || (hovered && modes.mouse_motion);
        if wanted && self.last_reported_cell != Some(at) {
            self.report_mouse(MouseEventKind::Move(held), at, &event.modifiers, cx);
        }
    }

    pub(crate) fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.selecting && event.button == MouseButton::Left {
            self.selecting = false;
            if cx.settings().terminal.copy_on_select {
                self.copy_selection(cx);
            }
        }
        if let Some(button) = self.reported_button.take()
            && let Some(geometry) = self.geometry.get()
        {
            let (at, _) = geometry.cell_at(event.position);
            self.report_mouse(MouseEventKind::Release(button), at, &event.modifiers, cx);
        }
    }

    pub(crate) fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some(geometry) = self.geometry.get() else {
            return;
        };
        let delta = event.delta.pixel_delta(geometry.line_height).y;
        self.scroll_remainder += f32::from(delta) / f32::from(geometry.line_height);
        let lines = self.scroll_remainder.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_remainder -= lines as f32;

        let modes = self.terminal.read(cx).emulator().modes();
        if modes.reports_mouse() {
            let (at, _) = geometry.cell_at(event.position);
            let kind = if lines > 0 {
                MouseEventKind::WheelUp
            } else {
                MouseEventKind::WheelDown
            };
            for _ in 0..lines.abs().min(MAX_WHEEL_REPORTS) {
                self.report_mouse(kind, at, &event.modifiers, cx);
            }
        } else if modes.alt_screen && modes.alternate_scroll {
            // A full-screen program without mouse support scrolls with arrows.
            let key = if lines > 0 { "up" } else { "down" };
            let press = KeyPress {
                key,
                modifiers: Modifiers::default(),
                text: None,
            };
            if let Some(bytes) = encode_key(&press, modes) {
                let terminal = self.terminal.read(cx);
                for _ in 0..lines.abs() {
                    terminal.send(bytes.clone());
                }
            }
        } else {
            self.terminal
                .update(cx, |terminal, cx| terminal.scroll(Scroll::Lines(lines), cx));
        }
    }

    fn report_mouse(
        &mut self,
        kind: MouseEventKind,
        at: CellPoint,
        modifiers: &gpui_kit::Modifiers,
        cx: &mut Context<Self>,
    ) {
        self.last_reported_cell = Some(at);
        let terminal = self.terminal.read(cx);
        let event = MouseEvent {
            kind,
            at,
            modifiers: self::modifiers(modifiers),
        };
        if let Some(report) = encode_mouse(&event, terminal.emulator().modes()) {
            terminal.send(report);
        }
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

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let modes = self.terminal.read(cx).emulator().modes();
        self.type_text(
            std::str::from_utf8(&encode_paste(&text, modes))
                .expect("paste wrapping preserves UTF-8"),
            cx,
        );
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

    // ── Prompts ──────────────────────────────────────────────────────────────

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
    fn sync_palette(&mut self, style: &TerminalStyle, cx: &mut Context<Self>) {
        let rgb = |color: nocterm_ui::Color| Rgb::new(color.r, color.g, color.b);
        let palette = Palette {
            foreground: rgb(style.foreground),
            background: rgb(style.background),
            cursor: rgb(style.cursor),
            ansi: style.ansi.map(rgb),
        };
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            self.terminal
                .update(cx, |terminal, _| terminal.set_palette(palette));
        }
    }

    fn render_prompt(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = match self.terminal.read(cx).prompt()? {
            Prompt::UnknownHostKey {
                host,
                algorithm,
                fingerprint,
                ..
            } => {
                let (host, algorithm, fingerprint) =
                    (host.clone(), algorithm.clone(), fingerprint.clone());
                self.render_host_key(host, algorithm, fingerprint, cx)
            }
            Prompt::Secret { request, .. } => {
                let request = request.clone();
                self.render_secret(&request, cx)?
            }
        };

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(cx.theme().overlay)
                .occlude()
                .child(card)
                .into_any_element(),
        )
    }

    fn card(&self, title: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
        let theme = cx.theme();
        v_flex()
            .w(rems(cx.design().layout.dialog_width))
            .max_w_full()
            .gap_3()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .shadow_lg()
            .child(div().font_semibold().child(title.into()))
    }

    fn render_host_key(
        &mut self,
        host: String,
        algorithm: String,
        fingerprint: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        self.card("Unknown host key", cx)
            .child(div().text_sm().child(format!(
                "This is the first connection to {host}. Check that the fingerprint \
                         below is the one the host's administrator gives you."
            )))
            .child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded(theme.radius)
                    .bg(theme.muted)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(algorithm),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_family(theme.mono_font_family.clone())
                            .child(fingerprint),
                    ),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("host-key-reject")
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_host_key(HostKeyDecision::Reject, window, cx);
                            })),
                    )
                    .child(Button::new("host-key-once").label("Connect Once").on_click(
                        cx.listener(|this, _, window, cx| {
                            this.answer_host_key(HostKeyDecision::AcceptOnce, window, cx);
                        }),
                    ))
                    .child(
                        Button::new("host-key-remember")
                            .primary()
                            .label("Trust and Connect")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_host_key(
                                    HostKeyDecision::AcceptAndRemember,
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_secret(
        &mut self,
        request: &SecretRequest,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let input = self.secret.as_ref()?.input.clone();
        let (title, retry, masked) = match request {
            SecretRequest::Password { target, retry } => {
                (format!("Password for {target}"), *retry, true)
            }
            SecretRequest::KeyPassphrase { path, retry } => {
                (format!("Passphrase for {}", path.display()), *retry, true)
            }
            SecretRequest::Interactive { prompt, echo } => {
                let prompt = prompt.trim();
                let title = if prompt.is_empty() {
                    "The host asks for a response"
                } else {
                    prompt
                };
                (title.to_owned(), false, !echo)
            }
        };
        let danger = cx.theme().danger;

        let rememberable = !matches!(request, SecretRequest::Interactive { .. });
        let unlocked = self.terminal.read(cx).vault_unlocked(cx);
        let remember = self.secret.as_ref().is_some_and(|field| field.remember);
        let message = self
            .terminal
            .read(cx)
            .credential_message()
            .map(str::to_owned);
        let field = Input::new(&input);
        let field = if masked { field.mask_toggle() } else { field };

        Some(
            self.card(title, cx)
                .when(retry, |card| {
                    card.child(
                        div()
                            .text_sm()
                            .text_color(danger)
                            .child("That was not accepted. Try again."),
                    )
                })
                .child(field)
                .when_some(message, |card, message| {
                    card.child(div().text_xs().child(message))
                })
                .when(rememberable, |card| {
                    card.child(
                        Button::new("remember-credential")
                            .ghost()
                            .small()
                            .label("Remember after successful sign in")
                            .selected(remember)
                            .disabled(!unlocked)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(field) = &mut this.secret {
                                    field.remember = !field.remember;
                                    cx.notify();
                                }
                            })),
                    )
                    .when(!unlocked, |card| {
                        card.child(
                            Button::new("auth-open-vault")
                                .ghost()
                                .small()
                                .label("Open credential vault")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(OpenVault), cx)
                                }),
                        )
                    })
                })
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("secret-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_secret(cx))),
                        )
                        .child(
                            Button::new("secret-submit")
                                .primary()
                                .label("Continue")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_secret(window, cx)
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let terminal = self.terminal.read(cx);
        if terminal.prompt().is_some() {
            return None;
        }
        let target = terminal.spec().target.to_string();
        let theme = cx.theme();

        match terminal.status().clone() {
            Status::Connected => None,
            Status::Connecting(stage) => {
                let text = match stage {
                    ConnectStage::Connecting => format!("Connecting to {target}…"),
                    ConnectStage::Authenticating => format!("Signing in to {target}…"),
                    ConnectStage::StartingShell => "Starting the shell…".to_owned(),
                };
                Some(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            h_flex()
                                .gap_2()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(Icon::new(IconName::Loader).small())
                                .child(text),
                        )
                        .into_any_element(),
                )
            }
            Status::Closed(reason) => {
                let (icon, color, text) = match &reason {
                    CloseReason::Exited(None | Some(0)) => (
                        IconName::Unplug,
                        theme.muted_foreground,
                        "The session ended.".to_owned(),
                    ),
                    CloseReason::Exited(Some(code)) => (
                        IconName::Unplug,
                        theme.muted_foreground,
                        format!("The shell exited with status {code}."),
                    ),
                    CloseReason::ClosedByUser => (
                        IconName::Unplug,
                        theme.muted_foreground,
                        "The session was closed.".to_owned(),
                    ),
                    CloseReason::Failed(error) => (
                        IconName::ServerOff,
                        theme.danger,
                        capitalize(&error.to_string()),
                    ),
                };
                Some(
                    h_flex()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .justify_center()
                        .p_3()
                        .child(
                            h_flex()
                                .occlude()
                                .max_w_full()
                                .gap_3()
                                .py_2()
                                .px_3()
                                .rounded(theme.radius_lg)
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .text_color(theme.popover_foreground)
                                .shadow_lg()
                                .child(Icon::new(icon).small().text_color(color))
                                .child(div().text_sm().min_w_0().child(text))
                                .child(
                                    Button::new("reconnect")
                                        .primary()
                                        .small()
                                        .icon(IconName::Plug)
                                        .label("Reconnect")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.reconnect(&Reconnect, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("close-ended-tab")
                                        .ghost()
                                        .small()
                                        .label("Close")
                                        .on_click(cx.listener(|_, _, _, cx| {
                                            cx.emit(ItemEvent::CloseRequested)
                                        })),
                                ),
                        )
                        .into_any_element(),
                )
            }
        }
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn modifiers(modifiers: &gpui_kit::Modifiers) -> Modifiers {
    Modifiers {
        ctrl: modifiers.control,
        alt: modifiers.alt,
        shift: modifiers.shift,
    }
}

fn vt_button(button: MouseButton) -> Option<nocterm_vt::MouseButton> {
    match button {
        MouseButton::Left => Some(nocterm_vt::MouseButton::Left),
        MouseButton::Middle => Some(nocterm_vt::MouseButton::Middle),
        MouseButton::Right => Some(nocterm_vt::MouseButton::Right),
        _ => None,
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.secret {
            Some(field) => field.input.read(cx).focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = TerminalStyle::current(cx);
        self.sync_palette(&style, cx);
        let background = nocterm_ui::hsla(style.background);

        let element = TerminalElement {
            view: cx.entity(),
            terminal: self.terminal.clone(),
            focus: self.focus_handle.clone(),
            style,
            focused: self.focus_handle.is_focused(window),
            cursor_lit: self.cursor_lit,
            marked_text: self.marked_text.clone().map(SharedString::from),
            frame: self.frame.clone(),
            geometry: self.geometry.clone(),
        };

        let grid = div()
            .flex_1()
            .min_h_0()
            .key_context(KEY_CONTEXT)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(background)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::scroll_page_up))
            .on_action(cx.listener(Self::scroll_page_down))
            .on_action(cx.listener(Self::scroll_to_top))
            .on_action(cx.listener(Self::scroll_to_bottom))
            .on_action(cx.listener(Self::reconnect))
            .on_action(cx.listener(Self::toggle_recording))
            .child(
                div()
                    .key_context(SCREEN_KEY_CONTEXT)
                    .track_focus(&self.focus_handle)
                    .size_full()
                    .child(element.render()),
            )
            .children(self.render_prompt(cx))
            .children(self.render_status(cx))
            .when(
                matches!(self.terminal.read(cx).status(), Status::Connected),
                |root| {
                    let message = self
                        .terminal
                        .read(cx)
                        .credential_message()
                        .map(str::to_owned);
                    root.when_some(message, |root, message| {
                        root.child(
                            h_flex()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .right_0()
                                .p_2()
                                .gap_2()
                                .bg(cx.theme().popover)
                                .child(div().flex_1().text_xs().child(message))
                                .child(
                                    Button::new("dismiss-credential-notice")
                                        .ghost()
                                        .small()
                                        .label("Dismiss")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.terminal.update(cx, |terminal, cx| {
                                                terminal.clear_credential_message(cx)
                                            })
                                        })),
                                ),
                        )
                    })
                },
            );
        let terminal = self.terminal.read(cx);
        let recording = terminal.recording_status();
        let message = terminal.text_error().or_else(|| {
            recording
                .as_ref()
                .and_then(|recording| recording.error.clone())
        });
        let active = terminal.is_recording();
        let connected = terminal.is_connected();
        let path = recording
            .and_then(|recording| recording.path)
            .map(|path| path.to_string_lossy().into_owned());
        v_flex().size_full().min_h_0().child(grid).child(
            v_flex()
                .flex_shrink_0()
                .px_2()
                .py_1()
                .gap_1()
                .bg(cx.theme().background)
                .when_some(message, |footer, message| {
                    footer.child(div().text_xs().text_color(cx.theme().danger).child(message))
                })
                .child(
                    h_flex()
                        .gap_2()
                        .min_w_0()
                        .child(
                            Button::new("session-recording")
                                .ghost()
                                .small()
                                .label(if active {
                                    "Stop recording"
                                } else {
                                    "Record output"
                                })
                                .disabled(!connected)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.terminal
                                        .update(cx, |terminal, cx| terminal.toggle_recording(cx))
                                })),
                        )
                        .when_some(path, |row, path| {
                            row.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(path),
                            )
                        }),
                ),
        )
    }
}

impl Item for TerminalView {
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

    fn on_close(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal.update(cx, |terminal, _| terminal.close());
    }
}

/// Text input: what the keyboard types once keys are composed, and what an
/// input method is composing. The terminal has no editable text of its own,
/// so ranges are relative to the composition.
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self
            .marked_text
            .as_ref()
            .map_or(0, |text| text.encode_utf16().count());
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = None;
        if !text.is_empty() {
            self.type_text(text, cx);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = Some(text.to_owned()).filter(|text| !text.is_empty());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let geometry = self.geometry.get()?;
        let cursor = self.frame.borrow().cursor?;
        Some(geometry.cell_bounds(
            CellPoint {
                row: cursor.row,
                col: cursor.col,
            },
            1,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
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
mod prompt_tests {
    use super::*;
    use gpui_kit::{TestAppContext, WindowOptions, test::TestWindowExt as _};
    use nocterm_session::{
        Auth, ConnectRequest, Event, Reply, Session, SessionDriver, Target, Transport,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct PromptTransport(Mutex<Option<Arc<SessionDriver>>>);
    impl Transport for PromptTransport {
        fn open(&self, _: ConnectRequest) -> Session {
            let (session, driver) = nocterm_session::channel(None);
            *self.0.lock().unwrap() = Some(Arc::new(driver));
            session
        }
    }
    fn emit(cx: &mut TestAppContext, driver: Arc<SessionDriver>, request: SecretRequest) {
        let (reply, _answer) = Reply::channel();
        cx.background_executor
            .spawn(async move {
                driver
                    .emit(Event::Prompt(Prompt::Secret { request, reply }))
                    .await;
            })
            .detach();
        cx.run_until_parked();
    }
    #[gpui_kit::test]
    fn superseded_secret_recreates_masked_empty_field_and_clears_old_text(cx: &mut TestAppContext) {
        let transport = Arc::new(PromptTransport::default());
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                nocterm_ui::SettingsStore::in_memory(Default::default()),
                cx,
            );
            crate::init(transport.clone(), cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let view = cx.new(|cx| {
                    TerminalView::new(
                        SessionSpec {
                            options: Default::default(),
                            title: "test".into(),
                            target: Target::new("me", "host", 22),
                            auth: Auth::Password,
                            credential: None,
                            launch: None,
                        },
                        window,
                        cx,
                    )
                });
                window.focus(&view.read(cx).focus_handle(cx), cx);
                view
            })
            .unwrap()
        });
        let driver = transport.0.lock().unwrap().as_ref().unwrap().clone();
        emit(
            cx,
            driver.clone(),
            SecretRequest::Interactive {
                prompt: "Visible code".into(),
                echo: true,
            },
        );
        let old_input = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.input("previous answer", cx);
                view.update(cx, |view, cx| {
                    let field = view.secret.as_mut().unwrap();
                    field.remember = true;
                    assert!(!field.input.read(cx).presentation().is_masked());
                    assert_eq!(field.input.read(cx).value(), "previous answer");
                    field.input.clone()
                })
            })
            .unwrap();
        emit(
            cx,
            driver.clone(),
            SecretRequest::Password {
                target: Target::new("me", "host", 22),
                retry: false,
            },
        );
        let password_input = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let field = view.read(cx).secret.as_ref().unwrap();
                assert_ne!(field.input.entity_id(), old_input.entity_id());
                assert!(field.input.read(cx).presentation().is_masked());
                assert!(field.input.read(cx).value().is_empty());
                assert!(!field.remember);
                assert!(field.input.read(cx).focus_handle(cx).is_focused(window));
                assert!(old_input.read(cx).value().is_empty());
                field.input.clone()
            })
            .unwrap();
        // Even the same question is a new authentication attempt.
        emit(
            cx,
            driver,
            SecretRequest::Password {
                target: Target::new("me", "host", 22),
                retry: false,
            },
        );
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let field = view.read(cx).secret.as_ref().unwrap();
            assert_ne!(field.input.entity_id(), password_input.entity_id());
            window.input("fresh password", cx);
            window.press("ctrl-z", cx);
            assert_ne!(
                view.read(cx)
                    .secret
                    .as_ref()
                    .unwrap()
                    .input
                    .read(cx)
                    .value(),
                "previous answer"
            );
        })
        .unwrap();
    }
}
