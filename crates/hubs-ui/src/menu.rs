//! The environment menu in the title bar: this computer, every known server with its
//! status, and a way to sign in to another one.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    Anchor, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, Hsla, KeyDownEvent, SharedString, Subscription, Task, TestSupportExt as _, Window,
    base::actions::Confirm,
    component::{
        ActiveTheme as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        popover::Popover,
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_hubs::{HubSummary, Hubs, HubsEvent};
use nocterm_hubs_model::Environment;
use nocterm_ui::IconName;
use nocterm_workspace::Workspace;

use crate::color::tint;
use crate::commands::{open_server, pick_server_to_open, pick_server_to_sign_out};
use crate::installation::Installed;
use crate::ports::{EnvironmentOpener, Placement};
use crate::status::{status_chip, status_icon};
use crate::switch::Switch;
use crate::{OpenEnvironmentMenu, OpenServerInNewWindow, SignInToServer, SignOutOfServer, sign_in};

/// How often the countdown of an offline server is redrawn while the menu shows.
const TICK: Duration = Duration::from_secs(1);

/// Puts the environment menu in the title bar of `workspace` and handles
/// [`OpenEnvironmentMenu`], [`SignInToServer`], [`OpenServerInNewWindow`] and
/// [`SignOutOfServer`] there. `environment` is what the window
/// shows; opening it again does nothing.
pub fn install_menu(
    installed: &Installed,
    environment: Environment,
    opener: Rc<dyn EnvironmentOpener>,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut gpui_kit::Context<Workspace>,
) {
    let switch = Switch::new(opener, cx.weak_entity(), environment);
    let hubs = installed.hubs().clone();
    let menu = cx.new(|cx| EnvironmentMenu::new(hubs.clone(), switch.clone(), window, cx));
    workspace.set_environment_menu(menu.clone(), cx);
    let opening = menu.downgrade();
    workspace.register_action(move |_, _: &OpenEnvironmentMenu, _, cx| {
        let _ = opening.update(cx, |menu, cx| menu.set_open(true, cx));
    });
    let (signing_in, opening, signing_out) = (hubs.clone(), hubs.clone(), hubs);
    let (sign_in_switch, open_switch) = (switch.clone(), switch);
    workspace.register_action(move |_, _: &SignInToServer, window, cx| {
        sign_in::start(&signing_in, None, sign_in_switch.clone(), window, cx);
    });
    workspace.register_action(move |_, _: &OpenServerInNewWindow, window, cx| {
        let (hubs, switch) = (opening.clone(), open_switch.clone());
        window.defer(cx, move |window, cx| {
            pick_server_to_open(&hubs, switch, window, cx);
        });
    });
    workspace.register_action(move |_, _: &SignOutOfServer, window, cx| {
        let hubs = signing_out.clone();
        window.defer(cx, move |window, cx| {
            pick_server_to_sign_out(&hubs, window, cx);
        });
    });
}

/// The title-bar button and the list it opens.
struct EnvironmentMenu {
    hubs: Entity<Hubs>,
    environment: Environment,
    list: Entity<EnvironmentList>,
    open: bool,
    _subscriptions: Vec<Subscription>,
}

impl EnvironmentMenu {
    fn new(hubs: Entity<Hubs>, switch: Switch, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let environment = switch.current().clone();
        let list = cx.new(|cx| EnvironmentList::new(hubs.clone(), switch, cx));
        let subscriptions = vec![
            cx.observe(&hubs, |_, _, cx| cx.notify()),
            cx.subscribe(&list, |menu, _, _: &ListDone, cx| menu.set_open(false, cx)),
        ];
        Self {
            hubs,
            environment,
            list,
            open: false,
            _subscriptions: subscriptions,
        }
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.list.update(cx, |list, cx| list.shown(open, cx));
        cx.notify();
    }

    /// The colour of the server this window shows, when the owner chose one.
    fn mark(&self, cx: &App) -> Option<Hsla> {
        let Environment::Hub(id) = &self.environment else {
            return None;
        };
        let color = self.hubs.read(cx).summary(id)?.color?;
        Some(tint(color, cx))
    }

    fn label(&self, cx: &App) -> SharedString {
        match &self.environment {
            Environment::Local => "This computer".into(),
            Environment::Hub(id) => self
                .hubs
                .read(cx)
                .summary(id)
                .map_or_else(|| id.address().to_string(), |summary| summary.label)
                .into(),
        }
    }
}

impl Render for EnvironmentMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let icon = match self.environment {
            Environment::Local => IconName::Terminal,
            Environment::Hub(_) => IconName::Server,
        };
        let mark = self.mark(cx);
        let trigger = Button::new("environment-menu")
            .ghost()
            .small()
            .icon(match mark {
                Some(color) => Icon::new(icon).text_color(color),
                None => Icon::new(icon),
            })
            .label(self.label(cx));
        let list = self.list.clone();
        let focus = list.read(cx).focus_handle(cx);
        let menu = cx.weak_entity();
        div().flex().items_center().h_full().child(
            Popover::new("environment-popover")
                .anchor(Anchor::BottomLeft)
                .trigger(trigger)
                .track_focus(&focus)
                .open(self.open)
                .on_open_change(move |open, _, cx| {
                    let _ = menu.update(cx, |menu, cx| menu.set_open(*open, cx));
                })
                .content(move |_, _, _| list.clone()),
        )
    }
}

/// The list asks its menu to close.
struct ListDone;

