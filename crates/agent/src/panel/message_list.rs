//! Transcript presentation binds reveal, navigation and previews to one visible chat.
use super::{
    AgentPanel,
    image_pipeline::{ImageKey, ImagePipeline, PreviewState, PreviewTicket},
    navigation::MessageNavigation,
    stream::StreamReveal,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EntityId, Image, Task,
    component::message_scroller::{MessageScroller, MessageScrollerState},
};
use nocterm_ai::{
    history::physical::RowSlot,
    images::{ImageInspector, ImageThumbnailer},
};
use std::{collections::HashSet, sync::Arc, time::Duration};

#[derive(Clone)]
pub(super) struct ActiveListBinding {
    chat: EntityId,
    generation: Arc<()>,
}
impl ActiveListBinding {
    fn same(&self, other: &Self) -> bool {
        self.chat == other.chat && Arc::ptr_eq(&self.generation, &other.generation)
    }
}
pub(super) struct MessageRows {
    previous: usize,
    count: usize,
    changed: Vec<usize>,
}
pub(super) struct MessageList {
    pipeline: ImagePipeline,
    scroller: Entity<MessageScrollerState>,
    reveal: StreamReveal,
    tick: Option<Task<()>>,
    row_count: usize,
    navigation: MessageNavigation,
    expanded: HashSet<usize>,
    binding: Option<ActiveListBinding>,
}
impl MessageList {
    pub(super) fn new(images: crate::ImageDeps, cx: &mut Context<AgentPanel>) -> Self {
        Self {
            pipeline: ImagePipeline::new(images),
            scroller: cx.new(|cx| MessageScrollerState::new(0, cx)),
            reveal: StreamReveal::default(),
            tick: None,
            row_count: 0,
            navigation: MessageNavigation::default(),
            expanded: HashSet::new(),
            binding: None,
        }
    }
    pub(super) fn bind_chat(&mut self, chat: EntityId, cx: &mut Context<AgentPanel>) {
        if self
            .binding
            .as_ref()
            .is_some_and(|binding| binding.chat == chat)
        {
            return;
        }
        self.reset(cx);
        self.binding = Some(ActiveListBinding {
            chat,
            generation: Arc::new(()),
        });
    }
    pub(super) fn binding(&self) -> Option<ActiveListBinding> {
        self.binding.clone()
    }
    fn current(&self, binding: &ActiveListBinding) -> bool {
        self.binding
            .as_ref()
            .is_some_and(|current| current.same(binding))
    }
    pub(super) fn reset(&mut self, cx: &mut Context<AgentPanel>) {
        self.tick = None;
        self.reveal = StreamReveal::default();
        self.scroller.update(cx, |list, cx| list.reset(0, cx));
        self.row_count = 0;
        self.navigation.clear();
        self.expanded.clear();
        self.pipeline.clear();
        self.binding = None;
    }
    pub(super) fn observe_rows(
        &mut self,
        chat: EntityId,
        entries: &[RowSlot],
        dirty: &[usize],
        live: bool,
        reduced: bool,
    ) -> MessageRows {
        let previous = self.row_count;
        self.row_count = entries.len();
        self.navigation.observe(chat, entries, dirty);
        let mut changed = dirty.to_vec();
        changed.extend(self.reveal.observe(entries, dirty, live, reduced));
        MessageRows {
            previous,
            count: self.row_count,
            changed,
        }
    }
    pub(super) fn apply_rows(&mut self, rows: MessageRows, cx: &mut Context<AgentPanel>) {
        if rows.count != rows.previous {
            if rows.count > rows.previous {
                self.scroller
                    .update(cx, |list, cx| list.append(rows.count - rows.previous, cx));
            } else {
                self.scroller
                    .update(cx, |list, cx| list.reset(rows.count, cx));
            }
        }
        self.remeasure_rows(rows.changed, cx);
        self.schedule_tick(cx);
    }
    pub(super) fn advance_reveal(
        &mut self,
        binding: &ActiveListBinding,
        entries: &[RowSlot],
        reduced: bool,
    ) -> Option<Vec<usize>> {
        if !self.current(binding) {
            return None;
        }
        self.tick = None;
        Some(self.reveal.advance(entries, reduced))
    }
    pub(super) fn apply_reveal(&mut self, changed: Vec<usize>, cx: &mut Context<AgentPanel>) {
        self.remeasure_rows(changed, cx);
        self.schedule_tick(cx);
    }
    fn schedule_tick(&mut self, cx: &mut Context<AgentPanel>) {
        if !self.reveal.pending() || self.tick.is_some() {
            return;
        }
        let Some(binding) = self.binding() else {
            return;
        };
        self.tick = Some(cx.spawn(async move |panel, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let _ = panel.update(cx, |panel, cx| panel.on_reveal_tick(binding, cx));
        }));
    }
    fn remeasure_rows(&self, mut rows: Vec<usize>, cx: &mut Context<AgentPanel>) {
        if rows.is_empty() {
            return;
        }
        rows.sort_unstable();
        rows.dedup();
        self.scroller.update(cx, |list, cx| {
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
    pub(super) fn toggle_expanded(
        &mut self,
        binding: &ActiveListBinding,
        row: usize,
        cx: &mut Context<AgentPanel>,
    ) -> bool {
        if !self.current(binding) {
            return false;
        }
        if !self.expanded.insert(row) {
            self.expanded.remove(&row);
        }
        self.remeasure_rows(vec![row], cx);
        true
    }
    pub(super) fn expanded(&self, row: usize) -> bool {
        self.expanded.contains(&row)
    }
    pub(super) fn revealed_text(&self, row: usize, text: &str) -> String {
        self.reveal.text(row, text)
    }
    pub(super) fn revealing(&self) -> bool {
        self.reveal.pending()
    }
    pub(super) fn preview_state(&self, key: &ImageKey) -> Option<&PreviewState> {
        self.pipeline.state(key)
    }
    pub(super) fn has_preview(&self, key: &ImageKey) -> bool {
        self.pipeline.contains_key(key)
    }
    pub(super) fn request_preview(&mut self, key: ImageKey) -> Option<PreviewTicket> {
        self.pipeline.request(key)
    }
    pub(super) fn install_preview_task(&mut self, ticket: PreviewTicket, task: Task<()>) {
        self.pipeline.install_task(ticket, task);
    }
    pub(super) fn accept_preview(
        &mut self,
        binding: &ActiveListBinding,
        ticket: PreviewTicket,
        row: Option<usize>,
        image: Option<Arc<Image>>,
        cx: &mut Context<AgentPanel>,
    ) -> bool {
        if !self.current(binding) || !self.pipeline.complete(ticket, image) {
            return false;
        }
        if let Some(row) = row {
            self.remeasure_rows(vec![row], cx);
        }
        true
    }
    pub(super) fn inspector(&self) -> Arc<dyn ImageInspector> {
        self.pipeline.inspector()
    }
    pub(super) fn thumbnailer(&self) -> Arc<dyn ImageThumbnailer> {
        self.pipeline.thumbnailer()
    }
    pub(super) fn scroller(&self, cx: &mut Context<AgentPanel>) -> MessageScroller {
        MessageScroller::new(
            "agent-transcript",
            self.scroller.clone(),
            cx.processor(|panel, index, _, cx| panel.render_entry(index, cx)),
        )
    }
    pub(super) fn navigate(&self, first: bool, cx: &mut Context<AgentPanel>) {
        let top = self.scroller.read(cx).logical_scroll_top();
        let target = if first {
            self.navigation.first(top)
        } else {
            self.navigation.previous(top)
        };
        if let Some(index) = target {
            self.scroller
                .update(cx, |list, cx| list.scroll_to_item(index, cx));
        }
    }
    pub(super) fn navigation_targets(&self, cx: &App) -> Option<(Option<usize>, Option<usize>)> {
        let list = self.scroller.read(cx);
        if !list.is_scrolled_up() || !self.navigation.has_rows() {
            return None;
        }
        let top = list.logical_scroll_top();
        Some((self.navigation.first(top), self.navigation.previous(top)))
    }
}
