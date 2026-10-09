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

impl TerminalView {
    pub(super) fn sync_pointer(&mut self, cx: &App) {
        let terminal = self.terminal.read(cx);
        let context = PointerContext {
            session_epoch: terminal.prompt_epoch(),
            connected: terminal.is_connected(),
            modes: terminal.emulator().modes(),
            size: terminal.emulator().size(),
            geometry: self.geometry.get(),
        };
        let selection_cleared =
            self.pointer.selection.is_some() && !terminal.emulator().has_selection();
        if self.pointer.context != Some(context) {
            self.pointer = PointerState {
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.sync_pointer(cx);
        let Some(geometry) = self.geometry.get() else {
            return;
        };
        let (at, side) = geometry.cell_at(event.position);
        let modes = self.terminal.read(cx).emulator().modes();

        // Shift reaches past a program's mouse handling to the selection.
        if modes.reports_mouse() && !event.modifiers.shift {
            if let Some(button) = vt_button(event.button) {
                self.pointer.reported_button = Some(button);
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
                self.pointer.reported_button = None;
                self.pointer.selection = Some(event.position);
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
        self.sync_pointer(cx);
        let Some(geometry) = self.geometry.get() else {
            return;
        };
        if self.pointer.selection.is_some() {
            if event.pressed_button != Some(MouseButton::Left) {
                self.stop_selection();
                return;
            }
            self.pointer.selection = Some(event.position);
            self.extend_selection(cx);
            if edge_scroll(geometry, event.position) != 0 {
                self.start_autoscroll(cx);
            } else {
                self.pointer.auto_scroll = None;
            }
            return;
        }

        if self.pointer.reported_button != event.pressed_button.and_then(vt_button) {
            self.pointer.reported_button = None;
        }
        if !self.focused || event.modifiers.shift {
            return;
        }
        let (at, _) = geometry.cell_at(event.position);
        let held = self.pointer.reported_button;
        let modes = self.terminal.read(cx).emulator().modes();
        let wanted = held.is_some() || (hovered && modes.mouse_motion);
        if wanted && self.pointer.last_reported_cell != Some(at) {
            self.report_mouse(MouseEventKind::Move(held), at, &event.modifiers, cx);
        }
    }

    pub(crate) fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        self.sync_pointer(cx);
        if self.pointer.selection.is_some() && event.button == MouseButton::Left {
            self.stop_selection();
            if cx
                .setting::<nocterm_settings::TerminalSettings>()
                .copy_on_select
            {
                self.copy_selection(cx);
            }
            return;
        }
        if let Some(button) = self.pointer.reported_button
            && Some(button) == vt_button(event.button)
            && let Some(geometry) = self.geometry.get()
        {
            self.pointer.reported_button = None;
            let (at, _) = geometry.cell_at(event.position);
            self.report_mouse(MouseEventKind::Release(button), at, &event.modifiers, cx);
        }
    }

    fn stop_selection(&mut self) {
        self.pointer.selection = None;
        self.pointer.auto_scroll = None;
    }

    /// Scroll and move the endpoint together; the emulator retains the anchor
    /// in absolute scrollback coordinates.
    fn extend_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let (Some(position), Some(geometry)) = (self.pointer.selection, self.geometry.get()) else {
            return false;
        };
        let lines = edge_scroll(geometry, position);
        let (at, side) = geometry.cell_at(position);
        self.terminal.update(cx, |terminal, cx| {
            terminal.update_emulator(cx, |emulator| {
                let scrolled = lines != 0 && emulator.scroll(Scroll::Lines(lines));
                emulator.update_selection(at, side);
                scrolled
            })
        })
    }

    fn start_autoscroll(&mut self, cx: &mut Context<Self>) {
        if self.pointer.auto_scroll.is_some() {
            return;
        }
        self.pointer.auto_scroll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTOSCROLL_INTERVAL).await;
                let keep_scrolling = this.update(cx, |this, cx| {
                    this.sync_pointer(cx);
                    if !this.focused || this.pointer.selection.is_none() {
                        return false;
                    }
                    let scrolled = this.extend_selection(cx);
                    if !scrolled {
                        this.pointer.auto_scroll = None;
                    }
                    scrolled
                });
                if !matches!(keep_scrolling, Ok(true)) {
                    break;
                }
            }
        }));
    }

    pub(crate) fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        self.sync_pointer(cx);
        let Some(geometry) = self.geometry.get() else {
            return;
        };
        if self.pointer.shifted_wheel != Some(event.modifiers.shift) {
            self.pointer.scroll_remainder = 0.;
            self.pointer.shifted_wheel = Some(event.modifiers.shift);
        }
        let delta = match event.delta {
            gpui_kit::ScrollDelta::Lines(delta) => delta.y,
            gpui_kit::ScrollDelta::Pixels(delta) => {
                f32::from(delta.y) / f32::from(geometry.line_height)
            }
        };
        let accumulated = self.pointer.scroll_remainder + delta;
        let lines = accumulated.trunc() as i32;
        self.pointer.scroll_remainder =
            if accumulated.is_finite() && accumulated.abs() < i32::MAX as f32 {
                accumulated - lines as f32
            } else {
                0.
            };
        if lines == 0 {
            return;
        }
        let reports = lines.unsigned_abs().min(MAX_WHEEL_REPORTS as u32);
        let modes = self.terminal.read(cx).emulator().modes();
        if modes.reports_mouse() && !event.modifiers.shift {
            let (at, _) = geometry.cell_at(event.position);
            let kind = if lines > 0 {
                MouseEventKind::WheelUp
            } else {
                MouseEventKind::WheelDown
            };
            for _ in 0..reports {
                self.report_mouse(kind, at, &event.modifiers, cx);
            }
        } else if modes.alt_screen && modes.alternate_scroll && !event.modifiers.shift {
            let key = if lines > 0 { "up" } else { "down" };
            let press = KeyPress {
                key,
                modifiers: Modifiers::default(),
                text: None,
            };
            if let Some(bytes) = encode_key(&press, modes) {
                let terminal = self.terminal.read(cx);
                for _ in 0..reports {
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
        self.pointer.last_reported_cell = Some(at);
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
