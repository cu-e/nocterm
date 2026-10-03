//! Plain text of the terminal's history and screen, bounded for a reader that
//! is not the user, such as an AI agent.

use alacritty_terminal::{
    grid::Dimensions as _,
    index::{Column, Line},
    term::cell::Flags,
};

use crate::Emulator;

/// How much text to return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextQuery {
    /// Most logical lines to return; the newest ones are kept.
    pub max_lines: usize,
    /// Most bytes to return; the newest text is kept.
    pub max_bytes: usize,
    /// Only lines numbered at least this. Pass [`TextTail::next_line`] of the
    /// previous answer to read what was printed since. Ignored in the
    /// alternate screen, which has no history.
    pub since_line: Option<u64>,
}

/// The tail of a terminal's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextTail {
    /// Logical lines joined with `\n`, without a trailing newline. Rows
    /// joined by soft wrapping are one line; trailing blanks are dropped.
    pub text: String,
    /// Number of the oldest line in `text`, as in [`crate::LineMetadata`].
    pub first_line: u64,
    /// Number the next line printed will get; the cursor for `since_line`.
    pub next_line: u64,
    /// Older text matching the query was left out to honour the limits.
    pub truncated: bool,
    /// The text is the visible screen of a full-screen program.
    pub alt_screen: bool,
}

impl Emulator {
    /// The most recent text, within the limits of `query`.
    ///
    /// Normally that is the scrollback and the screen. In the alternate screen
    /// it is only what the program currently shows. Cells a program marked
    /// hidden (the way password prompts are sometimes drawn) read as blanks.
    pub fn text(&self, query: TextQuery) -> TextTail {
        let grid = self.search_grid();
        let alt_screen = self.modes().alt_screen;
        let next_line = self.output_line_number().saturating_add(1);
        let since = query.since_line.filter(|_| !alt_screen);
        let top = grid.topmost_line();
        let columns = grid.columns();
        let wraps = |line: Line| {
            grid[line][Column(columns - 1)]
                .flags
                .contains(Flags::WRAPLINE)
        };

        // Newest first. Lines without a number (blank ones) belong to the
        // output that follows them.
        let mut lines: Vec<(u64, String)> = Vec::new();
        let mut used = 0usize;
        let mut truncated = false;
        let mut newer = next_line;
        let mut end = grid.bottommost_line();
        while end >= top {
            let mut start = end;
            while start > top && wraps(start - 1i32) {
                start -= 1i32;
            }
            let number = (start.0..=end.0)
                .find_map(|row| {
                    grid[Line(row)][..]
                        .iter()
                        .find_map(|cell| cell.logical_line())
                })
                .map_or(newer, |mark| mark.id);
            let next_end = start - 1i32;

            if since.is_some_and(|since| number < since) {
                break;
            }
            let blank_tail = lines.is_empty() && self.rows_blank(start, end);
            if !blank_tail {
                let separator = usize::from(!lines.is_empty());
                let room = query.max_bytes.saturating_sub(used + separator);
                if lines.len() >= query.max_lines || room == 0 {
                    truncated = true;
                    break;
                }
                let line = self.join_rows(start, end, room);
                if line.len() > room {
                    // Only the end of a long line fits; start on a character.
                    let mut cut = line.len() - room;
                    while !line.is_char_boundary(cut) {
                        cut += 1;
                    }
                    lines.push((number, line[cut..].to_owned()));
                    truncated = true;
                    break;
                }
                used += separator + line.len();
                lines.push((number, line));
            }
            newer = number;
            end = next_end;
        }

        let first_line = lines.last().map_or(next_line, |(number, _)| *number);
        let text = lines
            .into_iter()
            .rev()
            .map(|(_, line)| line)
            .collect::<Vec<_>>()
            .join("\n");
        TextTail {
            text,
            first_line,
            next_line,
            truncated,
            alt_screen,
        }
    }

    fn rows_blank(&self, start: Line, end: Line) -> bool {
        (start.0..=end.0).all(|row| self.row_text(Line(row), true).is_empty())
    }

    /// The rows `start..=end` as one line, built from the end until it is
    /// longer than `room` bytes (the caller keeps only the end).
    fn join_rows(&self, start: Line, end: Line, room: usize) -> String {
        let mut parts = Vec::new();
        let mut length = 0;
        let mut row = end;
        loop {
            let part = self.row_text(row, row == end);
            length += part.len();
            parts.push(part);
            if row == start || length > room {
                break;
            }
            row -= 1i32;
        }
        parts.iter().rev().map(String::as_str).collect()
    }

