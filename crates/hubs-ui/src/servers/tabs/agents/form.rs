//! The form of one agent: name, instructions, provider and models, default servers and whether
//! changing tools wait for an approval.

use std::collections::BTreeSet;

use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, TestSupportExt as _, Window,
    component::{
        Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        h_flex,
        input::{Input, InputState, Textarea, TextareaState},
        v_flex,
    },
    prelude::*,
};
use nocterm_hub_agents_model::{
    AgentDraft, AgentId, ApprovalMode, HubAgent, HubServer, Provider, ProviderId, ServerId,
};

use super::AgentsTab;
use crate::layout::{field, note};
use crate::servers::item;

/// What the person has typed and chosen for an agent.
pub(super) struct AgentForm {
    editing: Option<AgentId>,
    name: Entity<InputState>,
    instructions: Entity<TextareaState>,
    models: Entity<InputState>,
    model: Entity<InputState>,
    provider: Option<ProviderId>,
    approval: ApprovalMode,
    servers: BTreeSet<ServerId>,
}

fn input<T: 'static>(
    placeholder: &'static str,
    text: &str,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<InputState> {
    let text = text.to_owned();
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder(placeholder);
        input.set_value(text, window, cx);
        input
    })
}

impl AgentForm {
    /// A form for a new agent, or one that edits `agent`.
    pub(super) fn new<T: 'static>(
        agent: Option<&HubAgent>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        let draft = agent.map(HubAgent::draft).unwrap_or_default();
        let instructions = draft.instructions.clone();
        Self {
            editing: agent.map(|agent| agent.id.clone()),
            name: input("Name", &draft.name, window, cx),
            instructions: cx.new(|cx| {
                let mut state =
                    TextareaState::new(window, cx).placeholder("How the agent should behave");
                state.set_value(instructions, window, cx);
                state
            }),
            models: input(
                "Models, separated by commas",
                &draft.models.join(", "),
                window,
                cx,
            ),
            model: input("Default model", &draft.model, window, cx),
            provider: draft.provider,
            approval: draft.approval,
            servers: draft.servers.into_iter().collect(),
        }
    }

    pub(super) fn editing(&self) -> Option<&AgentId> {
        self.editing.as_ref()
    }

    /// The form as a draft; its rules are checked where it is saved.
    pub(super) fn draft(&self, cx: &gpui_kit::App) -> AgentDraft {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        AgentDraft {
            name: text(&self.name),
            instructions: self.instructions.read(cx).value().to_string(),
            provider: self.provider.clone(),
            model: text(&self.model),
            models: text(&self.models)
                .split(',')
                .map(|model| model.trim().to_owned())
                .collect(),
            approval: self.approval,
            servers: self.servers.iter().cloned().collect(),
        }
    }

    fn choose_provider(
        &mut self,
        provider: &Provider,
        window: &mut Window,
        cx: &mut gpui_kit::App,
    ) {
        self.provider = Some(provider.id.clone());
        // The provider's models are what the agent may use unless the person names others.
        if self.models.read(cx).value().trim().is_empty() {
            let models = provider.models.join(", ");
            self.models
                .update(cx, |input, cx| input.set_value(models, window, cx));
        }
    }

    pub(super) fn render(
        &self,
        providers: &[Provider],
        servers: &[HubServer],
        cx: &mut Context<AgentsTab>,
    ) -> gpui_kit::AnyElement {
        let defaults = if servers.is_empty() {
            note(
                "No servers are connected to agents yet: turn on Agent access of a saved server.",
                cx,
            )
            .into_any_element()
        } else {
            self.server_choices(servers, cx).into_any_element()
        };
        v_flex()
            .id("server-agent-form")
            .test_support()
            .gap_2()
            .child(field("Name", Input::new(&self.name)))
            .child(field("Instructions", Textarea::new(&self.instructions)))
            .child(field("Provider", self.provider_choices(providers, cx)))
            .child(field("Models", Input::new(&self.models)))
            .child(field("Default model", Input::new(&self.model)))
            .child(self.approval_choice(cx))
            .child(field("Default servers", defaults))
            .child(actions(cx))
            .into_any_element()
    }

    fn provider_choices(
        &self,
        providers: &[Provider],
        cx: &mut Context<AgentsTab>,
    ) -> impl IntoElement {
        h_flex()
            .flex_wrap()
            .gap_1()
            .children(providers.iter().map(|provider| {
                let chosen = provider.clone();
                Button::new(item("server-agent-provider", provider.id.as_str()))
                    .small()
                    .ghost()
                    .label(provider.name.clone())
                    .selected(self.provider.as_ref() == Some(&provider.id))
                    .on_click(cx.listener(move |tab, _: &ClickEvent, window, cx| {
                        if let Some(form) = tab.agent_form.as_mut() {
                            form.choose_provider(&chosen, window, cx);
                        }
                        cx.notify();
                    }))
            }))
    }

    fn approval_choice(&self, cx: &mut Context<AgentsTab>) -> impl IntoElement {
        let ask = self.approval == ApprovalMode::Ask;
        Checkbox::new("server-agent-ask")
            .label("Ask before changes: tools that change something wait for Allow")
            .checked(ask)
            .on_click(cx.listener(move |tab, _: &bool, _, cx| {
                if let Some(form) = tab.agent_form.as_mut() {
                    form.approval = if ask {
                        ApprovalMode::Auto
                    } else {
                        ApprovalMode::Ask
                    };
                }
                cx.notify();
            }))
    }

    fn server_choices(
        &self,
        servers: &[HubServer],
        cx: &mut Context<AgentsTab>,
    ) -> impl IntoElement {
        v_flex().gap_1().children(servers.iter().map(|server| {
            let id = server.id.clone();
            let on = self.servers.contains(&id);
            Checkbox::new(item("server-agent-server", server.id.as_str()))
                .label(format!("{} ({}@{})", server.name, server.user, server.host))
                .checked(on)
                .on_click(cx.listener(move |tab, _: &bool, _, cx| {
                    if let Some(form) = tab.agent_form.as_mut() {
                        if on {
                            form.servers.remove(&id);
                        } else {
                            form.servers.insert(id.clone());
                        }
                    }
                    cx.notify();
                }))
        }))
    }
}

/// Cancel and Save under the form.
fn actions(cx: &mut Context<AgentsTab>) -> impl IntoElement {
    h_flex()
        .gap_2()
        .justify_end()
        .child(
            Button::new("server-agent-cancel")
                .small()
                .ghost()
                .label("Cancel")
                .on_click(cx.listener(|tab, _: &ClickEvent, _, cx| {
                    tab.agent_form = None;
                    cx.notify();
                })),
        )
        .child(
            Button::new("server-agent-save")
                .small()
                .primary()
                .label("Save")
                .on_click(cx.listener(|tab, _: &ClickEvent, _, cx| tab.save_agent(cx))),
        )
}
