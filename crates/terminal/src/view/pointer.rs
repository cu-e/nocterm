//! Local selection and bounded program pointer input.
use super::*;

const AUTOSCROLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_AUTOSCROLL_LINES: i32 = 8;

#[derive(Default)]
pub(super) struct PointerState {
    context: Option<PointerContext>,
    selection: Option<Point<Pixels>>,
    auto_scroll: Option<Task<()>>,
    reported_button: Option<nocterm_vt::MouseButton>,
    last_reported_cell: Option<CellPoint>,
    scroll_remainder: f32,
    shifted_wheel: Option<bool>,
}

#[derive(Clone, Copy, PartialEq)]
struct PointerContext {
    session_epoch: u64,
    connected: bool,
    modes: nocterm_vt::Modes,
    size: nocterm_vt::TermSize,
    geometry: Option<Geometry>,
}

pub(super) struct PointerHost {
    terminal: Entity<Terminal>,
    focus: FocusHandle,
    geometry: Option<Geometry>,
    focused: bool,
}
impl PointerHost {
    pub(super) fn new(
        terminal: Entity<Terminal>,
        focus: FocusHandle,
        geometry: Option<Geometry>,
        focused: bool,
    ) -> Self {
        Self {
            terminal,
            focus,
            geometry,
            focused,
        }
    }
}

