use gpui_kit::{App, AppContext as _, Focusable, Window};
use gpui_kit::{Entity, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_ai::AiSettings;
use nocterm_session::LocalShellSettings;
use nocterm_settings::SettingsDocument;
use nocterm_settings_ui::{Installed, SettingsCatalog, SettingsView, open_page, register};
use nocterm_store::SettingsConfig;
use nocterm_ui::TerminalSettings;
use nocterm_workspace::{OpenSettings, Workspace};

struct SettingsFixture {
    installed: Installed,
    pages: SettingsCatalog,
    themes: nocterm_ui::ThemeUi,
}
fn init(cx: &mut App) -> SettingsFixture {
    init_with_settings(SettingsConfig::in_memory(SettingsDocument::default()), cx)
}
fn init_with_probe(
    probe: std::sync::Arc<dyn nocterm_ai::sandbox::SandboxProbe>,
    cx: &mut App,
) -> SettingsFixture {
    let booted = super::boot_with_ports_and_probe(
        tempfile::tempdir().unwrap(),
        SettingsConfig::in_memory(Default::default()),
        (
            std::sync::Arc::new(super::MockTransport::default()),
            nocterm_terminal::LocalSessions::disabled("/bin/bash".into()),
        ),
        false,
        None,
        None,
        probe,
        cx,
    );
    SettingsFixture {
        installed: booted.settings_deps(),
        pages: booted.settings_pages(),
        themes: booted.themes().clone(),
    }
}
fn init_with_settings(settings: SettingsConfig, cx: &mut App) -> SettingsFixture {
    let booted = super::boot_with_ports(
        tempfile::tempdir().unwrap(),
        settings,
        (
            std::sync::Arc::new(super::MockTransport::default()),
            nocterm_terminal::LocalSessions::disabled("/bin/bash".into()),
        ),
        false,
        None,
        None,
        cx,
    );
    SettingsFixture {
        installed: booted.settings_deps(),
        pages: booted.settings_pages(),
        themes: booted.themes().clone(),
    }
}
pub(super) fn test_window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
            gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)),
            gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(5000.)),
        ))),
        ..WindowOptions::default()
    }
}
pub(super) fn open(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<SettingsView>,
    nocterm_store::SettingsHandle,
) {
    cx.update(|cx| {
        let fixture = init(cx);
        let (handle, view) = gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            cx.new(|cx| {
                SettingsView::with_pages(
                    fixture.pages.clone(),
                    fixture.installed.clone(),
                    window,
                    cx,
                )
            })
        })
        .unwrap();
        (handle, view, fixture.installed.settings().clone())
    })
}

/// Types `text` into the field `key` and leaves it, as a user would.
fn type_and_leave(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    view: &Entity<SettingsView>,
    key: &'static str,
    text: &str,
) {
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        select_field_page(key, window, cx);
        window.click(key, cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(text, cx);
        assert_eq!(window.find(key).value(), Some(text));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.focus(&view.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn select_field_page(key: &str, window: &mut Window, cx: &mut App) {
    let page = if key.starts_with("terminal.") {
        1usize
    } else if key.starts_with("local.") {
        2
    } else if key.starts_with("ssh.") {
        3
    } else if key.starts_with("ai.") || key.starts_with("agent.") {
        4
    } else {
        0
    };
    window.render_frame(cx);
    window.click(page, cx);
    window.render_frame(cx);
}

#[gpui_kit::test]
fn text_saves_when_the_field_is_left_and_invalid_text_is_kept_with_its_error(
    cx: &mut TestAppContext,
) {
    let (handle, view, settings) = open(cx);
    type_and_leave(cx, handle, &view, "terminal.font_size", "100");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            settings.setting::<TerminalSettings>(cx).font_size,
            None,
            "invalid text is not saved"
        );
        window.render_frame(cx);
        assert!(
            window
                .try_find("settings-error-terminal.font_size")
                .is_some()
        );
    })
    .unwrap();
    type_and_leave(cx, handle, &view, "terminal.font_size", "18");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            settings.setting::<TerminalSettings>(cx).font_size,
            Some(18.)
        );
        window.render_frame(cx);
        assert!(
            window
                .try_find("settings-error-terminal.font_size")
                .is_none()
        );
    })
    .unwrap();
    type_and_leave(cx, handle, &view, "local.args", r#"["-l"]"#);
    type_and_leave(cx, handle, &view, "local.env", r#"{"BAD;NAME":"x"}"#);
    cx.update(|cx| {
        assert_eq!(settings.setting::<LocalShellSettings>(cx).args, vec!["-l"]);
        assert!(settings.setting::<LocalShellSettings>(cx).env.is_empty());
    });
}

#[gpui_kit::test]
fn typing_saves_after_a_pause(cx: &mut TestAppContext) {
    let (handle, _view, settings) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        select_field_page("terminal.scrollback", window, cx);
        window.click("terminal.scrollback", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input("5000", cx);
        assert_ne!(
            settings.setting::<TerminalSettings>(cx).scrollback_lines,
            5000
        );
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1200));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<TerminalSettings>(cx).scrollback_lines,
            5000
        )
    });
}

