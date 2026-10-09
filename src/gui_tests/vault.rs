//! Vault actions and the startup unlock prompt reach the vault page.
use std::sync::Arc;

use gpui_kit::{AppContext as _, TestAppContext, test::TestWindowExt as _};

use super::{FixtureVault, MockTransport, boot, fixture_with_vault, open_workspace};

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
        let mut settings = nocterm_settings::SettingsDocument::default();
        settings
            .update::<nocterm_vault_ui::VaultSettings>(|section| section.prompt_on_startup = true);
        let booted = boot(tempfile::tempdir().unwrap(), settings, transport, true, cx);
        let (window, workspace) = open_workspace(booted.vault_ready(), cx);
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
