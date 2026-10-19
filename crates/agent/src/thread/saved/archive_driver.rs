//! Archive generation decisions and reader completion are applied through one scoped owner.
use super::*;
pub(super) struct ArchiveDriver<'a> {
    thread: &'a mut AgentThread,
}
impl<'a> ArchiveDriver<'a> {
    pub(super) fn new(thread: &'a mut AgentThread) -> Self {
        Self { thread }
    }
    fn apply_intent(&mut self, intent: archive::ArchiveIntent, cx: &mut Context<AgentThread>) {
        match intent {
            archive::ArchiveIntent::Rename { name } => self.thread.rename(name, cx),
            archive::ArchiveIntent::Pin { pinned } => self.thread.set_pinned(pinned, cx),
            archive::ArchiveIntent::Open {
                panel,
                source,
                revision,
            } => cx.defer(move |cx| {
                let _ = panel.update(cx, |panel, cx| panel.archive_opened(source, revision, cx));
            }),
            archive::ArchiveIntent::Fork {
                panel,
                source,
                window,
                last,
            } => cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    let _ = panel.update(cx, |panel, cx| {
                        panel.fork_loaded_archive(source, last, window, cx)
                    });
                });
            }),
        }
    }

    pub(super) fn completed(
        &mut self,
        completion: archive::load_operation::LoadCompletion,
        cx: &mut Context<AgentThread>,
    ) {
        let (load_id, ticket, result, permit) = completion.into_parts();

        let current = self.thread.session.session_current(ticket)
            && self.thread.host.enabled(cx)
            && self.thread.state.chat_id().is_some_and(|id| {
                self.thread
                    .host
                    .runtime()
                    .document_is_current(&id, cx.entity_id(), cx)
            });
        match self.thread.archive.accept(load_id, current, result, permit) {
            archive::LoadOutcome::Stale => return,
            archive::LoadOutcome::Failed(delivery) => {
                if let Some(error) = delivery.error() {
                    self.thread
                        .notice
                        .present(format!("Could not load saved chat: {error}"));
                }
                self.thread.notice.mark_error();
            }
            archive::LoadOutcome::Refused(error) => {
                self.thread
                    .notice
                    .present(format!("Could not load saved chat: {error}"));
                self.thread.notice.mark_error();
            }
            archive::LoadOutcome::Hydrate {
                delivery,
                overlay,
                intents,
            } => {
                if let Err(error) = self.thread.restore_delivery(delivery, overlay) {
                    self.thread
                        .notice
                        .present(format!("Could not load saved chat: {error}"));
                    self.thread.notice.mark_error();
                    cx.notify();
                    return;
                }
                self.thread.preview_config(cx);
                if self.thread.state.draft_changed() {
                    self.thread.save(cx);
                }
                for intent in intents {
                    self.apply_intent(intent, cx);
                }
                self.thread.warm(cx);
            }
        }
        cx.notify();
    }

    pub(super) fn load(&mut self, intent: archive::ArchiveIntent, cx: &mut Context<AgentThread>) {
        let request = self
            .thread
            .archive
            .request(intent, self.thread.session.ticket());
        let archive::RequestOutcome::Start { load_id, ticket } = request else {
            if let archive::RequestOutcome::RunNow(intent) = request {
                self.apply_intent(intent, cx);
            }
            return;
        };
        self.thread.notice.present("Loading saved chat…".into());
        let reader = self.thread.host.runtime().history_reader(cx);
        let operation = match archive::load_operation::LoadOperation::prepare(
            load_id,
            ticket,
            &mut self.thread.state,
            reader,
        ) {
            Ok(operation) => operation,
            Err(error) => {
                self.completed(
                    archive::load_operation::LoadCompletion::refused(load_id, ticket, error),
                    cx,
                );
                return;
            }
        };
        let loading = cx
            .background_executor()
            .spawn_with_priority(Default::default(), operation);
        cx.spawn(async move |this, cx| {
            let completion = loading.await;
            let _ = this.update(cx, |this, cx| {
                ArchiveDriver::new(this).completed(completion, cx)
            });
        })
        .detach();
        cx.notify();
    }
}
