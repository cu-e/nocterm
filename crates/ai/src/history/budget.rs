//! Exact row and payload budgets are prepared once and reused on the UI thread.
use super::{MAX_SAVED_ENTRIES, SavedChat, check_size, prompt_size, size, wire};
use crate::thread::Entry;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct PreparedBudget {
    entries: Arc<Vec<usize>>,
    prompts: Arc<Vec<usize>>,
}
impl PreparedBudget {
    pub(super) fn new(chat: &SavedChat) -> Result<Self, String> {
        Ok(Self {
            entries: Arc::new(
                chat.entries
                    .iter()
                    .map(wire::entry_size)
                    .collect::<Result<_, _>>()
                    .map_err(|error| error.to_string())?,
            ),
            prompts: Arc::new(
                chat.queue
                    .iter()
                    .map(prompt_size)
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
    pub fn prompt_sizes(&self) -> &[usize] {
        &self.prompts
    }
}
#[derive(Default)]
pub struct HistoryBudget {
    entries: Vec<usize>,
}
impl HistoryBudget {
    pub fn restored(prepared: Option<&PreparedBudget>) -> Self {
        Self {
            entries: prepared.map_or_else(Vec::new, |prepared| (*prepared.entries).clone()),
        }
    }
    pub fn refresh(&mut self, entries: &[Entry], dirty: &[usize]) -> Result<(), String> {
        let old = self.entries.len();
        self.entries.resize(entries.len(), 0);
        for index in (old..entries.len())
            .chain(dirty.iter().copied())
            .filter(|index| *index < entries.len())
        {
            self.entries[index] =
                wire::entry_size(&entries[index]).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
    /// Metadata has empty entries and queue; cached elements fill those arrays exactly.
    pub fn validate(
        &self,
        metadata: &SavedChat,
        prompts: impl Iterator<Item = usize>,
    ) -> Result<(), String> {
        let mut metadata_size =
            size::encoded_len(&wire::Wire::new(metadata, None::<&[super::SavedPrompt]>))
                .map_err(|error| error.to_string())?;
        let entries = &self.entries[self.entries.len().saturating_sub(MAX_SAVED_ENTRIES)..];
        metadata_size += entries.iter().sum::<usize>() + entries.len().saturating_sub(1);
        let prompts = prompts.collect::<Vec<_>>();
        if !prompts.is_empty() {
            // The same serializer provides the field/empty-array framing; no mirrored field formula.
            let mut with_queue = SavedChat::new(metadata.agent_id.clone());
            with_queue.queue.push(super::SavedPrompt {
                id: 0,
                text: String::new(),
                images: Vec::new(),
                attachments: Vec::new(),
            });
            let framing = size::encoded_len(&wire::Wire::new(
                &with_queue,
                Some(&Vec::<super::SavedPrompt>::new()),
            ))
            .map_err(|error| error.to_string())?
                - size::encoded_len(&wire::Wire::new(&with_queue, None::<&[super::SavedPrompt]>))
                    .map_err(|error| error.to_string())?;
            metadata_size += framing + prompts.iter().sum::<usize>() + prompts.len() - 1;
        }
        check_size(metadata_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        acp,
        history::{MAX_FILE_BYTES, SavedAttachment, SavedPrompt},
    };
    #[test]
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn bounded_wire_and_cached_budget_match_actual_acp_json_and_exact_limit() {
        let mut chat = SavedChat::new("agent\"\\\n界".into());
        chat.title = Some("title\u{0}\n".into());
        chat.name = Some("name".into());
        chat.session_id = Some("session".into());
        chat.workdir = Some("/tmp/界".into());
        chat.model = Some("model".into());
        chat.pinned = true;
        chat.pending_history = true;
        chat.updated = u64::MAX;
        chat.draft = Some("literal draft\n\0界\"\\".into());
        chat.attachments = vec![
            SavedAttachment::LocalTerminal("local".into()),
            SavedAttachment::Connection("remote".into()),
        ];
        chat.entries = (0..2010)
            .map(|index| Entry::Agent(format!("entry {index}")))
            .collect();
        let blocks = vec![
            serde_json::json!({"type":"text", "text":"quote\"\\\n界", "_meta":{"number":0.1,"null":null}}),
            serde_json::json!({"type":"image", "data":"AAAA", "mimeType":"image/png"}),
            serde_json::json!({"type":"audio", "data":"AAAA", "mimeType":"audio/wav"}),
            serde_json::json!({"type":"resource_link", "name":"link", "uri":"file:///界"}),
            serde_json::json!({"type":"resource", "resource":{"uri":"file:///doc", "text":"body"}}),
            serde_json::json!({"type":"resource", "resource":{"uri":"file:///blob", "blob":"AAAA"}}),
        ].into_iter().map(|value| serde_json::from_value::<acp::ContentBlock>(value).unwrap()).collect::<Vec<_>>();
        chat.entries
            .extend(blocks.iter().cloned().map(Entry::Content));
        chat.entries.push(Entry::User(blocks.clone()));
        chat.entries.push(Entry::Thought("thinking\t界".into()));
        chat.entries.push(Entry::Tool(
            acp::ToolCall::new("id", "title")
                .raw_input(serde_json::json!({"nested":[1,-2,0.1,true,null,"\u{1}\"\\界"]}))
                .raw_output(serde_json::json!({"ok":false, "value":1e100})),
        ));
        chat.entries.push(Entry::Content(acp::ContentBlock::Image(
            acp::ImageContent::new(
                "A".repeat(super::super::MAX_SAVED_IMAGE_BYTES + 1),
                "image/png",
            ),
        )));
        chat.times = (0..chat.entries.len() as u64).collect();
        chat.queue = vec![SavedPrompt {
            id: u64::MAX,
            text: "queue\n界".into(),
            images: blocks,
            attachments: chat.attachments.clone(),
        }];
        let wire = wire::Wire::new(&chat, Some(&chat.queue));
        let expected = serde_json::to_vec(&wire).unwrap();
        assert_eq!(size::encoded_len(&wire).unwrap(), expected.len());
        let mut materialized = chat.clone();
        materialized.entries = SavedChat::bounded_entries(&chat.entries);
        materialized.times = SavedChat::bounded_times(chat.entries.len(), &chat.times);
        assert_eq!(serde_json::to_vec(&materialized).unwrap(), expected);
        assert_eq!(materialized.times[0], (chat.entries.len() - 2000) as u64);
        let prepared = PreparedBudget::new(&chat).unwrap();
        let mut budget = HistoryBudget::restored(Some(&prepared));
        let mut metadata = materialized.clone();
        metadata.entries.clear();
        metadata.queue.clear();
        assert!(
            budget
                .validate(&metadata, prepared.prompt_sizes().iter().copied())
                .is_ok()
        );
        // One changed row updates the cache without rebuilding unrelated rows.
        *chat.entries.last_mut().unwrap() = Entry::Agent("changed\n界".into());
        budget
            .refresh(&chat.entries, &[chat.entries.len() - 1])
            .unwrap();
        let mut prompt = SavedPrompt {
            id: 0,
            text: String::new(),
            images: Vec::new(),
            attachments: Vec::new(),
        };
        let mut wire_chat = chat.clone();
        wire_chat.queue = vec![prompt.clone()];
        let fixed =
            size::encoded_len(&wire::Wire::new(&wire_chat, Some(&wire_chat.queue))).unwrap();
        prompt.text = "a".repeat(MAX_FILE_BYTES as usize - fixed);
        assert!(
            budget
                .validate(
                    &metadata,
                    std::iter::once(super::super::prompt_size(&prompt).unwrap())
                )
                .is_ok()
        );
        prompt.text.push('a');
        assert!(
            budget
                .validate(
                    &metadata,
                    std::iter::once(super::super::prompt_size(&prompt).unwrap())
                )
                .is_err()
        );
    }
}
