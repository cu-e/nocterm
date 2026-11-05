//! One selected captured conversation and its original per-window observation.
//! Transcript/composer rendering remains with the existing panel owners.
use super::{AgentPanel, composer_view::InputTicket};
use gpui_kit::{App, Context, EntityId, SharedString, Subscription, WeakEntity};
use nocterm_agent_chat::conversation::{
    ConversationError, ConversationSource, ConversationSubmission,
};
use std::rc::Rc;

pub(super) struct ConversationSurface {
    source: Rc<dyn ConversationSource>,
    title: SharedString,
    _observations: [Subscription; 2],
    dirty: bool,
    previous_local: Option<EntityId>,
    pending: Option<PendingInput>,
    /// The key the history column lists this conversation under, when it lists it.
    origin: Option<String>,
    /// The Servers control of a source that binds servers (a Cloud chat, ADR 0139).
    servers: Option<gpui_kit::Entity<super::cloud_servers::CloudServersControl>>,
}
struct PendingInput {
    ticket: InputTicket,
    submission: Rc<dyn ConversationSubmission>,
    busy: bool,
}
/// The callback capability created by the real renderer, not a current-source
/// lookup deferred until the click. It may safely outlive its original frame.
pub(super) struct RenderedStop {
    identity: EntityId,
    actions: Rc<dyn nocterm_agent_chat::conversation::ConversationActions>,
    input: InputTicket,
}
impl RenderedStop {
    pub(super) fn capture(
        source: Rc<dyn ConversationSource>,
        input: InputTicket,
        cx: &App,
    ) -> Result<Rc<Self>, ConversationError> {
        Ok(Rc::new(Self {
            identity: source.identity(),
            actions: source.capture_actions(cx)?,
            input,
        }))
    }
    pub(super) fn activate(&self, panel: &mut AgentPanel, cx: &mut Context<AgentPanel>) {
        panel.perform_rendered_stop(self.identity, self.actions.clone(), self.input.clone(), cx);
    }
}
impl ConversationSurface {
    pub(super) fn attach(
        source: Rc<dyn ConversationSource>,
        title: SharedString,
        previous_local: Option<EntityId>,
        panel: WeakEntity<AgentPanel>,
        cx: &mut Context<AgentPanel>,
    ) -> Result<Self, ConversationError> {
        let identity = source.identity();
        let observation = source.observe(
            cx,
            Rc::new(move |cx| {
                let _ = panel.update(cx, |panel, cx| panel.on_conversation_changed(identity, cx));
            }),
        )?;
        source.set_shown(true, cx)?;
        let release = cx.on_release(|panel: &mut AgentPanel, cx| {
            if let Some(selected) = &panel.conversation {
                selected.detach(cx);
            }
        });
        let servers = source
            .servers(cx)
            .map(|servers| super::cloud_servers::control(servers, cx));
        Ok(Self {
            source,
            title,
            _observations: [observation, release],
            dirty: true,
            previous_local,
            pending: None,
            origin: None,
            servers,
        })
    }
    pub(super) fn set_origin(&mut self, origin: Option<String>) {
        self.origin = origin;
    }
    pub(super) fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }
    pub(super) fn identity(&self) -> EntityId {
        self.source.identity()
    }
    pub(super) fn source(&self) -> Rc<dyn ConversationSource> {
        self.source.clone()
    }
    /// The Servers control, when the source binds servers.
    pub(super) fn servers(
        &self,
    ) -> Option<gpui_kit::Entity<super::cloud_servers::CloudServersControl>> {
        self.servers.clone()
    }
    pub(super) fn title(&self) -> SharedString {
        self.title.clone()
    }
    pub(super) fn changed(&mut self) {
        self.dirty = true;
    }
    pub(super) fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
    pub(super) fn pending(
        &self,
        input: &InputTicket,
    ) -> Option<(Rc<dyn ConversationSubmission>, bool)> {
        self.pending
            .as_ref()
            .filter(|pending| &pending.ticket == input)
            .map(|pending| (pending.submission.clone(), pending.busy))
    }
    pub(super) fn submit(
        &mut self,
        action: nocterm_ai::conversation::ConversationAction,
        input: InputTicket,
        cx: &mut App,
    ) -> Result<Option<gpui_kit::Task<Result<(), ConversationError>>>, ConversationError> {
        use nocterm_ai::conversation::ConversationAction;
        let prompt = matches!(&action, ConversationAction::Prompt(_));
        let mut replaces = None;
        if prompt && let Some((submission, busy)) = self.pending(&input) {
            if busy {
                return Ok(None);
            }
            // A refused prompt is over: the same text sent again is a new prompt, never a
            // retry that can only be refused again. The new prompt replaces its row.
            if submission.refused(cx) {
                self.pending = None;
                replaces = submission.pending_prompt();
            } else {
                self.submitted(input, submission.clone());
                return Ok(Some(submission.retry(cx)));
            }
        }
        let mut allowed = false;
        self.source.with_view(cx, &mut |view| {
            allowed = match &action {
                ConversationAction::Prompt(_) => view.can_prompt,
                ConversationAction::Stop => view.can_stop,
                ConversationAction::Allow => view.can_allow,
            }
        })?;
        if !allowed {
            return Err(ConversationError::Unavailable);
        }
        let actions = self.source.capture_actions(cx)?;
        let (submission, completion) = actions.submit(action, cx)?;
        if prompt {
            self.submitted(input, submission);
        }
        if let Some(refused) = replaces {
            // The refused row may already be dismissed; nothing else depends on it.
            let _ = actions.dismiss_prompt(refused, cx);
        }
        Ok(Some(completion))
    }
    pub(super) fn submitted(
        &mut self,
        input: InputTicket,
        submission: Rc<dyn ConversationSubmission>,
    ) {
        self.pending = Some(PendingInput {
            ticket: input,
            submission,
            busy: true,
        });
    }
    pub(super) fn completed(&mut self, ticket: &InputTicket, failed: bool) {
        if let Some(pending) = self
            .pending
            .as_mut()
            .filter(|pending| &pending.ticket == ticket)
        {
            pending.busy = !failed;
        }
    }
    pub(super) fn previous_local(&self) -> Option<EntityId> {
        self.previous_local
    }
    pub(super) fn detach(&self, cx: &mut App) {
        // Security retirement can already have ended visibility. Dropping this
        // observer never sends a remote control command or touches a run lease.
        let _ = self.source.set_shown(false, cx);
    }
}