/// One line of the list.
#[derive(Clone)]
enum Row {
    Local,
    Hub(HubSummary),
    SignIn,
}

impl Row {
    fn environment(&self) -> Option<Environment> {
        match self {
            Self::Local => Some(Environment::Local),
            Self::Hub(summary) => Some(Environment::Hub(summary.id.clone())),
            Self::SignIn => None,
        }
    }
}

struct EnvironmentList {
    hubs: Entity<Hubs>,
    switch: Switch,
    selected: usize,
    focus: FocusHandle,
    /// Redraws the countdown of offline servers while the menu is open.
    tick: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ListDone> for EnvironmentList {}

impl EnvironmentList {
    fn new(hubs: Entity<Hubs>, switch: Switch, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.subscribe(&hubs, |_, _, _: &HubsEvent, cx| cx.notify())];
        Self {
            hubs,
            switch,
            selected: 0,
            focus: cx.focus_handle(),
            tick: None,
            _subscriptions: subscriptions,
        }
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        std::iter::once(Row::Local)
            .chain(self.hubs.read(cx).list().into_iter().map(Row::Hub))
            .chain(std::iter::once(Row::SignIn))
            .collect()
    }

    /// The menu opened or closed. An open menu starts on the row of the environment the
    /// window shows and counts down the retries of offline servers.
    fn shown(&mut self, open: bool, cx: &mut Context<Self>) {
        if !open {
            self.tick = None;
            return;
        }
        let current = self.switch.current().clone();
        self.selected = self
            .rows(cx)
            .iter()
            .position(|row| row.environment().as_ref() == Some(&current))
            .unwrap_or(0);
        self.tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn move_selection(&mut self, down: bool, cx: &mut Context<Self>) {
        let rows = self.rows(cx).len();
        self.selected = if down {
            (self.selected + 1) % rows
        } else {
            (self.selected + rows - 1) % rows
        };
        cx.notify();
    }

    /// Does what a row stands for: opens the environment, or deals with the trouble the
    /// server is in.
    fn activate(
        &mut self,
        row: Row,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(ListDone);
        let (hubs, switch) = (self.hubs.clone(), self.switch.clone());
        // The dialogs open once the popover has let go of the focus.
        window.defer(cx, move |window, cx| match row {
            Row::Local => switch.open(Environment::Local, placement, window, cx),
            Row::SignIn => sign_in::start(&hubs, None, switch, window, cx),
            Row::Hub(summary) => open_server(hubs, summary, placement, switch, window, cx),
        });
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "up" | "down" => {
                self.move_selection(key == "down", cx);
                cx.stop_propagation();
            }
            "enter" if event.keystroke.modifiers.secondary() => {
                self.activate_selected(Placement::NewWindow, window, cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn activate_selected(
        &mut self,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.rows(cx).get(self.selected).cloned() {
            self.activate(row, placement, window, cx);
        }
    }

    fn render_row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        let (muted, hover, accent) = (theme.muted_foreground, theme.accent, theme.accent);
        let icon_color = match row {
            Row::Hub(summary) => summary.color.map_or(muted, |color| tint(color, cx)),
            _ => muted,
        };
        let (icon, label, chip) = match row {
            Row::Local => (IconName::Terminal, "This computer".to_owned(), None),
            Row::Hub(summary) => (
                status_icon(&summary.status),
                summary.label.clone(),
                Some(status_chip(&summary.status, cx)),
            ),
            Row::SignIn => (
                IconName::ServerPlus,
                "Sign in to a server…".to_owned(),
                None,
            ),
        };
        let current = row.environment().as_ref() == Some(self.switch.current());
        let opens_elsewhere = row.environment().is_some();
        let (row_click, row_new) = (row.clone(), row.clone());
        h_flex()
            .id(("environment-row", index))
            .test_support()
            .gap_2()
            .px_2()
            .py_1()
            .items_center()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|style| style.bg(hover))
            .when(index == self.selected, |style| style.bg(accent))
            .child(Icon::new(icon).small().text_color(icon_color))
            .child(div().flex_1().min_w_0().truncate().text_sm().child(label))
            .when(current, |style| {
                style.child(Icon::new(IconName::Check).small().text_color(muted))
            })
            .when_some(chip, |style, (text, color)| {
                style.child(div().text_xs().text_color(color).child(text))
            })
            .when(opens_elsewhere, |style| {
                style.child(
                    Button::new(("environment-new-window", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::ArrowUpRight)
                        .tooltip("Open in a new window")
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.activate(row_new.clone(), Placement::NewWindow, window, cx);
                        })),
                )
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                let placement = if event.modifiers().secondary() {
                    Placement::NewWindow
                } else {
                    Placement::ThisWindow
                };
                this.activate(row_click.clone(), placement, window, cx);
            }))
            .into_any_element()
    }
}

impl Focusable for EnvironmentList {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for EnvironmentList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        let last = rows.len().saturating_sub(1);
        let border = cx.theme().border;
        v_flex()
            .id("environment-list")
            .track_focus(&self.focus)
            .key_context("EnvironmentMenu")
            .w(px(340.))
            .gap_px()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| {
                this.activate_selected(Placement::ThisWindow, window, cx);
            }))
            .children(rows.iter().enumerate().flat_map(|(index, row)| {
                let separator =
                    (index == last).then(|| div().my_1().h_px().bg(border).into_any_element());
                separator
                    .into_iter()
                    .chain(std::iter::once(self.render_row(index, row, cx)))
            }))
    }
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;
