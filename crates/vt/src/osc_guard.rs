//! Bound OSC input before VTE's std-mode payload allocation. Normal output is
//! passed in borrowed slices; no second terminal parser or output copy is used.
const MAX_OSC_BYTES: usize = 1024 * 1024;

#[derive(Default)]
enum State {
    #[default]
    Pass,
    Escape,
    Osc,
    Discard,
}
#[derive(Default)]
pub(crate) struct OscGuard {
    state: State,
    payload: Vec<u8>,
}
impl OscGuard {
    pub(crate) fn advance(&mut self, mut bytes: &[u8], mut feed: impl FnMut(&[u8])) {
        while !bytes.is_empty() {
            match self.state {
                State::Pass => {
                    // ESC itself reaches VTE. The OSC introducer `]` stays
                    // withheld until the bounded sequence can safely dispatch.
                    if let Some(index) = bytes.iter().position(|byte| *byte == 0x1b) {
                        feed(&bytes[..=index]);
                        bytes = &bytes[index + 1..];
                        self.state = State::Escape;
                    } else {
                        feed(bytes);
                        return;
                    }
                }
                State::Escape => {
                    let byte = bytes[0];
                    bytes = &bytes[1..];
                    if byte == b']' {
                        self.payload.clear();
                        self.payload.push(byte);
                        self.state = State::Osc;
                    } else {
                        feed(&[byte]);
                        // Match VTE's Escape state: C0 execution/high bytes and
                        // repeated ESC do not finish it; intermediates/finals do.
                        if matches!(byte, 0x18 | 0x1a | 0x20..=0x7e) {
                            self.state = State::Pass;
                        }
                    }
                }
                State::Osc => {
                    let byte = bytes[0];
                    bytes = &bytes[1..];
                    if matches!(byte, 0x07 | 0x18 | 0x1a | 0x1b) {
                        feed(&self.payload);
                        feed(&[byte]);
                        self.payload.clear();
                        self.state = if byte == 0x1b {
                            State::Escape
                        } else {
                            State::Pass
                        };
                    } else if self.payload.len() == MAX_OSC_BYTES {
                        // CAN/SUB are unsafe here: VTE dispatches the partial
                        // OSC when receiving them. VTE has seen only ESC, so its
                        // ST final safely clears that escape without any OSC.
                        feed(b"\\");
                        self.payload.clear();
                        self.state = State::Discard;
                    } else {
                        self.payload.push(byte);
                    }
                }
                State::Discard => {
                    if let Some(index) = bytes
                        .iter()
                        .position(|byte| matches!(*byte, 0x07 | 0x18 | 0x1a | 0x1b))
                    {
                        let byte = bytes[index];
                        bytes = &bytes[index + 1..];
                        if byte == 0x1b {
                            feed(&[byte]);
                            self.state = State::Escape;
                        } else {
                            self.state = State::Pass;
                        }
                    } else {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unterminated_osc_remains_bounded_and_no_truncated_payload_reaches_vte() {
        let mut guard = OscGuard::default();
        let mut output: Vec<u8> = Vec::new();
        guard.advance(b"prefix\x1b]52;c;", |bytes| output.extend(bytes));
        let chunk = [b'A'; 8192];
        for _ in 0..512 {
            guard.advance(&chunk, |bytes| output.extend(bytes));
        }
        assert!(guard.payload.capacity() <= MAX_OSC_BYTES);
        assert!(guard.payload.is_empty());
        assert_eq!(output, b"prefix\x1b\\");
        guard.advance(b"\x07suffix", |bytes| output.extend(bytes));
        assert_eq!(output, b"prefix\x1b\\suffix");
    }
    #[test]
    fn fragmented_escape_controls_intermediates_and_unicode_pass_without_reinterpretation() {
        let input =
            b"\xf0\x9f\x8c\x8d\x1b\0]0;\xce\xbb\x1b\\\x1b ]literal\x1b[31mred\x1b[0m\x1b\x18]text";
        let mut guard = OscGuard::default();
        let mut output: Vec<u8> = Vec::new();
        for byte in input {
            guard.advance(&[*byte], |bytes| output.extend(bytes));
        }
        assert_eq!(output, input);
    }
}
