//! The Cloud tab of the new-chat popover: the Cloud agents of the signed-in hubs, a search
//! field, the progress of a start and the pool-trust question.
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, SharedString, Subscription,
    TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
        input::{InputEvent, InputState},
        menu::ContextMenuExt as _,
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_cloud_agents::{CloudAgentsEvent, StartId};
use nocterm_cloud_agents_model::{AgentScope, CloudAgent, Section};
use nocterm_hubs_model::{HubColor, HubId};
use seats::Removing;

use super::{
    AgentPanel,
    widgets::{menu_row, menu_row_with},
};
use crate::cloud_access::CloudAgentsAccess;

mod chats;
mod seats;
mod starts;
mod status;

pub(super) use chats::notice_text;
use starts::StartEntry;

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
    /// The seat the person is asked to confirm removing.
    removing: Option<Removing>,
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
                        panel.cloud.start_ready(*id, window, cx);
                        cx.notify();
                    }
                    CloudAgentsEvent::Changed => {
                        panel.cloud.drop_stale_removal(cx);
                        panel.cloud.drop_orphaned_starts(cx);
                        cx.notify();
                    }
                },
            ));
            subscriptions.push(cx.observe(access.hubs(), |_, _, cx| cx.notify()));
        }
        Self {
            access,
            tab: MenuTab::Local,
            search,
            chats: chats::CloudChats::default(),
            removing: None,
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
        self.forget_transient();
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }
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

impl CloudMenu {
    /// The tab switch at the top of the "New agent" popover, or nothing when Cloud is not
    /// offered.
    pub(super) fn render_menu_tabs(&self, cx: &mut Context<AgentPanel>) -> Option<AnyElement> {
        if !self.offered(cx) {
            return None;
        }
        let current = self.tab(cx);
        let tab =
            |id: &'static str, label: &'static str, tab: MenuTab, cx: &mut Context<AgentPanel>| {
                div().flex_1().min_w_0().child(
                    menu_row(id, label, cx)
                        .selected(current == tab)
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.cloud.select(tab);
                            panel.cloud.refresh_chats(cx);
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
    pub(super) fn render_tab(&self, cx: &mut Context<AgentPanel>) -> Vec<AnyElement> {
        let Some(access) = self.access.clone() else {
            return Vec::new();
        };
        let mut rows = vec![
            gpui_kit::component::input::Input::new(&self.search)
                .small()
                .into_any_element(),
        ];
        let query = self.query(cx);
        let sections = access.sections(&query, cx);
        let mut starts = self.current_starts(&access, cx);
        // A start sits under the row of its agent; only those without a shown row go on top.
        let (under_rows, unplaced): (Vec<_>, Vec<_>) = starts.drain(..).partition(|start| {
            sections.iter().any(|section| {
                section
                    .agents
                    .iter()
                    .any(|agent| start.is_of(&section.hub, &agent.id))
            })
        });
        rows.extend(
            unplaced
                .iter()
                .filter_map(|start| self.render_start(start, false, cx)),
        );
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
            rows.push(self.render_section(&section, several_hubs, &under_rows, cx));
        }
        rows.extend(self.render_chats(&access, cx));
        rows.extend(self.render_footer(&access, cx));
        rows
    }

    fn render_section(
        &self,
        section: &Section,
        several_hubs: bool,
        starts: &[StartEntry],
        cx: &mut Context<AgentPanel>,
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
            let start = starts
                .iter()
                .find(|start| start.is_of(&section.hub, &agent.id));
            block = block
                .child(self.render_agent_row(&section.hub, &section.hub_name, agent, start, cx))
                .children(self.render_removal_question(&section.hub, agent, cx))
                .children(start.and_then(|start| self.render_start(start, true, cx)));
        }
        block.into_any_element()
    }

    fn render_agent_row(
        &self,
        hub: &HubId,
        hub_name: &str,
        agent: &CloudAgent,
        start: Option<&StartEntry>,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let id = format!("cloud-agent-{}-{}", hub.address(), agent.id);
        let startable = agent.status.can_start();
        let (hub_id, chosen) = (hub.clone(), agent.clone());
        let model = agent.primary_model().unwrap_or_default().to_owned();
        let seated = self.seat_of(hub, agent, cx);
        let status = status::row_status(agent.status, seated, start.map(StartEntry::phase));
        let mut row = menu_row_with(
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
        );
        if let Some(status) = status {
            let color = if status.attention {
                cx.theme().warning
            } else {
                cx.theme().muted_foreground
            };
            row = row.child(
                div()
                    .id(SharedString::from(format!(
                        "cloud-agent-status-{}-{}",
                        hub.address(),
                        agent.id
                    )))
                    .test_support()
                    .flex_none()
                    .text_xs()
                    .text_color(color)
                    .child(status.text),
            );
        }
        let row = row
            .disabled(!startable)
            .on_click(cx.listener(move |panel, _, window, cx| {
                panel.cloud.activate_agent(&hub_id, &chosen, window, cx);
            }));
        match self.seat_menu(hub, agent, cx) {
            Some(menu) => row.context_menu(menu).into_any_element(),
            None => row.into_any_element(),
        }
    }

    fn current_starts(
        &self,
        access: &CloudAgentsAccess,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<StartEntry> {
        let visible: Vec<HubId> = access
            .visible_hubs(cx)
            .into_iter()
            .map(|(hub, ..)| hub)
            .collect();
        access
            .service()
            .read(cx)
            .starts()
            .filter(|start| visible.contains(start.hub()))
            .map(StartEntry::of)
            .collect()
    }

    fn render_footer(
        &self,
        access: &CloudAgentsAccess,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<AnyElement> {
        let hubs = access.visible_hubs(cx);
        let Some((hub, ..)) = hubs.first().cloned() else {
            return Vec::new();
        };
        let url = access.service().read(cx).web_panel_url(&hub);
        let manage = menu_row("cloud-manage-agents", "Manage agents…", cx)
            .tooltip("Opens the web panel of the server")
            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)));
        let create = menu_row("cloud-new-personal-agent", "New personal agent…", cx)
            .on_click(cx.listener(|panel, _, window, cx| panel.new_personal_agent(window, cx)));
        vec![manage.into_any_element(), create.into_any_element()]
    }

    pub(super) fn start_agent(
        &mut self,
        hub: &HubId,
        agent: &CloudAgent,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        let Some(access) = &self.access else {
            return;
        };
        let id = access
            .service()
            .update(cx, |service, cx| service.start(hub, agent, cx));
        self.began_start(id, window, cx);
    }

    fn trust_pool(&self, id: StartId, cx: &mut Context<AgentPanel>) {
        if let Some(access) = &self.access {
            access
                .service()
                .update(cx, |service, cx| service.trust_pool(id, cx));
        }
    }

    fn refuse_pool(&self, id: StartId, cx: &mut Context<AgentPanel>) {
        if let Some(access) = &self.access {
            access
                .service()
                .update(cx, |service, cx| service.refuse_pool(id, cx));
        }
    }

    fn dismiss_start(&self, id: StartId, cx: &mut Context<AgentPanel>) {
        if let Some(access) = &self.access {
            access
                .service()
                .update(cx, |service, cx| service.dismiss(id, cx));
        }
    }
}
