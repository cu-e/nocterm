//! Visible notices and the single coalesced redraw own their transitions.
use gpui_kit::{EntityId, Task};
use std::sync::Arc;

struct Redraw {
    ticket: Arc<()>,
    task: Option<Task<()>>,
}
pub(super) enum ApprovalEffect {
    Unchanged,
    Show,
    Remove,
}
#[derive(Default)]
pub(super) struct PanelNotices {
    error: Option<String>,
    redraw: Option<Redraw>,
    approval: Option<Vec<(EntityId, u64)>>,
}
impl PanelNotices {
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub(super) fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }
    pub(super) fn approval_changed(
        &mut self,
        fingerprint: Option<Vec<(EntityId, u64)>>,
    ) -> ApprovalEffect {
        if self.approval == fingerprint {
            return ApprovalEffect::Unchanged;
        }
        let show = fingerprint.is_some();
        self.approval = fingerprint;
        if show {
            ApprovalEffect::Show
        } else {
            ApprovalEffect::Remove
        }
    }
    pub(super) fn request_redraw(&mut self) -> Option<Arc<()>> {
        if self.redraw.is_some() {
            return None;
        }
        let ticket = Arc::new(());
        self.redraw = Some(Redraw {
            ticket: ticket.clone(),
            task: None,
        });
        Some(ticket)
    }
    pub(super) fn install_redraw(&mut self, ticket: &Arc<()>, task: Task<()>) {
        if let Some(redraw) = self
            .redraw
            .as_mut()
            .filter(|redraw| Arc::ptr_eq(&redraw.ticket, ticket))
        {
            redraw.task = Some(task);
        }
    }
    pub(super) fn finish_redraw(&mut self, ticket: &Arc<()>) -> bool {
        if self
            .redraw
            .as_ref()
            .is_none_or(|redraw| !Arc::ptr_eq(&redraw.ticket, ticket))
        {
            return false;
        }
        self.redraw = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{ApprovalEffect, PanelNotices};

    #[test]
    fn redraws_coalesce_and_old_completion_cannot_finish_a_replacement() {
        let mut notices = PanelNotices::default();
        let original = notices.request_redraw().unwrap();
        assert!(notices.request_redraw().is_none());
        assert!(notices.finish_redraw(&original));
        let replacement = notices.request_redraw().unwrap();
        assert!(!notices.finish_redraw(&original));
        assert!(notices.request_redraw().is_none());
        assert!(notices.finish_redraw(&replacement));
        assert!(!notices.finish_redraw(&replacement));
    }

    #[test]
    fn approval_notice_changes_only_when_its_original_fingerprint_changes() {
        let mut notices = PanelNotices::default();
        let first = Some(Vec::new());
        assert!(matches!(
            notices.approval_changed(first.clone()),
            ApprovalEffect::Show
        ));
        assert!(matches!(
            notices.approval_changed(first),
            ApprovalEffect::Unchanged
        ));
        assert!(matches!(
            notices.approval_changed(None),
            ApprovalEffect::Remove
        ));
        assert!(matches!(
            notices.approval_changed(None),
            ApprovalEffect::Unchanged
        ));
    }
    #[gpui_kit::test]
    fn dropping_notices_cancels_owned_redraw_work(cx: &mut gpui_kit::TestAppContext) {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        struct Signal(Arc<AtomicBool>);
        impl Drop for Signal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = Signal(cancelled.clone());
        let task = cx.background_executor.spawn(async move {
            let _signal = signal;
            futures::future::pending::<()>().await;
        });
        let mut notices = PanelNotices::default();
        let ticket = notices.request_redraw().unwrap();
        notices.install_redraw(&ticket, task);
        cx.run_until_parked();
        assert!(!cancelled.load(Ordering::SeqCst));
        drop(notices);
        cx.run_until_parked();
        assert!(cancelled.load(Ordering::SeqCst));
    }
}
