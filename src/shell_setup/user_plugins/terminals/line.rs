//! The line editing of a plugin terminal: what is typed is echoed and handed on a line at a time,
//! and the terminal is told when the console shows a clean prompt.

use std::time::{Duration, Instant};

use nocterm_plugin_api::capability::terminals::MAX_LINE;

/// How long a console's output stays quiet after a line before its prompt counts as back.
pub(super) const SETTLE: Duration = Duration::from_secs(2);
/// The prompt marker (OSC 133 A): the console takes a line and nothing is typed.
const READY: &[u8] = b"\x1b]133;A\x1b\\";
/// The marker of a line handed on whose answer is awaited (OSC 133 C).
const ANSWERING: &[u8] = b"\x1b]133;C\x1b\\";

/// What one batch of input produced: bytes to show and finished lines.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Fed {
    pub(super) echo: Vec<u8>,
    pub(super) lines: Vec<String>,
}

/// The line being typed. Enter finishes it, Backspace removes the last character, Ctrl-C drops
/// it and Ctrl-U erases it. Cursor keys and other sequences are ignored, and typing stops at
/// [`MAX_LINE`] characters.
#[derive(Default)]
pub(super) struct LineEditor {
    line: String,
    /// The bytes of a character split between two batches.
    partial: Vec<u8>,
    /// A carriage return ended the last line, so a following line feed adds no empty line.
    after_return: bool,
    escape: Escape,
}

#[derive(Default, PartialEq, Eq)]
enum Escape {
    #[default]
    None,
    /// After ESC.
    Start,
    /// Inside `ESC [` up to the final byte.
    Sequence,
}

impl LineEditor {
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Fed {
        let mut fed = Fed::default();
        let mut text = std::mem::take(&mut self.partial);
        text.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&text) {
            Ok(valid) => valid.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => text.len(),
        };
        self.partial = text.split_off(valid);
        for c in String::from_utf8_lossy(&text).chars() {
            self.key(c, &mut fed);
        }
        fed
    }

    fn key(&mut self, c: char, fed: &mut Fed) {
        match self.escape {
            Escape::Start => {
                self.escape = if c == '[' {
                    Escape::Sequence
                } else {
                    Escape::None
                };
                return;
            }
            Escape::Sequence => {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    self.escape = Escape::None;
                }
                return;
            }
            Escape::None => {}
        }
        let after_return = std::mem::take(&mut self.after_return);
        match c {
            '\r' => {
                self.finish(fed);
                self.after_return = true;
            }
            '\n' if after_return => {}
            '\n' => self.finish(fed),
            '\u{7f}' | '\u{8}' => {
                if self.line.pop().is_some() {
                    fed.echo.extend_from_slice(b"\x08 \x08");
                }
            }
            '\u{3}' => {
                fed.echo.extend_from_slice(b"^C\r\n");
                self.line.clear();
            }
            '\u{15}' => {
                for _ in self.line.chars() {
                    fed.echo.extend_from_slice(b"\x08 \x08");
                }
                self.line.clear();
            }
            '\u{1b}' => self.escape = Escape::Start,
            c if c.is_control() => {}
            c => {
                if self.line.chars().count() < MAX_LINE {
                    self.line.push(c);
                    let mut buffer = [0; 4];
                    fed.echo
                        .extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                }
            }
        }
    }

    fn finish(&mut self, fed: &mut Fed) {
        fed.echo.extend_from_slice(b"\r\n");
        fed.lines.push(std::mem::take(&mut self.line));
    }

    /// Whether nothing is typed.
    pub(super) fn is_empty(&self) -> bool {
        self.line.is_empty()
    }

    /// `output` shown without breaking the line being typed: the line is cleared, the output
    /// written, and the line drawn again after it.
    pub(super) fn around_output(&self, output: &[u8]) -> Vec<u8> {
        if self.line.is_empty() {
            return output.to_vec();
        }
        let mut shown = b"\r\x1b[2K".to_vec();
        shown.extend_from_slice(output);
        if !output.ends_with(b"\n") {
            shown.extend_from_slice(b"\r\n");
        }
        shown.extend_from_slice(self.line.as_bytes());
        shown
    }
}

/// Whether the console awaits the answer to a line, told to the terminal as prompt markers, so an
/// AI chat may type at a clean prompt as it does in a shell that reports its prompt. The host
/// writes these markers itself; a plugin's output never carries one.
#[derive(Default)]
pub(super) struct Prompt {
    /// The last line or output, while the answer to a line is awaited.
    answering: Option<Instant>,
}

