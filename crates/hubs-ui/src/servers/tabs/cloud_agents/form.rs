//! The form of a personal agent: provider, endpoint, models, key and offline placement.
//! Managed edits seal a new definition and require its key again; an Own edit may keep the
//! saved key and starts from an authenticated model list, retained as its immutable edit base.
//! The own runner keeps its configured endpoint. Valid submissions clear the secret field.

use super::CloudAgentsTab;
use crate::{
    layout::{field, note},
    servers::item,
};
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Disableable as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        h_flex,
        input::{Input, InputState, Textarea, TextareaState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_cloud_agents_model::{
    AgentId, DraftError, KeyInput, PersonalAgentDraft, PoolChoice, PoolId, PoolOffer, ProviderKind,
};
use nocterm_session::Secret;

/// What the person has typed and chosen.
pub(super) struct Form {
    existing: Option<Prefill>,
    name: Entity<InputState>,
    endpoint: Entity<InputState>,
    models: Entity<TextareaState>,
    key: Entity<InputState>,
    instructions: Entity<TextareaState>,
    provider: ProviderKind,
    work_offline: bool,
    pools: Vec<PoolChoice>,
    pool: Option<PoolId>,
    /// The draft that waits for the person to trust `offer`.
    waiting: Option<(PersonalAgentDraft, PoolOffer)>,
}

/// The original listing of an edit, retained unchanged as its optimistic model base.
#[derive(Clone)]
pub(super) struct Prefill {
    pub(super) id: AgentId,
    pub(super) version: u64,
    pub(super) name: String,
    pub(super) models: PrefillModels,
    pub(super) provider: Option<ProviderKind>,
    pub(super) work_offline: bool,
}

/// The displayed initial IDs and the authority, if any, behind an Own edit's base.
#[derive(Clone)]
pub(super) enum PrefillModels {
    Listed(Vec<String>),
    Canonical(Vec<String>),
    Missing(Vec<String>),
}
impl PrefillModels {
    fn values(&self) -> &[String] {
        match self {
            Self::Listed(models) | Self::Canonical(models) | Self::Missing(models) => models,
        }
    }
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
        let (name, models) = prefill
            .as_ref()
            .map(|each| (each.name.clone(), each.models.values().join("\n")))
            .unwrap_or_default();
        Self {
            name: field("Name", false, &name, cx),
            endpoint: field("Endpoint, empty for the provider's own", false, "", cx),
            key: field("Provider key", true, "", cx),
            models: cx.new(|cx| {
                let mut input = TextareaState::new(window, cx)
                    .placeholder("One model ID per line; the first is the default")
                    .rows(3);
                input.set_value(models, window, cx);
                input
            }),
            instructions: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("How the agent should behave")
            }),
            provider: prefill
                .as_ref()
                .and_then(|each| each.provider)
                .unwrap_or(ProviderKind::OpenAiChat),
            work_offline: prefill.as_ref().is_none_or(|each| each.work_offline),
            pools: Vec::new(),
            pool: None,
            waiting: None,
            existing: prefill,
        }
    }

    pub(super) fn existing(&self) -> Option<(AgentId, u64)> {
        self.existing
            .as_ref()
            .map(|each| (each.id.clone(), each.version))
    }

    pub(super) fn is_edit(&self) -> bool {
        self.existing.is_some()
    }

    pub(super) fn replacing_missing_key(&self) -> bool {
        self.existing
            .as_ref()
            .is_some_and(|base| matches!(base.models, PrefillModels::Missing(_)))
    }

    pub(super) fn inputs(&self) -> [&Entity<InputState>; 3] {
        [&self.name, &self.endpoint, &self.key]
    }

    pub(super) fn models_input(&self) -> &Entity<TextareaState> {
        &self.models
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

    /// Validates the form, then empties the key field before the draft is sent. Invalid
    /// input keeps the fields so the person can correct it without retyping a secret.
    pub(super) fn draft<T: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Result<PersonalAgentDraft, DraftError> {
        let text = |input: &Entity<InputState>, cx: &Context<T>| input.read(cx).value().to_string();
        let key = text(&self.key, cx);
        let keeps_key = self
            .existing
            .as_ref()
            .is_some_and(|base| matches!(base.models, PrefillModels::Canonical(_)));
        let key = if key.is_empty() && keeps_key && !self.work_offline {
            KeyInput::Keep
        } else {
            KeyInput::Replace(Secret::new(key))
        };
        let models: Vec<_> = self
            .models
            .read(cx)
            .value()
            .lines()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .take(nocterm_cloud_agents_model::MAX_MODELS + 1)
            .map(str::to_owned)
            .collect();
        let draft = PersonalAgentDraft::new(
            &text(&self.name, cx),
            self.provider,
            &text(&self.endpoint, cx),
            models.first().map_or("", String::as_str),
            key,
            self.work_offline,
            self.pool.clone(),
        )?
        .with_models(models)?;
        let draft = match self
            .existing
            .as_ref()
            .filter(|_| !self.work_offline)
            .map(|base| &base.models)
        {
            Some(PrefillModels::Canonical(models)) => draft.with_expected_models(models.clone())?,
            Some(PrefillModels::Missing(_)) => draft.with_missing_key()?,
            _ => draft,
        };
        // The field is only shown while the agent works offline, and only a sealed definition
        // carries instructions: a hidden field's text is not sent.
        let draft = if self.work_offline {
            let instructions = self.instructions.read(cx).value();
            draft.with_instructions(&instructions)?
        } else {
            draft
        };
        self.key
            .update(cx, |input, cx| input.set_value("", window, cx));
        Ok(draft)
    }
}

