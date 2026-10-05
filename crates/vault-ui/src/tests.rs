use crate::{VaultView, init_with_device_unlock, view::Tab};
use futures::executor::block_on;
use gpui_kit::{
    AppContext as _, Entity, Focusable as _, TestAppContext, WindowOptions,
    test::TestWindowExt as _,
};
use nocterm_session::Secret;
use nocterm_settings::Settings;
use nocterm_ui::SettingsStore;
use nocterm_vault::{
    DeviceAvailability, DeviceCapability, DeviceUnlockProvider, VaultError, VaultService,
};
use nocterm_workspace::SettingsPage;
use std::{path::PathBuf, sync::Arc};
fn setup(
    cx: &mut TestAppContext,
    path: PathBuf,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<VaultView>,
    Arc<VaultService>,
) {
    setup_with_device(cx, path, None)
}
fn setup_with_device(
    cx: &mut TestAppContext,
    path: PathBuf,
    device: Option<Arc<dyn DeviceUnlockProvider>>,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<VaultView>,
    Arc<VaultService>,
) {
    // The vault worker is a real thread; let its completions wake the test scheduler.
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Settings::default()),
            cx,
        );
        let service = init_with_device_unlock(path, device, cx).unwrap();
        let (handle, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| VaultView::new(window, cx))
        })
        .unwrap();
        (handle, view, service)
    })
}

fn drain(service: &VaultService, cx: &mut TestAppContext) {
    loop {
        // Finish queued vault work before polling futures on the test scheduler.
        match block_on(service.list()) {
            Ok(_) | Err(VaultError::Locked) => {}
            Err(error) => panic!("vault worker barrier failed: {error}"),
        }
        if !cx.executor().tick() {
            break;
        }
    }
}

struct MissingBroker;
impl DeviceUnlockProvider for MissingBroker {
    fn probe(&self) -> Result<DeviceCapability, nocterm_vault::DeviceUnlockError> {
        Ok(DeviceCapability {
            availability: DeviceAvailability::BrokerMissing,
            label: "Fingerprint".into(),
            detail: "Install the optional broker; password unlock still works.".into(),
            enabled: false,
            armed: false,
            session_only: true,
        })
    }
    fn enroll(
        &self,
        _: nocterm_vault::VaultBinding,
        _: nocterm_vault::VaultKey,
        _: &nocterm_vault::DeviceCancellation,
    ) -> Result<Vec<u8>, nocterm_vault::DeviceUnlockError> {
        panic!("unavailable device enrollment must not be called")
    }
    fn release(
        &self,
        _: nocterm_vault::VaultBinding,
        _: &[u8],
        _: &nocterm_vault::DeviceCancellation,
    ) -> Result<nocterm_vault::VaultKey, nocterm_vault::DeviceUnlockError> {
        panic!("unavailable device release must not be called")
    }
    fn remove(
        &self,
        _: nocterm_vault::VaultBinding,
        _: &[u8],
    ) -> Result<(), nocterm_vault::DeviceUnlockError> {
        panic!("unavailable device registration does not exist")
    }
}

#[gpui_kit::test]
fn missing_broker_disables_native_buttons_and_preserves_password_fallback(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (handle, view, service) = setup_with_device(
        cx,
        directory.path().join("vault"),
        Some(Arc::new(MissingBroker)),
    );
    block_on(service.probe_device_unlock()).unwrap();
    drain(&service, cx);
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            view.read(cx).device.as_ref().unwrap().availability,
            DeviceAvailability::BrokerMissing
        );
        view.update(cx, |view, cx| view.select_tab(Tab::Security, window, cx));
        window.render_frame(cx);
        window.click("vault-device-enable", cx);
        assert!(!view.read(cx).busy);
        view.update(cx, |view, cx| {
            view.password.update(cx, |input, cx| {
                input.set_value("portable master password", window, cx)
            });
            view.confirm.update(cx, |input, cx| {
                input.set_value("portable master password", window, cx)
            });
            view.submit(window, cx);
        });
    })
    .unwrap();
    block_on(service.list()).unwrap();
    drain(&service, cx);
    cx.update_window(handle, |_, window, cx| {
        assert!(service.is_unlocked());
        window.render_frame(cx);
        window.click("vault-device-enable", cx);
        assert!(!view.read(cx).busy);
        view.update(cx, |view, cx| view.lock(window, cx));
        assert!(!service.is_unlocked());
    })
    .unwrap();
    block_on(service.unlock(Secret::new("portable master password"))).unwrap();
    assert!(service.is_unlocked());
}
#[gpui_kit::test]
fn refresh_and_delete_buttons_update_credentials_saved_by_another_feature(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (handle, view, service) = setup(cx, directory.path().join("vault"));
    block_on(service.create(Secret::new("portable master password"))).unwrap();
    block_on(service.put(
        None,
        "host account".into(),
        nocterm_vault::CredentialBinding::Password {
            target: nocterm_session::Target::new("me", "host", 22),
        },
        Secret::new("saved account password"),
    ))
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.select_tab(Tab::Credentials, window, cx));
        window.render_frame(cx);
        window.click("vault-refresh", cx);
    })
    .unwrap();
    block_on(service.list()).unwrap();
    drain(&service, cx);
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).records.len(), 1);
        window.render_frame(cx);
        window.click(("vault-delete", 0usize), cx);
    })
    .unwrap();
    let records = block_on(service.list()).unwrap();
    assert!(records.is_empty());
    drain(&service, cx);
    assert!(cx.update(|cx| view.read(cx).records.is_empty()));
}