    /// One row's characters. Only the last row of a line loses its blanks:
    /// earlier rows end where the line wrapped.
    fn row_text(&self, line: Line, last_row: bool) -> String {
        let mut text = String::new();
        for cell in &self.search_grid()[line][..] {
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            if cell.flags.contains(Flags::HIDDEN) {
                text.push(' ');
                continue;
            }
            text.push(cell.c);
            if let Some(combining) = cell.zerowidth() {
                text.extend(combining);
            }
        }
        if last_row {
            text.truncate(text.trim_end().len());
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use crate::{EmulatorOptions, TermSize};

    use super::*;

    fn emulator(cols: u16, rows: u16, scrollback: usize) -> Emulator {
        Emulator::new(
            TermSize::new(cols, rows, 8, 16),
            EmulatorOptions {
                scrollback_lines: scrollback,
                ..EmulatorOptions::default()
            },
        )
    }

    fn query(max_lines: usize, max_bytes: usize) -> TextQuery {
        TextQuery {
            max_lines,
            max_bytes,
            since_line: None,
        }
    }

    #[test]
    fn joins_soft_wrapped_rows_into_one_line() {
        let mut terminal = emulator(5, 4, 100);
        terminal.advance(b"abcdefghij\r\nxy\r\n");

        let tail = terminal.text(query(100, 1024));

        assert_eq!(tail.text, "abcdefghij\nxy");
        assert!(!tail.truncated);
        assert!(!tail.alt_screen);
    }

    #[test]
    fn spans_history_and_screen_and_drops_trailing_blanks() {
        let mut terminal = emulator(10, 2, 100);
        for number in 1..=5 {
            terminal.advance(format!("line {number}   \r\n").as_bytes());
        }

        let tail = terminal.text(query(100, 1024));

        assert_eq!(tail.text, "line 1\nline 2\nline 3\nline 4\nline 5");
        assert_eq!(tail.first_line, 1);
        assert_eq!(tail.next_line, 6);
    }

    #[test]
    fn keeps_blank_lines_between_output() {
        let mut terminal = emulator(10, 5, 100);
        terminal.advance(b"a\r\n\r\nb\r\n");

        assert_eq!(terminal.text(query(100, 1024)).text, "a\n\nb");
    }

    #[test]
    fn since_line_returns_only_new_output() {
        let mut terminal = emulator(10, 3, 100);
        terminal.advance(b"one\r\ntwo\r\n");
        let first = terminal.text(query(100, 1024));
        terminal.advance(b"three\r\n\r\nfour\r\n");

        let second = terminal.text(TextQuery {
            since_line: Some(first.next_line),
            ..query(100, 1024)
        });

        assert_eq!(second.text, "three\n\nfour");
        assert_eq!(second.first_line, first.next_line);
        assert!(!second.truncated);
        let third = terminal.text(TextQuery {
            since_line: Some(second.next_line),
            ..query(100, 1024)
        });
        assert_eq!(third.text, "");
        assert_eq!(third.first_line, third.next_line);
    }

    #[test]
    fn caps_lines_keeping_the_newest() {
        let mut terminal = emulator(10, 3, 100);
        terminal.advance(b"a\r\nb\r\nc\r\nd\r\n");

        let tail = terminal.text(query(2, 1024));

        assert_eq!(tail.text, "c\nd");
        assert_eq!(tail.first_line, 3);
        assert!(tail.truncated);
        assert_eq!(terminal.text(query(0, 1024)).text, "");
    }

    #[test]
    fn caps_bytes_on_utf8_boundaries() {
        let mut terminal = emulator(40, 3, 100);
        terminal.advance("ключ\r\nпароль\r\n".as_bytes());

        // Whole lines first: "пароль" is 12 bytes and the newline one more.
        let tail = terminal.text(query(100, 13));
        assert_eq!(tail.text, "пароль");
        assert!(tail.truncated);

        // Then the end of a line that no longer fits, never a split character.
        let tail = terminal.text(query(100, 11));
        assert_eq!(tail.text, "ароль");
        assert!(tail.truncated);
        assert!(tail.text.len() <= 11);

        let tail = terminal.text(query(100, 3));
        assert_eq!(tail.text, "ь");
        assert!(tail.text.len() <= 3);
    }

    #[test]
    fn byte_cap_counts_the_newlines() {
        let mut terminal = emulator(10, 3, 100);
        terminal.advance(b"aa\r\nbb\r\ncc\r\n");

        assert_eq!(terminal.text(query(100, 8)).text, "aa\nbb\ncc");
        assert_eq!(terminal.text(query(100, 7)).text, "a\nbb\ncc");
        assert_eq!(terminal.text(query(100, 6)).text, "bb\ncc");
    }

    #[test]
    fn a_very_long_wrapped_line_is_cut_to_its_end() {
        let mut terminal = emulator(10, 3, 1000);
        terminal.advance("x".repeat(500).as_bytes());
        terminal.advance(b"END");

        let tail = terminal.text(query(10, 8));

        assert_eq!(tail.text, "xxxxxEND");
        assert!(tail.truncated);
    }

    #[test]
    fn alternate_screen_shows_only_the_visible_screen() {
        let mut terminal = emulator(10, 3, 100);
        terminal.advance(b"history\r\nmore\r\n\x1b[?1049h\x1b[Hmenu\r\nitem");

        let tail = terminal.text(TextQuery {
            since_line: Some(99),
            ..query(100, 1024)
        });

        assert!(tail.alt_screen);
        assert_eq!(tail.text, "menu\nitem");

        terminal.advance(b"\x1b[?1049l");
        let tail = terminal.text(query(100, 1024));
        assert!(!tail.alt_screen);
        assert_eq!(tail.text, "history\nmore");
    }

    #[test]
    fn hidden_cells_read_as_blanks() {
        let mut terminal = emulator(20, 2, 10);
        terminal.advance(b"pass: \x1b[8msecret\x1b[0m!");

        assert_eq!(terminal.text(query(10, 1024)).text, "pass:       !");
    }

    #[test]
    fn wide_and_combining_characters_survive() {
        let mut terminal = emulator(10, 2, 10);
        terminal.advance("日本e\u{301}".as_bytes());

        assert_eq!(terminal.text(query(10, 1024)).text, "日本e\u{301}");
    }

    #[test]
    fn an_empty_terminal_has_no_text() {
        let terminal = emulator(10, 3, 10);

        let tail = terminal.text(query(10, 1024));

        assert_eq!(tail.text, "");
        assert_eq!(tail.first_line, 1);
        assert_eq!(tail.next_line, 1);
        assert!(!tail.truncated);
    }
}
