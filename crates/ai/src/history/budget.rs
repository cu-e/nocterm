//! Exact row and payload budgets are prepared once and reused on the UI thread.
use super::{HistoryCodecError, MAX_SAVED_ENTRIES, SavedChat, check_size, prompt_size, size, wire};
use crate::thread::Entry;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct PreparedBudget {
    pub(super) resident_bytes: usize,
    entries: Arc<Vec<usize>>,
    memory: Arc<Vec<usize>>,
    prompts: Arc<Vec<usize>>,
    preview: super::catalog_header::Preview,
}
impl PreparedBudget {
    pub(super) fn new(chat: &SavedChat) -> Result<Self, HistoryCodecError> {
        Ok(Self {
            resident_bytes: 0,
            preview: super::catalog_header::Preview::persisted(
                &chat.entries[SavedChat::saved_entry_start(chat.entries.len())..],
            ),
            memory: Arc::new(
                chat.entries
                    .iter()
                    .map(wire::entry_memory)
                    .collect::<Result<_, _>>()
                    .map_err(HistoryCodecError::from)?,
            ),
            entries: Arc::new(
                chat.entries
                    .iter()
                    .map(wire::entry_size)
                    .collect::<Result<_, _>>()
                    .map_err(HistoryCodecError::from)?,
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
    memory: Vec<usize>,
    entries: Vec<usize>,
    preview: super::catalog_header::Preview,
}
impl HistoryBudget {
    pub fn resident_bytes(&self) -> usize {
        self.memory.iter().sum()
    }
    pub fn restored(prepared: Option<&PreparedBudget>) -> Self {
        Self {
            preview: prepared.map_or_else(Default::default, |prepared| prepared.preview.clone()),
            memory: prepared.map_or_else(Vec::new, |prepared| (*prepared.memory).clone()),
            entries: prepared.map_or_else(Vec::new, |prepared| (*prepared.entries).clone()),
        }
    }
    pub fn refresh(&mut self, entries: &[Entry], dirty: &[usize]) -> Result<(), HistoryCodecError> {
        self.preview = super::catalog_header::Preview::persisted(
            &entries[SavedChat::saved_entry_start(entries.len())..],
        );
        let old = self.entries.len();
        self.entries.resize(entries.len(), 0);
        self.memory.resize(entries.len(), 0);
        for index in (old..entries.len())
            .chain(dirty.iter().copied())
            .filter(|index| *index < entries.len())
        {
            self.memory[index] =
                wire::entry_memory(&entries[index]).map_err(HistoryCodecError::from)?;
            self.entries[index] =
                wire::entry_size(&entries[index]).map_err(HistoryCodecError::from)?;
        }
        Ok(())
    }
    pub fn validate_memory(
        &self,
        metadata: &SavedChat,
        prompts: impl Iterator<Item = usize>,
    ) -> Result<(), HistoryCodecError> {
        let rows = &self.memory[self.memory.len().saturating_sub(MAX_SAVED_ENTRIES)..];
        let (count, prompts) =
            prompts.fold((0, 0), |(count, total), bytes| (count + 1, total + bytes));
        let base = size::structural_size(&wire::Wire::budget(
            metadata,
            None::<&[super::SavedPrompt]>,
            self.preview.clone().with_prompts(count != 0),
        ))
        .map_err(HistoryCodecError::from)?;
        super::check_memory(
            base + rows.iter().sum::<usize>() + prompts + if count == 0 { 0 } else { 2048 },
        )
    }
    /// Metadata has empty entries and queue; cached elements fill those arrays exactly.
    pub fn validate(
        &self,
        metadata: &SavedChat,
        prompts: impl Iterator<Item = usize>,
    ) -> Result<(), HistoryCodecError> {
        let prompts = prompts.collect::<Vec<_>>();
        let preview = self.preview.clone().with_prompts(!prompts.is_empty());
        let mut metadata_size = size::encoded_len(&wire::Wire::budget(
            metadata,
            None::<&[super::SavedPrompt]>,
            preview.clone(),
        ))
        .map_err(HistoryCodecError::from)?;
        let entries = &self.entries[self.entries.len().saturating_sub(MAX_SAVED_ENTRIES)..];
        metadata_size += entries.iter().sum::<usize>() + entries.len().saturating_sub(1);
        if !prompts.is_empty() {
            // Keep the authoritative projection fixed while measuring queue framing.
            let framing = size::encoded_len(&wire::Wire::budget(
                metadata,
                Some(&Vec::<super::SavedPrompt>::new()),
                preview.clone(),
            ))
            .map_err(HistoryCodecError::from)?
                - size::encoded_len(&wire::Wire::budget(
                    metadata,
                    None::<&[super::SavedPrompt]>,
                    preview,
                ))
                .map_err(HistoryCodecError::from)?;
            metadata_size += framing + prompts.iter().sum::<usize>() + prompts.len() - 1;
        }
        check_size(metadata_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{MAX_FILE_BYTES, SavedAttachment, SavedPrompt};
    use crate::model::{content::Content, tools::ToolRecord};
    #[test]
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn bounded_wire_and_cached_budget_match_actual_acp_json_and_exact_limit() {
        let mut chat = SavedChat::for_identity(
            "agent\"\\\n界".into(),
            "00000000-0000-4000-8000-000000000064".into(),
            1,
        );
        chat.header.title = Some("title\u{0}\n".into());
        chat.header.name = Some("name".into());
        chat.session_id = Some("session".into());
        chat.workdir = Some("/tmp/界".into());
        chat.model = Some("model".into());
        chat.header.pinned = true;
        chat.pending_history = true;
        chat.header.updated = u64::MAX;
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
        ].into_iter().map(|value| serde_json::from_value::<Content>(value).unwrap()).collect::<Vec<_>>();
        chat.entries
            .extend(blocks.iter().cloned().map(Entry::Content));
        chat.entries.push(Entry::User(blocks.clone()));
        chat.entries.push(Entry::Thought("thinking\t界".into()));
        chat.entries.push(Entry::Tool(
            ToolRecord::new("id", "title")
                .raw_input(serde_json::json!({"nested":[1,-2,0.1,true,null,"\u{1}\"\\界"]}))
                .raw_output(serde_json::json!({"ok":false, "value":1e100})),
        ));
        chat.entries.push(Entry::Content(Content::Image(
            crate::model::content::Image::new(
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
    #[test]
    fn fallback_title_budget_tracks_image_replacement_dirty_rows_and_an_empty_window() {
        fn exact_boundary(budget: &HistoryBudget, chat: &SavedChat) {
            let mut with_queue = chat.clone();
            with_queue.queue = vec![SavedPrompt {
                id: 1,
                text: String::new(),
                images: Vec::new(),
                attachments: Vec::new(),
            }];
            let framing = super::super::encode(&with_queue).unwrap().len();
            let mut prompt = with_queue.queue.remove(0);
            prompt.text = "a".repeat(MAX_FILE_BYTES as usize - framing);
            let mut metadata = chat.clone();
            metadata.entries.clear();
            metadata.queue.clear();
            assert!(
                budget
                    .validate(
                        &metadata,
                        std::iter::once(super::super::prompt_size(&prompt).unwrap())
                    )
                    .is_ok()
            );
            prompt.text.push('a');
            assert!(matches!(
                budget.validate(
                    &metadata,
                    std::iter::once(super::super::prompt_size(&prompt).unwrap())
                ),
                Err(HistoryCodecError::EncodedLimit)
            ));
        }
        let mut chat = SavedChat::for_identity(
            "agent".into(),
            "00000000-0000-4000-8000-000000000001".into(),
            1,
        );
        chat.entries.push(Entry::User(vec![
            Content::Image(crate::model::content::Image::new(
                "A".repeat(super::super::MAX_SAVED_IMAGE_BYTES + 1),
                "image/png",
            )),
            Content::text("question after image"),
        ]));
        let mut budget = HistoryBudget::default();
        budget.refresh(&chat.entries, &[]).unwrap();
        exact_boundary(&budget, &chat);
        chat.entries = vec![Entry::User(vec![Content::text("changed first question")])];
        budget.refresh(&chat.entries, &[0]).unwrap();
        exact_boundary(&budget, &chat);
        chat.entries.clear();
        budget.refresh(&chat.entries, &[]).unwrap();
        exact_boundary(&budget, &chat);
    }
}
