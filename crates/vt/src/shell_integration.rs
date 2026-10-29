//! Bounded OSC observer. Unknown output never authorizes injecting commands.

use std::path::PathBuf;

/// Observes bounded OSC 7 directory reports and OSC 133 shell prompt markers.
///
/// Markers are observations of the byte stream, not authenticated authority to
/// send commands. Callers retain session validation and input admission.
#[derive(Default)]
pub struct ShellIntegration {
    state: u8,
    payload: Vec<u8>,
    cwd: Option<PathBuf>,
    at_prompt: bool,
    known: bool,
    dirty_input: bool,
}

impl ShellIntegration {
    /// The last complete, valid local file URI reported by the shell.
    pub fn cwd(&self) -> Option<&std::path::Path> {
        self.cwd.as_deref()
    }
    /// Whether the observed shell markers indicate a prompt.
    pub fn at_prompt(&self) -> bool {
        self.at_prompt
    }
    /// Whether a recognized shell marker has been observed.
    pub fn known(&self) -> bool {
        self.known
    }
    /// Whether nonempty input has been admitted since the last clean marker.
    pub fn dirty_input(&self) -> bool {
        self.dirty_input
    }
    /// Observed prompt availability and whether its input is dirty.
    pub fn prompt_state(&self) -> (Option<bool>, bool) {
        (self.known.then_some(self.at_prompt), self.dirty_input)
    }

    /// Consumes output fragments; returns whether the reported directory changed.
    pub fn advance(&mut self, bytes: &[u8]) -> bool {
        let old = self.cwd.clone();
        for &byte in bytes {
            match (self.state, byte) {
                (0, 0x1b) => self.state = 1,
                (1, b']') => {
                    self.state = 2;
                    self.payload.clear();
                }
                (1, _) => self.state = 0,
                (2, 7) | (3, b'\\') => {
                    self.finish();
                    self.state = 0;
                }
                (2, 0x1b) => self.state = 3,
                (2, _) if self.payload.len() < 8192 => self.payload.push(byte),
                (2, _) => {
                    self.payload.clear();
                    self.state = 4;
                }
                (3, _) => {
                    self.payload.clear();
                    self.state = 0;
                }
                (4, 7) => self.state = 0,
                (4, 0x1b) => self.state = 5,
                (5, b'\\') => self.state = 0,
                (5, _) => self.state = 4,
                _ => {}
            }
        }
        self.cwd != old
    }

    fn finish(&mut self) {
        let Ok(text) = std::str::from_utf8(&self.payload) else {
            return;
        };
        if matches!(text, "133;A" | "133;B" | "133;C" | "133;D") {
            self.known = true;
        }
        match text {
            "133;A" => {
                self.at_prompt = true;
                self.dirty_input = false;
            }
            "133;B" => self.at_prompt = true,
            "133;C" | "133;D" => {
                self.at_prompt = false;
                self.dirty_input = false;
            }
            _ => {
                if let Some(uri) = text.strip_prefix("7;") {
                    let Some(rest) = uri.strip_prefix("file://") else {
                        return;
                    };
                    let Some(slash) = rest.find('/') else { return };
                    let host = &rest[..slash];
                    if !host.is_empty() && host != "localhost" {
                        return;
                    }
                    if let Some(path) = decode(&rest[slash..]) {
                        self.cwd = Some(PathBuf::from(path));
                    }
                }
            }
        }
    }

    /// Records bytes only after the caller admits them to the original session.
    pub fn input(&mut self, bytes: &[u8]) {
        if bytes.contains(&b'\r') || bytes.contains(&b'\n') {
            self.at_prompt = false;
        } else if !bytes.is_empty() {
            self.dirty_input = true;
        }
    }
}

