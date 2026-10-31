//! The Cloud tab of the new-chat popover: the Cloud agents of the signed-in hubs, a search
//! field, the progress of a start and the pool-trust question.
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, SharedString, Subscription,
    TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
        input::{InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_cloud_agents::{CloudAgentsEvent, StartId, StartPhase};
use nocterm_cloud_agents_model::{
    AgentScope, AgentStatus, CloudAgent, CloudAgentError, Section, web_panel_url,
};
use nocterm_hubs_model::{HubColor, HubId};

use super::{
    AgentPanel,
    widgets::{menu_row, menu_row_with},
};
use crate::cloud_access::CloudAgentsAccess;

mod chats;

/// Which list the new-chat popover shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuTab {
    Local,
    Cloud,
}

/// The state of the Cloud tab: the chosen tab and the search text. The agents themselves stay
/// in the shared service.
pub(super) struct CloudMenu {
    access: Option<CloudAgentsAccess>,
    tab: MenuTab,
    search: Entity<InputState>,
    chats: chats::CloudChats,
    _subscriptions: Vec<Subscription>,
}

impl CloudMenu {
    pub(super) fn new(
        access: Option<CloudAgentsAccess>,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search agents…"));
        let mut subscriptions = vec![cx.subscribe(&search, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        if let Some(access) = &access {
            subscriptions.push(cx.subscribe_in(
                access.service(),
                window,
                |panel, _, event, window, cx| match event {
                    CloudAgentsEvent::Ready(id) => {
                        panel.cloud_start_ready(*id, window, cx);
                        cx.notify();
                    }
                    CloudAgentsEvent::Changed => cx.notify(),
                },
            ));
            subscriptions.push(cx.observe(access.hubs(), |_, _, cx| cx.notify()));
        }
        Self {
            access,
            tab: MenuTab::Local,
            search,
            chats: chats::CloudChats::default(),
            _subscriptions: subscriptions,
        }
    }

    /// Whether the switch between Local and Cloud is shown: the tab is hidden when no hub
    /// offers Cloud agents (a hub that does not know them answers UNIMPLEMENTED).
    pub(super) fn offered(&self, cx: &App) -> bool {
        self.access
            .as_ref()
            .is_some_and(|access| access.offered(cx))
    }

    /// The tab in effect: Cloud only while something offers it.
    pub(super) fn tab(&self, cx: &App) -> MenuTab {
        if self.offered(cx) {
            self.tab
        } else {
            MenuTab::Local
        }
    }

    pub(super) fn select(&mut self, tab: MenuTab) {
        self.tab = tab;
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }
}

fn status_label(status: AgentStatus) -> &'static str {
    status.label()
}

fn limits_text(agent: &CloudAgent) -> String {
    let limits = agent.limits;
    format!(
        "CPU {:.1} core, {} MiB memory, {} MiB disk, {} parallel runs; stops after {} s idle",
        f64::from(limits.cpu_milli) / 1000.0,
        limits.mem_mib,
        limits.disk_mib,
        limits.max_parallel_runs,
        limits.idle_ttl_s
    )
}

fn owner_text(agent: &CloudAgent, hub_name: &str) -> String {
    match &agent.scope {
        AgentScope::Personal => "Owner: you".to_owned(),
        AgentScope::Org { name, .. } => format!("Owner: {name}"),
        AgentScope::Server => format!("Owner: server {hub_name}"),
    }
}

/// What hovering a row shows: limits, owner and pool.
pub(super) fn tooltip_text(agent: &CloudAgent, hub_name: &str) -> String {
    let pools = if agent.pools.is_empty() {
        "no pool yet".to_owned()
    } else {
        agent
            .pools
            .iter()
            .map(|pool| pool.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut lines = Vec::new();
    if !agent.description.is_empty() {
        lines.push(agent.description.clone());
    }
    lines.push(limits_text(agent));
    lines.push(owner_text(agent, hub_name));
    lines.push(format!("Pool: {pools}"));
    lines.join("\n")
}

fn hub_tint(color: HubColor, cx: &App) -> gpui_kit::Hsla {
    let theme = cx.theme();
    match color {
        HubColor::Red => theme.red,
        HubColor::Yellow => theme.yellow,
        HubColor::Green => theme.green,
        HubColor::Cyan => theme.cyan,
        HubColor::Blue => theme.blue,
        HubColor::Magenta => theme.magenta,
    }
}

fn failure_text(error: &CloudAgentError) -> String {
    error.to_string()
}

impl AgentPanel {
    /// The tab switch at the top of the "New agent" popover, or nothing when Cloud is not
    /// offered.
    pub(super) fn render_menu_tabs(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.cloud.offered(cx) {
            return None;
        }
        let current = self.cloud.tab(cx);
        let tab = |id: &'static str, label: &'static str, tab: MenuTab, cx: &mut Context<Self>| {
            div().flex_1().min_w_0().child(
                menu_row(id, label, cx)
                    .selected(current == tab)
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.cloud.select(tab);
                        panel.refresh_cloud_chats(cx);
                        cx.notify();
                    })),
            )
        };
        Some(
            gpui_kit::component::h_flex()
                .w_full()
                .gap_1()
                .child(tab("new-agent-tab-local", "Local", MenuTab::Local, cx))
                .child(tab("new-agent-tab-cloud", "Cloud", MenuTab::Cloud, cx))
                .into_any_element(),
        )
    }

