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
        let (window, workspace) = open_workspace(booted.vault.clone(), cx);
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

#[gpui_kit::test]
fn installed_vault_port_shares_credentials_and_pages_across_windows(cx: &mut TestAppContext) {
    use futures::executor::block_on;
    use gpui_kit::Focusable as _;
    use nocterm_session::{CredentialStore as _, Secret, SecretRequest, Target};
    cx.executor().allow_parking();
    let (first, second, installed) = cx.update(|cx| {
        let booted = boot(
            tempfile::tempdir().unwrap(),
            Default::default(),
            Arc::new(MockTransport::default()),
            true,
            cx,
        );
        let installed = booted.vault.unwrap();
        let first = open_workspace(Some(installed.clone()), cx);
        let second = open_workspace(Some(installed.clone()), cx);
        (first, second, installed)
    });
    let service = installed.service();
    block_on(service.create(Secret::new("long master password"))).unwrap();
    let credentials = nocterm_vault_ui::VaultCredentials::new(service.clone());
    let request = SecretRequest::Password {
        target: Target::new("user", "host", 22),
        retry: false,
    };
    let id = block_on(credentials.save("profile".into(), &request, Secret::new("saved password")))
        .unwrap();
    assert_eq!(
        block_on(credentials.get(id, &request)).unwrap().expose(),
        "saved password"
    );
    service.lock();
    assert!(!credentials.is_available());
    block_on(service.unlock(Secret::new("long master password"))).unwrap();
    assert!(credentials.is_available());
    assert_eq!(
        block_on(credentials.get(id, &request)).unwrap().expose(),
        "saved password"
    );
    for (handle, workspace) in [first, second] {
        cx.update_window(handle, |_, window, cx| {
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
            window.dispatch_action(Box::new(nocterm_workspace::OpenVault), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("vault-lock").visible());
            window.dispatch_action(Box::new(nocterm_workspace::CloseTab), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
            window.dispatch_action(Box::new(nocterm_workspace::OpenVault), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("vault-lock").visible());
        })
        .unwrap();
    }
    assert_eq!(
        block_on(credentials.get(id, &request)).unwrap().expose(),
        "saved password"
    );
}
