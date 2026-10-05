//! Display-only reveal visits changed rows and the live backlog, never the whole history.
use super::AgentPanel;
use gpui_kit::Context;
use nocterm_ai::thread::Entry;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
#[derive(Default)]
pub(super) struct StreamReveal {
    bytes: HashMap<usize, usize>,
    backlog: HashSet<usize>,
    known_rows: Option<usize>,
}
impl StreamReveal {
    pub(super) fn observe(
        &mut self,
        entries: &[Entry],
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
            let Some(Entry::Agent(text)) = entries.get(index) else {
                self.bytes.remove(&index);
                self.backlog.remove(&index);
                continue;
            };
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
                if let Some(Entry::Agent(text)) = entries.get(index) {
                    self.bytes.insert(index, text.len());
                    changed.push(index);
                }
            }
        }
        changed
    }
    pub(super) fn advance(&mut self, entries: &[Entry], reduced: bool) -> Vec<usize> {
        let mut changed = Vec::with_capacity(self.backlog.len());
        self.backlog.retain(|&index| {
            let Some(Entry::Agent(text)) = entries.get(index) else {
                return false;
            };
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
impl AgentPanel {
    pub(super) fn sync_stream(&mut self, dirty: &[usize], cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        let mut changed = dirty.to_vec();
        changed.extend(self.stream.observe(
            &thread.read(cx).state.entries,
            dirty,
            thread.read(cx).generating,
            cx.reduce_motion(),
        ));
        self.remeasure_rows(changed, cx);
        self.schedule_stream_tick(cx);
    }
    fn remeasure_rows(&mut self, mut rows: Vec<usize>, cx: &mut Context<Self>) {
        if rows.is_empty() {
            return;
        }
        rows.sort_unstable();
        rows.dedup();
        self.list.update(cx, |list, cx| {
            let mut start = rows[0];
            let mut end = start + 1;
            for index in rows.into_iter().skip(1) {
                if index == end {
                    end += 1;
                } else {
                    list.remeasure_items(start..end, cx);
                    start = index;
                    end = index + 1;
                }
            }
            list.remeasure_items(start..end, cx);
        });
    }
    fn schedule_stream_tick(&mut self, cx: &mut Context<Self>) {
        if !self.stream.pending() || self.stream_tick.is_some() {
            return;
        }
        self.stream_tick = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.stream_tick = None;
                let Some(thread) = this.current() else {
                    return;
                };
                let changed = this
                    .stream
                    .advance(&thread.read(cx).state.entries, cx.reduce_motion());
                this.remeasure_rows(changed, cx);
                this.schedule_stream_tick(cx);
                cx.notify();
            });
        }));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_backlog_visits_only_dirty_rows_and_advances_only_on_ticks() {
        let mut reveal = StreamReveal::default();
        let mut entries = (0..2000)
            .map(|_| Entry::Agent("historical".into()))
            .collect::<Vec<_>>();
        reveal.observe(&entries, &[], false, false);
        entries.push(Entry::Agent("Привет 👨‍👩‍👧‍👦 世界!".repeat(200)));
        let Entry::Agent(full) = &entries[2000] else {
            unreachable!()
        };
        reveal.observe(&entries, &[2000], true, false);
        assert_eq!(reveal.backlog_len(), 1);
        let initial = reveal.text(2000, full);
        for _ in 0..5000 {
            reveal.observe(&entries, &[], false, false);
            assert!(reveal.pending());
        }
        assert_eq!(reveal.text(2000, full), initial);
        reveal.advance(&entries, false);
        assert!(!reveal.text(2000, full).is_empty());
        assert!(reveal.text(2000, full).len() < full.len());
        for _ in 0..100 {
            reveal.advance(&entries, false);
        }
        assert_eq!(reveal.text(2000, full), *full);
    }
    #[test]
    fn historical_and_reduced_motion_text_is_immediate() {
        let mut reveal = StreamReveal::default();
        let mut entries = vec![Entry::Agent("old".into())];
        reveal.observe(&entries, &[], true, false);
        assert_eq!(reveal.text(0, "old"), "old");
        entries.push(Entry::Agent("new".into()));
        reveal.observe(&entries, &[1], true, true);
        assert!(!reveal.pending());
        assert_eq!(reveal.text(1, "new"), "new");
    }
}
