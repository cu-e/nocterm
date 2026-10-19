//! Display-only reveal visits changed rows and the live backlog, never the whole history.
use nocterm_ai::history::physical::{RowBody, RowSlot};
use std::collections::{HashMap, HashSet};
#[derive(Default)]
pub(super) struct StreamReveal {
    bytes: HashMap<usize, usize>,
    backlog: HashSet<usize>,
    known_rows: Option<usize>,
}
impl StreamReveal {
    pub(super) fn observe(
        &mut self,
        entries: &[RowSlot],
        dirty: &[usize],
        live: bool,
        reduced: bool,
    ) -> Vec<usize> {
        let Some(known) = self.known_rows else {
            self.known_rows = Some(entries.len());
            return Vec::new();
        };
        self.known_rows = Some(entries.len());
        let mut changed = Vec::new();
        for &index in dirty {
            let Some(RowBody::Agent(text)) = entries.get(index).map(RowSlot::view) else {
                self.bytes.remove(&index);
                self.backlog.remove(&index);
                continue;
            };
            let text = text.read();
            let shown = self
                .bytes
                .entry(index)
                .or_insert(if index < known || !live || reduced {
                    text.len()
                } else {
                    0
                });
            if *shown > text.len() {
                *shown = text.len();
            }
            while !text.is_char_boundary(*shown) {
                *shown -= 1;
            }
            if *shown < text.len() {
                self.backlog.insert(index);
            }
        }
        if reduced {
            for index in self.backlog.drain() {
                if let Some(RowBody::Agent(text)) = entries.get(index).map(RowSlot::view) {
                    self.bytes.insert(index, text.read().len());
                    changed.push(index);
                }
            }
        }
        changed
    }
    pub(super) fn advance(&mut self, entries: &[RowSlot], reduced: bool) -> Vec<usize> {
        let mut changed = Vec::with_capacity(self.backlog.len());
        self.backlog.retain(|&index| {
            let Some(RowBody::Agent(text)) = entries.get(index).map(RowSlot::view) else {
                return false;
            };
            let text = text.read();
            let shown = self
                .bytes
                .get_mut(&index)
                .expect("backlog owns reveal position");
            let mut end = if reduced {
                text.len()
            } else {
                (*shown + (text.len() - *shown).div_ceil(6).max(12)).min(text.len())
            };
            while !text.is_char_boundary(end) {
                end += 1;
            }
            *shown = end;
            changed.push(index);
            end < text.len()
        });
        changed
    }
    pub(super) fn text(&self, index: usize, text: &str) -> String {
        let mut end = self
            .bytes
            .get(&index)
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text[..end].to_owned()
    }
    pub(super) fn pending(&self) -> bool {
        !self.backlog.is_empty()
    }
    #[cfg(test)]
    pub(super) fn backlog_len(&self) -> usize {
        self.backlog.len()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_ai::{testing::ManagedRows, thread::Entry};
    #[test]
    fn unicode_backlog_visits_only_dirty_rows_and_advances_only_on_ticks() {
        let mut reveal = StreamReveal::default();
        let mut entries = (0..2000)
            .map(|_| Entry::Agent("historical".into()))
            .collect::<Vec<_>>();
        let mut rows = ManagedRows::new(entries.clone()).unwrap();
        reveal.observe(rows.rows(), &[], false, false);
        entries.push(Entry::Agent("Привет 👨‍👩‍👧‍👦 世界!".repeat(200)));
        rows.replace(entries.clone()).unwrap();
        let Entry::Agent(full) = &entries[2000] else {
            unreachable!()
        };
        reveal.observe(rows.rows(), &[2000], true, false);
        assert_eq!(reveal.backlog_len(), 1);
        let initial = reveal.text(2000, full);
        for _ in 0..5000 {
            reveal.observe(rows.rows(), &[], false, false);
            assert!(reveal.pending());
        }
        assert_eq!(reveal.text(2000, full), initial);
        reveal.advance(rows.rows(), false);
        assert!(!reveal.text(2000, full).is_empty());
        assert!(reveal.text(2000, full).len() < full.len());
        for _ in 0..100 {
            reveal.advance(rows.rows(), false);
        }
        assert_eq!(reveal.text(2000, full), *full);
    }
    #[test]
    fn historical_and_reduced_motion_text_is_immediate() {
        let mut reveal = StreamReveal::default();
        let mut entries = vec![Entry::Agent("old".into())];
        let mut rows = ManagedRows::new(entries.clone()).unwrap();
        reveal.observe(rows.rows(), &[], true, false);
        assert_eq!(reveal.text(0, "old"), "old");
        entries.push(Entry::Agent("new".into()));
        rows.replace(entries).unwrap();
        reveal.observe(rows.rows(), &[1], true, true);
        assert!(!reveal.pending());
        assert_eq!(reveal.text(1, "new"), "new");
    }
}
