//! What the composer of the open chat or conversation offers, read once per drawing from its
//! source. Every source draws the same composer; its capabilities, not its kind, decide which
//! controls show.
use super::{composer_view::ComposerView, transcript::TranscriptView};
use gpui_kit::{App, EntityId};
use nocterm_agent_chat::conversation::ConversationSource;
use nocterm_ai::model::usage::ContextUsage;

pub(super) struct ComposerFrame {
    origin: EntityId,
    can_prompt: bool,
    can_stop: bool,
    can_allow: bool,
    can_attach: bool,
    can_configure: bool,
    usage: Option<Option<ContextUsage>>,
    status: Option<(String, bool)>,
}
impl ComposerFrame {
    /// None when the source can no longer be read; the transcript names why.
    pub(super) fn read(source: &dyn ConversationSource, cx: &App) -> Option<Self> {
        let mut frame = None;
        source
            .with_view(cx, &mut |view| {
                frame = Some(Self {
                    origin: source.identity(),
                    can_prompt: view.can_prompt,
                    can_stop: view.can_stop,
                    can_allow: view.can_allow,
                    can_attach: view.composer.attach,
                    can_configure: view.composer.configure,
                    usage: view.composer.usage.then(|| view.transcript.usage.clone()),
                    status: TranscriptView::conversation_status(&view),
                });
            })
            .ok()?;
        frame
    }
    /// The source the composer sends to and its controls act on.
    pub(super) fn origin(&self) -> EntityId {
        self.origin
    }
    pub(super) fn can_prompt(&self) -> bool {
        self.can_prompt
    }
    pub(super) fn can_allow(&self) -> bool {
        self.can_allow
    }
    pub(super) fn can_attach(&self) -> bool {
        self.can_attach
    }
    pub(super) fn can_configure(&self) -> bool {
        self.can_configure
    }
    /// The context usage the usage ring shows, when the source reports one.
    pub(super) fn usage(&self) -> Option<Option<&ContextUsage>> {
        self.usage.as_ref().map(Option::as_ref)
    }
    /// The status line the source describes: its text and whether it is live.
    pub(super) fn status(&self) -> Option<(String, bool)> {
        self.status.clone()
    }
    /// Whether the send button stops the run instead: nothing is typed, attached or edited
    /// and there is a run to stop.
    pub(super) fn stops(&self, composer: &ComposerView, images: bool, cx: &App) -> bool {
        self.can_stop && composer.text(cx).trim().is_empty() && !images && !composer.editing()
    }
    /// Whether sending is unavailable: the source takes no prompts now, images are being
    /// prepared, or the composer's prompt is still on its way.
    pub(super) fn send_disabled(&self, stops: bool, composer: &ComposerView, busy: bool) -> bool {
        !stops && (!self.can_prompt || composer.preparing() || busy)
    }
}
