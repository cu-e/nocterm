//! Transient options for one terminal; saved connection profiles stay separate.
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, Subscription, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_ui::{ActiveDesign as _, SessionOptionsEditor};

use crate::Terminal;

pub(crate) fn open(terminal: Entity<Terminal>, window: &mut Window, cx: &mut App) {
    // Let the menu restore its action context before mounting the modal.
    window.defer(cx, move |window, cx| {
        let options = terminal.read(cx).spec().options.clone();
        let editor = cx.new(|cx| SessionSettings::new(terminal, options, window, cx));
        let focus = editor.read(cx).focus_handle(cx);
        let width = rems(cx.design().layout.dialog_width).to_pixels(window.rem_size());
        window.open_dialog(cx, move |dialog, _, _| {
            let apply = editor.clone();
            let confirm = editor.clone();
            let cancel = editor.clone();
            let dismiss = editor.clone();
            dialog
                .title("Session Settings")
                .w(width)
                .child(editor.clone())
                .footer(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("session-settings-cancel")
                                .label("Cancel")
                                .on_click(move |_, window, cx| {
                                    cancel.update(cx, |this, _| this.closed = true);
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("session-settings-apply")
                                .primary()
                                .label("Apply")
                                .on_click(move |_, window, cx| {
                                    apply.update(cx, |this, cx| this.apply(window, cx));
                                }),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    confirm.update(cx, |this, cx| this.apply(window, cx));
                    // Invalid options leave the dialog and draft intact.
                    false
                })
                .on_close(move |_, _, cx| {
                    dismiss.update(cx, |this, _| this.closed = true);
                })
        });
        window.focus(&focus, cx);
    });
}

struct SessionSettings {
    terminal: WeakEntity<Terminal>,
    options: Entity<SessionOptionsEditor>,
    focus: FocusHandle,
    error: Option<String>,
    closed: bool,
    _subscription: Subscription,
}

impl SessionSettings {
    fn new(
        terminal: Entity<Terminal>,
        options: nocterm_session::SessionOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = cx.new(|cx| SessionOptionsEditor::new(options, true, window, cx));
        let subscription = cx.observe(&options, |_, _, cx| cx.notify());
        Self {
            terminal: terminal.downgrade(),
            options,
            focus: cx.focus_handle(),
            error: None,
            closed: false,
            _subscription: subscription,
        }
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        let result = self.options.read(cx).options(cx).and_then(|options| {
            self.terminal
                .update(cx, |terminal, cx| terminal.set_session_options(options, cx))
                .map_err(|_| "This session is no longer available. Close this dialog.".to_owned())?
        });
        match result {
            Ok(()) => {
                self.closed = true;
                window.close_dialog(cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
}

impl Focusable for SessionSettings {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SessionSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("session-settings-form")
            .track_focus(&self.focus)
            .gap_3()
            .w_full()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Apply changes this tab only, starting with its next reconnect. Saved connections and global defaults stay unchanged."),
            )
            .child(self.options.clone())
            .when_some(self.error.clone(), |form, error| {
                form.child(
                    div()
                        .id("session-settings-error")
                        .test_support()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, test::TestWindowExt as _};
    use nocterm_session::{Auth, SessionOptions, ShellLaunch, Target};
    use nocterm_workspace::SessionSpec;

    fn fixture(
        options: SessionOptions,
        cx: &mut TestAppContext,
    ) -> (AnyWindowHandle, Entity<Terminal>, SessionSpec) {
        let spec = SessionSpec {
            profile: None,
            options,
            title: "Production".into(),
            target: Target::new("operator", "production.example", 2222),
            auth: Auth::Password,
            launch: Some(ShellLaunch {
                program: Some("/bin/bash".into()),
                ..Default::default()
            }),
            credential: Some(nocterm_session::CredentialId::generate().unwrap()),
        };
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                nocterm_ui::SettingsStore::in_memory(Default::default()),
                cx,
            );
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| crate::TerminalView::new(spec.clone(), window, cx))
            })
            .unwrap()
        });
        let terminal = view.read_with(cx, |view, _| view.terminal().clone());
        cx.update_window(handle, |_, window, cx| {
            window.activate_window();
            cx.set_reduce_motion(true);
            open(terminal.clone(), window, cx);
        })
        .unwrap();
        cx.simulate_window_resize(
            handle,
            gpui_kit::size(gpui_kit::px(900.), gpui_kit::px(700.)),
        );
        cx.run_until_parked();
        (handle, terminal, spec)
    }

    fn choose_term(cx: &mut TestAppContext, handle: AnyWindowHandle) {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.within("session-term").click("input", cx);
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn native_apply_changes_only_transient_options(cx: &mut TestAppContext) {
        let (handle, terminal, mut expected) = fixture(Default::default(), cx);
        choose_term(cx, handle);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("session-settings-apply", cx);
        })
        .unwrap();
        cx.run_until_parked();
        expected.options.term = Some("xterm-256color".into());
        terminal.read_with(cx, |terminal, _| assert_eq!(terminal.spec(), &expected));
        cx.update_window(handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx))
        })
        .unwrap();
        cx.read(|cx| {
            let store = cx.global::<nocterm_ui::SettingsStore>();
            assert_eq!(
                store.revision(),
                0,
                "applying a session setting saved nothing"
            );
        });
    }

    #[gpui_kit::test]
    fn invalid_custom_term_preserves_the_modal_and_original_spec(cx: &mut TestAppContext) {
        let (handle, terminal, expected) = fixture(Default::default(), cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.within("session-term").click("input", cx);
            for _ in 0..=nocterm_session::TERM_PRESETS.len() {
                window.press("down", cx);
            }
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("session-settings-apply", cx);
            window.render_frame(cx);
            assert!(window.has_active_dialog(cx));
            assert!(window.find("session-settings-error").visible());
        })
        .unwrap();
        terminal.read_with(cx, |terminal, _| assert_eq!(terminal.spec(), &expected));
    }

    #[gpui_kit::test]
    fn native_cancel_discards_the_changed_draft(cx: &mut TestAppContext) {
        let (handle, terminal, expected) = fixture(Default::default(), cx);
        choose_term(cx, handle);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("session-settings-cancel", cx);
        })
        .unwrap();
        cx.run_until_parked();
        terminal.read_with(cx, |terminal, _| assert_eq!(terminal.spec(), &expected));
        cx.update_window(handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx))
        })
        .unwrap();
    }
}
