use super::*;

#[tokio::test]
async fn a_new_host_is_confirmed_once_and_remembered() {
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");

    let session = transport.open(sshd.request());
    let mut asked = 0;
    connect(&session, |prompt| match prompt {
        Prompt::UnknownHostKey {
            host,
            algorithm,
            fingerprint,
            reply,
        } => {
            asked += 1;
            assert_eq!(host, sshd.host_label());
            assert_eq!(algorithm, "ssh-ed25519");
            assert_eq!(fingerprint, sshd.host_fingerprint());
            reply.send(HostKeyDecision::AcceptAndRemember);
        }
        other => panic!("unexpected prompt: {other:?}"),
    })
    .await;
    assert_eq!(asked, 1);

    // The shell is live: `$((40+2))` is only ever 42 after it ran remotely.
    session.input("printf 'nocterm-%s\\n' $((40+2))\r");
    output_containing(&session, "nocterm-42").await;
    session.input("exit 3\r");
    assert_eq!(
        closed(&session, no_prompts).await,
        CloseReason::Exited(Some(3))
    );

    let again = transport.open(sshd.request());
    connect(&again, no_prompts).await;
    again.close();
    assert_eq!(closed(&again, no_prompts).await, CloseReason::ClosedByUser);
}

#[tokio::test]
async fn revoked_host_key_cannot_be_accepted_or_reach_authentication() {
    let Some(sshd) = Sshd::start() else { return };
    let key = fs::read_to_string(sshd.root().join("host_key.pub")).unwrap();
    fs::write(
        sshd.root().join("known_hosts"),
        format!(
            "{} {key}\n@revoked {} {key}",
            sshd.host_label(),
            sshd.host_label()
        ),
    )
    .unwrap();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    let reason = closed(&session, no_prompts).await;
    assert!(
        matches!(reason, CloseReason::Failed(SessionError::Other(message)) if message.contains("revoked"))
    );
}

#[tokio::test]
async fn malformed_host_trust_store_never_becomes_an_unknown_host_prompt() {
    let Some(sshd) = Sshd::start() else { return };
    fs::write(
        sshd.root().join("known_hosts"),
        format!("{} ssh-ed25519 invalid-key", sshd.host_label()),
    )
    .unwrap();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    let reason = closed(&session, no_prompts).await;
    assert!(
        matches!(reason, CloseReason::Failed(SessionError::Other(message)) if message.contains("cannot verify host keys"))
    );
}

#[tokio::test]
async fn rejecting_the_host_key_ends_the_session() {
    let Some(sshd) = Sshd::start() else { return };

    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    let reason = closed(&session, |prompt| match prompt {
        Prompt::UnknownHostKey { reply, .. } => reply.send(HostKeyDecision::Reject),
        other => panic!("unexpected prompt: {other:?}"),
    })
    .await;

    assert_eq!(
        reason,
        CloseReason::Failed(SessionError::HostKeyRejected {
            host: "127.0.0.1".into()
        })
    );
    assert!(!sshd.root().join("known_hosts").exists());
}

fn stale_key(sshd: &Sshd) -> String {
    keygen(&sshd.root().join("impostor"), "");
    let stale = fs::read_to_string(sshd.root().join("impostor.pub")).unwrap();
    let record = format!("# recorded long ago\n{} {stale}", sshd.host_label());
    fs::write(sshd.root().join("known_hosts"), &record).unwrap();
    record
}
async fn changed_prompt(session: &Session) -> Prompt {
    loop {
        match next_event(session).await {
            Event::Prompt(prompt @ Prompt::ChangedHostKey { .. }) => return prompt,
            Event::Connecting(nocterm_session::ConnectStage::Connecting) => {}
            event => panic!("authentication must wait for host verification: {event:?}"),
        }
    }
}