    /// The Cloud tab: search, starts in progress, sections of agents, footer.
    pub(super) fn render_cloud_tab(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(access) = self.cloud.access.clone() else {
            return Vec::new();
        };
        let mut rows = vec![
            gpui_kit::component::input::Input::new(&self.cloud.search)
                .small()
                .into_any_element(),
        ];
        rows.extend(self.render_starts(&access, cx));
        let query = self.cloud.query(cx);
        let sections = access.sections(&query, cx);
        if sections.is_empty() {
            rows.push(
                div()
                    .id("cloud-agents-empty")
                    .test_support()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if query.trim().is_empty() {
                        "No Cloud agents yet"
                    } else {
                        "No agent matches"
                    })
                    .into_any_element(),
            );
        }
        let several_hubs = access.visible_hubs(cx).len() > 1;
        for section in sections {
            rows.push(self.render_section(&section, several_hubs, cx));
        }
        rows.extend(self.render_cloud_chats(&access, cx));
        rows.extend(self.render_cloud_footer(&access, cx));
        rows
    }

    fn render_section(
        &mut self,
        section: &Section,
        several_hubs: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = section.title.text();
        let heading = if several_hubs
            && !matches!(
                section.title,
                nocterm_cloud_agents_model::SectionTitle::Server(_)
            ) {
            format!("{title} ({})", section.hub_name)
        } else {
            title
        };
        let tint = hub_tint(section.color, cx);
        let mut block = v_flex().gap_1().child(
            div()
                .id(SharedString::from(format!(
                    "cloud-section-{}-{}",
                    section.hub.address(),
                    heading
                )))
                .test_support()
                .px_2()
                .text_xs()
                .font_semibold()
                .text_color(tint)
                .child(heading),
        );
        for agent in &section.agents {
            block = block.child(self.render_agent_row(&section.hub, &section.hub_name, agent, cx));
        }
        block.into_any_element()
    }

    fn render_agent_row(
        &mut self,
        hub: &HubId,
        hub_name: &str,
        agent: &CloudAgent,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = format!("cloud-agent-{}-{}", hub.address(), agent.id);
        let startable = agent.status.can_start();
        let (hub_id, chosen) = (hub.clone(), agent.clone());
        let model = agent.primary_model().unwrap_or_default().to_owned();
        let chip_color = if agent.status.needs_attention() {
            cx.theme().warning
        } else {
            cx.theme().muted_foreground
        };
        menu_row_with(
            SharedString::from(id),
            agent.name.clone(),
            Some(
                crate::agent_icon::agent_icon(None)
                    .small()
                    .into_any_element(),
            ),
            cx,
        )
        .tooltip(SharedString::from(tooltip_text(agent, hub_name)))
        .child(
            div()
                .flex_none()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(model),
        )
        .child(
            div()
                .id(SharedString::from(format!(
                    "cloud-agent-status-{}-{}",
                    hub.address(),
                    agent.id
                )))
                .test_support()
                .flex_none()
                .text_xs()
                .text_color(chip_color)
                .child(status_label(agent.status)),
        )
        .disabled(!startable)
        .on_click(cx.listener(move |panel, _, window, cx| {
            panel.start_cloud_agent(&hub_id, &chosen, window, cx);
        }))
        .into_any_element()
    }

    fn render_starts(
        &mut self,
        access: &CloudAgentsAccess,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let service = access.service().clone();
        let visible: Vec<HubId> = access
            .visible_hubs(cx)
            .into_iter()
            .map(|(hub, ..)| hub)
            .collect();
        let starts: Vec<_> = service
            .read(cx)
            .starts()
            .filter(|start| visible.contains(start.hub()))
            .map(|start| (start.id(), start.name().to_owned(), start.phase().clone()))
            .collect();
        starts
            .into_iter()
            .map(|(id, name, phase)| self.render_start(id, name, phase, cx))
            .collect()
    }

    fn render_start(
        &mut self,
        id: StartId,
        name: String,
        phase: StartPhase,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let frame = v_flex()
            .id(SharedString::from(format!("cloud-start-{id:?}")))
            .test_support()
            .gap_1()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(cx.theme().secondary);
        let text = |text: String, cx: &App| {
            div()
                .text_sm()
                .text_color(cx.theme().foreground)
                .child(text)
        };
        let button = |label: &'static str, key: &'static str, cx: &mut Context<Self>| {
            menu_row(SharedString::from(format!("{key}-{id:?}")), label, cx)
        };
        match phase {
            StartPhase::Step(step) => frame
                .child(text(format!("{name}: {}", step.text()), cx))
                .into_any_element(),
            StartPhase::AwaitingTrust(offer) => frame
                .child(text(
                    format!(
                        "{name}: trust pool \"{}\"? Compare this fingerprint with the one its owner publishes.",
                        offer.name
                    ),
                    cx,
                ))
                .child(
                    div()
                        .id(SharedString::from(format!("cloud-pool-fingerprint-{id:?}")))
                        .test_support()
                        .text_xs()
                        .font_family("monospace")
                        .child(offer.fingerprint.clone()),
                )
                .child(
                    button("Trust pool", "cloud-trust", cx).on_click(
                        cx.listener(move |panel, _, _, cx| panel.trust_cloud_pool(id, cx)),
                    ),
                )
                .child(
                    button("Do not trust", "cloud-refuse", cx).on_click(
                        cx.listener(move |panel, _, _, cx| panel.refuse_cloud_pool(id, cx)),
                    ),
                )
                .into_any_element(),
            StartPhase::Ready(_) => frame
                .child(text(format!("{name}: {}", nocterm_cloud_agents_model::StartStep::Ready.text()), cx))
                .child(
                    button("Open chat", "cloud-open-chat", cx).on_click(
                        cx.listener(move |panel, _, window, cx| panel.open_started_chat(id, window, cx)),
                    ),
                )
                .child(
                    button("Dismiss", "cloud-dismiss", cx)
                        .on_click(cx.listener(move |panel, _, _, cx| panel.dismiss_cloud_start(id, cx))),
                )
                .into_any_element(),
            StartPhase::Failed(error) => frame
                .child(text(format!("{name}: {}", failure_text(&error)), cx))
                .child(
                    button("Dismiss", "cloud-dismiss", cx)
                        .on_click(cx.listener(move |panel, _, _, cx| panel.dismiss_cloud_start(id, cx))),
                )
                .into_any_element(),
        }
    }

    fn render_cloud_footer(
        &mut self,
        access: &CloudAgentsAccess,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let hubs = access.visible_hubs(cx);
        let Some((hub, ..)) = hubs.first().cloned() else {
            return Vec::new();
        };
        let manage = menu_row("cloud-manage-agents", "Manage agents…", cx)
            .tooltip("Opens the web panel of the server")
            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&web_panel_url(&hub))));
        let create = menu_row("cloud-new-personal-agent", "New personal agent…", cx)
            .on_click(cx.listener(|panel, _, window, cx| panel.new_personal_agent(window, cx)));
        vec![manage.into_any_element(), create.into_any_element()]
    }

    fn start_cloud_agent(
        &mut self,
        hub: &HubId,
        agent: &CloudAgent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(access) = &self.cloud.access else {
            return;
        };
        let id = access
            .service()
            .update(cx, |service, cx| service.start(hub, agent, cx));
        self.began_cloud_start(id, window, cx);
    }

    fn trust_cloud_pool(&mut self, id: StartId, cx: &mut Context<Self>) {
        if let Some(access) = &self.cloud.access {
            access
                .service()
                .update(cx, |service, cx| service.trust_pool(id, cx));
        }
    }

    fn refuse_cloud_pool(&mut self, id: StartId, cx: &mut Context<Self>) {
        if let Some(access) = &self.cloud.access {
            access
                .service()
                .update(cx, |service, cx| service.refuse_pool(id, cx));
        }
    }

    fn dismiss_cloud_start(&mut self, id: StartId, cx: &mut Context<Self>) {
        if let Some(access) = &self.cloud.access {
            access
                .service()
                .update(cx, |service, cx| service.dismiss(id, cx));
        }
    }

    /// Opens the Servers page of the settings, where personal agents are created and edited.
    fn new_personal_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menus.reset();
        if let Ok(action) = cx.build_action("hubs::ManageServers", None) {
            window.dispatch_action(action, cx);
        }
        cx.notify();
    }
}
