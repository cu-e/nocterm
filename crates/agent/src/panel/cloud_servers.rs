//! The Servers control of a Cloud chat (ADR 0147): the servers the chat is bound to and where
//! each stands, the server the next prompts run on, and binding the chat to CA-trusted servers
//! for a window of one to thirty days (seven unless the person picks another).
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, ElementId, Entity, Subscription, Task,
    TestSupportExt as _, Window,
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

mod setup;
pub(super) use setup::SetupAccess;
use setup::{CloudServerSetup, SetupChanged};

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
    listing: Option<Task<()>>,
    setup: Option<Entity<CloudServerSetup>>,
    _setup_changed: Option<Subscription>,
}
impl CloudServersControl {
    pub(super) fn new(
        servers: Rc<dyn ConversationServers>,
        access: Option<SetupAccess>,
        cx: &mut Context<Self>,
    ) -> Self {
        let setup = access.map(|access| cx.new(|cx| CloudServerSetup::new(access, cx)));
        let changed = setup.as_ref().map(|setup| {
            cx.subscribe(setup, |control, _, event: &SetupChanged, cx| match event {
                SetupChanged::Access if control.form.is_some() => control.reload_options(cx),
                _ => cx.notify(),
            })
        });
        Self {
            servers,
            form: None,
            chosen: BTreeSet::new(),
            days: DEFAULT_DAYS,
            working: None,
            error: None,
            listing: None,
            setup,
            _setup_changed: changed,
        }
    }
    pub(super) fn toggle_form(&mut self, cx: &mut Context<Self>) {
        if self.working.is_some() {
            return;
        }
        if self.form.is_some() {
            self.listing = None;
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
        self.error = None;
        if let Some(setup) = &self.setup {
            setup.update(cx, |setup, cx| setup.reload(cx));
        }
        self.reload_options(cx);
    }
    fn reload_options(&mut self, cx: &mut Context<Self>) {
        let listing = self.servers.options(cx);
        self.listing = Some(cx.spawn(async move |this, cx| {
            let listed = listing.await;
            let _ = this.update(cx, |control, cx| {
                control.listing = None;
                if control.form.is_none() {
                    return;
                }
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
                        control.chosen.clear();
                        control.form = control.setup.is_some().then(|| Some(Vec::new()));
                        control.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn choose(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        if self.working.is_some() {
            return;
        }
        if on {
            self.chosen.insert(id.to_owned());
        } else {
            self.chosen.remove(id);
        }
        cx.notify();
    }
    fn bind(&mut self, cx: &mut Context<Self>) {
        if self.working.is_some() || self.listing.is_some() || self.chosen.is_empty() {
            return;
        }
        let ids: Vec<String> = self.chosen.iter().cloned().collect();
        let window = Duration::from_secs(self.days * 86_400);
        let binding = self.servers.bind(ids, window, cx);
        self.finish(binding, true, cx);
    }
    fn unbind(&mut self, cx: &mut Context<Self>) {
        if self.working.is_some() {
            return;
        }
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
                control.working = None;
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
    fn binding_window(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let windows = WINDOWS.into_iter().map(|days| {
            Button::new(ElementId::Name(format!("cloud-servers-days-{days}").into()))
                .small()
                .label(if days == 1 {
                    "1 day".to_owned()
                } else {
                    format!("{days} days")
                })
                .selected(self.days == days)
                .disabled(self.working.is_some())
                .on_click(cx.listener(move |control, _: &ClickEvent, _, cx| {
                    if control.working.is_some() {
                        return;
                    }
                    control.days = days;
                    cx.notify();
                }))
        });
        h_flex()
            .id("cloud-servers-actions")
            .test_support()
            .flex_shrink_0()
            .gap_1()
            .flex_wrap()
            .child(div().text_xs().child("For"))
            .children(windows)
            .child(
                Button::new("cloud-servers-bind")
                    .small()
                    .primary()
                    .label("Bind")
                    .disabled(
                        self.chosen.is_empty() || self.working.is_some() || self.listing.is_some(),
                    )
                    .on_click(cx.listener(|control, _: &ClickEvent, _, cx| control.bind(cx))),
            )
            .into_any_element()
    }
    fn form(
        &self,
        options: &[ServerOption],
        body_height: gpui_kit::Pixels,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
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
                        .disabled(!option.bindable || self.working.is_some())
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
        let content = v_flex()
            .id("cloud-servers-options")
            .test_support()
            .min_h_0()
            .max_h(body_height)
            .overflow_y_scroll()
            .gap_2()
            .when(options.is_empty() && self.setup.is_none(), |form| {
                form.child(
                    div()
                        .text_xs()
                        .child("No trusted servers are available for this chat."),
                )
            })
            .child(v_flex().gap_2().children(rows))
            .children(self.setup.clone());
        v_flex()
            .id("cloud-servers-form")
            .test_support()
            .min_h_0()
            .gap_2()
            .child(content)
            .child(self.binding_window(cx))
            .into_any_element()
    }
}
impl Render for CloudServersControl {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bound = self.bound_rows(cx);
        let summary = match self.servers.expires_at_ms(cx) {
            Some(until) if !bound.is_empty() => format!("Servers · bound until {}", utc(until)),
            _ => "Servers · not bound".to_owned(),
        };
        let is_bound = !bound.is_empty();
        let targets = self.targets(cx);
        let form = match &self.form {
            Some(Some(options)) => {
                let height = (window.viewport_size().height / 3.).min(gpui_kit::px(240.));
                Some(self.form(options, height, cx))
            }
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
        let setup_notice = self
            .setup
            .as_ref()
            .and_then(|setup| setup.read(cx).notice(cx));
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
                    .disabled(self.working.is_some())
                    .label("Bind servers…")
                    .on_click(
                        cx.listener(|control, _: &ClickEvent, _, cx| control.toggle_form(cx)),
                    ),
            )
            .when(is_bound, |row| {
                row.child(
                    Button::new("cloud-servers-unbind")
                        .small()
                        .disabled(self.working.is_some())
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
            .children(setup_notice)
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
    setup: Option<SetupAccess>,
    cx: &mut App,
) -> gpui_kit::Entity<CloudServersControl> {
    cx.new(|cx| CloudServersControl::new(servers, setup, cx))
}
