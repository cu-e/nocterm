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
