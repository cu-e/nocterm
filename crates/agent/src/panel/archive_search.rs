//! Search archived transcripts without materializing them in GUI documents.
use super::AgentPanel;
use crate::runtime::Runtime;
use gpui_kit::Context;
impl AgentPanel {
    pub(super) fn search_archives(&mut self, cx: &mut Context<Self>) {
        self.archive_search = None;
        self.archive_matches.clear();
        let query = self.history_search.read(cx).value().trim().to_lowercase();
        if query.is_empty() {
            return;
        }
        let ids = self
            .threads
            .iter()
            .filter(|thread| thread.read(cx).archive.is_some())
            .map(|thread| thread.read(cx).chat_id.clone())
            .collect::<Vec<_>>();
        let dir = Runtime::global(cx).read(cx).services.chats_dir.clone();
        let gate = Runtime::global(cx).read(cx).history_gate.clone();
        let expected = query.clone();
        let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = cancellation.clone();
        let search = cx.background_executor().spawn(async move {
            let mut matches = std::collections::HashSet::new();
            for id in ids {
                if flag.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                let _permit = gate.acquire().await;
                if let Ok(chat) = nocterm_ai::history::load(&dir, &id) {
                    let state = nocterm_ai::thread::ThreadState {
                        entries: chat.entries,
                        ..Default::default()
                    };
                    if state.mentions(&query) {
                        matches.insert(id);
                    }
                }
            }
            matches
        });
        self.archive_search = Some(cx.spawn(async move |this, cx| {
            struct Cancel(std::sync::Arc<std::sync::atomic::AtomicBool>);
            impl Drop for Cancel {
                fn drop(&mut self) {
                    self.0.store(true, std::sync::atomic::Ordering::Release);
                }
            }
            let _cancel = Cancel(cancellation);
            let matches = search.await;
            let _ = this.update(cx, |this, cx| {
                if this.history_search.read(cx).value().trim().to_lowercase() == expected {
                    this.archive_matches = matches;
                    cx.notify();
                }
            });
        }));
    }
}
