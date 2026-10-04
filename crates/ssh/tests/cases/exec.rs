use super::*;
use nocterm_session::{ExecError, ExecRequest};

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
