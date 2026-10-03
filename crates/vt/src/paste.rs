//! Pasting text and announcing focus.

use crate::Modes;

/// The bytes to send when the user pastes `text`.
pub fn encode_paste(text: &str, modes: Modes) -> Vec<u8> {
    if modes.bracketed_paste {
        // Brackets tell the program this is a paste, not typing, so it will
        // not run it line by line. Escape characters are dropped: a clipboard
        // holding the closing bracket could otherwise end the paste early and
        // have the rest executed.
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend(text.bytes().filter(|byte| *byte != 0x1b));
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        // A terminal's Enter is a carriage return.
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// The bytes to send when the terminal gains or loses focus, if the program
/// asked to be told.
pub fn encode_focus(focused: bool, modes: Modes) -> Option<&'static [u8]> {
    modes
        .focus_events
        .then_some(if focused { b"\x1b[I" } else { b"\x1b[O" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paste_turns_newlines_into_carriage_returns() {
        let bytes = encode_paste("a\nb\r\nc", Modes::default());
        assert_eq!(bytes, b"a\rb\rc");
    }

    #[test]
    fn bracketed_paste_is_wrapped_and_cannot_be_escaped_from() {
        let modes = Modes {
            bracketed_paste: true,
            ..Modes::default()
        };

        let bytes = encode_paste("ls\n\x1b[201~rm -rf /", modes);

        assert_eq!(bytes, b"\x1b[200~ls\n[201~rm -rf /\x1b[201~");
    }

    #[test]
    fn focus_is_announced_only_on_request() {
        assert_eq!(encode_focus(true, Modes::default()), None);

        let modes = Modes {
            focus_events: true,
            ..Modes::default()
        };
        assert_eq!(encode_focus(true, modes), Some(&b"\x1b[I"[..]));
        assert_eq!(encode_focus(false, modes), Some(&b"\x1b[O"[..]));
    }
}
