//! The menu behind the tab strip's "+": quick connect, recent connections
//! and a way to save a new one.

use gpui_kit::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, KeyBinding, NoAction, SharedString,
    Subscription, WeakEntity, Window,
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_session::{Auth, Target};
use nocterm_ui::{ActiveDesign as _, IconName};
use nocterm_workspace::{OpenVault, SessionSpec, Workspace};

use crate::{Connections, editor::local_user, model::connect, open_editor, store::Recent};

const KEY_CONTEXT: &str = "QuickConnect";

pub(crate) fn init(cx: &mut App) {
    // Popovers bind Space to Confirm. Inside this text field, preserve normal
    // character input while leaving keyboard activation of buttons intact.
    cx.bind_keys([KeyBinding::new(
        "space",
        NoAction,
        Some("QuickConnect > Input"),
    )]);
}

pub struct NewTabMenu {
    connections: Entity<Connections>,
    workspace: WeakEntity<Workspace>,
    /// The quick connect field receives focus whenever the menu opens.
    focus_handle: FocusHandle,
    quick_connect: Entity<InputState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl NewTabMenu {
    pub fn new(
        connections: Entity<Connections>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let quick_connect = cx.new(|cx| InputState::new(window, cx).placeholder("user@host:port"));
        let focus_handle = quick_connect.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.observe(&connections, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &quick_connect,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.quick_connect(window, cx),
                    InputEvent::Change if this.error.take().is_some() => cx.notify(),
                    _ => {}
                },
            ),
        ];
        Self {
            connections,
            workspace,
            focus_handle,
            quick_connect,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    /// Connects to what was typed, without saving it.
    fn quick_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.quick_connect.read(cx).value().to_string();
        let target = match Target::parse(&text, local_user().as_deref()) {
            Ok(target) => target,
            Err(error) => {
                self.error = Some(capitalize(&error.to_string()).into());
                cx.notify();
                return;
            }
        };
        self.quick_connect
            .update(cx, |input, cx| input.set_value("", window, cx));
        let spec = SessionSpec {
            profile: None,
            options: Default::default(),
            title: target.to_string().into(),
            target,
            auth: Auth::Auto,
            launch: None,
            credential: None,
        };
        connect(&self.workspace, spec, None, window, cx);
    }

    fn open_recent(&mut self, recent: &Recent, window: &mut Window, cx: &mut Context<Self>) {
        let spec = self.connections.read(cx).spec_for_recent(recent);
        connect(&self.workspace, spec, recent.profile, window, cx);
    }

    fn new_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self
            .workspace
            .update(cx, |workspace, cx| workspace.show_new_tab_menu(false, cx));
        open_editor(None, self.workspace.clone(), window, cx);
    }

    fn render_recents(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let connections = self.connections.read(cx);
        let recents: Vec<(Recent, SessionSpec)> = connections
            .recents()
            .iter()
            .map(|recent| (recent.clone(), connections.spec_for_recent(recent)))
            .collect();
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let hover = theme.accent;

        if recents.is_empty() {
            return div()
                .px_2()
                .text_sm()
                .text_color(muted)
                .child("Connections you open appear here.")
                .into_any_element();
        }

        v_flex()
            .gap_px()
            .children(recents.into_iter().enumerate().map(|(ix, (recent, spec))| {
                let destination = spec.target.to_string();
                let saved = recent.profile.is_some();
                h_flex()
                    .id(("recent", ix))
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .hover(|row| row.bg(hover))
                    .child(
                        Icon::new(if saved {
                            IconName::Server
                        } else {
                            IconName::Clock
                        })
                        .small()
                        .text_color(muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(spec.title.clone()),
                    )
                    .when(saved, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .truncate()
                                .child(destination),
                        )
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_recent(&recent, window, cx);
                    }))
            }))
            .into_any_element()
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn section_label(label: &'static str, cx: &App) -> impl IntoElement {
    div()
        .px_2()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(label)
}

