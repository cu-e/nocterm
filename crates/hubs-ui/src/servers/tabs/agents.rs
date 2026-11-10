//! The Agents tab of a server: the account's agents, created, edited and deleted in one form
//! (name, instructions, provider and models, default servers, approval mode), and the
//! providers the agents use, with the account's own ones added, changed, tested and removed.
//!
//! Every write goes to the hub through the agents port; the tab then lists the hub again. A
//! provider key is typed once and never shown again: the hub keeps it.

mod form;
mod providers;

use gpui_kit::{
    ClickEvent, Context, Entity, Subscription, Task, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hub_agents::HubAgents;
use nocterm_hub_agents_model::{AgentId, HubAgent, HubAgentsError, Provider};
use nocterm_hubs_model::HubId;

use crate::layout::{note, problem};
use crate::messages::agents_problem;
use crate::servers::item;
use form::AgentForm;
use providers::ProviderForm;

/// The agents and providers of the server that is shown, and the form open on one of them.
pub(in crate::servers) struct AgentsTab {
    service: Entity<HubAgents>,
    hub: Option<HubId>,
    providers: Vec<Provider>,
    agent_form: Option<AgentForm>,
    provider_form: Option<ProviderForm>,
    problem: Option<String>,
    working: Option<Task<()>>,
    _service: Subscription,
}

impl AgentsTab {
    pub(in crate::servers) fn new(service: Entity<HubAgents>, cx: &mut Context<Self>) -> Self {
        Self {
            _service: cx.observe(&service, |_, _, cx| cx.notify()),
            service,
            hub: None,
            providers: Vec::new(),
            agent_form: None,
            provider_form: None,
            problem: None,
            working: None,
        }
    }

    /// Shows the agents of `hub`; another server closes the forms.
    pub(in crate::servers) fn show(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self.hub.as_ref() == Some(hub) {
            return;
        }
        self.hub = Some(hub.clone());
        self.agent_form = None;
        self.provider_form = None;
        self.problem = None;
        self.service.update(cx, |service, cx| {
            service.track(hub, cx);
            service.refresh(hub, cx);
        });
        self.reload_providers(cx);
    }

    pub(in crate::servers) fn hide(&mut self) {
        self.hub = None;
        self.working = None;
        self.agent_form = None;
        self.provider_form = None;
    }

    fn reload_providers(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let listing = self.service.read(cx).directory().providers(&hub);
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = listing.await;
            this.update(cx, |tab, cx| {
                match result {
                    Ok(providers) => tab.providers = providers,
                    Err(error) => tab.problem = Some(agents_problem(&error)),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Runs a write against the hub, then lists the agents and providers again.
    fn write(&mut self, call: nocterm_hub_agents_model::AgentsFuture<()>, cx: &mut Context<Self>) {
        self.problem = None;
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |tab, cx| tab.written(result, cx)).ok();
        }));
        cx.notify();
    }

    fn written(&mut self, result: Result<(), HubAgentsError>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                self.agent_form = None;
                self.provider_form = None;
            }
            Err(error) => self.problem = Some(agents_problem(&error)),
        }
        if let Some(hub) = self.hub.clone() {
            self.service
                .update(cx, |service, cx| service.refresh(&hub, cx));
        }
        self.reload_providers(cx);
        cx.notify();
    }

    fn save_agent(&mut self, cx: &mut Context<Self>) {
        let (Some(hub), Some(form)) = (self.hub.clone(), &self.agent_form) else {
            return;
        };
        let draft = form.draft(cx);
        let directory = self.service.read(cx).directory().clone();
        let call: nocterm_hub_agents_model::AgentsFuture<()> = match form.editing().cloned() {
            Some(agent) => {
                let saving = directory.update_agent(&hub, &agent, draft);
                Box::pin(async move { saving.await.map(|_| ()) })
            }
            None => {
                let saving = directory.create_agent(&hub, draft);
                Box::pin(async move { saving.await.map(|_| ()) })
            }
        };
        self.write(call, cx);
    }

    fn delete_agent(&mut self, agent: &AgentId, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let deleting = self.service.read(cx).directory().delete_agent(&hub, agent);
        self.write(deleting, cx);
    }

    fn open_agent(
        &mut self,
        agent: Option<&HubAgent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.agent_form = Some(AgentForm::new(agent, window, cx));
        self.provider_form = None;
        cx.notify();
    }

    fn render_agents(&self, hub: &HubId, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let key = hub.to_string();
        let agents = self.service.read(cx).agents(hub).to_vec();
        let mut list = v_flex()
            .id(item("server-agents", &key))
            .test_support()
            .gap_1();
        if agents.is_empty() {
            list = list.child(note(
                "No agents yet. An agent is a name, instructions and a model; it costs nothing while no chat runs it.",
                cx,
            ));
        }
        for agent in agents {
            let (edit, remove) = (agent.clone(), agent.id.clone());
            list = list.child(
                h_flex()
                    .id(item("server-agent", agent.id.as_str()))
                    .test_support()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().font_semibold().child(agent.name.clone()))
                            .child(note(format!("{} · {}", agent.model, approval(&agent)), cx)),
                    )
                    .child(
                        Button::new(item("server-agent-edit", agent.id.as_str()))
                            .small()
                            .ghost()
                            .label("Edit")
                            .on_click(cx.listener(move |tab, _: &ClickEvent, window, cx| {
                                tab.open_agent(Some(&edit), window, cx);
                            })),
                    )
                    .child(
                        Button::new(item("server-agent-delete", agent.id.as_str()))
                            .small()
                            .ghost()
                            .label("Delete")
                            .on_click(cx.listener(move |tab, _: &ClickEvent, _, cx| {
                                tab.delete_agent(&remove, cx);
                            })),
                    ),
            );
        }
        list.into_any_element()
    }
}

fn approval(agent: &HubAgent) -> &'static str {
    match agent.approval {
        nocterm_hub_agents_model::ApprovalMode::Auto => "Auto",
        nocterm_hub_agents_model::ApprovalMode::Ask => "Ask before changes",
    }
}

impl Render for AgentsTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(hub) = self.hub.clone() else {
            return div().into_any_element();
        };
        let key = hub.to_string();
        let servers = self.service.read(cx).servers(&hub).to_vec();
        let agents = self.render_agents(&hub, cx);
        let agent_form = self
            .agent_form
            .as_ref()
            .map(|form| form.render(&self.providers, &servers, cx));
        let providers = providers::render(&self.providers, self.provider_form.as_ref(), cx);
        v_flex()
            .id(item("server-agents-tab", &key))
            .test_support()
            .gap_3()
            .children(self.problem.clone().map(|text| problem(text, cx)))
            .child(
                h_flex()
                    .justify_between()
                    .child(div().font_semibold().child("Agents"))
                    .child(
                        Button::new(item("server-agent-new", &key))
                            .small()
                            .primary()
                            .label("New agent")
                            .on_click(cx.listener(|tab, _: &ClickEvent, window, cx| {
                                tab.open_agent(None, window, cx);
                            })),
                    ),
            )
            .child(agents)
            .children(agent_form)
            .child(div().h_px().bg(cx.theme().border))
            .child(providers)
            .into_any_element()
    }
}
