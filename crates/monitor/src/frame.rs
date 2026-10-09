//! Splits bounded script output into frames, discarding malformed frames
//! through their closing `@@end` marker.

pub(crate) const END: &str = "@@end";
const MAX_FRAME: usize = 1024 * 1024;
const MAX_LINE: usize = 64 * 1024;
const MAX_LINES: usize = 16 * 1024;

#[derive(Default)]
pub(crate) struct FrameReader {
    partial: Vec<u8>,
    lines: Vec<String>,
    size: usize,
    discarding: bool,
    long_line: bool,
}

impl FrameReader {
    /// Delivers complete frames individually, without buffering the chunk's
    /// potentially unbounded number of frames.
    pub(crate) fn push(&mut self, bytes: &[u8], mut complete: impl FnMut(Vec<String>)) {
        let mut rest = bytes;
        while let Some(newline) = rest.iter().position(|byte| *byte == b'\n') {
            self.fragment(&rest[..newline]);
            self.line(&mut complete);
            rest = &rest[newline + 1..];
        }
        self.fragment(rest);
    }

    fn fragment(&mut self, bytes: &[u8]) {
        if self.long_line {
            return;
        }
        if bytes.len() > MAX_LINE.saturating_sub(self.partial.len()) {
            self.discard();
            self.partial.clear();
            self.long_line = true;
        } else {
            self.partial.extend_from_slice(bytes);
        }
    }

    fn line(&mut self, complete: &mut impl FnMut(Vec<String>)) {
        if self.long_line {
            self.long_line = false;
            return;
        }
        let raw = self.partial.strip_suffix(b"\r").unwrap_or(&self.partial);
        if raw == END.as_bytes() {
            if !self.discarding {
                complete(std::mem::take(&mut self.lines));
            }
            self.discarding = false;
            self.size = 0;
        } else if !self.discarding {
            // Account for lossy UTF-8 expansion and Vec<String> capacity, even
            // for empty lines, before allocating the decoded string.
            let cost = raw
                .len()
                .saturating_mul(3)
                .saturating_add(2 * std::mem::size_of::<String>() + 1);
            if self.lines.len() >= MAX_LINES || cost > MAX_FRAME.saturating_sub(self.size) {
                self.discard();
            } else {
                self.size += cost;
                // Box conversion removes decoder growth slack so the retained
                // text fits the conservative byte budget above.
                self.lines.push(
                    String::from_utf8_lossy(raw)
                        .into_owned()
                        .into_boxed_str()
                        .into_string(),
                );
            }
        }
        self.partial.clear();
    }

    fn discard(&mut self) {
        self.discarding = true;
        self.lines.clear();
        self.size = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(reader: &mut FrameReader, bytes: &[u8]) -> Vec<Vec<String>> {
        let mut frames = Vec::new();
        reader.push(bytes, |frame| frames.push(frame));
        frames
    }

    #[test]
    fn empty_lines_are_bounded_and_bad_frames_stay_discarded() {
        let mut reader = FrameReader::default();
        let empty = vec![b'\n'; 2_000_000];
        assert!(push(&mut reader, &empty).is_empty());
        assert!(reader.lines.len() <= MAX_LINES);
        assert!(push(&mut reader, b"tail\n@@end\n").is_empty());
        assert_eq!(push(&mut reader, b"ok\n@@end\n"), [vec!["ok"]]);
    }

    #[test]
    fn oversized_fragment_cannot_become_a_valid_suffix() {
        let mut reader = FrameReader::default();
        assert!(push(&mut reader, &vec![b'x'; 2 * MAX_FRAME]).is_empty());
        assert!(reader.partial.len() <= MAX_LINE);
        assert!(push(&mut reader, b"@@end\ntruncated\n@@end\n").is_empty());
        assert_eq!(push(&mut reader, b"valid\n@@end\n"), [vec!["valid"]]);
    }

    #[test]
    fn frames_are_reassembled_across_chunks() {
        let mut reader = FrameReader::default();
        assert!(push(&mut reader, b"@@a\r\none\ntw").is_empty());
        let frames = push(&mut reader, b"o\n@@end\n@@a\nthree\n@@e");
        assert_eq!(frames, [vec!["@@a", "one", "two"]]);
        let frames = push(&mut reader, b"nd\n");
        assert_eq!(frames, [vec!["@@a", "three"]]);
    }

    #[test]
    fn runaway_output_is_dropped() {
        let mut reader = FrameReader::default();
        let line = vec![b'x'; 4096];
        for _ in 0..300 {
            push(&mut reader, &line);
            push(&mut reader, b"\n");
        }
        let frames = push(&mut reader, b"last\n@@end\n");
        assert!(frames.is_empty());
        assert_eq!(push(&mut reader, b"fresh\n@@end\n"), [vec!["fresh"]]);
    }
}