impl Focusable for NewTabMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NewTabMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        let border = cx.theme().border;

        v_flex()
            .key_context(KEY_CONTEXT)
            .w(rems(cx.design().layout.picker_width))
            .gap_2()
            // Single-line inputs propagate Enter after emitting PressEnter.
            // Keep the popover open when quick-connect validation fails.
            .on_action(|_: &gpui_kit::component::input::Enter, _, cx| cx.stop_propagation())
            .child(section_label("QUICK CONNECT", cx))
            .child(
                div().px_1().child(
                    Input::new(&self.quick_connect)
                        .small()
                        .prefix(Icon::new(IconName::Zap).small()),
                ),
            )
            .when_some(self.error.clone(), |menu, error| {
                menu.child(div().px_2().text_xs().text_color(danger).child(error))
            })
            .child(
                Button::new("menu-open-vault")
                    .ghost()
                    .label("Credential vault")
                    .icon(IconName::KeyRound)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let _ = this
                            .workspace
                            .update(cx, |workspace, cx| workspace.show_new_tab_menu(false, cx));
                        window.dispatch_action(Box::new(OpenVault), cx);
                    })),
            )
            .child(section_label("RECENT", cx))
            .child(self.render_recents(cx))
            .child(div().h_px().bg(border))
            .child(
                Button::new("menu-new-connection")
                    .ghost()
                    .small()
                    .w_full()
                    .justify_start()
                    .icon(IconName::ServerPlus)
                    .label("New Connection…")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.new_connection(window, cx)
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, test::TestWindowExt as _};

    #[gpui_kit::test]
    fn menu_uses_input_focus_and_enter_opens_without_saving(cx: &mut TestAppContext) {
        let (handle, workspace, opened) = crate::test_support::workspace(cx);
        let menu = cx
            .update_window(handle, |_, window, cx| {
                let menu = cx.new(|cx| {
                    NewTabMenu::new(Connections::global(cx), workspace.downgrade(), window, cx)
                });
                workspace.update(cx, |workspace, cx| {
                    workspace.set_new_tab_menu(menu.clone(), cx);
                    workspace.show_new_tab_menu(true, cx);
                });
                assert_eq!(
                    menu.read(cx).focus_handle(cx),
                    menu.read(cx).quick_connect.read(cx).focus_handle(cx)
                );
                window.render_frame(cx);
                menu
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                menu.read(cx)
                    .quick_connect
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            window.input("alice@example.test:2222", cx);
            assert_eq!(
                menu.read(cx).quick_connect.read(cx).value(),
                "alice@example.test:2222"
            );
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            assert_eq!(opened.borrow().len(), 1);
            assert_eq!(opened.borrow()[0].target.port, 2222);
            assert_eq!(opened.borrow()[0].target.user, "alice");
            assert_eq!(
                Connections::global(cx).read(cx).profiles().iter().count(),
                0
            );
            assert_eq!(Connections::global(cx).read(cx).recents().iter().count(), 1);
            assert_eq!(menu.read(cx).quick_connect.read(cx).value(), "");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn space_keeps_quick_connect_open_and_inserts_text(cx: &mut TestAppContext) {
        let (handle, workspace, opened) = crate::test_support::workspace(cx);
        let menu = cx
            .update_window(handle, |_, window, cx| {
                let menu = cx.new(|cx| {
                    NewTabMenu::new(Connections::global(cx), workspace.downgrade(), window, cx)
                });
                workspace.update(cx, |workspace, cx| {
                    workspace.set_new_tab_menu(menu.clone(), cx);
                    workspace.show_new_tab_menu(true, cx);
                });
                window.render_frame(cx);
                menu
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.input("a", cx);
            window.press("space", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.find("menu-new-connection").visible(),
                "space must not dismiss the popover"
            );
            assert!(
                menu.read(cx)
                    .quick_connect
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            window.input("b", cx);
            assert_eq!(menu.read(cx).quick_connect.read(cx).value(), "a b");
            assert!(opened.borrow().is_empty());
            assert_eq!(Connections::global(cx).read(cx).recents().iter().count(), 0);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn invalid_enter_keeps_menu_and_error_visible_without_recording_a_connection(
        cx: &mut TestAppContext,
    ) {
        let (handle, workspace, opened) = crate::test_support::workspace(cx);
        let menu = cx
            .update_window(handle, |_, window, cx| {
                let menu = cx.new(|cx| {
                    NewTabMenu::new(Connections::global(cx), workspace.downgrade(), window, cx)
                });
                workspace.update(cx, |workspace, cx| {
                    workspace.set_new_tab_menu(menu.clone(), cx);
                    workspace.show_new_tab_menu(true, cx);
                });
                window.render_frame(cx);
                menu
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.input("alice@host:not-a-port", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(opened.borrow().is_empty());
            assert_eq!(
                Connections::global(cx).read(cx).profiles().iter().count(),
                0
            );
            assert_eq!(Connections::global(cx).read(cx).recents().iter().count(), 0);
            assert!(menu.read(cx).error.is_some());
            assert!(
                window.find("menu-new-connection").visible(),
                "invalid input must leave the popover open"
            );
            assert!(
                menu.read(cx)
                    .quick_connect
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
        .unwrap();
    }
}
