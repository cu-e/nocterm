//! The starts of Cloud agents in the Cloud tab. A start shows as the status of its agent's row;
//! only a question or a failure takes a line of its own, under the row or above the sections
//! when the row is not shown.

use gpui_kit::{
    AnyElement, App, Context, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_cloud_agents::{StartId, StartPhase, StartView};
use nocterm_cloud_agents_model::{AgentId, CloudAgentError, Placement};
use nocterm_hubs_model::HubId;

use super::CloudMenu;
use crate::panel::{AgentPanel, widgets::menu_row};

/// One start as the tab shows it.
#[derive(Clone)]
pub(super) struct StartEntry {
    id: StartId,
    hub: HubId,
    agent: AgentId,
    name: String,
    phase: StartPhase,
}

impl StartEntry {
    pub(super) fn of(start: StartView<'_>) -> Self {
        Self {
            id: start.id(),
            hub: start.hub().clone(),
            agent: start.agent().clone(),
            name: start.name().to_owned(),
            phase: start.phase().clone(),
        }
    }

    pub(super) fn phase(&self) -> &StartPhase {
        &self.phase
    }

    pub(super) fn is_of(&self, hub: &HubId, agent: &AgentId) -> bool {
        &self.hub == hub && &self.agent == agent
    }
}

/// Whether starting again could end differently without the person changing anything else.
fn retryable(error: &CloudAgentError) -> bool {
    !matches!(
        error,
        CloudAgentError::PoolKeyChanged { .. }
            | CloudAgentError::Unsupported
            | CloudAgentError::PoolNotTrusted
    )
}

impl CloudMenu {
    /// What a start adds under its agent's row (`placed`) or above the sections: the progress of
    /// a start without a row, the pool-trust question, a failure. A running or ready start is
    /// the status of its row and needs no box of its own.
    pub(super) fn render_start(
        &self,
        entry: &StartEntry,
        placed: bool,
        cx: &mut Context<AgentPanel>,
    ) -> Option<AnyElement> {
        let StartEntry {
            id, name, phase, ..
        } = entry.clone();
        let frame = v_flex()
            .id(SharedString::from(format!("cloud-start-{id:?}")))
            .test_support()
            .gap_1()
            .px_2()
            .py_1();
        let button = |label: &'static str, key: &'static str, cx: &mut Context<AgentPanel>| {
            menu_row(SharedString::from(format!("{key}-{id:?}")), label, cx)
        };
        match phase {
            StartPhase::Step(step) => (!placed).then(|| {
                frame
                    .child(text(format!("{name}: {}", step.text()), cx))
                    .into_any_element()
            }),
            StartPhase::AwaitingTrust(offer) => Some(
                frame
                    .rounded_md()
                    .bg(cx.theme().secondary)
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
                            cx.listener(move |panel, _, _, cx| panel.cloud.trust_pool(id, cx)),
                        ),
                    )
                    .child(
                        button("Do not trust", "cloud-refuse", cx).on_click(
                            cx.listener(move |panel, _, _, cx| panel.cloud.refuse_pool(id, cx)),
                        ),
                    )
                    .into_any_element(),
            ),
            StartPhase::Ready(_) => None,
            StartPhase::Failed(error) => Some(
                frame
                    .child(self.render_failure(id, &name, &error, cx))
                    .into_any_element(),
            ),
        }
    }

    /// Dismisses the starts that ended without a seat: the person has seen them.
    pub(super) fn dismiss_failed_starts(&self, cx: &mut Context<AgentPanel>) {
        let Some(access) = &self.access else {
            return;
        };
        let failed: Vec<StartId> = access
            .service()
            .read(cx)
            .starts()
            .filter(|start| matches!(start.phase(), StartPhase::Failed(_)))
            .map(|start| start.id())
            .collect();
        for id in failed {
            self.dismiss_start(id, cx);
        }
    }

    /// Dismisses the ready starts whose seat is gone (removed elsewhere): opening their chat
    /// would only fail.
    pub(super) fn drop_orphaned_starts(&self, cx: &mut Context<AgentPanel>) {
        let Some(access) = &self.access else {
            return;
        };
        let service = access.service().read(cx);
        let orphans: Vec<StartId> = service
            .starts()
            .filter(|start| {
                matches!(start.phase(), StartPhase::Ready(_))
                    && start.placement() == Placement::Pool
                    && !service.has_seat(start.hub(), start.agent())
            })
            .map(|start| start.id())
            .collect();
        for id in orphans {
            self.dismiss_start(id, cx);
        }
    }

    /// One line of text with small Retry and Dismiss buttons beside it.
    fn render_failure(
        &self,
        id: StartId,
        name: &str,
        error: &CloudAgentError,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let small = |label: &'static str, key: &'static str| {
            Button::new(SharedString::from(format!("{key}-{id:?}")))
                .ghost()
                .xsmall()
                .label(label)
        };
        let mut actions = h_flex().flex_none().gap_1();
        if retryable(error) {
            actions = actions.child(small("Retry", "cloud-retry").on_click(
                cx.listener(move |panel, _, window, cx| panel.cloud.retry_start(id, window, cx)),
            ));
        }
        actions = actions.child(
            small("Dismiss", "cloud-dismiss")
                .on_click(cx.listener(move |panel, _, _, cx| panel.cloud.dismiss_start(id, cx))),
        );
        h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(format!("{name}: {error}")),
            )
            .child(actions)
            .into_any_element()
    }

    pub(super) fn retry_start(
        &mut self,
        id: StartId,
        window: &mut gpui_kit::Window,
        cx: &mut Context<AgentPanel>,
    ) {
        let Some(access) = &self.access else {
            return;
        };
        if let Some(fresh) = access
            .service()
            .update(cx, |service, cx| service.retry(id, cx))
        {
            self.began_start(fresh, window, cx);
        }
    }
}

fn text(text: String, cx: &App) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(cx.theme().foreground)
        .child(text)
}
