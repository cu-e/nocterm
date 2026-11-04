//! The panel's regions as views, so a change redraws only the regions it touches.
//!
//! Every region is cached: the panel's own redraws reuse them unless they are
//! notified. Any notify of the panel invalidates all of them, so state the panel
//! changes stays correct by default; only thread changes and typing target a region.
//! The footer (approvals, status, queue, commands and composer) is cached to its
//! content: reused, it keeps the height its content last took.
use super::AgentPanel;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, Render, StyleRefinement, Styled,
    WeakEntity, Window, div,
};
use nocterm_ai::conversation::ConversationView;
use std::hash::{Hash, Hasher};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PartKind {
    Transcript,
    History,
    Footer,
}

pub(super) struct PanelPart {
    panel: WeakEntity<AgentPanel>,
    kind: PartKind,
}
impl Render for PanelPart {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        self.panel
            .update(cx, |panel, cx| panel.render_part(kind, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

pub(super) struct PanelParts {
    transcript: Entity<PanelPart>,
    history: Entity<PanelPart>,
    footer: Entity<PanelPart>,
    transcript_seen: Option<u64>,
    history_seen: Option<u64>,
}
impl PanelParts {
    pub(super) fn new(cx: &mut Context<AgentPanel>) -> Self {
        let panel = cx.weak_entity();
        let mut part = |kind| {
            let panel = panel.clone();
            cx.new(|_| PanelPart { panel, kind })
        };
        Self {
            transcript: part(PartKind::Transcript),
            history: part(PartKind::History),
            footer: part(PartKind::Footer),
            transcript_seen: None,
            history_seen: None,
        }
    }
    /// The transcript fills the chat column above the footer.
    pub(super) fn transcript(&self) -> impl IntoElement + use<> {
        self.transcript.clone().cached(
            StyleRefinement::default()
                .w_full()
                .flex_1()
                .min_h_0()
                .min_w_0(),
        )
    }
    /// The history fills its column, or the panel body when no chat is open.
    pub(super) fn history(&self) -> impl IntoElement + use<> {
        self.history.clone().cached(
            StyleRefinement::default()
                .w_full()
                .flex_1()
                .min_h_0()
                .min_w_0(),
        )
    }
    /// The footer spans the chat column below the transcript, as tall as its content.
    pub(super) fn footer(&self) -> impl IntoElement + use<> {
        self.footer
            .clone()
            .cached_to_content(StyleRefinement::default().w_full())
    }
    /// Redraws the cached regions: the panel's state changed in a way only it knows.
    pub(super) fn invalidate(&self, cx: &mut App) {
        self.transcript.update(cx, |_, cx| cx.notify());
        self.history.update(cx, |_, cx| cx.notify());
        self.footer.update(cx, |_, cx| cx.notify());
    }
    pub(super) fn notify(&self, kind: PartKind, cx: &mut App) {
        let part = match kind {
            PartKind::Transcript => &self.transcript,
            PartKind::History => &self.history,
            PartKind::Footer => &self.footer,
        };
        part.update(cx, |_, cx| cx.notify());
    }
    /// Redraws the transcript when its rows or what they show differ from its last
    /// drawing: a draft or a configuration change of the chat leaves it as it is.
    pub(super) fn transcript_changed(&self, rows: bool, stamp: u64, cx: &mut App) {
        if rows || self.transcript_seen != Some(stamp) {
            self.notify(PartKind::Transcript, cx);
        }
    }
    pub(super) fn drew_transcript(&mut self, stamp: Option<u64>) {
        self.transcript_seen = stamp;
    }
    /// Redraws the history when the rows it lists differ from its last drawing; without
    /// a stamp (a search matches transcripts) it always redraws.
    pub(super) fn history_changed(&self, stamp: Option<u64>, cx: &mut App) {
        if stamp.is_none() || self.history_seen != stamp {
            self.notify(PartKind::History, cx);
        }
    }
    pub(super) fn drew_history(&mut self, stamp: Option<u64>) {
        self.history_seen = stamp;
    }
}

/// What the transcript draws besides its changed rows: the rows' count and liveness,
/// the fork action, the pending prompts and the plan.
pub(super) fn transcript_stamp(view: &ConversationView<'_>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (
        view.transcript.entries.len(),
        view.generating,
        view.accepts_updates,
        view.can_fork,
    )
        .hash(&mut hasher);
    for prompt in view.pending_prompts {
        (prompt.key, prompt.state).hash(&mut hasher);
    }
    if let Some(plan) = &view.transcript.plan {
        for entry in &plan.entries {
            entry.content.hash(&mut hasher);
            std::mem::discriminant(&entry.priority).hash(&mut hasher);
            std::mem::discriminant(&entry.status).hash(&mut hasher);
        }
    }
    hasher.finish()
}