#[tokio::test]
async fn changed_host_can_connect_once_before_authentication_without_writing() {
    let Some(sshd) = Sshd::start() else { return };
    let before = stale_key(&sshd);
    let transport = sshd.transport("encrypted");
    for _ in 0..2 {
        let session = transport.open(sshd.request());
        let Prompt::ChangedHostKey {
            host,
            port,
            algorithm,
            fingerprint,
            old_fingerprints,
            known_hosts,
            line,
            replacement_error,
            reply,
        } = changed_prompt(&session).await
        else {
            unreachable!()
        };
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, sshd.port);
        assert_eq!(algorithm, "ssh-ed25519");
        assert_eq!(fingerprint, sshd.host_fingerprint());
        assert_eq!(old_fingerprints.len(), 1);
        assert_ne!(old_fingerprints[0], fingerprint);
        assert_eq!(known_hosts, sshd.root().join("known_hosts"));
        assert_eq!(line, 2);
        assert!(replacement_error.is_none());
        reply.send(HostKeyDecision::AcceptOnce);
        let mut authenticated = false;
        connect(&session, |prompt| match prompt {
            Prompt::Secret {
                reply,
                request: SecretRequest::KeyPassphrase { .. },
            } => {
                authenticated = true;
                reply.send(Some(Secret::new(PASSPHRASE)));
            }
            other => panic!("unexpected: {other:?}"),
        })
        .await;
        assert!(authenticated);
        assert_eq!(
            fs::read_to_string(sshd.root().join("known_hosts")).unwrap(),
            before
        );
        session.close();
        assert_eq!(
            closed(&session, no_prompts).await,
            CloseReason::ClosedByUser
        );
    }
}
#[tokio::test]
async fn rejecting_or_dropping_changed_host_prompt_cancels_without_writes() {
    let Some(sshd) = Sshd::start() else { return };
    let before = stale_key(&sshd);
    let transport = sshd.transport("encrypted");
    for reject in [true, false] {
        let session = transport.open(sshd.request());
        let Prompt::ChangedHostKey { reply, .. } = changed_prompt(&session).await else {
            unreachable!()
        };
        if reject {
            reply.send(HostKeyDecision::Reject);
        } else {
            drop(reply);
        }
        assert_eq!(
            closed(&session, no_prompts).await,
            CloseReason::Failed(SessionError::HostKeyRejected {
                host: "127.0.0.1".into()
            })
        );
        assert_eq!(
            fs::read_to_string(sshd.root().join("known_hosts")).unwrap(),
            before
        );
    }
}
#[tokio::test]
async fn changed_host_can_replace_app_trust_and_reconnect_without_prompt() {
    let Some(sshd) = Sshd::start() else { return };
    stale_key(&sshd);
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    let Prompt::ChangedHostKey {
        reply,
        replacement_error,
        ..
    } = changed_prompt(&session).await
    else {
        unreachable!()
    };
    assert!(replacement_error.is_none());
    reply.send(HostKeyDecision::AcceptAndRemember);
    connect(&session, no_prompts).await;
    let text = fs::read_to_string(sshd.root().join("known_hosts")).unwrap();
    assert!(text.starts_with("# recorded long ago\n"));
    assert!(
        text.contains(
            fs::read_to_string(sshd.root().join("host_key.pub"))
                .unwrap()
                .trim()
        )
    );
    session.close();
    closed(&session, no_prompts).await;
    let again = transport.open(sshd.request());
    connect(&again, no_prompts).await;
    again.close();
    closed(&again, no_prompts).await;
}
#[tokio::test]
async fn read_only_changed_key_can_connect_once_but_cannot_be_saved() {
    let Some(sshd) = Sshd::start() else { return };
    let before = stale_key(&sshd);
    let external = sshd.root().join("external_hosts");
    fs::rename(sshd.root().join("known_hosts"), &external).unwrap();
    let transport = SshTransport::new(SshConfig {
        known_hosts: sshd.root().join("known_hosts"),
        read_only_known_hosts: vec![external.clone()],
        identity_dir: Some(sshd.root().join("plain")),
        use_agent: false,
    })
    .unwrap();
    let session = transport.open(sshd.request());
    let Prompt::ChangedHostKey {
        replacement_error,
        reply,
        ..
    } = changed_prompt(&session).await
    else {
        unreachable!()
    };
    assert!(replacement_error.unwrap().contains("read-only"));
    reply.send(HostKeyDecision::AcceptOnce);
    connect(&session, no_prompts).await;
    session.close();
    closed(&session, no_prompts).await;
    assert_eq!(fs::read_to_string(&external).unwrap(), before);
    let attempt = transport.open(sshd.request());
    let Prompt::ChangedHostKey { reply, .. } = changed_prompt(&attempt).await else {
        unreachable!()
    };
    reply.send(HostKeyDecision::AcceptAndRemember);
    assert!(
        matches!(closed(&attempt, no_prompts).await, CloseReason::Failed(SessionError::Other(error)) if error.contains("read-only"))
    );
    assert!(!sshd.root().join("known_hosts").exists());
}
#[tokio::test]
async fn changed_trust_while_prompt_is_open_fails_before_authentication() {
    let Some(sshd) = Sshd::start() else { return };
    stale_key(&sshd);
    let transport = sshd.transport("encrypted");
    for decision in [
        HostKeyDecision::AcceptOnce,
        HostKeyDecision::AcceptAndRemember,
    ] {
        let session = transport.open(sshd.request());
        let Prompt::ChangedHostKey { reply, .. } = changed_prompt(&session).await else {
            unreachable!()
        };
        let path = sshd.root().join("known_hosts");
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("# externally changed\n");
        fs::write(&path, &text).unwrap();
        reply.send(decision);
        assert!(
            matches!(closed(&session, no_prompts).await, CloseReason::Failed(SessionError::Other(error)) if error.contains("changed while awaiting"))
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
    }
}

#[tokio::test]
async fn public_key_only_server_explains_password_auth_is_unavailable() {
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");
    let mut request = sshd.request();
    request.auth = Auth::Password;
    let session = transport.open(request);
    let reason = closed(&session, accept).await;
    assert!(
        matches!(reason, CloseReason::Failed(SessionError::Other(message)) if message.contains("does not allow password sign-in") && message.contains("Authentication to Key file") && message.contains("Private key"))
    );
}
#[tokio::test]
async fn public_key_only_server_distinguishes_a_rejected_key() {
    let Some(sshd) = Sshd::start() else { return };
    keygen(&sshd.root().join("unauthorized"), "");
    let transport = sshd.transport("empty");
    let mut request = sshd.request();
    request.auth = Auth::Key {
        path: sshd.root().join("unauthorized"),
    };
    let session = transport.open(request);
    let reason = closed(&session, accept).await;
    assert!(
        matches!(reason, CloseReason::Failed(SessionError::Other(message)) if message.contains("did not accept the offered SSH key") && message.contains("authorized_keys"))
    );
}

#[tokio::test]
async fn accepted_first_key_in_two_key_authentication_is_not_reported_as_rejected() {
    let Some(mut sshd) = Sshd::start() else {
        return;
    };
    sshd.process.kill().unwrap();
    sshd.process.wait().unwrap();
    let config_path = sshd.root().join("sshd_config");
    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str("AuthenticationMethods publickey,publickey\n");
    fs::write(&config_path, config).unwrap();
    sshd.process = Command::new(find_sshd().unwrap())
        .args(["-D", "-e", "-f"])
        .arg(&config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    sshd.wait_until_listening();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    let reason = closed(&session, accept).await;
    assert!(
        !matches!(&reason, CloseReason::Failed(SessionError::Other(message)) if message.contains("did not accept the offered SSH key")),
        "the first authorized key was accepted as one factor: {reason:?}"
    );
    assert!(
        matches!(reason, CloseReason::Failed(SessionError::Other(message))
        if message.contains("accepted an authentication factor")
            && message.contains("additional authorized SSH key"))
    );
}

#[tokio::test]
async fn revocation_while_unknown_prompt_is_open_cannot_fall_back_to_connecting_once() {
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("encrypted");
    for decision in [
        HostKeyDecision::AcceptOnce,
        HostKeyDecision::AcceptAndRemember,
    ] {
        let path = sshd.root().join("known_hosts");
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        let session = transport.open(sshd.request());
        let reply = loop {
            match next_event(&session).await {
                Event::Prompt(Prompt::UnknownHostKey { reply, .. }) => break reply,
                Event::Connecting(nocterm_session::ConnectStage::Connecting) => {}
                other => panic!("authentication must wait for trust: {other:?}"),
            }
        };
        let public = fs::read_to_string(sshd.root().join("host_key.pub")).unwrap();
        let revoked = format!("@revoked {} {public}", sshd.host_label());
        fs::write(&path, &revoked).unwrap();
        reply.send(decision);
        assert!(
            matches!(closed(&session, no_prompts).await, CloseReason::Failed(SessionError::Other(error)) if error.contains("changed while awaiting"))
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), revoked);
    }
}
