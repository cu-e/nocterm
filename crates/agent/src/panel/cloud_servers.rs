//! The Servers control of a Cloud chat (ADR 0139): the servers the chat is bound to and where
//! each stands, the server the next prompts run on, and binding the chat to CA-trusted servers
//! for a window of one to thirty days (seven unless the person picks another).
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, ElementId, Task, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_agent_chat::conversation::{BoundServerState, ConversationServers, ServerOption};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

/// The windows a person can pick, in days.
const WINDOWS: [u64; 4] = [1, 7, 14, 30];
const DEFAULT_DAYS: u64 = 7;

pub(super) fn item(kind: &str, id: &str) -> ElementId {
    ElementId::Name(format!("{kind}-{id}").into())
}

fn utc(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map_or_else(
        || "an unknown time".to_owned(),
        |at| at.format("%Y-%m-%d %H:%M UTC").to_string(),
    )
}

/// What a bound server's state says, and the slug of its element.
fn state_text(state: BoundServerState) -> (&'static str, String) {
    match state {
        BoundServerState::Pending => ("pending", "waiting for an unlocked device".to_owned()),
        BoundServerState::ActiveUntil(until) => ("active", format!("active until {}", utc(until))),
        BoundServerState::Unreachable => ("unreachable", "not reachable by this agent".to_owned()),
        BoundServerState::Refused => ("refused", "refused by the agent".to_owned()),
        BoundServerState::Expired => ("expired", "binding expired".to_owned()),
        BoundServerState::Revoked => ("revoked", "binding revoked".to_owned()),
    }
}

