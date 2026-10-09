use std::time::{Duration, Instant};

use super::*;

fn collect(mut output: ExecOutput) -> String {
    block_on(async {
        let mut collected = Vec::new();
        while let Some(chunk) = output.next().await {
            collected.extend(chunk);
        }
        String::from_utf8(collected).unwrap()
    })
}

#[test]
fn runs_a_script_from_its_input() {
    let request = ExecRequest::new("sh")
        .arg("-s")
        .stdin("echo one; echo \"$((40 + 2))\"\n");
    let output = block_on(LocalExec.exec(request)).unwrap();
    assert_eq!(collect(output), "one\n42\n");
}

#[test]
fn dropping_the_output_stops_the_program_and_its_children() {
    let marker = tempfile::tempdir().unwrap();
    let file = marker.path().join("alive");
    let script = format!(
        "echo started; (sleep 0.3; touch '{}') & wait",
        file.display()
    );
    let request = ExecRequest::new("sh").arg("-c").arg(script);
    let mut output = block_on(LocalExec.exec(request)).unwrap();
    assert_eq!(block_on(output.next()).as_deref(), Some(&b"started\n"[..]));
    drop(output);
    thread::sleep(Duration::from_millis(600));
    assert!(!file.exists(), "the background child was stopped too");
}

#[test]
fn programs_report_their_exit_and_errors() {
    let request = ExecRequest::new("sh")
        .arg("-c")
        .arg("echo out; echo 'no such container' >&2; exit 3");
    let collected = block_on(block_on(LocalExec.exec(request)).unwrap().collect(1024));
    assert_eq!(collected.text(), "out\n");
    assert_eq!(collected.exit.status, Some(3));
    assert_eq!(collected.exit.error(), "no such container");
    let fine = block_on(LocalExec.exec(ExecRequest::new("true"))).unwrap();
    assert!(block_on(fine.exit()).success());
}

#[test]
fn programs_run_on_a_terminal_of_their_own() {
    use nocterm_session::{CloseReason, Event, PtySize};
    let session = LocalExec.terminal(TerminalRequest {
        program: ExecRequest::new("sh")
            .arg("-c")
            .arg("test -t 1 && echo on-a-tty; exit 4"),
        term: "xterm-256color".into(),
        size: PtySize::default(),
    });
    let mut output = String::new();
    let reason = block_on(async {
        loop {
            match session.next_event().await {
                Some(Event::Output(bytes)) => output.push_str(&String::from_utf8_lossy(&bytes)),
                Some(Event::Closed(reason)) => break reason,
                Some(_) => {}
                None => panic!("the session ended without saying why"),
            }
        }
    });
    assert!(output.contains("on-a-tty"), "{output:?}");
    assert_eq!(reason, CloseReason::Exited(Some(4)));
}

#[test]
fn a_missing_program_is_an_error() {
    let started = Instant::now();
    let error = block_on(LocalExec.exec(ExecRequest::new("/nonexistent/nocterm"))).unwrap_err();
    assert_eq!(error, ExecError::NotFound("/nonexistent/nocterm".into()));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn constructing_or_dropping_an_unpolled_exec_never_spawns() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unexpected");
    let request = ExecRequest::new("touch").arg(marker.to_string_lossy().to_string());
    let future = LocalExec.exec(request);
    thread::sleep(Duration::from_millis(50));
    assert!(!marker.exists());
    drop(future);
    thread::sleep(Duration::from_millis(50));
    assert!(!marker.exists());
}

#[test]
fn exited_leader_does_not_leave_descendants_or_hold_stdout_open() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unexpected");
    let request = ExecRequest::new("sh").arg("-c").arg(format!(
        "echo started; (sleep 1; touch '{}') & exit 7",
        marker.display()
    ));
    let started = Instant::now();
    let collected = block_on(block_on(LocalExec.exec(request)).unwrap().collect(1024));
    assert_eq!(collected.text(), "started\n");
    assert_eq!(collected.exit.status, Some(7));
    assert!(started.elapsed() < Duration::from_millis(900));
    thread::sleep(Duration::from_millis(1200));
    assert!(!marker.exists());
}

#[test]
fn cancellation_after_parent_exit_kills_a_child_that_closed_its_output() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unexpected");
    let request = ExecRequest::new("sh").arg("-c").arg(format!(
        "echo started; (exec >/dev/null 2>&1; sleep 0.3; touch '{}') & exit",
        marker.display()
    ));
    let output = block_on(LocalExec.exec(request)).unwrap();
    thread::sleep(Duration::from_millis(100));
    drop(output);
    thread::sleep(Duration::from_millis(500));
    assert!(!marker.exists());
}

#[test]
fn exit_observation_does_not_discard_buffered_output() {
    let request = ExecRequest::new("sh")
        .arg("-c")
        .arg("i=0; while [ $i -lt 4096 ]; do printf '1234567890\\n'; i=$((i+1)); done; exit 9");
    let collected = block_on(
        block_on(LocalExec.exec(request))
            .unwrap()
            .collect(128 * 1024),
    );
    assert_eq!(collected.stdout.len(), 4096 * 11);
    assert_eq!(collected.exit.status, Some(9));
}

#[test]
fn dropping_the_startup_process_guard_stops_the_whole_group() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unexpected");
    let request = ExecRequest::new("sh")
        .arg("-c")
        .arg(format!("(sleep 0.3; touch '{}') & wait", marker.display()));
    let process = process::Process::spawn(&request).unwrap();
    thread::sleep(Duration::from_millis(50));
    // This is also the cleanup path when pipe/worker setup fails.
    drop(process);
    thread::sleep(Duration::from_millis(500));
    assert!(!marker.exists());
}

#[test]
fn cancellation_interrupts_a_backpressured_output_worker_and_stops_descendants() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unexpected");
    let request = ExecRequest::new("sh").arg("-c").arg(format!(
        "(sleep 1; touch '{}') & while :; do printf '%01024d\\n' 0; done",
        marker.display()
    ));
    let mut output = block_on(LocalExec.exec(request)).unwrap();
    assert!(block_on(output.next()).is_some());
    // Leave stdout unread long enough to fill the bounded ExecOutput queue.
    thread::sleep(Duration::from_millis(100));
    drop(output);
    thread::sleep(Duration::from_millis(1200));
    assert!(
        !marker.exists(),
        "cancellation must bypass output backpressure"
    );
}
