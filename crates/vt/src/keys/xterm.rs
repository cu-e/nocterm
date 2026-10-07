//! xterm's CSI 27;modifier;codepoint~ encoding of modified ordinary keys.
use super::*;

pub(super) fn encode(press: &KeyPress<'_>, mode: ModifyOtherKeys) -> Option<Vec<u8>> {
    let parameter = press.modifiers.parameter();
    if mode == ModifyOtherKeys::Off || parameter == 1 {
        return None;
    }
    let ch = match press.key {
        "enter" => '\r',
        "tab" => '\t',
        "backspace" => '\x7f',
        "escape" => '\x1b',
        key => character(key)?,
    };
    let typed = if ch.is_control() {
        ch
    } else {
        press
            .text
            .and_then(character)
            .filter(|ch| !ch.is_control())
            .unwrap_or_else(|| {
                if press.modifiers.shift {
                    ch.to_uppercase().next().unwrap_or(ch)
                } else {
                    ch
                }
            })
    };
    let modifiers = press.modifiers;
    let shift_only = modifiers.shift && !modifiers.ctrl && !modifiers.alt;
    // GPUI's Shift-Tab denotes ISO_Left_Tab, which retains the backtab form.
    if press.key == "tab" && shift_only {
        return None;
    }
    if mode == ModifyOtherKeys::ExceptWellDefined {
        if press.key == "backspace" {
            return None;
        }
        if !matches!(press.key, "enter" | "tab") && !modifiers.alt {
            // Level 1 preserves control aliases and printable shift input.
            if press.key == "escape"
                || (modifiers.ctrl && (ch == ' ' || legacy::control_byte(ch).is_some()))
                || !modifiers.ctrl
            {
                return None;
            }
        }
    } else {
        // Backspace's pure Control toggle is already represented by legacy BS.
        if press.key == "backspace" && modifiers.ctrl && !modifiers.shift && !modifiers.alt {
            return None;
        }
        // xterm's IsControlInput category is ASCII @..DEL. Other printable
        // symbols already consume Shift; Space is the explicit exception.
        if shift_only
            && !matches!(press.key, "enter" | "backspace" | "escape")
            && typed != ' '
            && !('@'..='\x7f').contains(&typed)
        {
            return None;
        }
    }
    Some(format!("\x1b[27;{parameter};{}~", typed as u32).into_bytes())
}