fn provider_buttons(
    form: &Form,
    epoch: u64,
    busy: bool,
    key: &str,
    view: &Entity<CloudAgentsTab>,
) -> impl IntoIterator<Item = Button> {
    ProviderKind::ALL.map(|kind| {
        let view = view.clone();
        Button::new(item(&format!("cloud-agent-provider-{}", kind.id()), key))
            .label(kind.label())
            .disabled(busy)
            .when(form.provider() == kind, |button| button.primary())
            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                view.update(cx, |tab, cx| {
                    if tab.form_epoch != epoch {
                        return;
                    }
                    if let Some(form) = &mut tab.form {
                        form.set_provider(kind);
                    }
                    cx.notify();
                });
            })
    })
}

fn offline_checkbox(
    form: &Form,
    epoch: u64,
    busy: bool,
    key: &str,
    view: &Entity<CloudAgentsTab>,
) -> Checkbox {
    let offline_view = view.clone();
    Checkbox::new(item("cloud-agent-offline", key))
        .label("Agents work when I'm offline")
        .checked(form.work_offline())
        .disabled(busy)
        .on_click(move |checked: &bool, _: &mut Window, cx: &mut App| {
            let on = *checked;
            offline_view.update(cx, |tab, cx| {
                if tab.form_epoch != epoch {
                    return;
                }
                if let Some(form) = &mut tab.form {
                    form.set_work_offline(on);
                }
                cx.notify();
            });
        })
}

fn pool_choices(
    form: &Form,
    epoch: u64,
    busy: bool,
    key: &str,
    view: &Entity<CloudAgentsTab>,
) -> Option<impl IntoElement> {
    form.work_offline().then(|| {
        let buttons = form
            .pools()
            .iter()
            .filter(|pool| pool.accepts_personal)
            .map(|pool| {
                let (view, id) = (view.clone(), pool.pool.clone());
                Button::new(item(&format!("cloud-agent-pool-{}", pool.pool), key))
                    .label(pool.name.clone())
                    .disabled(busy)
                    .when(form.chosen_pool() == Some(&pool.pool), |button| {
                        button.primary()
                    })
                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        view.update(cx, |tab, cx| {
                            if tab.form_epoch != epoch {
                                return;
                            }
                            if let Some(form) = &mut tab.form {
                                form.choose_pool(id.clone());
                            }
                            cx.notify();
                        });
                    })
            });
        field("Pool", h_flex().gap_2().children(buttons))
    })
}

