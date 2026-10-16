use crate::VaultView;
use futures::executor::block_on;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_session::{Secret, Target};
use nocterm_ui::SettingsPage;
use nocterm_vault::{
    CredentialBinding, DeviceAvailability, DeviceCapability, DeviceUnlockError, VaultAccess,
    VaultError, testing::ScriptedVaultAccess,
};
use std::sync::Arc;
fn setup(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<VaultView>,
    Arc<ScriptedVaultAccess>,
) {
    let (handle, view, service, _) =
        setup_with_store(cx, nocterm_ui::SettingsStore::in_memory(Default::default()));
    (handle, view, service)
}
fn setup_with_store(
    cx: &mut TestAppContext,
    store: nocterm_ui::SettingsStore,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<VaultView>,
    Arc<ScriptedVaultAccess>,
    std::rc::Rc<crate::Installed>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        // The unlock dialog slides in on wall-clock time; a click whose press and release
        // land on different frames of that motion misses its button under load.
        cx.set_reduce_motion(true);
        let ready = nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), store, cx);
        let service = Arc::new(ScriptedVaultAccess::default());
        let installed = std::rc::Rc::new(crate::install(
            crate::Deps::new(service.clone()),
            &ready,
            cx,
        ));
        // Application ownership is proved by bootstrap integration; a view receives its handle directly.
        let (handle, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| VaultView::new(service.clone(), installed.settings().clone(), window, cx))
        })
        .unwrap();
        (handle, view, service, installed)
    })
}
fn drain(cx: &mut TestAppContext) {
    cx.run_until_parked();
}
fn type_field(
    handle: gpui_kit::AnyWindowHandle,
    field: &'static str,
    text: &str,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(field, cx);
        window.press("ctrl-a", cx);
        window.input(text, cx);
    })
    .unwrap();
}
fn click(handle: gpui_kit::AnyWindowHandle, id: &'static str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
}
fn tab(handle: gpui_kit::AnyWindowHandle, index: usize, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("vault-tabs").click(index, cx);
    })
    .unwrap();
}
fn fingerprint(service: &ScriptedVaultAccess) {
    service.set_device_capability(DeviceCapability {
        availability: DeviceAvailability::Available,
        label: "Fingerprint".into(),
        detail: String::new(),
        enabled: true,
        armed: true,
        session_only: true,
    });
}
#[gpui_kit::test]
fn embedded_form_checks_confirmation_before_create_and_reports_password_errors(
    cx: &mut TestAppContext,
) {
    let (handle, _, service) = setup(cx);
    drain(cx);
    type_field(handle, "vault-password", "long master password", cx);
    type_field(handle, "vault-confirm", "different", cx);
    click(handle, "vault-submit", cx);
    drain(cx);
    assert!(!service.exists());
    type_field(handle, "vault-confirm", "long master password", cx);
    click(handle, "vault-submit", cx);
    drain(cx);
    assert!(service.is_unlocked());
    click(handle, "vault-lock", cx);
    assert!(!service.is_unlocked());
    service.fail_next_unlock(VaultError::Authentication);
    type_field(handle, "vault-password", "wrong password", cx);
    click(handle, "vault-submit", cx);
    drain(cx);
    assert!(!service.is_unlocked());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("vault-message")
                .label()
                .unwrap()
                .contains("incorrect")
        );
    })
    .unwrap();
}
#[gpui_kit::test]
fn refresh_and_delete_buttons_update_credentials_saved_by_another_feature(cx: &mut TestAppContext) {
    let (handle, _, service) = setup(cx);
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
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("vault-delete", 0usize), cx);
    })
    .unwrap();
    drain(cx);
    assert!(block_on(service.list()).unwrap().is_empty());
}
#[gpui_kit::test]
fn switching_tabs_clears_master_password_drafts(cx: &mut TestAppContext) {
    let (handle, _, service) = setup(cx);
    drain(cx);
    type_field(handle, "vault-password", "typed master password", cx);
    type_field(handle, "vault-confirm", "typed master password", cx);
    tab(handle, 3, cx);
    tab(handle, 0, cx);
    click(handle, "vault-submit", cx);
    drain(cx);
    assert_eq!(service.last_password().as_deref(), Some(""));
}
#[gpui_kit::test]
fn page_deactivation_and_close_clear_drafts_without_revoking_shared_credentials(
    cx: &mut TestAppContext,
) {
    let (handle, view, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    drain(cx);
    tab(handle, 2, cx);
    for close in [false, true] {
        type_field(handle, "vault-password", "unfinished password", cx);
        type_field(handle, "vault-confirm", "unfinished password", cx);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                if close {
                    SettingsPage::on_close(view, window, cx)
                } else {
                    SettingsPage::on_deactivate(view, window, cx)
                }
            });
        })
        .unwrap();
        click(handle, "vault-submit", cx);
        drain(cx);
        assert_eq!(service.last_password().as_deref(), Some(""));
        assert!(service.is_unlocked());
    }
}
#[gpui_kit::test]
fn options_save_lock_delay_and_reject_out_of_range(cx: &mut TestAppContext) {
    use nocterm_ui::SettingsExt as _;
    let (handle, _, _) = setup(cx);
    drain(cx);
    tab(handle, 3, cx);
    type_field(handle, "vault-auto-lock", "0", cx);
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    drain(cx);
    assert_eq!(
        cx.update(|cx| cx.setting::<crate::VaultSettings>().auto_lock_minutes),
        15
    );
    type_field(handle, "vault-auto-lock", "30", cx);
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    drain(cx);
    assert_eq!(
        cx.update(|cx| cx.setting::<crate::VaultSettings>().auto_lock_minutes),
        30
    );
}
#[gpui_kit::test]
fn missing_broker_disables_native_controls_and_keeps_password_fallback(cx: &mut TestAppContext) {
    let (handle, _, service) = setup(cx);
    service.set_device_capability(DeviceCapability {
        availability: DeviceAvailability::BrokerMissing,
        label: "Fingerprint".into(),
        detail: "Install broker".into(),
        enabled: false,
        armed: false,
        session_only: true,
    });
    block_on(service.create(Secret::new("master password"))).unwrap();
    drain(cx);
    tab(handle, 2, cx);
    click(handle, "vault-device-refresh", cx);
    drain(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-device-enable", cx);
    })
    .unwrap();
    drain(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("vault-message").is_none());
    })
    .unwrap();
    service.lock();
    drain(cx);
    tab(handle, 0, cx);
    type_field(handle, "vault-password", "master password", cx);
    click(handle, "vault-submit", cx);
    drain(cx);
    assert!(service.is_unlocked());
}
#[gpui_kit::test]
fn enter_submits_password_and_keeps_unlock_dialog_error_visible(cx: &mut TestAppContext) {
    let (handle, _, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    service.lock();
    drain(cx);
    service.fail_next_unlock(VaultError::Authentication);
    cx.update_window(handle, |_, window, cx| {
        crate::unlock::open(service.clone(), window, cx)
    })
    .unwrap();
    cx.simulate_input(handle, "wrong password");
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    drain(cx);
    assert!(!service.is_unlocked());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("vault-unlock-error")
                .label()
                .unwrap()
                .contains("incorrect")
        );
    })
    .unwrap();
    cx.simulate_input(handle, "master password");
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    drain(cx);
    assert!(service.is_unlocked());
}
#[gpui_kit::test]
fn fingerprint_failure_shows_reason_and_allows_password_unlock(cx: &mut TestAppContext) {
    let (handle, _, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    fingerprint(&service);
    service.fail_device_unlock(DeviceUnlockError::Platform(
        "Fingerprint reader disconnected".into(),
    ));
    service.lock();
    cx.update_window(handle, |_, window, cx| {
        crate::unlock::open(service.clone(), window, cx)
    })
    .unwrap();
    drain(cx);
    assert!(!service.is_unlocked());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("vault-unlock-error")
                .label()
                .unwrap()
                .contains("disconnected")
        );
    })
    .unwrap();
    cx.simulate_input(handle, "master password");
    click(handle, "vault-unlock-submit", cx);
    drain(cx);
    assert!(service.is_unlocked());
}
#[gpui_kit::test]
fn unlock_dialog_accepts_enabled_fingerprint(cx: &mut TestAppContext) {
    let (handle, _, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    fingerprint(&service);
    service.lock();
    cx.update_window(handle, |_, window, cx| {
        crate::unlock::open(service.clone(), window, cx)
    })
    .unwrap();
    drain(cx);
    assert!(service.is_unlocked());
}
#[gpui_kit::test]
fn progress_counts_down_and_locked_dialog_refuses_another_scan(cx: &mut TestAppContext) {
    let (handle, _, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    fingerprint(&service);
    service.hold_device_unlock();
    service.lock();
    cx.update_window(handle, |_, window, cx| {
        crate::unlock::open(service.clone(), window, cx)
    })
    .unwrap();
    drain(cx);
    for remaining in [3, 2, 1] {
        service.report_progress(remaining);
        drain(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .find("vault-scan-progress")
                    .label()
                    .unwrap()
                    .contains(&remaining.to_string())
            );
        })
        .unwrap();
    }
    service.complete_device_unlock(Err(DeviceUnlockError::Locked.into()));
    drain(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-unlock-device", cx);
    })
    .unwrap();
    service.report_progress(29);
    drain(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("vault-scan-progress").is_none());
    })
    .unwrap();
    assert!(!service.is_unlocked());
}
#[gpui_kit::test]
fn cancel_during_scan_keeps_vault_locked_and_denies_late_completion(cx: &mut TestAppContext) {
    let (handle, _, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    fingerprint(&service);
    service.hold_device_unlock();
    service.lock();
    cx.update_window(handle, |_, window, cx| {
        crate::unlock::open(service.clone(), window, cx)
    })
    .unwrap();
    drain(cx);
    click(handle, "vault-unlock-cancel", cx);
    drain(cx);
    service.complete_device_unlock(Ok(()));
    drain(cx);
    assert!(!service.is_unlocked());
}

#[gpui_kit::test]
fn closing_before_capability_probe_finishes_prevents_a_late_native_scan(cx: &mut TestAppContext) {
    let (handle, _, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    fingerprint(&service);
    service.hold_device_probe();
    service.hold_device_unlock();
    service.lock();
    cx.update_window(handle, |_, window, cx| {
        crate::unlock::open(service.clone(), window, cx)
    })
    .unwrap();
    drain(cx);
    click(handle, "vault-unlock-cancel", cx);
    service.complete_device_probe();
    drain(cx);
    service.complete_device_unlock(Ok(()));
    drain(cx);
    assert!(!service.is_unlocked());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("vault-unlock-prompt").is_none());
    })
    .unwrap();
}

mod settings_authority;