impl PointerState {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
    pub(super) fn sync(&mut self, terminal: &Terminal, geometry: Option<Geometry>) {
        let context = PointerContext {
            session_epoch: terminal.prompt_epoch(),
            connected: terminal.is_connected(),
            modes: terminal.emulator().modes(),
            size: terminal.emulator().size(),
            geometry,
        };
        let selection_cleared = self.selection.is_some() && !terminal.emulator().has_selection();
        if self.context != Some(context) {
            *self = PointerState {
                context: Some(context),
                ..Default::default()
            };
        } else if selection_cleared {
            self.stop_selection();
        }
    }

    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        host: &PointerHost,
        window: &mut Window,
        cx: &mut Context<TerminalView>,
    ) -> bool {
        window.focus(&host.focus, cx);
        self.sync(host.terminal.read(cx), host.geometry);
        let Some(geometry) = host.geometry else {
            return false;
        };
        let (at, side) = geometry.cell_at(event.position);
        let modes = host.terminal.read(cx).emulator().modes();

        // Shift reaches past a program's mouse handling to the selection.
        if modes.reports_mouse() && !event.modifiers.shift {
            if let Some(button) = vt_button(event.button) {
                self.reported_button = Some(button);
                self.report_mouse(
                    host,
                    MouseEventKind::Press(button),
                    at,
                    &event.modifiers,
                    cx,
                );
            }
            return false;
        }

        match event.button {
            MouseButton::Left => {
                let extend = event.modifiers.shift
                    && event.click_count <= 1
                    && host.terminal.read(cx).emulator().selection_text().is_some();
                let kind = match event.click_count {
                    0 | 1 => SelectionKind::Cells,
                    2 => SelectionKind::Words,
                    _ => SelectionKind::Lines,
                };
                host.terminal.update(cx, |terminal, cx| {
                    if extend {
                        terminal.extend_selection(at, side, 0, cx);
                    } else {
                        terminal.begin_selection(kind, at, side, cx);
                    }
                });
                self.reported_button = None;
                self.selection = Some(event.position);
            }
            MouseButton::Middle => return true,
            _ => {}
        }
        false
    }

    pub(crate) fn mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        host: &PointerHost,
        hovered: bool,
        cx: &mut Context<TerminalView>,
    ) {
        self.sync(host.terminal.read(cx), host.geometry);
        let Some(geometry) = host.geometry else {
            return;
        };
        if self.selection.is_some() {
            if event.pressed_button != Some(MouseButton::Left) {
                self.stop_selection();
                return;
            }
            self.selection = Some(event.position);
            self.extend_selection(host, cx);
            if edge_scroll(geometry, event.position) != 0 {
                self.start_autoscroll(cx);
            } else {
                self.auto_scroll = None;
            }
            return;
        }

        if self.reported_button != event.pressed_button.and_then(vt_button) {
            self.reported_button = None;
        }
        if !host.focused || event.modifiers.shift {
            return;
        }
        let (at, _) = geometry.cell_at(event.position);
        let held = self.reported_button;
        let modes = host.terminal.read(cx).emulator().modes();
        let wanted = held.is_some() || (hovered && modes.mouse_motion);
        if wanted && self.last_reported_cell != Some(at) {
            self.report_mouse(host, MouseEventKind::Move(held), at, &event.modifiers, cx);
        }
    }

    pub(super) fn mouse_up(
        &mut self,
        event: &MouseUpEvent,
        host: &PointerHost,
        cx: &mut Context<TerminalView>,
    ) {
        self.sync(host.terminal.read(cx), host.geometry);
        if self.selection.is_some() && event.button == MouseButton::Left {
            self.stop_selection();
            if cx.setting::<nocterm_ui::TerminalSettings>().copy_on_select
                && let Some(text) = host.terminal.read(cx).emulator().selection_text()
            {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            return;
        }
        if let Some(button) = self.reported_button
            && Some(button) == vt_button(event.button)
            && let Some(geometry) = host.geometry
        {
            self.reported_button = None;
            let (at, _) = geometry.cell_at(event.position);
            self.report_mouse(
                host,
                MouseEventKind::Release(button),
                at,
                &event.modifiers,
                cx,
            );
        }
    }

    fn stop_selection(&mut self) {
        self.selection = None;
        self.auto_scroll = None;
    }

    /// Scroll and move the endpoint together; the emulator retains the anchor
    /// in absolute scrollback coordinates.
    pub(super) fn extend_selection(
        &mut self,
        host: &PointerHost,
        cx: &mut Context<TerminalView>,
    ) -> bool {
        let (Some(position), Some(geometry)) = (self.selection, host.geometry) else {
            return false;
        };
        let lines = edge_scroll(geometry, position);
        let (at, side) = geometry.cell_at(position);
        host.terminal.update(cx, |terminal, cx| {
            terminal.extend_selection(at, side, lines, cx)
        })
    }

    fn start_autoscroll(&mut self, cx: &mut Context<TerminalView>) {
        if self.auto_scroll.is_some() {
            return;
        }
        self.auto_scroll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTOSCROLL_INTERVAL).await;
                let keep_scrolling = this.update(cx, |this, cx| this.autoscroll_tick(cx));
                if !matches!(keep_scrolling, Ok(true)) {
                    break;
                }
            }
        }));
    }

    pub(super) fn scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        host: &PointerHost,
        cx: &mut Context<TerminalView>,
    ) {
        self.sync(host.terminal.read(cx), host.geometry);
        let Some(geometry) = host.geometry else {
            return;
        };
        if self.shifted_wheel != Some(event.modifiers.shift) {
            self.scroll_remainder = 0.;
            self.shifted_wheel = Some(event.modifiers.shift);
        }
        let delta = match event.delta {
            gpui_kit::ScrollDelta::Lines(delta) => delta.y,
            gpui_kit::ScrollDelta::Pixels(delta) => {
                f32::from(delta.y) / f32::from(geometry.line_height)
            }
        };
        let accumulated = self.scroll_remainder + delta;
        let lines = accumulated.trunc() as i32;
        self.scroll_remainder = if accumulated.is_finite() && accumulated.abs() < i32::MAX as f32 {
            accumulated - lines as f32
        } else {
            0.
        };
        if lines == 0 {
            return;
        }
        let reports = lines.unsigned_abs().min(MAX_WHEEL_REPORTS as u32);
        let modes = host.terminal.read(cx).emulator().modes();
        if modes.reports_mouse() && !event.modifiers.shift {
            let (at, _) = geometry.cell_at(event.position);
            let kind = if lines > 0 {
                MouseEventKind::WheelUp
            } else {
                MouseEventKind::WheelDown
            };
            for _ in 0..reports {
                self.report_mouse(host, kind, at, &event.modifiers, cx);
            }
        } else if modes.alt_screen && modes.alternate_scroll && !event.modifiers.shift {
            let key = if lines > 0 { "up" } else { "down" };
            let press = KeyPress {
                key,
                modifiers: Modifiers::default(),
                text: None,
            };
            if let Some(bytes) = encode_key(&press, modes) {
                let terminal = host.terminal.read(cx);
                for _ in 0..reports {
                    terminal.send(bytes.clone());
                }
            }
        } else {
            host.terminal
                .update(cx, |terminal, cx| terminal.scroll(Scroll::Lines(lines), cx));
        }
    }

    fn report_mouse(
        &mut self,
        host: &PointerHost,
        kind: MouseEventKind,
        at: CellPoint,
        modifiers: &gpui_kit::Modifiers,
        cx: &mut Context<TerminalView>,
    ) {
        self.last_reported_cell = Some(at);
        let terminal = host.terminal.read(cx);
        let event = MouseEvent {
            kind,
            at,
            modifiers: self::modifiers(modifiers),
        };
        if let Some(report) = encode_mouse(&event, terminal.emulator().modes()) {
            terminal.send(report);
        }
    }
    pub(super) fn dragging(&self) -> bool {
        self.selection.is_some()
    }
    pub(super) fn stop_autoscroll(&mut self) {
        self.auto_scroll = None;
    }
}

fn edge_scroll(geometry: Geometry, position: Point<Pixels>) -> i32 {
    let top = f32::from(geometry.origin.y);
    let bottom = top + f32::from(geometry.line_height) * f32::from(geometry.rows);
    let y = f32::from(position.y);
    let distance = if y < top {
        top - y
    } else if y >= bottom {
        bottom - y
    } else {
        return 0;
    };
    let lines = (distance / f32::from(geometry.line_height)).ceil() as i32;
    if y < top {
        lines.clamp(1, MAX_AUTOSCROLL_LINES)
    } else {
        lines.clamp(-MAX_AUTOSCROLL_LINES, -1)
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
