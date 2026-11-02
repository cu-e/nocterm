//! Stopping and removing the person's seat of an agent, from the agent's row.

use gpui_kit::{
    AnyElement, Context, SharedString,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
    },
    div,
    prelude::*,
};
use nocterm_cloud_agents_model::{AgentId, CloudAgent, Placement};
use nocterm_hubs_model::HubId;

use super::CloudMenu;
use crate::panel::AgentPanel;

/// The seat whose removal the person is asked to confirm.
pub(super) type Removing = (HubId, AgentId);

impl CloudMenu {
    /// Stop, and Remove after a confirmation, under the row of an agent the person has a seat
    /// of. A seat on the person's own runner lives on this device, not on a host, so it has
    /// neither.
    pub(super) fn render_seat_actions(
        &self,
        hub: &HubId,
        agent: &CloudAgent,
        cx: &mut Context<AgentPanel>,
    ) -> Option<AnyElement> {
        let access = self.access.as_ref()?;
        if agent.placement() != Placement::Pool
            || !access.service().read(cx).has_seat(hub, &agent.id)
        {
            return None;
        }
        let key = format!("{}-{}", hub.address(), agent.id);
        let small = |label: &'static str, name: &'static str| {
            Button::new(SharedString::from(format!("{name}-{key}")))
                .ghost()
                .xsmall()
                .label(label)
        };
        let chosen = (hub.clone(), agent.id.clone());
        let mut actions = h_flex().px_2().gap_1().items_center().flex_wrap();
        if self.removing.as_ref() == Some(&chosen) {
            let to_remove = chosen.clone();
            actions = actions
                .child(
                    div()
                        .min_w_0()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Remove the instance and its files?"),
                )
                .child(
                    small("Remove", "cloud-remove-confirm").on_click(cx.listener(
                        move |panel, _, _, cx| {
                            panel.cloud.removing = None;
                            panel.cloud.release_seat(&to_remove, true, cx);
                        },
                    )),
                )
                .child(small("Keep", "cloud-remove-keep").on_click(cx.listener(
                    |panel, _, _, cx| {
                        panel.cloud.removing = None;
                        cx.notify();
                    },
                )));
        } else {
            let to_stop = chosen.clone();
            actions = actions
                .child(
                    small("Stop", "cloud-stop")
                        .tooltip(
                            "Stops the agent's runner; its files stay and starting it again resumes it",
                        )
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.cloud.release_seat(&to_stop, false, cx);
                        })),
                )
                .child(
                    small("Remove instance…", "cloud-remove")
                        .tooltip("Deletes the agent's runner with its files on the server")
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.cloud.removing = Some(chosen.clone());
                            cx.notify();
                        })),
                );
        }
        Some(actions.into_any_element())
    }

    /// Stops (or removes) the seat of an agent; a refusal shows as the tab's notice.
    fn release_seat(&mut self, seat: &Removing, remove: bool, cx: &mut Context<AgentPanel>) {
        let Some(access) = &self.access else {
            return;
        };
        let (hub, agent) = seat;
        let task = access.service().update(cx, |service, cx| {
            if remove {
                service.remove(hub, agent, cx)
            } else {
                service.stop(hub, agent, cx)
            }
        });
        cx.spawn(async move |panel, cx| {
            if let Err(error) = task.await {
                panel
                    .update(cx, |panel, cx| {
                        panel.cloud.show_notice(error.to_string());
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();
    }

    /// Forgets a removal question whose seat is gone.
    pub(super) fn drop_stale_removal(&mut self, cx: &gpui_kit::App) {
        let Some(access) = &self.access else {
            return;
        };
        let service = access.service().read(cx);
        if self
            .removing
            .as_ref()
            .is_some_and(|(hub, agent)| !service.has_seat(hub, agent))
        {
            self.removing = None;
        }
    }
}
