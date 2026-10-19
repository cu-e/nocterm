//! Vault's published policy and native controls retain original installation.
use super::*;
use std::time::Duration;

fn another_settings(cx: &mut gpui_kit::App) -> nocterm_store::SettingsHandle {
    let settings = nocterm_store::SettingsConfig::in_memory(Default::default())
        .with(crate::VaultSettings {
            auto_lock_minutes: 32,
            ..Default::default()
        })
        .install(cx);
    let ready = nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), settings.clone(), cx);
    let service = Arc::new(ScriptedVaultAccess::default());
    let installed = crate::install(crate::Deps::new(service), &ready, cx);
    page(&installed, cx);
    settings
}

fn page(
    installed: &crate::Installed,
    cx: &mut gpui_kit::App,
) -> (gpui_kit::AnyWindowHandle, Entity<VaultView>) {
    gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
        cx.new(|cx| {
            VaultView::new(
                installed.service(),
                installed.settings().clone(),
                window,
                cx,
            )
        })
    })
    .unwrap()
}

#[gpui_kit::test]
fn vault_policy_observer_survives_first_page_and_never_follows_unrelated_settings(
    cx: &mut TestAppContext,
) {
    let (first, first_page, service, installed) = setup_with_store(
        cx,
        nocterm_store::SettingsConfig::in_memory(Default::default()),
    );
    let (second, _second_page) = cx.update(|cx| page(&installed, cx));
    tab(second, 3, cx);
    let other = cx.update(another_settings);
    cx.update(|cx| {
        installed
            .settings()
            .update::<crate::VaultSettings>(cx, |settings| settings.auto_lock_minutes = 20)
            .detach()
    });
    drain(cx);
    assert_eq!(service.auto_lock_updates(), [Duration::from_secs(1200)]);
    cx.update_window(second, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("vault-auto-lock-input").value(), Some("20"));
    })
    .unwrap();
    let weak = first_page.downgrade();
    cx.update_window(first, |_, window, _| window.remove_window())
        .unwrap();
    drop(first_page);
    drain(cx);
    assert!(weak.upgrade().is_none());
    let (third, _third_page) = cx.update(|cx| page(&installed, cx));
    tab(third, 3, cx);
    type_field(third, "vault-auto-lock", "21", cx);
    cx.update_window(third, |_, window, cx| window.press("enter", cx))
        .unwrap();
    drain(cx);
    assert_eq!(
        service.auto_lock_updates().last(),
        Some(&Duration::from_secs(1260))
    );
    cx.update(|cx| {
        assert_eq!(
            other.setting::<crate::VaultSettings>(cx).auto_lock_minutes,
            32
        )
    });
}

#[gpui_kit::test]
async fn rejected_native_setting_write_keeps_old_service_delay_and_current_literal(
    cx: &mut TestAppContext,
) {
    let refused = tempfile::tempdir().unwrap();
    let store = nocterm_store::SettingsConfig::new(
        Default::default(),
        nocterm_settings::SettingsFile::new(refused.path()),
    );
    let (handle, _view, service, installed) = setup_with_store(cx, store);
    drain(cx);
    tab(handle, 3, cx);
    type_field(handle, "vault-auto-lock", "30", cx);
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    drain(cx);
    let barrier = cx.update(|cx| {
        installed
            .settings()
            .update::<crate::VaultSettings>(cx, |_| {})
    });
    drain(cx);
    barrier.await.unwrap();
    assert!(refused.path().is_dir());
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<crate::VaultSettings>(cx)
                .auto_lock_minutes,
            15
        );
        assert_eq!(installed.settings().revision(cx), 0);
    });
    assert!(service.auto_lock_updates().is_empty());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("vault-auto-lock-input").value(), Some("30"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn hiding_a_native_device_operation_clears_draft_and_blocks_late_unlock(cx: &mut TestAppContext) {
    let (handle, view, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    fingerprint(&service);
    service.hold_device_unlock();
    service.lock();
    drain(cx);
    tab(handle, 2, cx);
    click(handle, "vault-device-refresh", cx);
    drain(cx);
    tab(handle, 0, cx);
    type_field(handle, "vault-password", "unfinished draft", cx);
    click(handle, "vault-device-unlock", cx);
    drain(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| SettingsPage::on_deactivate(view, window, cx));
    })
    .unwrap();
    service.complete_device_unlock(Ok(()));
    drain(cx);
    assert!(!service.is_unlocked());
    service.fail_next_unlock(VaultError::Authentication);
    click(handle, "vault-submit", cx);
    drain(cx);
    assert_eq!(service.last_password().as_deref(), Some(""));
    assert!(!service.is_unlocked());
}

