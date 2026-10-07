use super::*;
use nocterm_session::{ExecError, ExecRequest, PtySize, TerminalRequest};

#[tokio::test]
async fn programs_run_beside_the_shell_with_their_input() {
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let exec = session.exec().expect("SSH sessions run programs");

    let request = ExecRequest::new("sh")
        .arg("-s")
        .stdin("printf 'one\\n'; printf 'two %s\\n' \"$((1 + 1))\"\n");
    let mut output = exec.exec(request).await.unwrap();
    let mut collected = Vec::new();
    while let Some(chunk) = output.next().await {
        collected.extend(chunk);
    }
    assert_eq!(String::from_utf8(collected).unwrap(), "one\ntwo 2\n");

    // A program that never ends stops with its reader.
    let endless = ExecRequest::new("sh")
        .arg("-c")
        .arg("while :; do echo tick; sleep 0.1; done");
    let mut output = exec.exec(endless).await.unwrap();
    assert!(output.next().await.is_some());
    drop(output);

    session.close();
    closed(&session, no_prompts).await;
    let after = exec.exec(ExecRequest::new("true")).await.unwrap_err();
    assert_eq!(after, ExecError::Disconnected);
}

#[tokio::test]
async fn programs_report_their_exit_and_errors() {
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let exec = session.exec().expect("SSH sessions run programs");

    let request = ExecRequest::new("sh")
        .arg("-c")
        .arg("echo out; echo 'no such container' >&2; exit 3");
    let collected = exec.exec(request).await.unwrap().collect(1024).await;
    assert_eq!(collected.text(), "out\n");
    assert_eq!(collected.exit.status, Some(3));
    assert_eq!(collected.exit.error(), "no such container");

    let fine = exec.exec(ExecRequest::new("true")).await.unwrap();
    assert!(fine.exit().await.success());
    session.close();
    closed(&session, no_prompts).await;
}

#[tokio::test]
async fn programs_run_on_a_terminal_of_their_own() {
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let exec = session.exec().expect("SSH sessions run programs");

    let request = |script: &str| TerminalRequest {
        program: ExecRequest::new("sh").arg("-c").arg(script),
        term: "xterm-256color".into(),
        size: PtySize::default(),
    };
    let program = exec.terminal(request(
        "test -t 0 && echo on-a-tty; read line; echo got $line",
    ));
    connect(&program, no_prompts).await;
    output_containing(&program, "on-a-tty").await;
    program.input("hello\r");
    output_containing(&program, "got hello").await;
    assert_eq!(
        closed(&program, no_prompts).await,
        CloseReason::Exited(Some(0))
    );

    // The connection ending ends its programs, with a reason.
    let endless = exec.terminal(request("while :; do sleep 1; done"));
    connect(&endless, no_prompts).await;
    session.close();
    closed(&session, no_prompts).await;
    assert!(matches!(
        closed(&endless, no_prompts).await,
        CloseReason::Failed(SessionError::ConnectionLost(_))
    ));
}

#[tokio::test]
async fn programs_read_input_while_writing_more_than_a_window() {
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let exec = session.exec().expect("SSH sessions run programs");
    let request = ExecRequest::new("sh")
        .arg("-c")
        .arg("head -c 4194304 /dev/zero; wc -c")
        .stdin(vec![b'x'; 4 * 1024 * 1024]);
    let output = tokio::time::timeout(Duration::from_secs(10), exec.exec(request))
        .await
        .expect("startup must not wait for stdin")
        .unwrap();
    let collected = tokio::time::timeout(Duration::from_secs(10), output.collect(5 * 1024 * 1024))
        .await
        .expect("stdin and output must progress together");
    assert!(collected.exit.success());
    assert!(collected.text().ends_with("4194304\n"));
    session.close();
    closed(&session, no_prompts).await;
}

#[tokio::test]
async fn cancelled_queued_program_never_executes() {
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let exec = session.exec().expect("SSH sessions run programs");
    let mut running = Vec::new();
    for _ in 0..8 {
        running.push(
            exec.exec(ExecRequest::new("sleep").arg("20"))
                .await
                .unwrap(),
        );
    }
    let folder = tempfile::tempdir().unwrap();
    let marker = folder.path().join("should-not-exist");
    let queued = ExecRequest::new("touch").arg(marker.to_str().unwrap());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), exec.exec(queued))
            .await
            .is_err()
    );
    drop(running);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !marker.exists(),
        "a cancelled queued command must not reach the host"
    );
    session.close();
    closed(&session, no_prompts).await;
}
