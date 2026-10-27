//! The Servers page of the settings: every known server with its name, colour, status and
//! certificate, and the ways to sign out of it or remove it.
//!
//! The page edits through the service and never decides who may be trusted: a certificate is
//! shown, never accepted, and the only way out of a changed one is to remove the server.

mod row;

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Subscription, Task,
    TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hubs::{HubStatus, Hubs};
use nocterm_hubs_model::HubId;
use nocterm_ui::{IconName, SettingsPage, SettingsPageSpec};

use crate::SignInToServer;
use crate::installation::Installed;
use crate::layout::note;

/// The Servers page of the settings, listed with the other pages by the application.
pub fn settings_page(installed: &Installed) -> SettingsPageSpec {
    let hubs = installed.hubs().clone();
    SettingsPageSpec::new("servers", "Servers", move |_, cx| {
        let hubs = hubs.clone();
        cx.new(|cx| ServersPage::new(hubs, cx))
    })
    .with_icon(IconName::Server)
}

/// How often the countdown of an offline server is redrawn.
const TICK: Duration = Duration::from_secs(1);

/// The name field of one server. A name is saved when Enter is pressed, when the field loses
/// focus and when the page is flushed.
struct NameField {
    input: Entity<InputState>,
    _events: Subscription,
}

/// The settings page that lists the known servers.
pub(crate) struct ServersPage {
    hubs: Entity<Hubs>,
    names: HashMap<HubId, NameField>,
    focus: FocusHandle,
    _redraw: Task<()>,
    _hubs: Subscription,
}

impl ServersPage {
    pub(crate) fn new(hubs: Entity<Hubs>, cx: &mut Context<Self>) -> Self {
        let redraw = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update(cx, |page, cx| {
                    if page.counts_down(cx) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        Self {
            names: HashMap::new(),
            focus: cx.focus_handle(),
            _redraw: redraw,
            _hubs: cx.observe(&hubs, |_, _, cx| cx.notify()),
            hubs,
        }
    }

    /// Whether a server shows a countdown that must be redrawn.
    fn counts_down(&self, cx: &App) -> bool {
        self.hubs
            .read(cx)
            .list()
            .iter()
            .any(|summary| matches!(summary.status, HubStatus::Offline { .. }))
    }

    /// Gives each known server a name field and drops the fields of forgotten ones. A field
    /// that exists keeps what the user is typing.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let known: Vec<HubId> = self
            .hubs
            .read(cx)
            .list()
            .into_iter()
            .map(|s| s.id)
            .collect();
        self.names.retain(|hub, _| known.contains(hub));
        for hub in known {
            if self.names.contains_key(&hub) {
                continue;
            }
            let (stored, address) = match self.hubs.read(cx).record(&hub) {
                Some(record) => (record.label.clone(), record.address.to_string()),
                None => continue,
            };
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(address)
                    .default_value(stored)
            });
            let id = hub.clone();
            let events = cx.subscribe_in(&input, window, move |page, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    page.save_name(&id, window, cx);
                }
            });
            self.names.insert(
                hub,
                NameField {
                    input,
                    _events: events,
                },
            );
        }
    }

    /// Saves the typed name when it differs from the stored one, then shows the stored name.
    fn save_name(&mut self, hub: &HubId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.names.get(hub) else {
            return;
        };
        let typed = field.input.read(cx).value().to_string();
        let stored = self
            .hubs
            .read(cx)
            .record(hub)
            .map(|record| record.label.clone());
        if stored
            .as_deref()
            .is_none_or(|stored| stored == typed.trim())
        {
            return;
        }
        let saving = self
            .hubs
            .update(cx, |hubs, cx| hubs.set_label(hub, &typed, cx));
        let id = hub.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = saving.await;
            this.update_in(cx, |page, window, cx| {
                if let Err(error) = result {
                    nocterm_ui::notice::error(
                        window,
                        cx,
                        "hub-manage",
                        "Could not rename the server",
                        crate::messages::hubs_problem(&error),
                    );
                }
                page.show_stored_name(&id, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Puts the stored name in the field, so it reads as the service cleaned it.
    fn show_stored_name(&mut self, hub: &HubId, window: &mut Window, cx: &mut Context<Self>) {
        let stored = self
            .hubs
            .read(cx)
            .record(hub)
            .map(|record| record.label.clone());
        if let (Some(field), Some(stored)) = (self.names.get(hub), stored) {
            field.input.update(cx, |input, cx| {
                if input.value().as_ref() != stored.as_str() {
                    input.set_value(stored, window, cx);
                }
            });
        }
    }
}

impl SettingsPage for ServersPage {
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hubs: Vec<HubId> = self.names.keys().cloned().collect();
        for hub in hubs {
            self.save_name(&hub, window, cx);
        }
    }
}

impl Focusable for ServersPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ServersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_fields(window, cx);
        let summaries = self.hubs.read(cx).list();
        let rows: Vec<_> = summaries
            .iter()
            .filter_map(|summary| {
                let record = self.hubs.read(cx).record(&summary.id)?.clone();
                let field = self.names.get(&summary.id)?.input.clone();
                Some(row::server_row(summary, &record, &field, &self.hubs, cx))
            })
            .collect();
        let empty = rows.is_empty();
        v_flex()
            .id("servers-page")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .p_4()
            .gap_4()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_lg().font_semibold().child("Servers"))
                            .child(note(
                                "Nocterm hubs this device is signed in to. Credentials stay in a private file on this device.",
                                cx,
                            )),
                    )
                    .child(
                        Button::new("servers-add")
                            .primary()
                            .label("Sign in to a server…")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(SignInToServer), cx);
                            }),
                    ),
            )
            .when(empty, |page| {
                page.child(div().id("servers-empty").test_support().child(note(
                    "No servers yet. Sign in to one to open its terminal sessions from any computer.",
                    cx,
                )))
            })
            .children(rows)
            .text_color(cx.theme().foreground)
    }
}

#[cfg(test)]
#[path = "servers_tests.rs"]
mod tests;
