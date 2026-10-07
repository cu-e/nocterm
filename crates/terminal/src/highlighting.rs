//! Bounded, presentation-only semantic decoration of the visible viewport.
//!
//! The emulator remains the source of truth for ANSI styles, selection, copying
//! and recording. This cache holds roles, so changing a palette needs no parsing.
use nocterm_vt::{Cell, Color, Frame, Style, TermSize};

mod rules;
#[cfg(test)]
mod tests;

const GROUP_BYTES: usize = 4096;
const FRAME_BYTES: usize = 64 * 1024;
const MAX_SPANS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Error,
    Warning,
    Success,
    Address,
    Timestamp,
    Identifier,
    Version,
    Info,
}

impl Role {
    pub(crate) const COUNT: usize = 8;
    pub(crate) fn index(self) -> usize {
        self as usize
    }
    pub(crate) fn ansi(self) -> usize {
        match self {
            Self::Error => 1,
            Self::Warning => 3,
            Self::Success | Self::Address => 2,
            Self::Timestamp => 6,
            Self::Identifier | Self::Version => 5,
            Self::Info => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    generation: u64,
    size: TermSize,
    display_offset: usize,
    alt_screen: bool,
}

// u32 indices bound the retained cache to 48 KiB (4096 twelve-byte spans).
#[derive(Clone, Copy, Debug)]
struct Span {
    start: u32,
    end: u32,
    role: Role,
}

#[derive(Default)]
pub(crate) struct Highlights {
    key: Option<Key>,
    spans: Vec<Span>,
    #[cfg(test)]
    scans: usize,
}

pub(crate) fn plain(cell: &Cell) -> bool {
    cell.fg == Color::Foreground
        && cell.bg == Color::Background
        && cell.style == Style::default()
        && !cell.spacer
}

impl Highlights {
    #[cfg(test)]
    pub(crate) fn scan_count(&self) -> usize {
        self.scans
    }

    pub(crate) fn clear(&mut self) {
        self.key = None;
        self.spans = Vec::new();
    }

    pub(crate) fn update(&mut self, frame: &Frame, generation: u64, enabled: bool) {
        if !enabled {
            self.clear();
            return;
        }
        let key = Key {
            generation,
            size: frame.size,
            display_offset: frame.display_offset,
            alt_screen: frame.alt_screen,
        };
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.spans.clear();
        #[cfg(test)]
        {
            self.scans += 1;
        }
        let cols = usize::from(frame.size.cols);
        let mut row = 0;
        let mut remaining = FRAME_BYTES;
        let mut cell_budget = FRAME_BYTES;
        while row < frame.size.rows && remaining > 0 && self.spans.len() < MAX_SPANS {
            let start_complete = row != 0 || !frame.starts_with_continuation;
            let mut end_complete = true;
            let mut text = String::new();
            let mut mapping = Vec::new();
            let mut overflow = false;
            loop {
                let wraps = frame.wraps.get(usize::from(row)).copied().unwrap_or(false);
                let cells = frame.row(row);
                // Even blank rows consume scan budget; trimming must not walk
                // an arbitrarily large viewport that produces no text.
                if cells.len() > cell_budget {
                    return;
                }
                cell_budget -= cells.len();
                let end = if wraps {
                    cells.len()
                } else {
                    cells
                        .iter()
                        .rposition(|cell| cell.ch != ' ' || cell.spacer)
                        .map_or(0, |col| col + 1)
                };
                for (col, cell) in cells[..end].iter().enumerate() {
                    if cell.spacer {
                        continue;
                    }
                    let index = usize::from(row) * cols + col;
                    let marks = frame
                        .combining
                        .binary_search_by_key(&index, |(ix, _)| *ix)
                        .ok()
                        .map(|ix| frame.combining[ix].1.as_str())
                        .unwrap_or("");
                    let ch = if cell.style.hidden || cell.ch.is_control() {
                        '\0'
                    } else {
                        cell.ch
                    };
                    let marks = if cell.style.hidden { "" } else { marks };
                    let len = ch.len_utf8() + marks.len();
                    if len > GROUP_BYTES.saturating_sub(text.len()) || len > remaining {
                        overflow = true;
                        break;
                    }
                    let start = text.len();
                    text.push(ch);
                    text.push_str(marks);
                    mapping.push((start, text.len(), index));
                    remaining -= len;
                }
                row += 1;
                if !wraps || row == frame.size.rows {
                    end_complete = !wraps;
                    break;
                }
                if overflow {
                    // Skip the rest of an oversized logical line without scanning its cells.
                    while row < frame.size.rows {
                        let wraps = frame.wraps.get(usize::from(row)).copied().unwrap_or(false);
                        row += 1;
                        if !wraps {
                            break;
                        }
                    }
                    break;
                }
            }
            if overflow {
                continue;
            }
            let mut roles = vec![None; text.len()];
            for token in rules::recognize(&text, start_complete, end_complete) {
                for slot in &mut roles[token.start..token.end] {
                    if slot.is_none() {
                        *slot = Some(token.role);
                    }
                }
            }
            for &(start, _end, index) in &mapping {
                if index > u32::MAX as usize || !plain(&frame.cells[index]) {
                    continue;
                }
                let role = roles[start];
                let Some(role) = role else {
                    continue;
                };
                if let Some(last) = self.spans.last_mut()
                    && last.end as usize == index
                    && last.role == role
                {
                    last.end += 1;
                } else if self.spans.len() < MAX_SPANS {
                    self.spans.push(Span {
                        start: index as u32,
                        end: index as u32 + 1,
                        role,
                    });
                } else {
                    return;
                }
            }
        }
    }

    pub(crate) fn role_at(&self, index: usize, cell: &Cell) -> Option<Role> {
        if cell.selected || cell.search_hit || !plain(cell) {
            return None;
        }
        let next = self
            .spans
            .partition_point(|span| span.end as usize <= index);
        self.spans
            .get(next)
            .filter(|span| span.start as usize <= index)
            .map(|span| span.role)
    }
}