#[gpui_kit::test]
fn page_deactivation_clears_master_drafts_without_locking_the_shared_vault(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let (handle, view, service) = setup(cx, directory.path().join("vault"));
    block_on(service.create(Secret::new("portable master password"))).unwrap();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.password.update(cx, |input, cx| {
                input.set_value("unfinished replacement", window, cx)
            });
            view.confirm.update(cx, |input, cx| {
                input.set_value("unfinished confirmation", window, cx)
            });
            SettingsPage::on_deactivate(view, window, cx);
            assert!(view.password.read(cx).value().is_empty());
            assert!(view.confirm.read(cx).value().is_empty());
            assert!(
                service.is_unlocked(),
                "hiding the page does not revoke credentials in use"
            );
            view.password.update(cx, |input, cx| {
                input.set_value("closing replacement", window, cx)
            });
            let before = view.generation;
            SettingsPage::on_close(view, window, cx);
            assert!(view.password.read(cx).value().is_empty());
            assert_ne!(
                view.generation, before,
                "late UI results must be stale after close"
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn embedded_form_checks_confirmation_and_clears_fields_before_async_create(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let (handle, view, service) = setup(cx, directory.path().join("vault"));
    cx.update_window(handle, |_, window, cx| {
        window.focus(&view.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
    })
    .unwrap();
    drain(&service, cx);
    let view = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            view.update(cx, |view, cx| {
                view.password.update(cx, |input, cx| {
                    input.set_value("long master password", window, cx)
                });
                view.confirm
                    .update(cx, |input, cx| input.set_value("different", window, cx));
                view.submit(window, cx);
                assert!(!view.busy);
                assert!(view.message.as_ref().unwrap().1);
                assert!(!service.exists());
                view.confirm.update(cx, |input, cx| {
                    input.set_value("long master password", window, cx)
                });
                view.submit(window, cx);
                assert!(view.busy);
                assert!(view.password.read(cx).value().is_empty());
                assert!(view.confirm.read(cx).value().is_empty());
            });
            view
        })
        .unwrap();
    block_on(service.list()).unwrap(); // Worker barrier, never used in production UI.
    drain(&service, cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            assert!(service.is_unlocked());
            assert!(!view.busy);
            assert!(!view.message.as_ref().unwrap().1);
            view.lock(window, cx);
            assert!(!service.is_unlocked());
            assert!(view.records.is_empty());
            view.password.update(cx, |input, cx| {
                input.set_value("wrong password", window, cx)
            });
            view.submit(window, cx);
        })
    })
    .unwrap();
    let _ = block_on(service.list());
    drain(&service, cx);
    assert!(cx.update(|cx| {
        view.read(cx)
            .message
            .as_ref()
            .is_some_and(|(_, error)| *error)
    }));
    assert!(!service.is_unlocked());
}

#[gpui_kit::test]
fn options_save_the_lock_delay_and_reject_values_out_of_range(cx: &mut TestAppContext) {
    use nocterm_ui::ActiveSettings as _;
    let directory = tempfile::tempdir().unwrap();
    let (handle, view, _) = setup(cx, directory.path().join("vault"));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.select_tab(Tab::Options, window, cx);
            view.auto_lock
                .update(cx, |input, cx| input.set_value("0", window, cx));
            view.save_auto_lock(cx);
            assert!(view.auto_lock_error.is_some());
            view.auto_lock
                .update(cx, |input, cx| input.set_value("30", window, cx));
            view.save_auto_lock(cx);
            assert!(view.auto_lock_error.is_none());
        });
        assert_eq!(cx.settings().vault.auto_lock_minutes, 30);
    })
    .unwrap();
}

