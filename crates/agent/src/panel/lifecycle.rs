//! Chat activation, restoration and replacement share one lifecycle.
use super::AgentPanel;
use crate::{
    runtime::Runtime,
    thread::{AgentThread, Attachment},
};
use gpui_kit::{AppContext as _, Context, Entity, Window};
use nocterm_ui::{ActiveAi as _, SettingsExt as _};
use std::time::Duration;
enum ThreadStart {
    New(String),
    Restart(Box<crate::thread::RestartData>),
}
impl AgentPanel {
    pub(super) fn reset_chat_view(&mut self, cx: &mut Context<Self>) {
        self.stream_tick = None;
        self.stream = Default::default();
        self.list.update(cx, |list, cx| list.reset(0, cx));
        self.list_count = 0;
        self.navigation.clear();
        self.expanded.clear();
        self.image_cache.clear();
        self.reset_commands();
        self.menu = None;
    }
    pub(super) fn select_thread(&mut self, id: gpui_kit::EntityId, cx: &mut Context<Self>) {
        if let Some(previous) = self.current() {
            previous.update(cx, |thread, cx| thread.set_shown(false, cx));
        }
        self.active = self
            .threads
            .iter()
            .position(|thread| thread.entity_id() == id);
        if let Some(current) = self.current() {
            current.update(cx, |thread, cx| thread.set_shown(true, cx));
        }
        self.reset_chat_view(cx);
        if let Some(thread) = self
            .current()
            .filter(|thread| thread.read(cx).is_archived())
        {
            let panel = cx.weak_entity();
            let revision = self.composer.revision;
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
            if self.composer.revision == revision {
                self.load_composer(cx);
            } else {
                let text = self.input.read(cx).value().to_string();
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
        match Runtime::global(cx).update(cx, |runtime, cx| {
            runtime.capture_and_detach_document(id, cx)
        }) {
            Ok(true) => {
                if let Some(thread) = self.threads.iter().find(|thread| thread.entity_id() == id) {
                    thread.update(cx, |thread, cx| {
                        thread.document_admitted(thread.document().desired_revision(), cx)
                    });
                }
                true
            }
            Ok(false) => true,
            Err(error) => {
                self.error = Some(format!("Could not retain chat: {error}"));
                false
            }
        }
    }
    pub(super) fn clear_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.original_composer_text(cx);
        if let Some(thread) = self.current() {
            thread.update(cx, |thread, cx| thread.set_draft(draft, cx));
        }
        self.leave_composer_with(true, cx);
        self.retain_threads(|_, _| false, cx);
        if !self.threads.is_empty() {
            return;
        }
        self.active = None;
        self.composer = Default::default();
        self.queue_heights.clear();
        self.queue_open = false;
        self.sign_in_inputs.clear();
        self.renaming = None;
        self.error = None;
        self.reset_chat_view(cx);
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
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
        let Some(_launch) = nocterm_ai::AgentRegistry::new(cx.setting::<nocterm_ai::AiSettings>())
            .get(&id)
            .cloned()
        else {
            return;
        };
        Runtime::global(cx).update(cx, |runtime, cx| runtime.set_last_agent(&id, cx));
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
        self.track(&thread, window, cx);

        self.threads.push(thread);
        let id = self.threads.last().unwrap().entity_id();
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
        Runtime::global(cx).update(cx, |runtime, cx| {
            runtime.register_document(crate::thread::client(&thread), cx)
        });
        thread.update(cx, |thread, cx| {
            thread.save(cx);
            cx.notify();
        });
        self.load_composer(cx);
        let old = old.entity_id();
        Runtime::global(cx).update(cx, |runtime, cx| runtime.unregister_document(old, cx));
        self.retain_threads(|thread, _| thread.entity_id() != old, cx);
    }
    /// Shows the chats saved in earlier runs, oldest first, before the
    /// chats of this run.
    pub(super) fn adopt_saved_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.ai_enabled() {
            return;
        }
        let (chats, catalog) = Runtime::global(cx).update(cx, |runtime, _| {
            (runtime.take_saved_chats(), runtime.take_saved_catalog())
        });
        let mut restored = Vec::new();
        if let Some(chats) = chats {
            for chat in chats.into_iter().rev() {
                let thread = cx.new(|cx| AgentThread::restored(chat, self.host.clone(), cx));
                self.track(&thread, window, cx);
                restored.push(thread);
            }
        }
        if let Some(catalog) = catalog {
            for summary in catalog.into_iter().rev() {
                if self
                    .threads
                    .iter()
                    .chain(&restored)
                    .any(|thread| thread.read(cx).chat_id() == summary.id)
                {
                    continue;
                }
                let thread = cx.new(|cx| AgentThread::archived(summary, self.host.clone(), cx));
                self.track(&thread, window, cx);
                restored.push(thread);
            }
        }
        let count = restored.len();
        self.threads.splice(0..0, restored);
        self.active = self.active.map(|active| active + count);
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
        let mut candidates = self.threads.clone();
        candidates.sort_by_key(|thread| thread.read(cx).document().updated());
        for thread in candidates {
            if resident <= BUDGET {
                break;
            }
            if Some(thread.entity_id()) == active {
                continue;
            }
            let bytes = thread.read(cx).resident_history_bytes();
            let query = self.history_search.read(cx).value().trim().to_lowercase();
            let matched = !query.is_empty() && thread.read(cx).transcript().mentions(&query);
            let chat_id = thread.read(cx).document().chat_id().to_owned();
            if thread.update(cx, |thread, cx| thread.evict_history(cx)) {
                resident = resident.saturating_sub(bytes);
                if matched {
                    self.archive_matches.insert(chat_id);
                }
            }
        }
    }
    /// Explicit activation is reserved for work, never for opening history.
    #[cfg(test)]
    pub(super) fn wake(&mut self, thread: &Entity<AgentThread>, cx: &mut Context<Self>) {
        Runtime::global(cx).update(cx, |runtime, cx| {
            runtime.register_document(crate::thread::client(thread), cx)
        });
        thread.update(cx, |thread, cx| thread.request_activation(cx));
    }
    pub(super) fn track(
        &mut self,
        thread: &Entity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        Runtime::global(cx).update(cx, |runtime, cx| {
            runtime.register_document(crate::thread::client(thread), cx)
        });
        // Background sessions the chat opens belong to this window.
        let handle = window.window_handle();
        thread.update(cx, |thread, _| thread.window = Some(handle));
        self.subscriptions
            .push(cx.observe_in(thread, window, |this, loaded, window, cx| {
                this.trim_resident_history(cx);
                loaded.update(cx, |thread, _| {
                    thread.release_history_delivery();
                });
                this.refresh_commands(window, cx);
                let panel = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = panel.update(cx, |panel, cx| panel.sync_approval_attention(window, cx));
                });
                if this.notify_queued {
                    return;
                }
                this.notify_queued = true;
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(33))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.notify_queued = false;
                        cx.notify();
                    });
                })
                .detach();
            }));
    }
}