#[gpui_kit::test]
fn switches_save_at_once_and_fields_follow_changes_made_elsewhere(cx: &mut TestAppContext) {
    let (handle, _view, settings) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(1usize, cx);
        window.render_frame(cx);
        window.click("copy-select", cx);
        assert!(settings.setting::<TerminalSettings>(cx).copy_on_select);
        settings
            .update::<TerminalSettings>(cx, |s| s.font_family = Some("Iosevka".into()))
            .detach();
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("terminal.font_family").value(), Some("Iosevka"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn ai_switches_and_agent_fields_save_validated_values(cx: &mut TestAppContext) {
    let (handle, view, settings) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(5000.)));
        select_field_page("ai.enabled", window, cx);
        window.click("ai-read-approval", cx);
        assert_eq!(
            settings.setting::<AiSettings>(cx).approval.terminal_read,
            nocterm_ai::ApprovalPolicy::Ask
        );
        window.render_frame(cx);
        window.click("ai-agent-claude", cx);
    })
    .unwrap();
    cx.run_until_parked();
    type_and_leave(cx, handle, &view, "agent.claude.args", "bad JSON");
    cx.update(|cx| assert!(settings.setting::<AiSettings>(cx).agents.is_empty()));
    type_and_leave(cx, handle, &view, "agent.claude.command", "/opt/claude");
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<AiSettings>(cx).agents["claude"]
                .command
                .as_deref(),
            Some("/opt/claude")
        )
    });
    type_and_leave(cx, handle, &view, "agent.claude.command", "");
    cx.update(|cx| {
        assert!(
            settings.setting::<AiSettings>(cx).agents.is_empty(),
            "defaults need no override"
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("ai-enabled", cx);
        assert!(!settings.setting::<AiSettings>(cx).enabled);
    })
    .unwrap();
}

