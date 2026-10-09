use super::*;
use gpui_kit::{
    Entity, TestAppContext, WindowOptions,
    component::input::{Input, InputState},
    test::TestWindowExt as _,
};
use nocterm_ai::AiSettings;
use nocterm_session::LocalShellSettings;
use nocterm_settings::SettingsDocument;
use nocterm_ui::TerminalSettings;
use nocterm_workspace::SettingsPage;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) fn init(cx: &mut App) {
    gpui_kit::init(cx);
    nocterm_ui::init(
        nocterm_ui::DesignTokens::builtin(),
        SettingsStore::in_memory(SettingsDocument::default()),
        cx,
    );
}

pub(super) fn open(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<SettingsView>) {
    cx.update(|cx| {
        init(cx);
        let (handle, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| SettingsView::new(window, cx))
        })
        .unwrap();
        (handle, view)
    })
}

/// Types `text` into the field `key` and leaves it, as a user would.
fn type_and_leave(
    view: &Entity<SettingsView>,
    key: &'static str,
    text: &str,
    window: &mut Window,
    cx: &mut App,
) {
    view.update(cx, |view, cx| {
        view.fields[key]
            .input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        view.field_event(&key.into(), &InputEvent::Blur, window, cx);
    });
}

#[gpui_kit::test]
fn text_saves_when_the_field_is_left_and_invalid_text_is_kept_with_its_error(
    cx: &mut TestAppContext,
) {
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        type_and_leave(&view, "terminal.font_size", "100", window, cx);
        assert_eq!(
            cx.setting::<TerminalSettings>().font_size,
            None,
            "invalid text is not saved"
        );
        assert!(view.read(cx).fields["terminal.font_size"].error.is_some());
        type_and_leave(&view, "terminal.font_size", "18", window, cx);
        assert_eq!(cx.setting::<TerminalSettings>().font_size, Some(18.));
        assert!(view.read(cx).fields["terminal.font_size"].error.is_none());
        type_and_leave(&view, "local.args", r#"["-l"]"#, window, cx);
        type_and_leave(&view, "local.env", r#"{"BAD;NAME":"x"}"#, window, cx);
        assert_eq!(cx.setting::<LocalShellSettings>().args, vec!["-l"]);
        assert!(cx.setting::<LocalShellSettings>().env.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn typing_saves_after_a_pause(cx: &mut TestAppContext) {
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.fields["terminal.scrollback"]
                .input
                .update(cx, |input, cx| input.set_value("5000", window, cx));
            view.field_event(
                &"terminal.scrollback".into(),
                &InputEvent::Change,
                window,
                cx,
            );
        });
        assert_ne!(cx.setting::<TerminalSettings>().scrollback_lines, 5000);
    })
    .unwrap();
    cx.executor().advance_clock(field::DEBOUNCE * 2);
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(cx.setting::<TerminalSettings>().scrollback_lines, 5000));
}

#[gpui_kit::test]
fn switches_save_at_once_and_fields_follow_changes_made_elsewhere(cx: &mut TestAppContext) {
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.select_page(1, window, cx));
        window.render_frame(cx);
        window.click("copy-select", cx);
        assert!(cx.setting::<TerminalSettings>().copy_on_select);
        cx.update_setting::<TerminalSettings>(|s| s.font_family = Some("Iosevka".into()))
            .detach();
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).fields["terminal.font_family"]
                .input
                .read(cx)
                .value(),
            "Iosevka"
        );
    });
}

