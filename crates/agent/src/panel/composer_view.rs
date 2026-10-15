//! The actual composer control and its disposable edit, command and queue presentation.
mod commands;
use super::{AgentPanel, commands::CommandState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EntityId, FocusHandle, Focusable as _, Pixels,
    SharedString, Subscription, Task, Window,
    component::input::{InputEvent, Textarea, TextareaState},
    px,
};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
pub(super) struct ComposerBinding {
    chat: EntityId,
    identity: Arc<()>,
}
impl ComposerBinding {
    fn same(&self, other: &Self) -> bool {
        self.chat == other.chat && Arc::ptr_eq(&self.identity, &other.identity)
    }
}
#[derive(Default)]
struct ComposerState {
    edit: Option<QueueEdit>,
    loading: Option<(EntityId, String)>,
    revision: u64,
    pending_images: usize,
}
struct QueueEdit {
    chat: EntityId,
    id: u64,
    draft: String,
    ready: bool,
    preparation: Option<Task<()>>,
}
#[derive(Default)]
struct QueueView {
    open: bool,
    heights: HashMap<EntityId, Pixels>,
}
pub(super) struct EditRestoration {
    chat: EntityId,
    draft: String,
}
impl EditRestoration {
    pub(super) fn into_parts(self) -> (EntityId, String) {
        (self.chat, self.draft)
    }
}
pub(super) struct ComposerView {
    input: Entity<TextareaState>,
    queue: QueueView,
    state: ComposerState,
    commands: CommandState,
    _bindings: Vec<Subscription>,
    binding: Option<ComposerBinding>,
}
impl ComposerView {
    pub(super) fn new(window: &mut Window, cx: &mut Context<AgentPanel>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 10)
                .submit_on_enter(true)
                .placeholder("Ask about your terminals…")
        });
        let submit = cx.subscribe_in(&input, window, |panel, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                panel.submit_composer(window, cx);
            }
        });
        let observe = cx.observe_in(&input, window, |panel, _, window, cx| {
            panel.on_composer_input_changed(window, cx)
        });
        Self {
            input,
            queue: QueueView::default(),
            state: ComposerState::default(),
            commands: CommandState::default(),
            _bindings: vec![submit, observe],
            binding: None,
        }
    }
    pub(super) fn text(&self, cx: &App) -> SharedString {
        self.input.read(cx).value()
    }
    pub(super) fn focus(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
    pub(super) fn set_text(
        &self,
        text: impl Into<gpui_kit::component::input::InputContent>,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        self.input
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }
    pub(super) fn textarea(&self) -> Textarea {
        Textarea::new(&self.input)
    }
    pub(super) fn revision(&self) -> u64 {
        self.state.revision
    }
    pub(super) fn editing(&self) -> bool {
        self.state.edit.is_some()
    }
    pub(super) fn editing_id(&self) -> Option<u64> {
        self.state.edit.as_ref().map(|edit| edit.id)
    }
    pub(super) fn editing_chat(&self) -> Option<EntityId> {
        self.state.edit.as_ref().map(|edit| edit.chat)
    }
    pub(super) fn replacement_for(&self, chat: EntityId) -> Option<u64> {
        self.state
            .edit
            .as_ref()
            .filter(|edit| edit.chat == chat)
            .map(|edit| edit.id)
    }
    pub(super) fn preparing(&self) -> bool {
        self.state.pending_images > 0 || self.preparing_edit()
    }
    pub(super) fn preparing_edit(&self) -> bool {
        self.state.edit.as_ref().is_some_and(|edit| !edit.ready)
    }
    pub(super) fn original_draft(&self) -> Option<&str> {
        self.state.edit.as_ref().map(|edit| edit.draft.as_str())
    }
    pub(super) fn waiting_for_draft(&self, text: &str) -> bool {
        self.state
            .loading
            .as_ref()
            .is_some_and(|(_, previous)| previous == text)
    }
    fn invalidate(&mut self) {
        self.state.revision = self.state.revision.wrapping_add(1);
        self.state.pending_images = 0;
        if let Some(binding) = &mut self.binding {
            binding.identity = Arc::new(());
        }
    }
    pub(super) fn clear(&mut self) {
        self.commands.reset();
        self.state = ComposerState::default();
        self.queue = QueueView::default();
        self.binding = None;
    }
    pub(super) fn leave(&mut self) {
        self.commands.reset();
        self.invalidate();
        self.queue.open = false;
    }
    pub(super) fn take_edit(&mut self) -> Option<EditRestoration> {
        let edit = self.state.edit.take()?;
        self.invalidate();
        Some(EditRestoration {
            chat: edit.chat,
            draft: edit.draft,
        })
    }
    pub(super) fn bind_loading(&mut self, chat: EntityId, cx: &App) -> ComposerBinding {
        let binding = ComposerBinding {
            chat,
            identity: Arc::new(()),
        };
        self.binding = Some(binding.clone());
        self.state.loading = Some((chat, self.text(cx).to_string()));
        binding
    }
    pub(super) fn current(&self, binding: &ComposerBinding) -> bool {
        self.binding
            .as_ref()
            .is_some_and(|current| current.same(binding))
    }
    pub(super) fn finish_loading(
        &mut self,
        binding: &ComposerBinding,
        draft: String,
        archived: bool,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) -> bool {
        if !self.current(binding) {
            return false;
        }
        self.state.loading = archived.then(|| (binding.chat, String::new()));
        self.set_text(draft, window, cx);
        true
    }
    pub(super) fn draft_changed(&mut self, chat: EntityId, text: &str) -> bool {
        if self.editing() {
            return false;
        }
        if let Some((owner, previous)) = &self.state.loading {
            if *owner != chat || previous == text {
                return false;
            }
            self.state.loading = None;
            self.invalidate();
        }
        true
    }
    pub(super) fn begin_edit(
        &mut self,
        chat: EntityId,
        id: u64,
        draft: String,
        ready: bool,
    ) -> ComposerBinding {
        self.invalidate();
        self.state.loading = None;
        let binding = ComposerBinding {
            chat,
            identity: Arc::new(()),
        };
        self.binding = Some(binding.clone());
        self.state.edit = Some(QueueEdit {
            chat,
            id,
            draft,
            ready,
            preparation: None,
        });
        binding
    }
    pub(super) fn install_edit_preparation(
        &mut self,
        binding: &ComposerBinding,
        id: u64,
        task: Task<()>,
    ) {
        if self.current(binding)
            && let Some(edit) = self.state.edit.as_mut().filter(|edit| edit.id == id)
        {
            edit.preparation = Some(task);
        }
    }
    pub(super) fn edit_preparation_current(&self, binding: &ComposerBinding, id: u64) -> bool {
        self.current(binding)
            && self
                .state
                .edit
                .as_ref()
                .is_some_and(|edit| edit.id == id && !edit.ready)
    }
    pub(super) fn edit_images_accepted(&mut self, binding: &ComposerBinding, id: u64) {
        if self.current(binding)
            && let Some(edit) = self.state.edit.as_mut().filter(|edit| edit.id == id)
        {
            edit.ready = true;
        }
    }
    pub(super) fn prepare_images(&mut self, chat: EntityId) -> ComposerBinding {
        let binding = match self.binding.as_ref().filter(|binding| binding.chat == chat) {
            Some(binding) => binding.clone(),
            None => {
                let binding = ComposerBinding {
                    chat,
                    identity: Arc::new(()),
                };
                self.binding = Some(binding.clone());
                binding
            }
        };
        self.state.pending_images += 1;
        binding
    }
    pub(super) fn images_completed(&mut self, binding: &ComposerBinding) -> bool {
        if !self.current(binding) {
            return false;
        }
        self.state.pending_images = self.state.pending_images.saturating_sub(1);
        true
    }
    pub(super) fn queue_open(&self) -> bool {
        self.queue.open
    }
    pub(super) fn open_queue(&mut self) {
        self.queue.open = true;
    }
    pub(super) fn toggle_queue(&mut self) {
        self.queue.open = !self.queue.open;
    }
    pub(super) fn queue_height(&self, chat: EntityId) -> Pixels {
        self.queue.heights.get(&chat).copied().unwrap_or(px(0.))
    }
    pub(super) fn observe_queue_height(&mut self, chat: EntityId, height: Pixels) -> bool {
        if self
            .queue
            .heights
            .get(&chat)
            .is_some_and(|old| (*old - height).abs() <= px(0.5))
        {
            return false;
        }
        self.queue.heights.insert(chat, height);
        true
    }
    pub(super) fn removed_chats(&mut self, chats: &[EntityId]) {
        self.queue.heights.retain(|id, _| !chats.contains(id));
    }
}
