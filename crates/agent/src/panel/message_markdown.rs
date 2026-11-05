//! Parsed Markdown of the visible chat's agent messages and reasoning, kept per row: a
//! row scrolled back into view keeps its parse, and text streamed into a row parses
//! only what was appended instead of the whole message again.
use super::{AgentPanel, message_list::MessageList, parts::PartKind, widgets::safe_markdown};
use gpui_kit::{AppContext as _, Context, Entity, Subscription, component::text::TextViewState};
use nocterm_ai::thread::Entry;
use std::collections::HashMap;

struct RowMarkdown {
    state: Entity<TextViewState>,
    text: String,
    source_len: Option<usize>,
    _parsed: Subscription,
}

#[derive(Default)]
pub(super) struct MessageMarkdown {
    rows: HashMap<usize, RowMarkdown>,
}
impl MessageMarkdown {
    /// The Markdown a row draws now: an agent message as revealed so far, or expanded
    /// reasoning. Other rows draw no kept Markdown.
    pub(super) fn text(
        entry: &Entry,
        row: usize,
        messages: &MessageList,
    ) -> Option<(String, usize)> {
        let source = match entry {
            Entry::Agent(text) => messages.revealed_slice(row, text),
            Entry::Thought(text) if messages.expanded(row) => text,
            _ => return None,
        };
        if messages.markdown_current(row, source.len()) {
            return None;
        }
        Some((safe_markdown(source), source.len()))
    }
    pub(super) fn current(&self, row: usize, source_len: usize) -> bool {
        self.rows
            .get(&row)
            .is_some_and(|kept| kept.source_len == Some(source_len))
    }
    pub(super) fn invalidate(&mut self, dirty: &[usize]) {
        for row in dirty {
            if let Some(kept) = self.rows.get_mut(row) {
                kept.source_len = None;
            }
        }
    }
    /// Shows `text` in the row's state, appending when it extends what the row showed.
    pub(super) fn prepare(
        &mut self,
        row: usize,
        text: String,
        source_len: usize,
        cx: &mut Context<AgentPanel>,
    ) {
        if let Some(kept) = self.rows.get_mut(&row) {
            kept.source_len = Some(source_len);
            if kept.text == text {
                return;
            }
            kept.state.update(cx, |state, cx| {
                match text.strip_prefix(kept.text.as_str()) {
                    Some(appended) => state.push_str(appended, cx),
                    None => state.set_text(&text, cx),
                }
            });
            kept.text = text;
            return;
        }
        let state = cx.new(|cx| TextViewState::markdown(&text, cx));
        // As an element's keyed state would: a finished parse redraws the transcript.
        let parsed = cx.observe(&state, |panel, _, cx| {
            panel.layout.parts().notify(PartKind::Transcript, cx);
        });
        self.rows.insert(
            row,
            RowMarkdown {
                state,
                text,
                source_len: Some(source_len),
                _parsed: parsed,
            },
        );
    }
    pub(super) fn state(&self, row: usize) -> Option<&Entity<TextViewState>> {
        self.rows.get(&row).map(|kept| &kept.state)
    }
    /// Forgets the rows a shortened transcript no longer has.
    pub(super) fn retain(&mut self, count: usize) {
        self.rows.retain(|row, _| *row < count);
    }
    pub(super) fn clear(&mut self) {
        self.rows.clear();
    }
}
