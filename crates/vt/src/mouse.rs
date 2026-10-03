//! Reporting pointer input to programs that asked for it (xterm mouse tracking).

use crate::{CellPoint, Modes, Modifiers};

/// A pointer button a program can be told about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl MouseButton {
    fn code(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
        }
    }
}

/// What the pointer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEventKind {
    Press(MouseButton),
    Release(MouseButton),
    /// The pointer moved to another cell, with this button held if any.
    Move(Option<MouseButton>),
    WheelUp,
    WheelDown,
}

/// A pointer event over a cell of the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseEventKind,
    pub at: CellPoint,
    pub modifiers: Modifiers,
}

/// The report to send for a pointer event, when the program wants one.
pub fn encode_mouse(event: &MouseEvent, modes: Modes) -> Option<Vec<u8>> {
    let wanted = match event.kind {
        MouseEventKind::Move(None) => modes.mouse_motion,
        MouseEventKind::Move(Some(_)) => modes.mouse_drag || modes.mouse_motion,
        _ => modes.reports_mouse(),
    };
    if !wanted {
        return None;
    }

    const MOTION: u8 = 32;
    const RELEASED: u8 = 3;
    let button = match event.kind {
        MouseEventKind::Press(button) | MouseEventKind::Release(button) => button.code(),
        MouseEventKind::Move(Some(button)) => MOTION + button.code(),
        MouseEventKind::Move(None) => MOTION + RELEASED,
        MouseEventKind::WheelUp => 64,
        MouseEventKind::WheelDown => 65,
    };
    let modifiers = 4 * u8::from(event.modifiers.shift)
        + 8 * u8::from(event.modifiers.alt)
        + 16 * u8::from(event.modifiers.ctrl);
    let released = matches!(event.kind, MouseEventKind::Release(_));

    // Reports are 1-based.
    let col = u32::from(event.at.col) + 1;
    let row = u32::from(event.at.row) + 1;

    if modes.mouse_sgr {
        let suffix = if released { 'm' } else { 'M' };
        let code = button + modifiers;
        return Some(format!("\x1b[<{code};{col};{row}{suffix}").into_bytes());
    }

    // The legacy encoding cannot say which button was released.
    let code = if released { RELEASED } else { button } + modifiers;
    let mut report = b"\x1b[M".to_vec();
    for value in [u32::from(code), col, row] {
        let value = value + 32;
        if modes.mouse_utf8 {
            let mut buffer = [0; 4];
            report.extend_from_slice(char::from_u32(value)?.encode_utf8(&mut buffer).as_bytes());
        } else {
            // One byte per value: positions past column 223 cannot be reported.
            report.push(u8::try_from(value).ok()?);
        }
    }
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            at: CellPoint { row, col },
            modifiers: Modifiers::default(),
        }
    }

    fn clicks() -> Modes {
        Modes {
            mouse_clicks: true,
            ..Modes::default()
        }
    }

    #[test]
    fn nothing_is_reported_unless_asked_for() {
        let press = event(MouseEventKind::Press(MouseButton::Left), 0, 0);
        assert_eq!(encode_mouse(&press, Modes::default()), None);
    }

    #[test]
    fn sgr_reports_name_the_button_on_release() {
        let modes = Modes {
            mouse_sgr: true,
            ..clicks()
        };

        let press = event(MouseEventKind::Press(MouseButton::Right), 9, 4);
        assert_eq!(encode_mouse(&press, modes).unwrap(), b"\x1b[<2;10;5M");

        let release = event(MouseEventKind::Release(MouseButton::Right), 9, 4);
        assert_eq!(encode_mouse(&release, modes).unwrap(), b"\x1b[<2;10;5m");
    }

    #[test]
    fn legacy_reports_are_three_offset_bytes() {
        let press = event(MouseEventKind::Press(MouseButton::Left), 0, 0);
        assert_eq!(encode_mouse(&press, clicks()).unwrap(), b"\x1b[M !!");

        let release = event(MouseEventKind::Release(MouseButton::Left), 0, 0);
        assert_eq!(encode_mouse(&release, clicks()).unwrap(), b"\x1b[M#!!");
    }

    #[test]
    fn legacy_reports_cannot_reach_far_columns_without_utf8() {
        let far = event(MouseEventKind::Press(MouseButton::Left), 300, 0);
        assert_eq!(encode_mouse(&far, clicks()), None);

        let utf8 = Modes {
            mouse_utf8: true,
            ..clicks()
        };
        let report = encode_mouse(&far, utf8).unwrap();
        assert_eq!(report, "\x1b[M \u{14d}!".as_bytes());
    }

    #[test]
    fn wheel_and_modifiers() {
        let modes = Modes {
            mouse_sgr: true,
            ..clicks()
        };
        let mut wheel = event(MouseEventKind::WheelDown, 0, 0);
        wheel.modifiers.ctrl = true;

        assert_eq!(encode_mouse(&wheel, modes).unwrap(), b"\x1b[<81;1;1M");
    }

    #[test]
    fn motion_is_reported_only_in_the_matching_mode() {
        let hover = event(MouseEventKind::Move(None), 1, 1);
        let drag = event(MouseEventKind::Move(Some(MouseButton::Left)), 1, 1);
        let sgr = |modes: Modes| Modes {
            mouse_sgr: true,
            ..modes
        };

        assert_eq!(encode_mouse(&hover, sgr(clicks())), None);
        assert_eq!(encode_mouse(&drag, sgr(clicks())), None);

        let dragging = sgr(Modes {
            mouse_drag: true,
            ..clicks()
        });
        assert_eq!(encode_mouse(&hover, dragging), None);
        assert_eq!(encode_mouse(&drag, dragging).unwrap(), b"\x1b[<32;2;2M");

        let all_motion = sgr(Modes {
            mouse_motion: true,
            ..clicks()
        });
        assert_eq!(encode_mouse(&hover, all_motion).unwrap(), b"\x1b[<35;2;2M");
    }
}