impl Prompt {
    /// The marker of a console just opened: it takes a line.
    pub(super) fn opened() -> &'static [u8] {
        READY
    }

    /// The marker after typing: answering once a line was handed on, a clean prompt again when
    /// the line being typed was cleared, and none while something is typed.
    pub(super) fn typed(&mut self, fed: &Fed, empty: bool, now: Instant) -> &'static [u8] {
        if !fed.lines.is_empty() {
            self.answering = Some(now);
            ANSWERING
        } else if empty && self.answering.is_none() {
            READY
        } else {
            b""
        }
    }

    /// The console showed output: an awaited answer goes on.
    pub(super) fn shown(&mut self, now: Instant) {
        if let Some(last) = &mut self.answering {
            *last = now;
        }
    }

    /// When the prompt comes back unless more output is shown first.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.answering.map(|last| last + SETTLE)
    }

    /// The marker at `now`: the prompt is back once the output was quiet for [`SETTLE`], clean
    /// when nothing is typed; with a line half typed it comes back when that line is cleared or
    /// handed on.
    pub(super) fn settle(&mut self, empty: bool, now: Instant) -> &'static [u8] {
        match self.answering {
            Some(last) if now >= last + SETTLE => {
                self.answering = None;
                if empty { READY } else { b"" }
            }
            _ => b"",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(editor: &mut LineEditor, input: &str) -> Vec<String> {
        editor.feed(input.as_bytes()).lines
    }

    #[test]
    fn enter_finishes_a_line_and_the_typing_is_echoed() {
        let mut editor = LineEditor::default();
        let fed = editor.feed(b"say hi\r");
        assert_eq!(fed.lines, ["say hi"]);
        assert_eq!(fed.echo, b"say hi\r\n");
        assert_eq!(lines(&mut editor, "list\r\nstop\n"), ["list", "stop"]);
    }

    #[test]
    fn backspace_ctrl_c_and_ctrl_u_edit_the_line_and_keys_are_ignored() {
        let mut editor = LineEditor::default();
        assert_eq!(lines(&mut editor, "sayy\x7f hi\r"), ["say hi"]);
        assert_eq!(lines(&mut editor, "oops\x03again\r"), ["again"]);
        assert_eq!(lines(&mut editor, "x\x15ok\r"), ["ok"]);
        assert_eq!(lines(&mut editor, "a\x1b[Db\x1b[1;5C\r"), ["ab"]);
        assert_eq!(lines(&mut editor, "tab\there\x07\r"), ["tabhere"]);
    }

    #[test]
    fn a_character_split_between_batches_arrives_whole_and_a_line_is_bounded() {
        let mut editor = LineEditor::default();
        let bytes = "привет\r".as_bytes();
        assert!(editor.feed(&bytes[..3]).lines.is_empty());
        assert_eq!(editor.feed(&bytes[3..]).lines, ["привет"]);
        let long = "x".repeat(MAX_LINE + 10) + "\r";
        assert_eq!(lines(&mut editor, &long)[0].len(), MAX_LINE);
    }

    #[test]
    fn output_is_drawn_above_the_line_being_typed() {
        let mut editor = LineEditor::default();
        assert_eq!(editor.around_output(b"log\r\n"), b"log\r\n");
        editor.feed(b"sa");
        assert_eq!(editor.around_output(b"log\r\n"), b"\r\x1b[2Klog\r\nsa");
        assert_eq!(editor.around_output(b"part"), b"\r\x1b[2Kpart\r\nsa");
    }

    #[test]
    fn the_prompt_is_clean_until_a_line_and_back_once_its_answer_went_quiet() {
        let (mut editor, mut prompt) = (LineEditor::default(), Prompt::default());
        let start = Instant::now();
        assert_eq!(Prompt::opened(), READY);
        let fed = editor.feed(b"list\r");
        assert_eq!(prompt.typed(&fed, editor.is_empty(), start), ANSWERING);
        let answer = start + SETTLE / 2;
        prompt.shown(answer);
        assert_eq!(prompt.deadline(), Some(answer + SETTLE));
        assert_eq!(prompt.settle(true, start + SETTLE), b"", "output went on");
        assert_eq!(prompt.settle(true, answer + SETTLE), READY);
        assert_eq!(prompt.deadline(), None);
    }

    #[test]
    fn a_half_typed_line_keeps_the_prompt_unclean_until_it_is_cleared() {
        let (mut editor, mut prompt) = (LineEditor::default(), Prompt::default());
        let start = Instant::now();
        let fed = editor.feed(b"say");
        assert_eq!(prompt.typed(&fed, editor.is_empty(), start), b"");
        let fed = editor.feed(b"\x15");
        assert_eq!(prompt.typed(&fed, editor.is_empty(), start), READY);
        let fed = editor.feed(b"stop\rx");
        assert_eq!(prompt.typed(&fed, editor.is_empty(), start), ANSWERING);
        assert_eq!(prompt.settle(editor.is_empty(), start + SETTLE), b"");
        let fed = editor.feed(b"\x7f");
        assert_eq!(prompt.typed(&fed, editor.is_empty(), start + SETTLE), READY);
    }
}
