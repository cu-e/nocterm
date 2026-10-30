//! The Devices tab of a server: the devices signed in to the account, and revoking them.
//!
//! The list is read from the hub through the service whenever the tab is shown for a server,
//! and again after a device was revoked. Revoking this device is signing out of the server.

use gpui_kit::{
    App, ClickEvent, Context, Entity, Task, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hubs::Hubs;
use nocterm_hubs_model::{DeviceSummary, HubError, HubId};

use crate::layout::{note, problem};
use crate::manage;
use crate::messages::hub_problem;
use crate::servers::item;

/// What the list of devices shows.
enum Load {
    Loading,
    Ready(Vec<DeviceSummary>),
    Failed(HubError),
}

/// The devices of the account on the server that is shown.
pub(in crate::servers) struct Devices {
    hubs: Entity<Hubs>,
    hub: Option<HubId>,
    load: Load,
    loading: Option<Task<()>>,
}

impl Devices {
    pub(in crate::servers) fn new(hubs: Entity<Hubs>) -> Self {
        Self {
            hubs,
            hub: None,
            load: Load::Loading,
            loading: None,
        }
    }

    /// Shows the devices of `hub`, reading them when another server was shown before. Showing
    /// the same server again changes nothing.
    pub(in crate::servers) fn show(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self.hub.as_ref() == Some(hub) {
            return;
        }
        self.hub = Some(hub.clone());
        self.reload(cx);
    }

    /// Forgets what was read: the next [`Devices::show`] reads again.
    pub(in crate::servers) fn hide(&mut self) {
        self.hub = None;
        self.loading = None;
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        self.load = Load::Loading;
        let listing = self.hubs.read(cx).list_devices(&hub);
        self.loading = Some(cx.spawn(async move |this, cx| {
            let result = listing.await;
            this.update(cx, |devices, cx| {
                if devices.hub.as_ref() != Some(&hub) {
                    return;
                }
                devices.load = match result {
                    Ok(listed) => Load::Ready(sorted(listed)),
                    Err(error) => Load::Failed(error),
                };
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Revokes `device`; this device signs out instead. A failure is reported, and the list is
    /// read again either way.
    fn revoke(&mut self, device: &DeviceSummary, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        if device.is_current {
            manage::sign_out(&self.hubs, &hub, window, cx);
            return;
        }
        let revoking = self.hubs.read(cx).revoke_device(&hub, &device.id);
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = revoking.await;
                cx.update(|window, cx| {
                    if let Err(error) = &result {
                        nocterm_ui::notice::error(
                            window,
                            cx,
                            "hub-devices",
                            "Could not revoke the device",
                            hub_problem(error),
                        );
                    }
                    let _ = view.update(cx, |devices, cx| devices.reload(cx));
                })
                .ok();
            })
            .detach();
    }

    /// Asks before revoking: the device is signed out and has to sign in again.
    fn confirm_revoke(
        view: Entity<Self>,
        device: DeviceSummary,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (view, chosen) = (view.clone(), device.clone());
            let confirm = Button::new("revoke-device-confirm")
                .danger()
                .label("Revoke")
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                    view.update(cx, |devices, cx| devices.revoke(&chosen, window, cx));
                });
            let keep = Button::new("revoke-device-keep")
                .primary()
                .label("Keep it")
                .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                });
            let consequence = if device.is_current {
                "This is this device: revoking it signs you out of the server."
            } else {
                "It is signed out of the account and has to sign in again."
            };
            alert
                .title(format!("Revoke {}?", device.name))
                .description(consequence)
                .on_ok(|_, _, _| true)
                .footer(DialogFooter::new().child(confirm).child(keep))
        });
    }
}

/// This device first, then the others by when they were last seen, newest first.
fn sorted(mut devices: Vec<DeviceSummary>) -> Vec<DeviceSummary> {
    devices.sort_by_key(|device| {
        (
            !device.is_current,
            std::cmp::Reverse(device.last_seen_at_ms),
        )
    });
    devices
}

impl Render for Devices {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let body = match &self.load {
            Load::Loading => div()
                .id("devices-loading")
                .test_support()
                .child(note("Reading the devices of your account…", cx))
                .into_any_element(),
            Load::Failed(error) => self.failed(error, cx),
            Load::Ready(devices) if devices.is_empty() => div()
                .id("devices-empty")
                .test_support()
                .child(note("The server lists no devices.", cx))
                .into_any_element(),
            Load::Ready(devices) => v_flex()
                .gap_2()
                .children(devices.iter().map(|device| row(device, &view, cx)))
                .into_any_element(),
        };
        v_flex()
            .id("devices-tab")
            .test_support()
            .gap_3()
            .child(note(
                "The devices signed in to your account on this server. Revoking one signs it out.",
                cx,
            ))
            .child(body)
    }
}

impl Devices {
    fn failed(&self, error: &HubError, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let view = cx.entity();
        v_flex()
            .id("devices-error")
            .test_support()
            .gap_2()
            .child(problem(hub_problem(error), cx))
            .child(
                h_flex().child(Button::new("devices-retry").label("Try again").on_click(
                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        view.update(cx, |devices, cx| devices.reload(cx));
                    },
                )),
            )
            .into_any_element()
    }
}

/// One device: its name, kind and last sign-in, and the button that revokes it.
fn row(device: &DeviceSummary, view: &Entity<Devices>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (view, chosen) = (view.clone(), device.clone());
    let seen = if device.last_seen_at_ms > 0 {
        format!("Last seen {}", utc(device.last_seen_at_ms))
    } else {
        "Never seen".to_owned()
    };
    h_flex()
        .id(item("device", &device.id))
        .test_support()
        .gap_3()
        .p_3()
        .items_center()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().font_semibold().truncate().child(device.name.clone()))
                        .when(device.is_current, |line| {
                            line.child(
                                div()
                                    .id(item("device-current", &device.id))
                                    .test_support()
                                    .text_xs()
                                    .text_color(theme.success)
                                    .child("This device"),
                            )
                        }),
                )
                .child(
                    div()
                        .id(item("device-seen", &device.id))
                        .test_support()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("{} · {seen}", kind(&device.platform))),
                ),
        )
        .child(
            Button::new(item("device-revoke", &device.id))
                .danger()
                .small()
                .label("Revoke")
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    Devices::confirm_revoke(view.clone(), chosen.clone(), window, cx);
                }),
        )
}

/// The kind of client as a person reads it; an empty report is an unknown kind.
fn kind(platform: &str) -> String {
    let mut chars = platform.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Unknown kind".to_owned(),
    }
}

/// A moment as `2023-11-14 22:13 UTC`, so a list reads the same wherever it is shown.
pub(in crate::servers) fn utc(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3_600,
        rest % 3_600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moment_reads_as_a_utc_date_and_time() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(1_700_000_000_000), "2023-11-14 22:13 UTC");
        assert_eq!(utc(951_782_400_000), "2000-02-29 00:00 UTC");
    }

    #[test]
    fn this_device_is_listed_first_and_the_others_by_recency() {
        let device = |id: &str, seen, current| DeviceSummary {
            id: id.into(),
            name: id.into(),
            platform: "desktop".into(),
            created_at_ms: 0,
            last_seen_at_ms: seen,
            is_current: current,
        };
        let order: Vec<_> = sorted(vec![
            device("old", 1, false),
            device("new", 9, false),
            device("here", 0, true),
        ])
        .into_iter()
        .map(|device| device.id)
        .collect();
        assert_eq!(order, ["here", "new", "old"]);
    }
}