fn append_secret(
    handle: gpui_kit::AnyWindowHandle,
    id: &'static str,
    suffix: &str,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
        window.press("end", cx);
        window.input(suffix, cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn both_masked_controls_are_cleared_before_a_held_password_future_completes(
    cx: &mut TestAppContext,
) {
    use nocterm_vault::testing::{Gate, VaultOperation};
    let (handle, _view, service) = setup(cx);
    block_on(service.create(Secret::new("initial master password"))).unwrap();
    drain(cx);
    tab(handle, 2, cx);
    let gate = Gate::pending();
    service.hold_next(VaultOperation::ChangePassword, gate.clone());
    type_field(handle, "vault-password", "old password", cx);
    type_field(handle, "vault-confirm", "old password", cx);
    click(handle, "vault-submit", cx);
    drain(cx);
    assert_eq!(
        service.last_password().as_deref(),
        Some("initial master password")
    );
    // Append without selecting or clearing: a retained old native literal would be observable
    // at the next real port submission, without exposing a masked accessibility value.
    append_secret(handle, "vault-password", "new password", cx);
    append_secret(handle, "vault-confirm", "new password", cx);
    gate.release();
    drain(cx);
    assert_eq!(service.last_password().as_deref(), Some("old password"));
    click(handle, "vault-submit", cx);
    drain(cx);
    assert_eq!(service.last_password().as_deref(), Some("new password"));
}

#[gpui_kit::test]
fn late_credential_listing_cannot_repopulate_a_closed_page_generation(cx: &mut TestAppContext) {
    use nocterm_vault::testing::{Gate, VaultOperation};
    let (handle, view, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    block_on(service.put(
        None,
        "account".into(),
        CredentialBinding::Password {
            target: Target::new("test", "host", 22),
        },
        Secret::new("secret"),
    ))
    .unwrap();
    drain(cx);
    tab(handle, 1, cx);
    let gate = Gate::pending();
    service.hold_next(VaultOperation::List, gate.clone());
    click(handle, "vault-refresh", cx);
    drain(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| SettingsPage::on_close(view, window, cx));
    })
    .unwrap();
    gate.release();
    drain(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("vault-delete", 0usize)).is_none());
    })
    .unwrap();
    assert!(service.is_unlocked());
    assert_eq!(block_on(service.list()).unwrap().len(), 1);
}

#[gpui_kit::test]
fn accepted_delete_keeps_its_original_service_after_origin_window_closes(cx: &mut TestAppContext) {
    use nocterm_vault::testing::{Gate, VaultOperation};
    let (handle, view, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    block_on(service.put(
        None,
        "account".into(),
        CredentialBinding::Password {
            target: Target::new("test", "host", 22),
        },
        Secret::new("secret"),
    ))
    .unwrap();
    drain(cx);
    tab(handle, 1, cx);
    click(handle, "vault-refresh", cx);
    drain(cx);
    let gate = Gate::pending();
    service.hold_next(VaultOperation::Delete, gate.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("vault-delete", 0usize), cx);
    })
    .unwrap();
    drain(cx);
    assert_eq!(block_on(service.list()).unwrap().len(), 1);
    let weak = view.downgrade();
    cx.update_window(handle, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    drain(cx);
    assert!(weak.upgrade().is_none());
    let (_, _other_page, other) = setup(cx);
    block_on(other.create(Secret::new("other master password"))).unwrap();
    let other_id = block_on(other.put(
        None,
        "other account".into(),
        CredentialBinding::Password {
            target: Target::new("other", "host", 22),
        },
        Secret::new("other secret"),
    ))
    .unwrap();
    gate.release();
    drain(cx);
    assert!(block_on(service.list()).unwrap().is_empty());
    assert_eq!(block_on(other.list()).unwrap()[0].id, other_id);
    assert!(weak.upgrade().is_none());
}

fn blur_auto_lock(handle: gpui_kit::AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("vault-auto-lock-input").focused(), Some(true));
        window.blur(cx);
        window.render_frame(cx);
    })
    .unwrap();
    drain(cx);
}

#[gpui_kit::test]
fn auto_lock_waits_for_native_enter_or_blur_and_preserves_its_bounds(cx: &mut TestAppContext) {
    let (handle, _view, service, installed) = setup_with_store(
        cx,
        nocterm_store::SettingsConfig::in_memory(Default::default()),
    );
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    drain(cx);
    tab(handle, 3, cx);
    type_field(handle, "vault-auto-lock", "23", cx);
    drain(cx);
    cx.executor().advance_clock(Duration::from_millis(1200));
    drain(cx);
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<crate::VaultSettings>(cx)
                .auto_lock_minutes,
            15
        )
    });
    assert!(service.auto_lock_updates().is_empty());
    blur_auto_lock(handle, cx);
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<crate::VaultSettings>(cx)
                .auto_lock_minutes,
            23
        )
    });
    assert_eq!(
        service.auto_lock_updates().last(),
        Some(&Duration::from_secs(1380))
    );
    type_field(handle, "vault-auto-lock", "1441", cx);
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    drain(cx);
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<crate::VaultSettings>(cx)
                .auto_lock_minutes,
            23
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("vault-auto-lock-input").value(), Some("1441"));
    })
    .unwrap();
    type_field(handle, "vault-auto-lock", "1440", cx);
    blur_auto_lock(handle, cx);
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<crate::VaultSettings>(cx)
                .auto_lock_minutes,
            1440
        )
    });
    assert_eq!(
        service.auto_lock_updates().last(),
        Some(&Duration::from_secs(86400))
    );
}
