use super::*;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "missing marker {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn shell_request(script: &str, directory: &Path) -> ProgramOpenRequest {
    ProgramOpenRequest::new(
        "/bin/sh".into(),
        vec!["-c".into(), script.into()],
        Some(directory.into()),
    )
    .unwrap()
}
fn one_slot() -> LocalProgramOpener {
    LocalProgramOpener {
        slots: Arc::new(LaunchSlots::new(1)),
    }
}
struct ReleaseOnDrop(std::path::PathBuf);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        let _ = fs::write(&self.0, b"release");
    }
}

#[test]
fn literal_arguments_native_directory_and_null_input_are_preserved() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("editor with spaces");
    fs::write(&script, "#!/bin/sh\nprintf '%s\\n' \"$PWD\" \"$1\" \"$2\" > opened\nif read input; then exit 9; fi\nprintf eof >> opened\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let request = ProgramOpenRequest::new(
        script.to_str().unwrap().into(),
        vec!["a b.txt".into(), "$(touch injected); *".into()],
        Some(directory.path().into()),
    )
    .unwrap();
    block_on(LocalProgramOpener::default().open(request)).unwrap();
    let marker = directory.path().join("opened");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let bytes = fs::read_to_string(&marker).unwrap_or_default();
        if bytes.ends_with("eof") {
            assert_eq!(
                bytes,
                format!(
                    "{}\na b.txt\n$(touch injected); *\neof",
                    directory.path().display()
                )
            );
            break;
        }
        assert!(Instant::now() < deadline, "editor did not finish writing");
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!directory.path().join("injected").exists());
}

#[test]
fn admission_remains_busy_until_the_accepted_child_is_reaped() {
    let directory = tempfile::tempdir().unwrap();
    let release = ReleaseOnDrop(directory.path().join("release"));
    let opener = one_slot();
    let request = shell_request(
        "touch started; while [ ! -f release ]; do sleep 0.01; done; touch exited",
        directory.path(),
    );
    block_on(opener.open(request)).unwrap();
    wait_for(&directory.path().join("started"));
    let next = shell_request("touch next", directory.path());
    assert!(matches!(
        block_on(opener.open(next.clone())),
        Err(ProgramOpenError::Busy { limit: 1 })
    ));
    drop(release);
    wait_for(&directory.path().join("exited"));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match block_on(opener.open(next.clone())) {
            Ok(()) => break,
            Err(ProgramOpenError::Busy { .. }) => {
                assert!(
                    Instant::now() < deadline,
                    "reaped child did not release capacity"
                );
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("unexpected launch failure: {error}"),
        }
    }
    wait_for(&directory.path().join("next"));
}

#[test]
fn cancelled_acknowledgement_and_dropped_opener_do_not_kill_owned_child() {
    let directory = tempfile::tempdir().unwrap();
    let release = ReleaseOnDrop(directory.path().join("release"));
    let opener = one_slot();
    let (ready, receipt) = oneshot::channel();
    let request = shell_request(
        "touch started; while [ ! -f release ]; do sleep 0.01; done; touch survived",
        directory.path(),
    );
    let mut owned_worker = None;
    spawn_worker(request, opener.slots.reserve().unwrap(), ready, |worker| {
        owned_worker = Some(worker);
        Ok(())
    })
    .unwrap();
    // Admission owns the worker before the acknowledgement can be cancelled.
    drop(receipt);
    drop(opener);
    let worker = thread::spawn(owned_worker.unwrap());
    wait_for(&directory.path().join("started"));
    assert!(!directory.path().join("survived").exists());
    drop(release);
    worker.join().unwrap();
    assert!(directory.path().join("survived").exists());
}

#[test]
fn unpolled_request_does_not_launch_a_physical_child() {
    let directory = tempfile::tempdir().unwrap();
    let opener = LocalProgramOpener::default();
    drop(opener.open(shell_request("touch unwanted", directory.path())));
    assert!(!directory.path().join("unwanted").exists());
}

#[test]
fn first_poll_transfers_child_ownership_beyond_receipt_and_opener_lifetimes() {
    let directory = tempfile::tempdir().unwrap();
    let release = ReleaseOnDrop(directory.path().join("release"));
    let opener = LocalProgramOpener::default();
    let mut receipt = opener.open(shell_request(
        "touch started; while [ ! -f release ]; do sleep 0.01; done; touch survived",
        directory.path(),
    ));
    let waker = futures::task::noop_waker();
    let mut context = std::task::Context::from_waker(&waker);
    if let std::task::Poll::Ready(result) = receipt.as_mut().poll(&mut context) {
        result.unwrap();
    }
    drop(receipt);
    drop(opener);
    wait_for(&directory.path().join("started"));
    assert!(!directory.path().join("survived").exists());
    drop(release);
    wait_for(&directory.path().join("survived"));
}

#[test]
fn external_program_outlives_its_launching_application_process() {
    const CHILD_DIRECTORY: &str = "NOCTERM_OPENER_TEST_DIRECTORY";
    if let Some(directory) = std::env::var_os(CHILD_DIRECTORY) {
        let request = shell_request(
            "touch started; while [ ! -f release ]; do sleep 0.01; done; touch survived",
            Path::new(&directory),
        );
        block_on(LocalProgramOpener::default().open(request)).unwrap();
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let release = ReleaseOnDrop(directory.path().join("release"));
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "program_opener::tests::native::external_program_outlives_its_launching_application_process", "--nocapture"])
        .env(CHILD_DIRECTORY, directory.path()).status().unwrap();
    assert!(status.success());
    wait_for(&directory.path().join("started"));
    assert!(!directory.path().join("survived").exists());
    drop(release);
    wait_for(&directory.path().join("survived"));
}