#[gpui_kit::test]
fn ai_switches_and_agent_fields_save_validated_values(cx: &mut TestAppContext) {
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(1100.)));
        view.update(cx, |view, cx| view.select_page(4, window, cx));
        window.render_frame(cx);
        window.click("ai-read-approval", cx);
        assert_eq!(
            cx.setting::<AiSettings>().approval.terminal_read,
            nocterm_ai::ApprovalPolicy::Ask
        );
        type_and_leave(&view, "agent.claude.args", "bad JSON", window, cx);
        assert!(cx.setting::<AiSettings>().agents.is_empty());
        type_and_leave(&view, "agent.claude.command", "/opt/claude", window, cx);
        assert_eq!(
            cx.setting::<AiSettings>().agents["claude"]
                .command
                .as_deref(),
            Some("/opt/claude")
        );
        type_and_leave(&view, "agent.claude.command", "", window, cx);
        assert!(
            cx.setting::<AiSettings>().agents.is_empty(),
            "defaults need no override"
        );
        window.click("ai-enabled", cx);
        assert!(!cx.setting::<AiSettings>().enabled);
    })
    .unwrap();
}

#[gpui_kit::test]
fn agent_permission_switch_is_independent_and_disabled_with_ai(cx: &mut TestAppContext) {
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(1100.)));
        view.update(cx, |view, cx| view.select_page(4, window, cx));
        window.render_frame(cx);
        let terminal_read = cx.setting::<AiSettings>().approval.terminal_read;
        let terminal_write = cx.setting::<AiSettings>().approval.terminal_write;
        assert_eq!(
            cx.setting::<AiSettings>().approval.agent_permissions,
            nocterm_ai::ApprovalPolicy::Ask
        );
        window.click("ai-agent-permissions", cx);
        assert_eq!(
            cx.setting::<AiSettings>().approval.agent_permissions,
            nocterm_ai::ApprovalPolicy::Allow
        );
        assert_eq!(
            cx.setting::<AiSettings>().approval.terminal_read,
            terminal_read
        );
        assert_eq!(
            cx.setting::<AiSettings>().approval.terminal_write,
            terminal_write
        );
        window.render_frame(cx);
        window.click("ai-agent-permissions", cx);
        assert_eq!(
            cx.setting::<AiSettings>().approval.agent_permissions,
            nocterm_ai::ApprovalPolicy::Ask
        );
        window.click("ai-enabled", cx);
        window.render_frame(cx);
        window.click("ai-agent-permissions", cx);
        assert_eq!(
            cx.setting::<AiSettings>().approval.agent_permissions,
            nocterm_ai::ApprovalPolicy::Ask
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn custom_agents_are_added_with_an_executable_and_get_fields(cx: &mut TestAppContext) {
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.ai
                .new_id_input()
                .update(cx, |input, cx| input.set_value("mine", window, cx));
            ai::add_for_test(view, window, cx);
            assert!(view.ai.has_add_error(), "an executable is required");
            view.ai
                .new_command_input()
                .update(cx, |input, cx| input.set_value("/opt/mine", window, cx));
            ai::add_for_test(view, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            cx.setting::<AiSettings>().agents["mine"].command.as_deref(),
            Some("/opt/mine")
        );
        assert!(view.read(cx).fields.contains_key("agent.mine.command"));
    });
}