fn trust_block(
    form: &Form,
    epoch: u64,
    busy: bool,
    key: &str,
    view: &Entity<CloudAgentsTab>,
    cx: &App,
) -> Option<impl IntoElement> {
    let theme = cx.theme().clone();
    form.waiting_offer().map(|offer| {
        let view = view.clone();
        v_flex()
            .id(item("cloud-agent-trust", key))
            .test_support()
            .gap_2()
            .child(note(
                format!(
                    "Trust pool \"{}\"? Compare this fingerprint with the one its owner publishes before you do.",
                    offer.name
                ),
                cx,
            ))
            .child(
                div()
                    .id(item("cloud-agent-trust-fingerprint", key))
                    .test_support()
                    .font_family(theme.mono_font_family.clone())
                    .text_sm()
                    .child(offer.fingerprint.clone()),
            )
            .child(
                Button::new(item("cloud-agent-trust-confirm", key))
                    .primary()
                    .label("Trust pool and save")
                    .disabled(busy)
                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        view.update(cx, |tab, cx| { if tab.form_epoch == epoch { tab.trust_and_save(cx); } });
                    }),
            )
    })
}

pub(super) fn render_form(
    form: &Form,
    epoch: u64,
    working: bool,
    key: &str,
    view: &Entity<CloudAgentsTab>,
    cx: &mut Context<CloudAgentsTab>,
) -> impl IntoElement {
    let busy = working || form.waiting_offer().is_some();
    let [name, endpoint, secret] = form.inputs();
    let theme = cx.theme().clone();
    let (provider_buttons, offline) = (
        provider_buttons(form, epoch, busy, key, view),
        offline_checkbox(form, epoch, busy, key, view),
    );
    let instructions = form.work_offline().then(|| {
        field(
            "Instructions",
            div()
                .id(item("cloud-agent-instructions", key))
                .test_support()
                .child(
                    Textarea::new(form.instructions_input())
                        .disabled(busy)
                        .h(rems(8.))
                        .aria_label("Instructions"),
                ),
        )
    });
    let (pools, trust) = (
        pool_choices(form, epoch, busy, key, view),
        trust_block(form, epoch, working, key, view, cx),
    );
    v_flex()
        .id(item("cloud-agent-form", key))
        .test_support()
        .gap_3()
        .p_3()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .child(field("Name", div().id(item("cloud-agent-name", key)).test_support().child(Input::new(name).disabled(busy))))
        .child(field("Provider", h_flex().gap_2().children(provider_buttons)))
        .when(form.work_offline(), |form| form.child(field("Endpoint", div().id(item("cloud-agent-endpoint", key)).test_support().child(Input::new(endpoint).disabled(busy)))))
        .child(field("Models · one per line, up to 32", div().id(item("cloud-agent-model", key)).test_support()
            .child(Textarea::new(form.models_input()).disabled(busy).h(rems(4.)).aria_label("Models, one per line; first is the default"))))
        .child(field("Provider key", div().id(item("cloud-agent-key", key)).test_support().child(Input::new(secret).disabled(busy))))
        .child(note(
            if !form.work_offline() && form.replacing_missing_key() {
                "No saved key was found. These models come from the agent listing; review them and enter a replacement key. A key saved elsewhere meanwhile will stop this replacement."
            } else if !form.work_offline() {
                "The own runner uses its configured provider connection. Leave the key empty when editing to keep the saved key."
            } else if form.is_edit() {
                "Saving replaces the whole definition, so enter the endpoint, the key and the instructions again; leave the instructions empty to remove them. The key is stored encrypted and is never shown."
            } else {
                "The key is stored encrypted and is never shown again. Use a provider key with a spending limit."
            },
            cx,
        ))
        .child(offline)
        .child(note(
            "On: the agent keeps working on a server pool while this computer is off, which means the pool's host can use the key. Off: it runs only on this computer.",
            cx,
        ))
        .children(instructions)
        .children(pools)
        .children(trust)
        .child(form_actions(epoch, busy, key, view))
}

fn form_actions(
    epoch: u64,
    busy: bool,
    key: &str,
    view: &Entity<CloudAgentsTab>,
) -> impl IntoElement {
    let submit_view = view.clone();
    let cancel_view = view.clone();
    h_flex()
        .gap_2()
        .justify_end()
        .child(
            Button::new(item("cloud-agent-cancel", key))
                .label("Cancel")
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    cancel_view.update(cx, |tab, cx| {
                        if tab.form_epoch == epoch {
                            tab.close_form(cx);
                        }
                    });
                }),
        )
        .child(
            Button::new(item("cloud-agent-save", key))
                .primary()
                .label("Save")
                .disabled(busy)
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    submit_view.update(cx, |tab, cx| {
                        if tab.form_epoch == epoch {
                            tab.submit(window, cx);
                        }
                    });
                }),
        )
}
