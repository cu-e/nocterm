//! A child-exit wake is independent from slave EOF and preserves the leader's tail.
use super::*;
use std::time::Instant;

#[test]
fn an_exited_leader_closes_after_its_tail_even_when_a_child_holds_the_slave() {
    let directory = tempfile::tempdir().unwrap();
    let child_file = directory.path().join("child");
    let ready_file = directory.path().join("ready");
    let launch = ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec![
            "-c".into(),
            "(trap '' HUP; printf ready > \"$2\"; exec sleep 30) & printf '%s\\n' \"$!\" > \"$1\"; while [ ! -e \"$2\" ]; do sleep 0.001; done; printf leader-tail; exit 7"
                .into(),
            "leader-test".into(),
            child_file.to_string_lossy().into_owned(),
            ready_file.to_string_lossy().into_owned(),
        ],
        integration: false,
        ..Default::default()
    };
    let session = LocalTransport(launch.clone()).open(request(launch));
    let events = events(&session);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut output = Vec::new();
    let reason = loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Event::Output(bytes)) => output.extend(bytes),
            Ok(Event::Closed(reason)) => break Some(reason),
            Ok(_) => {}
            Err(_) => break None,
        }
    };
    // This intentionally orphaned descendant is owned by the fixture. The
    // transport must finish before it is removed, without waiting for slave EOF.
    if let Ok(pid) = std::fs::read_to_string(child_file) {
        let pid = nix::unistd::Pid::from_raw(pid.trim().parse().unwrap());
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
    }
    session.close();
    assert_eq!(reason, Some(CloseReason::Exited(Some(7))));
    assert_eq!(output, b"leader-tail");
}
