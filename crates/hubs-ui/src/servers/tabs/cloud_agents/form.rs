//! The form of a personal agent: provider, endpoint, model, key, and whether it works while the
//! person is offline. The key is write-only: it is read out of its field once, and an edit asks
//! for it again because the definition is sealed anew.

use gpui_kit::{
    AppContext as _, Context, Entity, Window,
    component::input::{InputState, TextareaState},
};
use nocterm_cloud_agents_model::{
    AgentId, DraftError, KeyInput, PersonalAgentDraft, PoolChoice, PoolId, PoolOffer, ProviderKind,
};
use nocterm_session::Secret;

/// What the person has typed and chosen.
pub(super) struct Form {
    existing: Option<(AgentId, u64)>,
    name: Entity<InputState>,
    endpoint: Entity<InputState>,
    model: Entity<InputState>,
    key: Entity<InputState>,
    instructions: Entity<TextareaState>,
    provider: ProviderKind,
    work_offline: bool,
    pools: Vec<PoolChoice>,
    pool: Option<PoolId>,
    /// The draft that waits for the person to trust `offer`.
    waiting: Option<(PersonalAgentDraft, PoolOffer)>,
}

/// The starting values of an edit.
pub(super) struct Prefill {
    pub(super) id: AgentId,
    pub(super) version: u64,
    pub(super) name: String,
    pub(super) model: String,
    pub(super) provider: Option<ProviderKind>,
}

impl Form {
    pub(super) fn new<T: 'static>(
        prefill: Option<Prefill>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        let mut field =
            |placeholder: &'static str, masked: bool, text: &str, cx: &mut Context<T>| {
                let text = text.to_owned();
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx)
                        .masked(masked)
                        .placeholder(placeholder);
                    input.set_value(text, window, cx);
                    input
                })
            };
        let (name, model) = prefill
            .as_ref()
            .map(|each| (each.name.clone(), each.model.clone()))
            .unwrap_or_default();
        Self {
            name: field("Name", false, &name, cx),
            endpoint: field("Endpoint, empty for the provider's own", false, "", cx),
            model: field("Model", false, &model, cx),
            key: field("Provider key", true, "", cx),
            instructions: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("How the agent should behave")
            }),
            provider: prefill
                .as_ref()
                .and_then(|each| each.provider)
                .unwrap_or(ProviderKind::OpenAiChat),
            work_offline: true,
            pools: Vec::new(),
            pool: None,
            waiting: None,
            existing: prefill.map(|each| (each.id, each.version)),
        }
    }

    pub(super) fn existing(&self) -> Option<(AgentId, u64)> {
        self.existing.clone()
    }

    pub(super) fn is_edit(&self) -> bool {
        self.existing.is_some()
    }

    pub(super) fn inputs(&self) -> [&Entity<InputState>; 4] {
        [&self.name, &self.endpoint, &self.model, &self.key]
    }

    /// The instructions field. They are sealed with the definition, so an edit asks for them
    /// again like the key.
    pub(super) fn instructions_input(&self) -> &Entity<TextareaState> {
        &self.instructions
    }

    pub(super) fn provider(&self) -> ProviderKind {
        self.provider
    }

    pub(super) fn set_provider(&mut self, provider: ProviderKind) {
        self.provider = provider;
    }

    pub(super) fn work_offline(&self) -> bool {
        self.work_offline
    }

    pub(super) fn set_work_offline(&mut self, on: bool) {
        self.work_offline = on;
    }

    pub(super) fn pools(&self) -> &[PoolChoice] {
        &self.pools
    }

    pub(super) fn chosen_pool(&self) -> Option<&PoolId> {
        self.pool.as_ref()
    }

    pub(super) fn choose_pool(&mut self, pool: PoolId) {
        self.pool = Some(pool);
    }

    /// The pools the hub offers; the first that takes personal keys is chosen unless the
    /// person already chose one.
    pub(super) fn offer_pools(&mut self, pools: Vec<PoolChoice>) {
        if self.pool.is_none() {
            self.pool = pools
                .iter()
                .find(|pool| pool.accepts_personal)
                .map(|pool| pool.pool.clone());
        }
        self.pools = pools;
    }

    pub(super) fn waiting_offer(&self) -> Option<&PoolOffer> {
        self.waiting.as_ref().map(|(_, offer)| offer)
    }

    pub(super) fn wait_for_trust(&mut self, draft: PersonalAgentDraft, offer: PoolOffer) {
        self.waiting = Some((draft, offer));
    }

    /// The draft that waited for trust, once the person gave it.
    pub(super) fn take_waiting(&mut self) -> Option<(PersonalAgentDraft, PoolOffer)> {
        self.waiting.take()
    }

    /// Reads the form. The key field is emptied: an edit that leaves it blank keeps the key,
    /// but only for a definition that is not resealed, so it is required here.
    pub(super) fn draft<T: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Result<PersonalAgentDraft, DraftError> {
        let text = |input: &Entity<InputState>, cx: &Context<T>| input.read(cx).value().to_string();
        let key = text(&self.key, cx);
        self.key
            .update(cx, |input, cx| input.set_value("", window, cx));
        let key = if key.is_empty() && self.existing.is_some() && !self.work_offline {
            KeyInput::Keep
        } else {
            KeyInput::Replace(Secret::new(key))
        };
        let draft = PersonalAgentDraft::new(
            &text(&self.name, cx),
            self.provider,
            &text(&self.endpoint, cx),
            &text(&self.model, cx),
            key,
            self.work_offline,
            self.pool.clone(),
        )?;
        // The field is only shown while the agent works offline, and only a sealed definition
        // carries instructions: a hidden field's text is not sent.
        if !self.work_offline {
            return Ok(draft);
        }
        let instructions = self.instructions.read(cx).value();
        draft.with_instructions(&instructions)
    }
}
