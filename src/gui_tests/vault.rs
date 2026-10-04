//! Vault actions and the startup unlock prompt reach the vault page.
use std::sync::Arc;

use gpui_kit::{AppContext as _, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_terminal::open_session;
use nocterm_ui::ActiveSettings as _;
use nocterm_workspace::Workspace;

use super::{FixtureDirectory, FixtureVault, MockTransport, fixture_with_vault};

#[gpui_kit::test]
fn unlock_action_opens_the_unlock_dialog_for_a_locked_vault(cx: &mut TestAppContext) {
    let (handle, _, _, _) = fixture_with_vault(cx, true);
    cx.update(|cx| (cx.global::<FixtureVault>().0)());
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(nocterm_workspace::UnlockVault), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("vault-unlock-prompt").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn vault_action_opens_the_vault_page_in_the_single_settings_item(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture_with_vault(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(nocterm_workspace::OpenSettings), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let original = workspace.read_with(cx, |w, _| {
        w.find_item::<nocterm_settings_ui::SettingsView>()
            .unwrap()
            .entity_id()
    });
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("vault-submit").is_none());
        window.dispatch_action(Box::new(nocterm_workspace::OpenVault), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("vault-submit").visible());
        window.dispatch_action(Box::new(nocterm_workspace::OpenVault), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("vault-submit").visible());
    })
    .unwrap();
    assert_eq!(
        workspace.read_with(cx, |w, _| w
            .find_item::<nocterm_settings_ui::SettingsView>()
            .unwrap()
            .entity_id()),
        original
    );
}

#[gpui_kit::test]
fn prompt_on_startup_opens_vault_page_on_launch(cx: &mut TestAppContext) {
    let transport = Arc::new(MockTransport::default());
    let (handle, workspace, _) = cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        let directory = tempfile::tempdir().unwrap();
        let mut settings = nocterm_settings::Settings::default();
        settings.vault.prompt_on_startup = true;
        let vault_path = directory.path().join("vault.bin");
        let paths = nocterm_core::Paths::rooted_at(directory.path());
        cx.set_global(FixtureDirectory {
            _directory: directory,
        });
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(settings),
            cx,
        );
        nocterm_vault_ui::init(vault_path, cx).unwrap();
        nocterm_terminal::init(transport.clone(), cx);
        nocterm_connections::init(None, cx);
        nocterm_agent::init(
            nocterm_agent::AgentServices {
                terminal_auth: None,
                private_dirs: Vec::new(),
                shared_dirs: Vec::new(),
                connector: Arc::new(nocterm_acp::AcpConnector),
                bridge: Arc::new(nocterm_acp::BridgeServer::new(paths.clone())),
                state_file: paths.state_dir().join("agents.toml"),
                chats_dir: paths.state_dir().join("agent-chats"),
                codex_home: None,
                workdir: paths.state_dir().join("agent-workspace"),
            },
            cx,
        );
        crate::keymap::load(None, cx);
        super::super::application::register(paths, true, cx);
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    workspace.set_session_opener(open_session);
                    nocterm_connections::register(&mut workspace, window, cx);
                    super::super::register_settings(&mut workspace, true);
                    nocterm_files::register(&mut workspace, window, cx);
                    nocterm_agent::register(&mut workspace, window, cx);
                    workspace.set_menu_builder(super::super::app_menus::build, window, cx);
                    if cx.settings().vault.prompt_on_startup {
                        let pages = vec![nocterm_vault_ui::settings_page()];
                        nocterm_settings_ui::open_page(&mut workspace, "vault", &pages, window, cx);
                    }
                    workspace
                })
            })
            .unwrap();
        (window, workspace, ())
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("vault-submit").visible());
        assert!(
            workspace
                .read(cx)
                .find_item::<nocterm_settings_ui::SettingsView>()
                .is_some()
        );
    })
    .unwrap();
}