#[gpui_kit::test]
fn switching_tabs_clears_master_password_drafts(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (handle, view, _) = setup(cx, directory.path().join("vault"));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.password
                .update(cx, |input, cx| input.set_value("typed", window, cx));
            view.select_tab(Tab::Options, window, cx);
            assert!(view.password.read(cx).value().is_empty());
        });
    })
    .unwrap();
}

/// A device that is enrolled and releases the stored key without a prompt.
#[derive(Default)]
struct EnrolledDevice(std::sync::Mutex<Option<nocterm_vault::VaultKey>>);
impl DeviceUnlockProvider for EnrolledDevice {
    fn probe(&self) -> Result<DeviceCapability, nocterm_vault::DeviceUnlockError> {
        Ok(DeviceCapability {
            availability: DeviceAvailability::Available,
            label: "Fingerprint".into(),
            detail: String::new(),
            enabled: false,
            armed: false,
            session_only: true,
        })
    }
    fn enroll(
        &self,
        _: nocterm_vault::VaultBinding,
        key: nocterm_vault::VaultKey,
        _: &nocterm_vault::DeviceCancellation,
    ) -> Result<Vec<u8>, nocterm_vault::DeviceUnlockError> {
        *self.0.lock().unwrap() = Some(key);
        Ok(vec![1])
    }
    fn release(
        &self,
        _: nocterm_vault::VaultBinding,
        _: &[u8],
        _: &nocterm_vault::DeviceCancellation,
    ) -> Result<nocterm_vault::VaultKey, nocterm_vault::DeviceUnlockError> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .ok_or(nocterm_vault::DeviceUnlockError::Invalidated)
    }
    fn remove(
        &self,
        _: nocterm_vault::VaultBinding,
        _: &[u8],
    ) -> Result<(), nocterm_vault::DeviceUnlockError> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
    fn registered(&self, _: nocterm_vault::VaultBinding, _: &[u8]) -> bool {
        self.0.lock().unwrap().is_some()
    }
}

#[gpui_kit::test]
fn unlock_dialog_offers_and_accepts_an_enabled_fingerprint(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (_, _, service) = setup_with_device(
        cx,
        directory.path().join("vault"),
        Some(Arc::new(EnrolledDevice::default())),
    );
    block_on(service.create(Secret::new("portable master password"))).unwrap();
    block_on(service.enable_device_unlock()).unwrap();
    service.lock();
    let (handle, prompt) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| crate::unlock::UnlockPrompt::new(service.clone(), window, cx))
        })
        .unwrap()
    });
    // The prompt probes the device, then starts device unlock by itself.
    block_on(service.probe_device_unlock()).unwrap();
    drain(&service, cx);
    assert!(service.is_unlocked());
    cx.update(|cx| assert_eq!(prompt.read(cx).device.as_deref(), Some("Fingerprint")));
    // The fingerprint button beside the password starts it again.
    service.lock();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-unlock-device", cx);
        assert!(prompt.read(cx).scanning);
    })
    .unwrap();
    block_on(service.probe_device_unlock()).unwrap();
    drain(&service, cx);
    assert!(service.is_unlocked());
    cx.update(|cx| assert!(!prompt.read(cx).scanning && prompt.read(cx).error.is_none()));
}

#[gpui_kit::test]
fn enter_in_the_unlock_dialog_submits_the_password_and_keeps_errors_visible(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let (handle, _, service) = setup(cx, directory.path().join("vault"));
    block_on(service.create(Secret::new("portable master password"))).unwrap();
    service.lock();
    cx.update_window(handle, |_, window, cx| {
        crate::unlock::open(window, cx);
        window.render_frame(cx);
        assert!(window.find("vault-unlock-prompt").visible());
    })
    .unwrap();
    // A wrong password leaves the dialog open with its error.
    cx.simulate_input(handle, "wrong master password");
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    drain(&service, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("vault-unlock-prompt").visible());
    })
    .unwrap();
    assert!(!service.is_unlocked());
    cx.simulate_input(handle, "portable master password");
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    drain(&service, cx);
    assert!(service.is_unlocked());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("vault-unlock-prompt").is_none());
    })
    .unwrap();
}
