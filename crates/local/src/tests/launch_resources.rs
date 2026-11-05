//! Native launch resources remain owned by the one physical PTY worker.
use super::*;

#[cfg(unix)]
#[test]
fn pty_launch_resource_survives_original_owner_until_child_exit() {
    let directory = Arc::new(tempfile::tempdir().unwrap());
    let path = directory.path().to_owned();
    std::fs::write(path.join("asset"), b"retained").unwrap();
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec![
            "-c".into(),
            "test -r asset && printf 'resource-ready\n'; read -r line; cat asset".into(),
        ],
        cwd: Some(path.to_str().unwrap().into()),
        integration: false,
        ..Default::default()
    };
    let session = open_scoped_retained(
        launch,
        true,
        "xterm".into(),
        PtySize::default(),
        Default::default(),
        Some(directory.clone()),
    );
    drop(directory);
    let rx = events(&session);
    let mut output = Vec::new();
    while !String::from_utf8_lossy(&output).contains("resource-ready\r\n") {
        match next(&rx) {
            Event::Output(bytes) => output.extend(bytes),
            Event::Closed(reason) => panic!("child closed before ready: {reason:?}"),
            _ => {}
        }
    }
    assert!(path.join("asset").exists());
    assert!(session.input(b"continue\n".to_vec()));
    loop {
        match next(&rx) {
            Event::Output(bytes) => output.extend(bytes),
            Event::Closed(reason) => {
                assert_eq!(reason, CloseReason::Exited(Some(0)));
                break;
            }
            _ => {}
        }
    }
    assert!(String::from_utf8_lossy(&output).contains("retained"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        !path.exists(),
        "worker must release its launch resource after actual exit"
    );
}

#[test]
fn closing_a_session_during_native_preparation_never_launches_the_child() {
    use nocterm_session::testing::ScriptedLaunchPreparation;
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("launched");
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec![
            "-c".into(),
            "printf launched > \"$1\"".into(),
            "test".into(),
            marker.to_string_lossy().into_owned(),
        ],
        integration: false,
        ..Default::default()
    };
    let preparation = Arc::new(ScriptedLaunchPreparation::default());
    let transport = LocalTransport::prepared(launch.clone(), Some(preparation.clone()));
    let session = transport.open(request(launch));
    let rx = events(&session);
    block_on(preparation.started());
    session.close();
    preparation.release();
    assert!(matches!(
        next(&rx),
        Event::Closed(CloseReason::ClosedByUser)
    ));
    assert!(!marker.exists());
}