#[gpui_kit::test]
fn agent_permission_switch_is_independent_and_disabled_with_ai(cx: &mut TestAppContext) {
    let (handle, _view, settings) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(1100.)));
        window.render_frame(cx);
        window.click(4usize, cx);
        window.render_frame(cx);
        let terminal_read = settings.setting::<AiSettings>(cx).approval.terminal_read;
        let terminal_write = settings.setting::<AiSettings>(cx).approval.terminal_write;
        assert_eq!(
            settings
                .setting::<AiSettings>(cx)
                .approval
                .agent_permissions,
            nocterm_ai::ApprovalPolicy::Ask
        );
        window.click("ai-agent-permissions", cx);
        assert_eq!(
            settings
                .setting::<AiSettings>(cx)
                .approval
                .agent_permissions,
            nocterm_ai::ApprovalPolicy::Allow
        );
        assert_eq!(
            settings.setting::<AiSettings>(cx).approval.terminal_read,
            terminal_read
        );
        assert_eq!(
            settings.setting::<AiSettings>(cx).approval.terminal_write,
            terminal_write
        );
        window.render_frame(cx);
        window.click("ai-agent-permissions", cx);
        assert_eq!(
            settings
                .setting::<AiSettings>(cx)
                .approval
                .agent_permissions,
            nocterm_ai::ApprovalPolicy::Ask
        );
        window.click("ai-enabled", cx);
        window.render_frame(cx);
        window.click("ai-agent-permissions", cx);
        assert_eq!(
            settings
                .setting::<AiSettings>(cx)
                .approval
                .agent_permissions,
            nocterm_ai::ApprovalPolicy::Ask
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn custom_agents_are_added_with_an_executable_and_get_fields(cx: &mut TestAppContext) {
    let (handle, _view, settings) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(5000.)));
        select_field_page("ai.enabled", window, cx);
        window.click("ai-new-id", cx);
        window.input("mine", cx);
        window.click("ai-add-agent", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("ai-add-error").is_some(),
            "an executable is required"
        );
        window.click("ai-new-command", cx);
        window.input("/opt/mine", cx);
        window.click("ai-add-agent", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<AiSettings>(cx).agents["mine"]
                .command
                .as_deref(),
            Some("/opt/mine")
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find("settings-field-agent.mine.command")
                .is_some()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_failed_save_is_reported_and_keeps_the_active_settings(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (handle, _view, settings) = cx.update(|cx| {
        let file = nocterm_settings::SettingsFile::new(directory.path());
        let fixture =
            init_with_settings(SettingsConfig::new(SettingsDocument::default(), file), cx);
        let (handle, view) = gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            cx.new(|cx| {
                SettingsView::with_pages(
                    fixture.pages.clone(),
                    fixture.installed.clone(),
                    window,
                    cx,
                )
            })
        })
        .unwrap();
        (handle, view, fixture.installed.settings().clone())
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(1usize, cx);
        window.render_frame(cx);
        window.click("copy-select", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!settings.setting::<TerminalSettings>(cx).copy_on_select);
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("settings-save-error")
                .label()
                .is_some_and(|error| error.starts_with("Could not save settings"))
        );
        window.click(0usize, cx);
        window.render_frame(cx);
        assert!(
            window
                .find("settings-save-error")
                .label()
                .is_some_and(|error| error.starts_with("Could not save settings")),
            "the per-Item feedback survives a builtin page switch"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn terminal_highlighting_switch_saves_and_can_be_reenabled(cx: &mut TestAppContext) {
    let (handle, _view, settings) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(1usize, cx);
        window.render_frame(cx);
        assert!(
            settings
                .setting::<TerminalSettings>(cx)
                .semantic_highlighting
        );
        window.click("semantic-highlighting", cx);
        assert!(
            !settings
                .setting::<TerminalSettings>(cx)
                .semantic_highlighting
        );
        window.render_frame(cx);
        window.click("semantic-highlighting", cx);
        assert!(
            settings
                .setting::<TerminalSettings>(cx)
                .semantic_highlighting
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn sandbox_controls_use_the_injected_probe_on_every_platform(cx: &mut TestAppContext) {
    use nocterm_ai::{sandbox::Availability, testing::ScriptedSandboxProbe};
    for (availability, disabled) in [
        (Availability::Available("/fake/bwrap".into()), false),
        (Availability::Missing, true),
        (Availability::Unsupported, true),
    ] {
        let probe = std::sync::Arc::new(ScriptedSandboxProbe::new(availability));
        let (handle, workspace, settings) = cx.update(|cx| {
            let fixture = init_with_probe(probe.clone(), cx);
            let layout = nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
            let (handle, workspace) =
                gpui_kit::open_window(test_window_options(), cx, |window, cx| {
                    let workspace = cx.new(|cx| Workspace::new(layout, window, cx));
                    workspace.update(cx, |workspace, cx| {
                        open_page(
                            workspace,
                            "appearance",
                            &fixture.pages,
                            fixture.installed.clone(),
                            window,
                            cx,
                        );
                    });
                    workspace
                })
                .unwrap();
            (handle, workspace, fixture.installed.settings().clone())
        });
        assert_eq!(
            probe.calls(),
            1,
            "the actual AI page probes during eager construction"
        );
        cx.update(|cx| {
            let settings = workspace.read(cx).find_item::<SettingsView>(cx).unwrap();
            assert_eq!(settings.read(cx).selected_page_id(), "appearance");
        });
        cx.update_window(handle, |_, window, cx| {
            window.resize(gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(2400.)));
            window.render_frame(cx);
            window.click(4usize, cx);
            window.render_frame(cx);
            window.click("ai-sandbox", cx);
            assert_eq!(
                settings.setting::<AiSettings>(cx).sandbox,
                if disabled {
                    nocterm_ai::SandboxMode::Off
                } else {
                    nocterm_ai::SandboxMode::Workspace
                }
            );
            assert_eq!(probe.calls(), 1);
            assert_eq!(workspace.read(cx).items(cx).count(), 1);
        })
        .unwrap();
    }
}

mod appearance;
mod dependencies;
mod plugins;

mod text;

use text::enter_font;
