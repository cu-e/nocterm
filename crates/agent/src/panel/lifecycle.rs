use super::history_column::SearchTicket;
use super::*;
use crate::thread::Attachment;
use std::collections::HashSet;
enum ThreadStart {
    New(String),
    Restart(Box<crate::thread::RestartData>),
}

impl AgentPanel {
    pub(super) fn search_archives(&mut self, cx: &mut Context<Self>) {
        self.on_history_catalog_changed(cx);
    }
    pub(super) fn on_history_query_changed(&mut self, cx: &mut Context<Self>) {
        self.start_history_search(cx);
    }
    pub(super) fn on_history_catalog_changed(&mut self, cx: &mut Context<Self>) {
        self.start_history_search(cx);
    }
    pub(super) fn start_history_search(&mut self, cx: &mut Context<Self>) {
        if !self.history.search_requested(cx) {
            return;
        }
        let ids = self
            .chats
            .iter()
            .filter(|thread| thread.read(cx).is_archived())
            .map(|thread| thread.read(cx).document().chat_id().to_owned())
            .collect();
        let reader = self.host.runtime().history_reader(cx);
        let catalog = self
            .chats
            .iter()
            .map(|thread| (thread.entity_id(), thread.read(cx).is_archived()))
            .collect();
        self.history.start_search(ids, reader, catalog, cx);
        cx.notify();
    }
    pub(super) fn on_archive_search_completed(
        &mut self,
        ticket: SearchTicket,
        matches: HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        if self.history.accept_matches(&ticket, matches) {
            cx.notify();
        }
    }
    /// Keeps the chats `keep` accepts, and the open chat where it is kept.
    pub(super) fn retain_threads(
        &mut self,
        keep: impl Fn(&Entity<AgentThread>, &App) -> bool,
        cx: &mut Context<Self>,
    ) {
        let active = self.current().map(|thread| thread.entity_id());
        let proposed: Vec<_> = self
            .chats
            .iter()
            .filter(|thread| !keep(thread, cx))
            .map(|thread| thread.entity_id())
            .collect();
        let removed = proposed
            .into_iter()
            .filter(|id| self.admit_before_removal(*id, cx))
            .collect::<Vec<_>>();
        if self
            .composer
            .editing_chat()
            .is_some_and(|chat| removed.contains(&chat))
        {
            self.leave_composer_with(true, cx);
        }
        self.composer.removed_chats(&removed);
        self.chats.remove_admitted(&removed);
        if active.is_some_and(|id| removed.contains(&id)) {
            self.reset_chat_view(cx);
        }
        if !removed.is_empty() {
            self.on_history_catalog_changed(cx);
        }
    }
    /// Drops empty, unnamed chats other than `keep`: a chat nobody wrote in
    /// is not history.
    pub(super) fn discard_drafts(&mut self, keep: Option<EntityId>, cx: &mut Context<Self>) {
        self.retain_threads(
            |thread, cx| Some(thread.entity_id()) == keep || !thread.read(cx).is_draft(),
            cx,
        );
    }
    /// Shows chat `id`, connecting it if it came from history.
    pub(super) fn open_thread(&mut self, id: EntityId, cx: &mut Context<Self>) {
        self.leave_composer(cx);
        self.discard_drafts(Some(id), cx);
        if !self.chats.contains(id) {
            return;
        }
        self.select_thread(id, cx);
    }
    pub(super) fn delete_thread(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(thread) = self.chats.find(id).cloned() {
            let chat = thread.read(cx).document().chat_id().to_owned();
            thread.update(cx, |thread, cx| thread.release_resources(cx));
            self.host.runtime().delete_chat(chat, cx);
            self.retain_threads(|thread, _| thread.entity_id() != id, cx);
            if self.current().is_none() {
                self.composer.set_text("", window, cx);
            }
        }
        self.sync_approval_attention(window, cx);
        cx.notify();
    }
    /// Copies chat `id` into a new chat and opens the copy.
    pub(super) fn fork_thread(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fork_thread_at(id, None, window, cx);
    }
    /// Copies chat `id` up to and including entry `last` (all of it, with
    /// `None`) into a new chat and opens the copy.
    pub(super) fn fork_thread_at(
        &mut self,
        id: EntityId,
        last: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.chats.find(id).cloned() else {
            return;
        };
        if source.read(cx).is_archived() {
            let panel = cx.weak_entity();
            let handle = window.window_handle();
            source.update(cx, |thread, cx| {
                thread.load_archive(
                    crate::thread::archive::ArchiveIntent::Fork {
                        panel,
                        source: id,
                        window: handle,
                        last,
                    },
                    cx,
                )
            });
            return;
        }
        let fork = match last {
            Some(last) => source.read(cx).fork_at(last + 1),
            None => source.read(cx).fork(),
        };
        let host = self.host.clone();
        let thread = cx.new(|cx| AgentThread::forked(fork, host, cx));
        thread.update(cx, |thread, cx| thread.persist(cx));
        let slot = self.track(&thread, window, cx);
        let id = thread.entity_id();
        self.chats.append(slot);
        self.on_history_catalog_changed(cx);
        self.open_thread(id, cx);
    }
    pub(crate) fn fork_loaded_archive(
        &mut self,
        source: EntityId,
        last: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.chats.contains(source) {
            self.fork_thread_at(source, last, window, cx);
        }
    }
    pub(super) fn set_pinned(&mut self, id: EntityId, pinned: bool, cx: &mut Context<Self>) {
        if let Some(thread) = self.chats.find(id).cloned() {
            thread.update(cx, |thread, cx| thread.set_pinned(pinned, cx));
        }
    }
    pub(super) fn start_rename(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.chats.find(id).cloned() else {
            return;
        };
        let title = thread.read(cx).title();
        self.history.begin_rename(id, title.to_string(), window, cx);
        cx.notify();
    }
    pub(super) fn finish_rename(
        &mut self,
        accept: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if accept {
            if let Some(binding) = self.history.rename_binding() {
                self.commit_history_rename(binding, window, cx);
            }
        } else {
            self.history.cancel_rename();
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }
    pub(super) fn commit_history_rename(
        &mut self,
        binding: history_column::RenameBinding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.history.rename_submission(binding, cx) else {
            return;
        };
        let (chat, title) = request.into_parts();
        if let Some(thread) = self.chats.find(chat).cloned() {
            thread.update(cx, |thread, cx| thread.rename(Some(title), cx));
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }
    pub(super) fn on_history_selected(&mut self, id: EntityId, cx: &mut Context<Self>) {
        if !self.history.renaming() {
            self.open_thread(id, cx);
        }
    }
    pub(super) fn on_history_rename_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" {
            cx.stop_propagation();
            self.finish_rename(false, window, cx);
        }
    }
    pub(super) fn release_history_resources(&mut self, id: EntityId, cx: &mut Context<Self>) {
        if let Some(thread) = self.chats.find(id).cloned() {
            thread.update(cx, |thread, cx| thread.release_resources(cx));
        }
    }
    pub(super) fn reset_chat_view(&mut self, cx: &mut Context<Self>) {
        self.messages.reset(cx);
        self.reset_commands();
        self.menus.reset();
    }
    pub(super) fn select_thread(&mut self, id: gpui_kit::EntityId, cx: &mut Context<Self>) {
        if let Some(previous) = self.current() {
            previous.update(cx, |thread, cx| thread.set_shown(false, cx));
        }
        self.chats.select(id);
        if let Some(current) = self.current() {
            current.update(cx, |thread, cx| thread.set_shown(true, cx));
        }
        self.reset_chat_view(cx);
        if let Some(thread) = self
            .current()
            .filter(|thread| thread.read(cx).is_archived())
        {
            let panel = cx.weak_entity();
            let revision = self.composer.revision();
            thread.update(cx, |thread, cx| {
                thread.load_archive(
                    crate::thread::archive::ArchiveIntent::Open {
                        panel,
                        source: id,
                        revision,
                    },
                    cx,
                )
            });
        }
        self.load_composer(cx);
        self.trim_resident_history(cx);
        cx.notify();
    }
    pub(crate) fn archive_opened(
        &mut self,
        source: gpui_kit::EntityId,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        if self
            .current()
            .is_some_and(|thread| thread.entity_id() == source)
        {
            self.messages.reset(cx);
            if self.composer.revision() == revision {
                self.load_composer(cx);
            } else {
                let text = self.composer.text(cx).to_string();
                if let Some(thread) = self.current() {
                    thread.update(cx, |thread, cx| thread.set_draft(text, cx));
                }
            }
        }
        self.trim_resident_history(cx);
        cx.notify();
    }
    pub(super) fn admit_before_removal(
        &mut self,
        id: gpui_kit::EntityId,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.host.runtime().capture_and_detach_document(id, cx) {
            Ok(true) => {
                if let Some(thread) = self.chats.iter().find(|thread| thread.entity_id() == id) {
                    thread.update(cx, |thread, cx| {
                        thread.document_admitted(thread.document().desired_revision(), cx)
                    });
                }
                true
            }
            Ok(false) => true,
            Err(error) => {
                self.notices
                    .set_error(Some(format!("Could not retain chat: {error}")));
                false
            }
        }
    }
    pub(super) fn clear_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.original_composer_text(cx);
        if let Some(thread) = self.current() {
            thread.update(cx, |thread, cx| thread.set_draft(draft, cx));
        }
        self.retain_threads(|_, _| false, cx);
        if !self.chats.is_empty() {
            return;
        }
        self.composer.clear();
        self.sign_ins.clear();
        self.history.cancel_rename();
        self.notices.set_error(None);
        self.reset_chat_view(cx);
        self.composer.set_text("", window, cx);
        self.sync_approval_attention(window, cx);
    }
    pub(super) fn new_thread(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.new_thread_connection(ThreadStart::New(id), window, cx);
    }
    fn new_thread_connection(
        &mut self,
        start: ThreadStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let restarting = matches!(&start, ThreadStart::Restart(_));
        let id = match &start {
            ThreadStart::New(id) => id.clone(),
            ThreadStart::Restart(data) => data.agent_id().to_owned(),
        };
        let Some(_launch) =
            nocterm_ai::AgentRegistry::new(self.settings.setting::<nocterm_ai::AiSettings>(cx))
                .get(&id)
                .cloned()
        else {
            return;
        };
        self.host.runtime().set_last_agent(&id, cx);
        self.leave_composer(cx);
        let active = self
            .host
            .workspace()
            .upgrade()
            .and_then(|workspace| workspace.read(cx).active_terminal(cx));
        // An empty chat left behind is not history.
        self.discard_drafts(None, cx);
        let thread = cx.new(|cx| match start {
            ThreadStart::New(id) => {
                let mut thread = AgentThread::new(id, self.host.clone(), cx);
                thread.preview_config(cx);
                thread
            }
            ThreadStart::Restart(data) => AgentThread::restarted(*data, self.host.clone(), cx),
        });
        if !restarting && let Some(id) = active {
            thread.update(cx, |thread, cx| {
                thread.add_composer_attachment(Attachment::Terminal(id), cx)
            });
        }
        let id = thread.entity_id();
        let slot = self.track(&thread, window, cx);
        self.chats.append(slot);
        self.on_history_catalog_changed(cx);
        self.select_thread(id, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }
    /// Replaces the open chat, whose session ended, with the same chat in a
    /// new agent process.
    pub(super) fn restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(old) = self.current() else {
            return;
        };
        let draft = self.original_composer_text(cx);
        old.update(cx, |thread, cx| thread.set_draft(draft, cx));
        self.leave_composer_with(true, cx);
        // Release ACP even if another observer still holds the old document entity.
        old.update(cx, |thread, cx| thread.release_resources(cx));
        // Edits and provider tool statuses are final before taking the restart snapshot.
        let data = old.read(cx).restart_data();
        self.new_thread_connection(ThreadStart::Restart(Box::new(data)), window, cx);
        let Some(thread) = self
            .current()
            .filter(|thread| thread.entity_id() != old.entity_id())
        else {
            return;
        };
        self.host
            .runtime()
            .register_document(crate::thread::client(&thread), cx);
        thread.update(cx, |thread, cx| {
            thread.save(cx);
            cx.notify();
        });
        self.load_composer(cx);
        let old = old.entity_id();
        self.host.runtime().unregister_document(old, cx);
        self.retain_threads(|thread, _| thread.entity_id() != old, cx);
    }
    /// Shows the chats saved in earlier runs, oldest first, before the
    /// chats of this run.
    pub(super) fn adopt_saved_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.setting::<nocterm_ai::AiSettings>(cx).enabled {
            return;
        }
        let (chats, catalog) = self.host.runtime().take_history(cx);
        let mut restored = Vec::new();
        if let Some(chats) = chats {
            for chat in chats.into_iter().rev() {
                let thread = cx.new(|cx| AgentThread::restored(chat, self.host.clone(), cx));
                let slot = self.track(&thread, window, cx);
                restored.push(slot);
            }
        }
        if let Some(catalog) = catalog {
            for summary in catalog.into_iter().rev() {
                if self
                    .chats
                    .iter()
                    .chain(restored.iter().map(chat_list::ChatSlot::thread))
                    .any(|thread| thread.read(cx).chat_id() == summary.id)
                {
                    continue;
                }
                let thread = cx.new(|cx| AgentThread::archived(summary, self.host.clone(), cx));
                let slot = self.track(&thread, window, cx);
                restored.push(slot);
            }
        }
        let count = restored.len();
        self.chats.prepend(restored);
        if count > 0 {
            self.search_archives(cx);
        }
        cx.notify();
    }
    pub(super) fn trim_resident_history(&mut self, cx: &mut Context<Self>) {
        const BUDGET: usize = 64 * 1024 * 1024;
        let active = self.current().map(|thread| thread.entity_id());
        let mut resident: usize = self
            .chat_documents()
            .map(|thread| thread.read(cx).resident_history_bytes())
            .sum();
        let mut candidates = self.chats.iter().cloned().collect::<Vec<_>>();
        candidates.sort_by_key(|thread| thread.read(cx).document().updated());
        for thread in candidates {
            if resident <= BUDGET {
                break;
            }
            if Some(thread.entity_id()) == active {
                continue;
            }
            let bytes = thread.read(cx).resident_history_bytes();
            let query = self.history.query(cx);
            let matched = !query.is_empty() && thread.read(cx).transcript().mentions(&query);
            let chat_id = thread.read(cx).document().chat_id().to_owned();
            if thread.update(cx, |thread, cx| thread.evict_history(cx)) {
                resident = resident.saturating_sub(bytes);
                if matched {
                    self.history.record_loaded_match(chat_id);
                }
            }
        }
    }
    pub(super) fn track(
        &mut self,
        thread: &Entity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> chat_list::ChatSlot {
        self.host
            .runtime()
            .register_document(crate::thread::client(thread), cx);
        // Background sessions the chat opens belong to this window.
        let handle = window.window_handle();
        thread.update(cx, |thread, _| thread.bind_window(handle));
        let mut archived = thread.read(cx).is_archived();
        let observation = cx.observe_in(thread, window, move |panel, loaded, window, cx| {
            let current_archived = loaded.read(cx).is_archived();
            let changed = archived != current_archived;
            archived = current_archived;
            panel.on_chat_changed(loaded, changed, window, cx);
        });
        chat_list::ChatSlot::new(thread.clone(), observation)
    }
    pub(super) fn on_chat_changed(
        &mut self,
        loaded: Entity<AgentThread>,
        archived_changed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.chats.contains(loaded.entity_id()) {
            loaded.update(cx, |thread, _| thread.release_history_delivery());
            return;
        }
        self.trim_resident_history(cx);
        if archived_changed {
            self.on_history_catalog_changed(cx);
        }
        loaded.update(cx, |thread, _| thread.release_history_delivery());
        self.refresh_commands(window, cx);
        let panel = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = panel.update(cx, |panel, cx| panel.sync_approval_attention(window, cx));
        });
        let Some(ticket) = self.notices.request_redraw() else {
            return;
        };
        let completion = ticket.clone();
        let task = cx.spawn(async move |panel, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(33))
                .await;
            let _ = panel.update(cx, |panel, cx| panel.on_coalesced_redraw(&completion, cx));
        });
        self.notices.install_redraw(&ticket, task);
    }
    pub(super) fn on_coalesced_redraw(
        &mut self,
        ticket: &std::sync::Arc<()>,
        cx: &mut Context<Self>,
    ) {
        if self.notices.finish_redraw(ticket) {
            cx.notify();
        }
    }
}
