//! Splits a script's output stream into frames.
//!
//! Every frame is a run of lines closed by a line reading `@@end`. Output
//! arrives in arbitrary chunks, so lines are reassembled across them.

/// The line that closes a frame.
pub(crate) const END: &str = "@@end";
/// The largest frame accepted; a longer one is dropped as garbage.
const MAX_FRAME: usize = 1024 * 1024;

#[derive(Default)]
pub(crate) struct FrameReader {
    partial: Vec<u8>,
    lines: Vec<String>,
    size: usize,
}

impl FrameReader {
    /// Feeds output and returns the frames it completed, oldest first.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<Vec<String>> {
        let mut frames = Vec::new();
        let mut rest = bytes;
        while let Some(newline) = rest.iter().position(|byte| *byte == b'\n') {
            self.partial.extend_from_slice(&rest[..newline]);
            rest = &rest[newline + 1..];
            let line = String::from_utf8_lossy(&self.partial)
                .trim_end_matches('\r')
                .to_owned();
            self.partial.clear();
            if line == END {
                self.size = 0;
                frames.push(std::mem::take(&mut self.lines));
            } else {
                self.size += line.len();
                self.lines.push(line);
                if self.size > MAX_FRAME {
                    self.size = 0;
                    self.lines.clear();
                }
            }
        }
        self.partial.extend_from_slice(rest);
        if self.partial.len() > MAX_FRAME {
            self.partial.clear();
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_reassembled_across_chunks() {
        let mut reader = FrameReader::default();
        assert!(reader.push(b"@@a\r\none\ntw").is_empty());
        let frames = reader.push(b"o\n@@end\n@@a\nthree\n@@e");
        assert_eq!(frames, [vec!["@@a", "one", "two"]]);
        let frames = reader.push(b"nd\n");
        assert_eq!(frames, [vec!["@@a", "three"]]);
    }

    #[test]
    fn runaway_output_is_dropped() {
        let mut reader = FrameReader::default();
        let line = vec![b'x'; 4096];
        for _ in 0..300 {
            reader.push(&line);
            reader.push(b"\n");
        }
        let frames = reader.push(b"last\n@@end\n");
        assert_eq!(frames.len(), 1);
        assert!(frames[0].len() < 300);
    }
}
