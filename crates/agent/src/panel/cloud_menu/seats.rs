//! Stopping and removing the person's seat of an agent, from the context menu of the agent's
//! row.

use gpui_kit::{
    AnyElement, Context, SharedString, Window,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        menu::{PopupMenu, PopupMenuItem},
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
    /// Whether the person has a seat of `agent` on a host, which Stop and Remove concern. A
    /// seat on the person's own runner lives on this device, not on a host, so it has neither.
    pub(super) fn seat_of(
        &self,
        hub: &HubId,
        agent: &CloudAgent,
        cx: &Context<AgentPanel>,
    ) -> bool {
        agent.placement() == Placement::Pool
            && self
                .access
                .as_ref()
                .is_some_and(|access| access.service().read(cx).has_seat(hub, &agent.id))
    }

    /// The right-click menu of an agent row the person has a seat of: Stop, and Remove, which
    /// asks first under the row.
    pub(super) fn seat_menu(
        &self,
        hub: &HubId,
        agent: &CloudAgent,
        cx: &mut Context<AgentPanel>,
    ) -> Option<impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static>
    {
        if !self.seat_of(hub, agent, cx) {
            return None;
        }
        let seat: Removing = (hub.clone(), agent.id.clone());
        let panel = cx.entity().downgrade();
        Some(
            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let (stop_panel, remove_panel) = (panel.clone(), panel.clone());
                let (stop_seat, remove_seat) = (seat.clone(), seat.clone());
                menu.item(PopupMenuItem::new("Stop").on_click(move |_, _, cx| {
                    let _ = stop_panel.update(cx, |panel, cx| {
                        panel.cloud.release_seat(&stop_seat, false, cx);
                    });
                }))
                .item(
                    PopupMenuItem::new("Remove instance…").on_click(move |_, _, cx| {
                        let _ = remove_panel.update(cx, |panel, cx| {
                            panel.cloud.removing = Some(remove_seat.clone());
                            cx.notify();
                        });
                    }),
                )
            },
        )
    }

    /// The question under the row of the seat the person is removing.
    pub(super) fn render_removal_question(
        &self,
        hub: &HubId,
        agent: &CloudAgent,
        cx: &mut Context<AgentPanel>,
    ) -> Option<AnyElement> {
        let chosen = (hub.clone(), agent.id.clone());
        if self.removing.as_ref() != Some(&chosen) {
            return None;
        }
        let key = format!("{}-{}", hub.address(), agent.id);
        let small = |label: &'static str, name: &'static str| {
            Button::new(SharedString::from(format!("{name}-{key}")))
                .ghost()
                .xsmall()
                .label(label)
        };
        Some(
            h_flex()
                .px_2()
                .gap_1()
                .items_center()
                .flex_wrap()
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
                            panel.cloud.release_seat(&chosen, true, cx);
                        },
                    )),
                )
                .child(small("Keep", "cloud-remove-keep").on_click(cx.listener(
                    |panel, _, _, cx| {
                        panel.cloud.removing = None;
                        cx.notify();
                    },
                )))
                .into_any_element(),
        )
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
