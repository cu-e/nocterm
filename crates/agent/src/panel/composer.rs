use super::*;
use super::{
    commands::CommandOrigin,
    images::ImageAttachError,
    menu::{ConfigurationSelection, FavoriteSelection, ModeSelection},
};
use crate::thread::Attachment;
use gpui_kit::{AnyElement, PathPromptOptions};
use nocterm_ai::images::PromptImage;
use nocterm_ai::model::ids::AuthMethodId;
use nocterm_ai::model::usage::ContextUsage;
use std::path::PathBuf;

impl AgentPanel {
    pub(super) fn on_composer_input_changed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.composer.observe_input(cx) {
            return;
        }
        if self.menus.is_open(&MenuKind::Usage) {
            self.close_usage(window, cx);
        }
        self.sync_composer_draft(cx);
        self.sync_command_token(window, cx);
        cx.notify();
    }
    pub(super) fn submit_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.send(window, cx);
    }
    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preparing_images() {
            return;
        }
        let Some(thread) = self.current() else {
            return;
        };
        let text = if self.composer.editing() {
            self.composer.text(cx).to_string()
        } else {
            self.original_composer_text(cx)
        };
        let replace = self.composer.replacement_for(thread.entity_id());
        let result = thread.update(cx, |thread, cx| thread.submit(text, replace, cx));
        match result {
            Ok(true) => {
                self.composer.set_text("", window, cx);
                self.finish_queue_edit(window, cx);
            }
            Ok(false) => return,
            Err(error) => {
                self.notices.set_error(Some(error.to_string()));
                cx.notify();
                return;
            }
        }
        self.menus.reset();
        cx.notify();
    }
    pub(super) fn toggle_menu(&mut self, menu: MenuKind, cx: &mut Context<Self>) {
        self.menus.toggle(menu);
        cx.notify();
    }
    pub(super) fn reset_commands(&mut self) {
        self.composer.reset_commands();
    }
    pub(super) fn refresh_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        let key = (
            thread.entity_id(),
            thread.read(cx).configuration().commands_revision(),
        );
        let commands = thread.read(cx).configuration().commands().to_vec();
        if self.composer.refresh_commands(key, &commands, window, cx) {
            cx.notify();
        }
    }
    pub(super) fn command_matches(&self, cx: &App) -> Vec<usize> {
        self.current()
            .map(|thread| {
                self.composer
                    .command_matches(thread.read(cx).configuration().commands(), cx)
            })
            .unwrap_or_default()
    }
    pub(super) fn sync_command_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commands = self
            .current()
            .map(|thread| thread.read(cx).configuration().commands().to_vec())
            .unwrap_or_default();
        self.composer.sync_command_token(&commands, window, cx);
    }
    pub(super) fn complete_command(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.confirm_command(name, window, cx);
        cx.notify();
    }
    pub(super) fn command_action(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.composer.focus(cx).is_focused(window) {
            return;
        }
        let indices = self.command_matches(cx);
        if indices.is_empty()
            || !matches!(
                key,
                "up" | "down" | "escape" | "tab" | "shift-tab" | "enter"
            )
        {
            return;
        }
        let Some(thread) = self.current() else {
            return;
        };
        let names = indices
            .iter()
            .map(|index| {
                thread.read(cx).configuration().commands()[*index]
                    .name
                    .clone()
            })
            .collect::<Vec<_>>();
        self.composer.command_action(key, &names, window, cx);
        cx.stop_propagation();
        cx.notify();
    }
    pub(super) fn render_commands(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.composer.command_menu(self.current(), window, cx)
    }
    pub(super) fn on_command_selected(
        &mut self,
        origin: CommandOrigin,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.current().is_some_and(|thread| {
            origin.matches(
                thread.entity_id(),
                thread.read(cx).configuration().commands_revision(),
            )
        }) {
            self.complete_command(name, window, cx);
        }
    }
    pub(super) fn on_command_token_clicked(
        &mut self,
        range: std::ops::Range<usize>,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.select_command_token(range, window, cx);
    }
    pub(super) fn render_composer(
        &mut self,
        thread: &Entity<AgentThread>,
        usage: Option<&ContextUsage>,
        labels: &attachments::AttachmentLabels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chips =
            composer_renderer::ComposerRenderer::render_attachment_chips(thread, labels, cx);
        let previews =
            preview_operation::PreviewOperation::composer_previews(&mut self.messages, thread, cx);
        let images = composer_renderer::ComposerRenderer::render_image_strip(
            thread.entity_id(),
            previews,
            cx,
        );
        let input = composer_renderer::ComposerRenderer::render_input(&self.composer, cx);
        let footer = composer_renderer::ComposerRenderer::render_footer(
            &self.composer,
            &self.menus,
            thread,
            usage,
            cx,
        );
        let notices = composer_renderer::ComposerRenderer::render_notices(
            &self.composer,
            self.host.sign_in().is_some(),
            thread,
            cx,
        );
        composer_renderer::ComposerRenderer::compose(
            chips,
            images,
            input,
            footer,
            notices,
            self.command_matches(cx),
            cx,
        )
    }
    pub(super) fn prepare_images(
        &mut self,
        work: impl std::future::Future<Output = Result<Vec<PromptImage>, ImageAttachError>>
        + Send
        + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.current() else {
            return;
        };
        if self.preparing_queue_edit()
            || !thread
                .read(cx)
                .agent_info()
                .is_some_and(|info| info.capabilities.image_prompts)
        {
            return;
        }
        let binding = self.composer.prepare_images(thread.entity_id());
        cx.notify();
        let ticket = thread.read(cx).session_ticket();
        let owner = thread.downgrade();
        image_preparation::ImagePreparation::new(binding, owner, ticket, Box::pin(work)).launch(cx);
    }
    pub(super) fn on_composer_images_prepared(
        &mut self,
        binding: composer_view::ComposerBinding,
        owner: gpui_kit::WeakEntity<crate::thread::AgentThread>,
        ticket: nocterm_ai::session::Ticket,
        result: Result<Vec<PromptImage>, ImageAttachError>,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = owner.upgrade() else {
            return;
        };
        if !self.settings.setting::<nocterm_ai::AiSettings>(cx).enabled
            || self
                .current()
                .is_none_or(|current| current.entity_id() != thread.entity_id())
            || !self.composer.images_completed(&binding)
        {
            return;
        }
        thread.update(cx, |thread, cx| {
            if !thread.session_request_current(ticket) {
                return;
            }
            match result {
                Ok(images) => {
                    let mut collection = thread.composer().images().to_vec();
                    collection.extend(images);
                    if let Err(error) = thread.accept_composer_images(collection, cx) {
                        self.notices.set_error(Some(error.to_string()));
                    }
                }
                Err(error) => self.notices.set_error(Some(error.to_string())),
            }
            cx.notify();
        });
        cx.notify();
    }
    pub(super) fn add_image_bytes(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let inspector = self.messages.inspector();
        self.prepare_images(
            async move { Ok(vec![PromptImage::validate(bytes, inspector.as_ref())?]) },
            cx,
        );
    }
    pub(super) fn add_images(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let inspector = self.messages.inspector();
        self.prepare_images(
            async move { images::read_images(paths, inspector.as_ref()) },
            cx,
        );
    }
    pub(super) fn on_image_paths_picked(
        &mut self,
        operation: image_picker::ImagePickerOperation,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if operation.current(
            self.current().map(|thread| thread.entity_id()),
            self.composer.revision(),
        ) {
            self.add_images(paths, cx);
        }
    }
    pub(super) fn pick_images(&mut self, cx: &mut Context<Self>) {
        let operation = image_picker::ImagePickerOperation::new(
            self.current().map(|thread| thread.entity_id()),
            self.composer.revision(),
        );
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| {
                    this.on_image_paths_picked(operation, paths, cx)
                });
            }
        })
        .detach();
    }
    pub(super) fn on_menu_open_changed(
        &mut self,
        kind: MenuKind,
        open: bool,
        cx: &mut Context<Self>,
    ) {
        if open {
            self.menus.open(kind);
        } else {
            self.menus.close_if_same(&kind);
        }
        cx.notify();
    }

    pub(super) fn on_configuration_selected(
        &mut self,
        action: ConfigurationSelection,
        cx: &mut Context<Self>,
    ) {
        let (thread, config, value) = action.into_parts();
        if self.chats.contains(thread.entity_id()) {
            thread.update(cx, |thread, cx| thread.set_config(config, value, cx));
        }
        self.menus.reset();
        cx.notify();
    }
    pub(super) fn on_mode_selected(&mut self, action: ModeSelection, cx: &mut Context<Self>) {
        let (thread, mode) = action.into_parts();
        if self.chats.contains(thread.entity_id()) {
            thread.update(cx, |thread, cx| thread.set_mode(mode, cx));
        }
        self.menus.reset();
        cx.notify();
    }
    pub(super) fn on_favorite_selected(
        &mut self,
        action: FavoriteSelection,
        cx: &mut Context<Self>,
    ) {
        let (agent, config, value, _selected) = action.into_parts();
        self.host
            .runtime()
            .toggle_favorite(&agent, &config, &value, cx);
    }
    pub(super) fn preparing_images(&self) -> bool {
        self.composer.preparing()
    }
    pub(super) fn preparing_queue_edit(&self) -> bool {
        self.composer.preparing_edit()
    }
    pub(super) fn original_composer_text(&self, cx: &gpui_kit::App) -> String {
        if let Some(draft) = self.composer.original_draft() {
            return draft.to_owned();
        }
        let value = self.composer_text(cx).to_string();
        if self.composer.waiting_for_draft(&value) {
            self.current()
                .and_then(|thread| thread.read(cx).draft_text().map(str::to_owned))
                .unwrap_or_default()
        } else {
            value
        }
    }
    pub(super) fn restore_queue_edit(
        &mut self,
        resume: bool,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let (chat, draft) = self.composer.take_edit()?.into_parts();
        if let Some(thread) = self
            .chats
            .iter()
            .find(|thread| thread.entity_id() == chat)
            .cloned()
        {
            thread.update(cx, |thread, cx| thread.end_composer_edit(resume, cx));
        }
        Some(draft)
    }
    pub(super) fn leave_composer(&mut self, cx: &mut Context<Self>) {
        self.leave_composer_with(false, cx);
    }
    pub(super) fn leave_composer_with(&mut self, discard: bool, cx: &mut Context<Self>) {
        self.composer.leave();
        let draft = self
            .restore_queue_edit(!discard, cx)
            .unwrap_or_else(|| self.original_composer_text(cx));
        if !discard && let Some(thread) = self.current() {
            thread.update(cx, |thread, cx| {
                thread.set_draft(draft, cx);
                thread.save(cx);
            });
        }
    }
    pub(super) fn load_composer(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        let binding = self.composer.bind_loading(thread.entity_id(), cx);
        let Some(window) = thread.read(cx).window() else {
            return;
        };
        let owner = thread.downgrade();
        let panel = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = cx.update_window(window, |_, window, cx| {
                let _ = panel.update(cx, |panel, cx| {
                    panel.on_draft_hydrated(binding, owner, window, cx)
                });
            });
        });
    }
    pub(super) fn on_draft_hydrated(
        &mut self,
        binding: composer_view::ComposerBinding,
        owner: gpui_kit::WeakEntity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = owner.upgrade() else {
            return;
        };
        if self
            .current()
            .is_none_or(|current| current.entity_id() != thread.entity_id())
            || !self.composer.current(&binding)
        {
            return;
        }
        self.sync_composer_draft(cx);
        if !self.composer.current(&binding) {
            return;
        }
        let archived = thread.read(cx).is_archived();
        let draft = if archived {
            String::new()
        } else {
            thread
                .read(cx)
                .composer()
                .draft()
                .cloned()
                .unwrap_or_default()
        };
        self.composer
            .finish_loading(&binding, draft, archived, window, cx);
    }
    pub(super) fn edit_queued(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.editing() || self.preparing_images() {
            return;
        }
        let Some(thread) = self.current() else {
            return;
        };
        let Some(prompt) = thread
            .read(cx)
            .composer()
            .queue()
            .iter()
            .find(|prompt| prompt.saved().id == id)
            .cloned()
        else {
            return;
        };
        let draft = self.original_composer_text(cx);
        let ready = prompt.saved().images.is_empty();
        let binding = self
            .composer
            .begin_edit(thread.entity_id(), id, draft.clone(), ready);
        thread.update(cx, |thread, cx| {
            thread.set_draft(draft, cx);
            thread.begin_composer_edit(Vec::new(), prompt.attachments().to_vec(), cx);
            cx.notify();
        });
        if !ready {
            self.prepare_queue_images(&thread, id, binding, prompt.saved().clone(), cx);
        }
        self.reset_commands();
        self.composer
            .set_text(prompt.saved().text.clone(), window, cx);
        self.composer.open_queue();
        self.focus_composer(window, cx);
        cx.notify();
    }
    pub(super) fn prepare_queue_images(
        &mut self,
        thread: &Entity<AgentThread>,
        id: u64,
        binding: composer_view::ComposerBinding,
        saved: std::sync::Arc<nocterm_ai::history::SavedPrompt>,
        cx: &mut Context<Self>,
    ) {
        let task = queue_image_preparation::QueueImagePreparation::new(
            binding.clone(),
            thread.downgrade(),
            thread.read(cx).session_ticket(),
            id,
            saved,
            self.messages.inspector(),
        )
        .launch(cx);
        self.composer.install_edit_preparation(&binding, id, task);
    }
    pub(super) fn on_queue_image_prepared(
        &mut self,
        binding: composer_view::ComposerBinding,
        id: u64,
        owner: gpui_kit::WeakEntity<AgentThread>,
        ticket: nocterm_ai::session::Ticket,
        result: Result<Vec<PromptImage>, ImageAttachError>,
        cx: &mut Context<Self>,
    ) {
        if !self.composer.edit_preparation_current(&binding, id)
            || !self.settings.setting::<nocterm_ai::AiSettings>(cx).enabled
        {
            return;
        }
        let _ = owner.update(cx, |thread, cx| {
            if !thread.session_request_current(ticket) {
                return;
            }
            match result {
                Ok(images) => match thread.accept_composer_images(images, cx) {
                    Ok(()) => self.composer.edit_images_accepted(&binding, id),
                    Err(error) => self.notices.set_error(Some(error.to_string())),
                },
                Err(error) => self.notices.set_error(Some(error.to_string())),
            }
            cx.notify();
        });
        cx.notify();
    }
    pub(super) fn remove_queued(&mut self, id: u64, cx: &mut Context<Self>) {
        let editing = self.composer.editing_id();
        if let Some(thread) = self.current() {
            thread.update(cx, |thread, cx| thread.remove_queued(id, editing, cx));
        }
    }
    pub(super) fn finish_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.restore_queue_edit(true, cx) else {
            return;
        };
        self.reset_commands();
        self.composer.set_text(draft, window, cx);
    }

    pub(super) fn on_queue_height_observed(
        &mut self,
        chat: EntityId,
        height: gpui_kit::Pixels,
        cx: &mut Context<Self>,
    ) {
        if self.composer.observe_queue_height(chat, height) {
            cx.notify();
        }
    }
    pub(super) fn sync_composer_draft(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        let text = self.composer.text(cx).to_string();
        if self.composer.draft_changed(thread.entity_id(), &text) {
            thread.update(cx, |thread, cx| thread.set_draft(text, cx));
        }
    }
    /// Reads the plan limits again when the card opens.
    pub(super) fn close_menus(&mut self, cx: &mut Context<Self>) {
        self.menus.reset();
        cx.notify();
    }
    pub(super) fn on_queue_toggled(&mut self, origin: EntityId, cx: &mut Context<Self>) {
        if self
            .current()
            .is_none_or(|thread| thread.entity_id() != origin)
        {
            return;
        }
        self.composer.toggle_queue();
        cx.notify();
    }
    pub(super) fn on_attachment_selected(
        &mut self,
        chat: gpui_kit::EntityId,
        value: Attachment,
        cx: &mut Context<Self>,
    ) {
        if let Some(thread) = self.chats.find(chat).cloned() {
            thread.update(cx, |thread, cx| thread.attach(value, cx));
        }
    }
    pub(super) fn on_image_removed(
        &mut self,
        chat: gpui_kit::EntityId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(thread) = self.chats.find(chat).cloned() {
            thread.update(cx, |thread, cx| {
                thread.remove_composer_image(index, cx);
                cx.notify();
            });
        }
    }
    pub(super) fn on_send_clicked(
        &mut self,
        chat: gpui_kit::EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(thread) = self.current().filter(|thread| thread.entity_id() == chat) {
            if composer_renderer::ComposerRenderer::stops(&self.composer, &thread, cx) {
                thread.update(cx, |thread, cx| thread.stop(cx));
            } else {
                self.send(window, cx);
            }
        }
    }
    pub(super) fn on_restart_clicked(
        &mut self,
        chat: gpui_kit::EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .current()
            .is_some_and(|thread| thread.entity_id() == chat)
        {
            self.restart(window, cx);
        }
    }
    pub(super) fn on_authenticate_clicked(
        &mut self,
        chat: gpui_kit::EntityId,
        id: AuthMethodId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(thread) = self.chats.find(chat).cloned() {
            thread.update(cx, |thread, cx| thread.authenticate(id, window, cx));
        }
    }
}

impl AgentPanel {
    pub(super) fn on_queue_send_now(&mut self, chat: EntityId, id: u64, cx: &mut Context<Self>) {
        if let Some(thread) = self.current().filter(|thread| thread.entity_id() == chat) {
            thread.update(cx, |thread, cx| thread.send_now(id, cx));
        }
    }
    pub(super) fn on_queue_edit_requested(
        &mut self,
        chat: EntityId,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .current()
            .is_some_and(|thread| thread.entity_id() == chat)
        {
            self.edit_queued(id, window, cx);
        }
    }
    pub(super) fn on_queue_remove_requested(
        &mut self,
        chat: EntityId,
        id: u64,
        cx: &mut Context<Self>,
    ) {
        if self
            .current()
            .is_some_and(|thread| thread.entity_id() == chat)
        {
            self.remove_queued(id, cx);
        }
    }
}