struct GuestPage {
    input: Entity<InputState>,
    deactivations: Rc<Cell<usize>>,
    closures: Rc<Cell<usize>>,
}
impl Focusable for GuestPage {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}
impl Render for GuestPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("guest-page").child(Input::new(&self.input))
    }
}
impl SettingsPage for GuestPage {
    fn on_deactivate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.deactivations.set(self.deactivations.get() + 1);
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.closures.set(self.closures.get() + 1);
        self.on_deactivate(window, cx);
    }
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn guest_pages_are_lazy_reused_and_told_when_hidden_or_closed(cx: &mut TestAppContext) {
    let creations = Rc::new(Cell::new(0));
    let deactivations = Rc::new(Cell::new(0));
    let closures = Rc::new(Cell::new(0));
    let guest = Rc::new(RefCell::new(None::<Entity<GuestPage>>));
    let descriptor = SettingsPageSpec::new("vault", "Vault", {
        let creations = creations.clone();
        let deactivations = deactivations.clone();
        let closures = closures.clone();
        let guest = guest.clone();
        move |window, cx| {
            creations.set(creations.get() + 1);
            let entity = cx.new(|cx| GuestPage {
                input: cx.new(|cx| InputState::new(window, cx).masked(true)),
                deactivations: deactivations.clone(),
                closures: closures.clone(),
            });
            *guest.borrow_mut() = Some(entity.clone());
            entity
        }
    });
    let (handle, workspace) = cx.update(|cx| {
        init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(window, cx);
                register_with_pages(&mut workspace, vec![descriptor.clone()]);
                workspace
            })
        })
        .unwrap()
    });
    let guest_value = |cx: &App| {
        guest
            .borrow()
            .as_ref()
            .unwrap()
            .read(cx)
            .input
            .read(cx)
            .value()
            .to_string()
    };
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            open_page(workspace, "", std::slice::from_ref(&descriptor), window, cx);
        });
        window.render_frame(cx);
        assert_eq!(creations.get(), 0, "guest must be lazy");
        window.within("settings-pages").click(1usize, cx);
        let settings = workspace.read(cx).find_item::<SettingsView>().unwrap();
        assert_eq!(settings.read(cx).selected_page_id(), "terminal");
        workspace.update(cx, |workspace, cx| {
            open_page(
                workspace,
                "vault",
                std::slice::from_ref(&descriptor),
                window,
                cx,
            )
        });
        assert_eq!(workspace.read(cx).items().count(), 1);
        assert_eq!(creations.get(), 1);
        window.render_frame(cx);
        window.input("guest master draft", cx);
        assert_eq!(guest_value(cx), "guest master draft");
        window.within("settings-pages").click(0usize, cx);
        assert_eq!(deactivations.get(), 1);
        assert!(guest_value(cx).is_empty());
        assert_eq!(settings.read(cx).selected_page_id(), "appearance");
        assert!(settings.read(cx).focus_handle(cx).is_focused(window));
        workspace.update(cx, |workspace, cx| {
            open_page(
                workspace,
                "vault",
                std::slice::from_ref(&descriptor),
                window,
                cx,
            )
        });
        assert_eq!(creations.get(), 1, "reuse the guest entity");
        window.render_frame(cx);
        window.input("closing guest draft", cx);
        workspace.update(cx, |workspace, cx| workspace.close_item(0, window, cx));
        assert_eq!(closures.get(), 1);
        assert!(guest_value(cx).is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn open_settings_reuses_one_tab(cx: &mut TestAppContext) {
    let (handle, workspace) = cx.update(|cx| {
        init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let workspace = cx.new(|cx| {
                let mut workspace = Workspace::new(window, cx);
                register(&mut workspace);
                workspace
            });
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
            workspace
        })
        .unwrap()
    });
    for _ in 0..2 {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_action(Box::new(OpenSettings), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).items().count(), 1);
        let settings = workspace.read(cx).find_item::<SettingsView>().unwrap();
        assert!(settings.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_failed_save_is_reported_and_keeps_the_active_settings(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        // A directory cannot be replaced with a settings file.
        let file = nocterm_settings::SettingsFile::new(directory.path());
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::new(SettingsDocument::default(), file),
            cx,
        );
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| SettingsView::new(window, cx))
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.select_page(1, window, cx));
        window.render_frame(cx);
        window.click("copy-select", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!cx.setting::<TerminalSettings>().copy_on_select);
        assert!(
            view.read(cx)
                .error
                .as_ref()
                .is_some_and(|error| error.starts_with("Could not save settings"))
        );
    });
}

#[gpui_kit::test]
fn terminal_highlighting_switch_saves_and_can_be_reenabled(cx: &mut TestAppContext) {
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.select_page(1, window, cx));
        window.render_frame(cx);
        assert!(cx.setting::<TerminalSettings>().semantic_highlighting);
        window.click("semantic-highlighting", cx);
        assert!(!cx.setting::<TerminalSettings>().semantic_highlighting);
        window.render_frame(cx);
        window.click("semantic-highlighting", cx);
        assert!(cx.setting::<TerminalSettings>().semantic_highlighting);
    })
    .unwrap();
}