/// The control of one Cloud chat's servers.
pub(super) struct CloudServersControl {
    servers: Rc<dyn ConversationServers>,
    /// The bind form is open, with the servers it offers once listed.
    form: Option<Option<Vec<ServerOption>>>,
    chosen: BTreeSet<String>,
    days: u64,
    working: Option<Task<()>>,
    error: Option<String>,
}
impl CloudServersControl {
    pub(super) fn new(servers: Rc<dyn ConversationServers>) -> Self {
        Self {
            servers,
            form: None,
            chosen: BTreeSet::new(),
            days: DEFAULT_DAYS,
            working: None,
            error: None,
        }
    }
    fn open_form(&mut self, cx: &mut Context<Self>) {
        if self.form.is_some() {
            self.form = None;
            cx.notify();
            return;
        }
        self.form = Some(None);
        self.chosen = self
            .servers
            .bound(cx)
            .into_iter()
            .map(|server| server.id)
            .collect();
        self.days = DEFAULT_DAYS;
        let listing = self.servers.options(cx);
        self.working = Some(cx.spawn(async move |this, cx| {
            let listed = listing.await;
            let _ = this.update(cx, |control, cx| {
                match listed {
                    Ok(options) => {
                        control.chosen.retain(|id| {
                            options
                                .iter()
                                .any(|option| &option.id == id && option.bindable)
                        });
                        control.form = Some(Some(options));
                    }
                    Err(error) => {
                        control.form = None;
                        control.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn choose(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        if on {
            self.chosen.insert(id.to_owned());
        } else {
            self.chosen.remove(id);
        }
        cx.notify();
    }
    fn bind(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.chosen.iter().cloned().collect();
        let window = Duration::from_secs(self.days * 86_400);
        let binding = self.servers.bind(ids, window, cx);
        self.finish(binding, true, cx);
    }
    fn unbind(&mut self, cx: &mut Context<Self>) {
        let unbinding = self.servers.unbind(cx);
        self.finish(unbinding, false, cx);
    }
    fn finish(
        &mut self,
        work: Task<Result<(), nocterm_agent_chat::conversation::ConversationError>>,
        close_form: bool,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |control, cx| {
                match result {
                    Ok(()) if close_form => control.form = None,
                    Ok(()) => {}
                    Err(error) => control.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn pick(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.error = self
            .servers
            .set_target(id, cx)
            .err()
            .map(|error| error.to_string());
        cx.notify();
    }
    /// One badge per bound server with where it stands.
    fn bound_rows(&self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let theme = cx.theme();
        self.servers
            .bound(cx)
            .into_iter()
            .map(|server| {
                let (slug, text) = state_text(server.state);
                let colour = match server.state {
                    BoundServerState::ActiveUntil(_) => theme.success,
                    BoundServerState::Pending => theme.warning,
                    _ => theme.muted_foreground,
                };
                h_flex()
                    .id(item("cloud-server", &server.id))
                    .test_support()
                    .gap_1()
                    .child(div().text_xs().child(server.label))
                    .child(
                        div()
                            .id(item(&format!("cloud-server-{slug}"), &server.id))
                            .test_support()
                            .text_xs()
                            .text_color(colour)
                            .child(text),
                    )
                    .into_any_element()
            })
            .collect()
    }
    /// The server the next prompts run on: the default destination or a selectable server.
    fn targets(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let selectable: Vec<_> = self
            .servers
            .bound(cx)
            .into_iter()
            .filter(|server| server.selectable)
            .collect();
        if selectable.is_empty() {
            return None;
        }
        let target = self.servers.target(cx);
        let default = Button::new("cloud-target-default")
            .small()
            .label("Default")
            .selected(target.is_none())
            .on_click(cx.listener(|control, _: &ClickEvent, _, cx| control.pick(None, cx)));
        let chips = selectable.into_iter().map(|server| {
            let id = server.id.clone();
            Button::new(item("cloud-target", &server.id))
                .small()
                .label(server.label)
                .selected(target.as_deref() == Some(server.id.as_str()))
                .on_click(cx.listener(move |control, _: &ClickEvent, _, cx| {
                    control.pick(Some(id.clone()), cx);
                }))
        });
        Some(
            h_flex()
                .id("cloud-targets")
                .test_support()
                .gap_1()
                .flex_wrap()
                .child(div().text_xs().child("Prompts run on"))
                .child(default)
                .children(chips)
                .into_any_element(),
        )
    }
    fn form(&self, options: &[ServerOption], cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        let rows = options.iter().map(|option| {
            let id = option.id.clone();
            let note = if !option.bindable {
                Some("connect to it once to bind it")
            } else if option.needs_ca_update {
                Some("needs a CA update: connect to it once")
            } else {
                None
            };
            h_flex()
                .gap_2()
                .child(
                    Checkbox::new(item("cloud-server-option", &option.id))
                        .checked(self.chosen.contains(&option.id))
                        .disabled(!option.bindable)
                        .label(option.label.clone())
                        .on_click(cx.listener(move |control, on: &bool, _, cx| {
                            control.choose(&id, *on, cx);
                        })),
                )
                .children(note.map(|note| {
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(note)
                }))
        });
        let windows = WINDOWS.into_iter().map(|days| {
            Button::new(ElementId::Name(format!("cloud-servers-days-{days}").into()))
                .small()
                .label(if days == 1 {
                    "1 day".to_owned()
                } else {
                    format!("{days} days")
                })
                .selected(self.days == days)
                .on_click(cx.listener(move |control, _: &ClickEvent, _, cx| {
                    control.days = days;
                    cx.notify();
                }))
        });
        v_flex()
            .id("cloud-servers-form")
            .test_support()
            .gap_1()
            .when(options.is_empty(), |form| {
                form.child(
                    div()
                        .text_xs()
                        .child("No server trusts the account's agent CA yet."),
                )
            })
            .child(h_flex().gap_2().flex_wrap().children(rows))
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(div().text_xs().child("For"))
                    .children(windows)
                    .child(
                        Button::new("cloud-servers-bind")
                            .small()
                            .primary()
                            .label("Bind")
                            .disabled(self.chosen.is_empty())
                            .on_click(
                                cx.listener(|control, _: &ClickEvent, _, cx| control.bind(cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}
impl Render for CloudServersControl {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bound = self.bound_rows(cx);
        let summary = match self.servers.expires_at_ms(cx) {
            Some(until) if !bound.is_empty() => format!("Servers · bound until {}", utc(until)),
            _ => "Servers · not bound".to_owned(),
        };
        let is_bound = !bound.is_empty();
        let targets = self.targets(cx);
        let form = match &self.form {
            Some(Some(options)) => Some(self.form(&options.clone(), cx)),
            Some(None) => Some(
                div()
                    .text_xs()
                    .child("Listing trusted servers…")
                    .into_any_element(),
            ),
            None => None,
        };
        let error = self.error.clone().map(|error| {
            div()
                .id("cloud-servers-error")
                .test_support()
                .text_xs()
                .text_color(cx.theme().danger)
                .child(error)
        });
        let header = h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(summary),
            )
            .child(
                Button::new("cloud-servers-edit")
                    .small()
                    .label("Bind servers…")
                    .on_click(cx.listener(|control, _: &ClickEvent, _, cx| control.open_form(cx))),
            )
            .when(is_bound, |row| {
                row.child(
                    Button::new("cloud-servers-unbind")
                        .small()
                        .danger()
                        .label("Unbind")
                        .on_click(cx.listener(|control, _: &ClickEvent, _, cx| control.unbind(cx))),
                )
            });
        // The bar sits right above the composer; what it details grows upwards, and the form
        // replaces the details while it is open.
        let editing = form.is_some();
        v_flex()
            .id("cloud-servers")
            .test_support()
            .px_3()
            .pb_2()
            .gap_1()
            .children(error)
            .when(!editing && !bound.is_empty(), |control| {
                control.child(h_flex().gap_3().flex_wrap().children(bound))
            })
            .when(!editing, |control| control.children(targets))
            .children(form)
            .child(header)
    }
}

/// The control of `servers`, for a conversation surface.
pub(super) fn control(
    servers: Rc<dyn ConversationServers>,
    cx: &mut App,
) -> gpui_kit::Entity<CloudServersControl> {
    cx.new(|_| CloudServersControl::new(servers))
}
