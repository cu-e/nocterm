use super::*;
use nocterm_vault::{DeviceCancellation, DeviceUnlockError, VaultBinding, VaultKey};
use std::sync::{Mutex, atomic::Ordering, mpsc};

struct ScanningDevice {
    device: EnrolledDevice,
    resume: Mutex<mpsc::Receiver<()>>,
}
impl DeviceUnlockProvider for ScanningDevice {
    fn probe(&self) -> Result<DeviceCapability, DeviceUnlockError> {
        self.device.probe()
    }
    fn enroll(
        &self,
        binding: VaultBinding,
        key: VaultKey,
        cancel: &DeviceCancellation,
    ) -> Result<Vec<u8>, DeviceUnlockError> {
        self.device.enroll(binding, key, cancel)
    }
    fn release(
        &self,
        _: VaultBinding,
        _: &[u8],
        cancel: &DeviceCancellation,
    ) -> Result<VaultKey, DeviceUnlockError> {
        for remaining in [3, 2, 1] {
            cancel.report_attempts_remaining(remaining);
            self.resume
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            cancel.check()?;
        }
        self.device.2.store(true, Ordering::SeqCst);
        cancel.report_attempts_remaining(0);
        Err(DeviceUnlockError::Locked)
    }
    fn remove(&self, binding: VaultBinding, token: &[u8]) -> Result<(), DeviceUnlockError> {
        self.device.remove(binding, token)
    }
    fn registration_state(
        &self,
        binding: VaultBinding,
        token: &[u8],
    ) -> Result<nocterm_vault::DeviceRegistrationState, DeviceUnlockError> {
        self.device.registration_state(binding, token)
    }
}
fn wait_for(cx: &mut TestAppContext, mut predicate: impl FnMut(&mut TestAppContext) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !predicate(cx) {
        assert!(
            std::time::Instant::now() < deadline,
            "UI update did not arrive"
        );
        cx.executor().tick();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
#[gpui_kit::test]
fn scan_progress_counts_down_and_locked_dialog_cannot_scan_again(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (resume, wait) = mpsc::channel();
    let device = Arc::new(ScanningDevice {
        device: EnrolledDevice::default(),
        resume: Mutex::new(wait),
    });
    let (_, _, service) =
        setup_with_device(cx, directory.path().join("vault"), Some(device.clone()));
    block_on(service.create(Secret::new("portable master password"))).unwrap();
    block_on(service.enable_device_unlock()).unwrap();
    service.lock();
    let (handle, prompt) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| crate::unlock::UnlockPrompt::new(service.clone(), window, cx))
        })
        .unwrap()
    });
    for remaining in [3, 2, 1] {
        wait_for(cx, |cx| {
            cx.update(|cx| prompt.read(cx).attempts_remaining == Some(remaining))
        });
        cx.update(|cx| {
            prompt.update(cx, |prompt, _| {
                prompt.scan_progress(0, 0);
                assert_eq!(
                    prompt.attempts_remaining,
                    Some(remaining),
                    "older scan progress is stale"
                );
            })
        });
        resume.send(()).unwrap();
    }
    wait_for(cx, |cx| cx.update(|cx| prompt.read(cx).device_locked));
    assert!(!service.is_unlocked());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-unlock-device", cx);
        assert!(!prompt.read(cx).scanning);
        prompt.update(cx, |prompt, _| {
            prompt.scan_progress(1, 2);
            assert_eq!(
                prompt.attempts_remaining,
                Some(0),
                "completed scan progress is stale"
            );
        });
    })
    .unwrap();
    drop(prompt);
    // A reopened dialog observes broker lockout before starting authentication.
    let (_, reopened) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| crate::unlock::UnlockPrompt::new(service.clone(), window, cx))
        })
        .unwrap()
    });
    drain(&service, cx);
    cx.update(|cx| {
        assert!(reopened.read(cx).device_locked);
        assert!(!reopened.read(cx).scanning);
    });
    block_on(service.unlock(Secret::new("portable master password"))).unwrap();
    block_on(service.probe_device_unlock()).unwrap();
    assert!(!device.device.2.load(Ordering::SeqCst));
}
