//! Bounded OSC observer. Unknown output never authorizes injecting commands.

use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct ShellIntegration {
    state: u8,
    payload: Vec<u8>,
    pub cwd: Option<PathBuf>,
    pub at_prompt: bool,
    pub known: bool,
    pub dirty_input: bool,
}

impl ShellIntegration {
    pub(crate) fn advance(&mut self, bytes: &[u8]) -> bool {
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

    pub(crate) fn input(&mut self, bytes: &[u8]) {
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
        assert_eq!(s.cwd, Some(PathBuf::from("/home/a b")));
        assert!(s.at_prompt);
        s.input(b"vim\r");
        assert!(!s.at_prompt);
        s.advance(b"\x1b]133;A\x07");
        s.input(b"x");
        assert!(s.dirty_input);
    }
    #[test]
    fn hostile_sequences_never_escape_bounds_or_change_cwd() {
        let mut s = ShellIntegration::default();
        s.advance(b"\x1b]7;file://attacker/etc\x07");
        assert!(s.cwd.is_none());
        s.advance(b"\x1b]7;file://localhost/%00\x07");
        assert!(s.cwd.is_none());
        s.advance(b"\x1b]");
        s.advance(&vec![b'x'; 100_000]);
        assert!(s.payload.len() <= 8192);
    }
}