fn decode(value: &str) -> Option<String> {
    let mut bytes = Vec::new();
    let mut input = value.bytes();
    while let Some(b) = input.next() {
        if b == b'%' {
            let high = (input.next()? as char).to_digit(16)?;
            let low = (input.next()? as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(b);
        }
    }
    if bytes.contains(&0) || bytes.contains(&b'\r') || bytes.contains(&b'\n') {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_osc_prompt_and_cwd() {
        let mut s = ShellIntegration::default();
        for b in b"\x1b]7;file://localhost/home/a%20b\x1b\\\x1b]133;A\x07" {
            s.advance(&[*b]);
        }
        assert_eq!(s.cwd(), Some(std::path::Path::new("/home/a b")));
        assert!(s.at_prompt());
        s.input(b"vim\r");
        assert!(!s.at_prompt());
        s.advance(b"\x1b]133;A\x07");
        s.input(b"x");
        assert!(s.dirty_input());
    }
    #[test]
    fn ordinary_output_and_unknown_markers_never_establish_a_prompt() {
        let mut s = ShellIntegration::default();
        s.advance(b"$ prompt-looking text\x1b]133;unknown\x07");
        assert!(!s.known());
        assert_eq!(s.prompt_state(), (None, false));
        s.input(b"echo hi\r");
        s.advance(b"command output stopped");
        assert_eq!(s.prompt_state(), (None, false));
    }

    #[test]
    fn shell_markers_preserve_input_and_completion_transitions() {
        let mut s = ShellIntegration::default();
        s.advance(b"\x1b]133;A\x07");
        assert_eq!(s.prompt_state(), (Some(true), false));
        s.input(b"x");
        s.advance(b"\x1b]133;B\x07");
        assert_eq!(s.prompt_state(), (Some(true), true));
        s.input(b"\r");
        assert_eq!(s.prompt_state(), (Some(false), true));
        for marker in *b"CD" {
            s.advance(&[0x1b, b']', b'1', b'3', b'3', b';', marker, 7]);
            assert_eq!(s.prompt_state(), (Some(false), false));
        }
        s.advance(b"\x1b]133;A\x07");
        assert_eq!(s.prompt_state(), (Some(true), false));
    }

    #[test]
    fn cwd_changes_only_after_a_complete_valid_local_uri() {
        let mut s = ShellIntegration::default();
        assert!(!s.advance(b"\x1b]7;file:///home/a"));
        assert!(s.advance(b"\x07"));
        assert!(!s.advance(b"\x1b]7;file:///home/a\x07"));
        for uri in [
            "file://attacker/etc",
            "https://localhost/etc",
            "file:///bad%",
            "file:///bad%GG",
            "file:///bad%0A",
            "file:///bad%0D",
            "file:///bad%FF",
        ] {
            assert!(!s.advance(format!("\x1b]7;{uri}\x07").as_bytes()));
            assert_eq!(s.cwd(), Some(std::path::Path::new("/home/a")));
        }
    }

    #[test]
    fn oversized_osc_cannot_smuggle_a_prompt_and_recovers_at_both_terminators() {
        for terminator in [b"\x07".as_slice(), b"\x1b\\".as_slice()] {
            let mut s = ShellIntegration::default();
            s.advance(b"\x1b]");
            s.advance(&vec![b'x'; 8193]);
            s.advance(b"133;A");
            s.advance(terminator);
            assert_eq!(s.prompt_state(), (None, false));
            s.advance(b"\x1b]133;A\x07");
            assert_eq!(s.prompt_state(), (Some(true), false));
        }
    }

    #[test]
    fn hostile_sequences_never_escape_bounds_or_change_cwd() {
        let mut s = ShellIntegration::default();
        s.advance(b"\x1b]7;file://attacker/etc\x07");
        assert!(s.cwd().is_none());
        s.advance(b"\x1b]7;file://localhost/%00\x07");
        assert!(s.cwd().is_none());
        s.advance(b"\x1b]");
        s.advance(&vec![b'x'; 100_000]);
        assert_eq!(s.prompt_state(), (None, false));
        s.advance(b"\x07\x1b]133;A\x07");
        assert_eq!(s.prompt_state(), (Some(true), false));
    }
}
