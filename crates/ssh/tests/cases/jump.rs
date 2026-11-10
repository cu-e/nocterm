//! Sessions that reach the destination through one or more jump hosts.
//!
//! Hop `A` is the jump host and `B` the destination, both real `sshd`s. Every
//! hop is signed in with `A`'s plain key, so `B` authorizes it.
use super::*;
use nocterm_session::{ExecRequest, JumpHost, PromptHop};
#[path = "../support/relay.rs"]
mod relay;
use relay::{proxy_listener, relay_to};

fn target_of(sshd: &Sshd) -> Target {
    Target::new(current_user(), "127.0.0.1", sshd.port())
}

fn jump_to(sshd: &Sshd) -> JumpHost {
    JumpHost {
        target: target_of(sshd),
        auth: Auth::Auto,
    }
}

/// A jump host, a destination that trusts the same key, and the request for the latter.
fn chain(jump_config: &str) -> Option<(Sshd, Sshd, ConnectRequest)> {
    let jump = Sshd::start_with(jump_config)?;
    let destination = Sshd::start()?;
    destination.authorize(&jump);
    let mut request = destination.request();
    request.jumps = vec![jump_to(&jump)];
    Some((jump, destination, request))
}

/// Remembers every host key asked about, with the hop it was asked for.
fn remember_all(asked: &mut Vec<(Option<PromptHop>, String)>) -> impl FnMut(Prompt) + '_ {
    |prompt| match prompt {
        Prompt::UnknownHostKey {
            host, hop, reply, ..
        } => {
            asked.push((hop, host));
            reply.send(HostKeyDecision::AcceptAndRemember);
        }
        other => panic!("unexpected prompt: {other:?}"),
    }
}

#[tokio::test]
async fn an_ordinary_direct_session_starts_in_a_literal_folder_with_spaces_and_an_apostrophe() {
    ordinary_folder(0).await;
}

#[tokio::test]
async fn an_ordinary_one_jump_session_starts_in_its_destination_folder() {
    ordinary_folder(1).await;
}

#[tokio::test]
async fn an_ordinary_two_jump_session_preserves_route_order_and_its_destination_folder() {
    ordinary_folder(2).await;
}

async fn ordinary_folder(hops: usize) {
    let mut servers = Vec::new();
    for _ in 0..=hops {
        let Some(server) = Sshd::start() else { return };
        servers.push(server);
    }
    let first = &servers[0];
    for server in &servers[1..] {
        server.authorize(first);
    }
    let destination = servers.last().unwrap();
    let folder = destination.root().join("ordinary folder's code");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("inside.txt"), b"inside").unwrap();
    fs::write(destination.root().join("outside.txt"), b"outside").unwrap();
    let mut request = destination.request();
    request.jumps = servers[..hops].iter().map(jump_to).collect();
    request.launch = nocterm_session::ShellLaunch {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), "printf 'cwd=%s\\n' \"$(pwd -P)\"; printf 'destination=%s\\n' \"$SSH_CONNECTION\"; printf ordinary-folder-ready; exec /bin/sh".into()],
        cwd: Some(folder.to_str().unwrap().into()),
        integration: false,
        ..Default::default()
    };
    let relay = if hops > 0 {
        let (listener, proxy) = proxy_listener().await;
        request.proxy = proxy;
        Some(relay_to(first.port(), listener))
    } else {
        None
    };
    let transport = first.transport("plain");
    let session = transport.open(request);
    let mut asked = Vec::new();
    connect(&session, remember_all(&mut asked)).await;
    let expected: Vec<_> = servers
        .iter()
        .enumerate()
        .map(|(index, server)| {
            (
                (index < hops).then(|| PromptHop {
                    index,
                    target: target_of(server),
                }),
                server.host_label(),
            )
        })
        .collect();
    assert_eq!(asked, expected);
    assert_ordinary_folder(&session, destination, &folder).await;
    if let Some(relay) = relay {
        assert_eq!(
            timeout(Duration::from_secs(2), relay)
                .await
                .unwrap()
                .unwrap(),
            format!("CONNECT 127.0.0.1:{} HTTP/1.1", first.port())
        );
    }
}

async fn assert_ordinary_folder(session: &Session, destination: &Sshd, folder: &Path) {
    let output = output_containing(session, "ordinary-folder-ready").await;
    let physical = fs::canonicalize(folder).unwrap();
    assert!(
        output.contains(&format!("cwd={}", physical.display())),
        "{output:?}"
    );
    assert!(
        output.contains(&format!("127.0.0.1 {}", destination.port())),
        "{output:?}"
    );
    let files = session.fs().unwrap();
    assert!(
        files
            .read_dir(folder.to_str().unwrap())
            .await
            .unwrap()
            .iter()
            .any(|entry| entry.name == "inside.txt")
    );
    assert!(
        files
            .read_dir(destination.root().to_str().unwrap())
            .await
            .unwrap()
            .iter()
            .any(|entry| entry.name == "outside.txt")
    );
    session.close();
    assert_eq!(closed(session, no_prompts).await, CloseReason::ClosedByUser);
}

#[tokio::test]
async fn chain_reaches_the_destination_for_terminal_files_and_programs() {
    let Some((jump, destination, mut request)) = chain("") else {
        return;
    };
    let (listener, proxy) = proxy_listener().await;
    request.proxy = proxy;
    let relay = relay_to(jump.port(), listener);

    let transport = jump.transport("plain");
    let session = transport.open(request);
    let mut asked = Vec::new();
    connect(&session, remember_all(&mut asked)).await;
    assert_eq!(
        asked,
        [
            (
                Some(PromptHop {
                    index: 0,
                    target: target_of(&jump)
                }),
                jump.host_label()
            ),
            (None, destination.host_label()),
        ]
    );

    session.input("printf 'conn=%s\\n' \"$SSH_CONNECTION\"\r");
    output_containing(&session, &format!("127.0.0.1 {}", destination.port())).await;
    assert!(session.fs().unwrap().home().await.unwrap().starts_with('/'));
    let program = ExecRequest::new("sh")
        .arg("-c")
        .arg("printf 'program=%s\\n' \"$SSH_CONNECTION\"");
    let collected = session
        .exec()
        .unwrap()
        .exec(program)
        .await
        .unwrap()
        .collect(1024)
        .await;
    assert!(
        collected
            .text()
            .contains(&format!("127.0.0.1 {}", destination.port()))
    );

    let recorded = fs::read_to_string(jump.root().join("known_hosts")).unwrap();
    assert!(recorded.contains(&jump.host_label()));
    assert!(recorded.contains(&destination.host_label()));
    session.close();
    closed(&session, no_prompts).await;
    // The proxy carried only the first connection, to the jump host.
    let connect_line = timeout(Duration::from_secs(2), relay)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        connect_line,
        format!("CONNECT 127.0.0.1:{} HTTP/1.1", jump.port())
    );
}

#[tokio::test]
async fn forwarding_disabled_on_the_jump_host_is_reported_against_that_hop() {
    let Some((jump, _destination, request)) = chain("AllowTcpForwarding no\n") else {
        return;
    };
    let transport = jump.transport("plain");
    let session = transport.open(request);
    let reason = closed(&session, accept).await;
    let CloseReason::Failed(SessionError::JumpHost { hop, host, source }) = reason else {
        panic!("expected a jump host failure, got {reason:?}");
    };
    assert_eq!(hop, 0);
    assert_eq!(host, target_of(&jump).to_string());
    assert!(matches!(*source, SessionError::Unreachable { .. }));
}

#[tokio::test]
async fn closing_the_session_closes_every_hop() {
    let Some((jump, _destination, mut request)) = chain("") else {
        return;
    };
    let (listener, proxy) = proxy_listener().await;
    request.proxy = proxy;
    let relay = relay_to(jump.port(), listener);
    let transport = jump.transport("plain");
    let session = transport.open(request);
    connect(&session, accept).await;

    session.close();
    assert_eq!(
        closed(&session, no_prompts).await,
        CloseReason::ClosedByUser
    );
    timeout(Duration::from_secs(2), relay)
        .await
        .expect("the connection to the jump host was left open")
        .unwrap();
}

#[tokio::test]
async fn rejecting_a_jump_host_key_stops_before_the_destination() {
    let Some((jump, _destination, request)) = chain("") else {
        return;
    };
    let transport = jump.transport("plain");
    let session = transport.open(request);
    let mut prompts = 0;
    let reason = closed(&session, |prompt| {
        prompts += 1;
        match prompt {
            Prompt::UnknownHostKey { reply, .. } => reply.send(HostKeyDecision::Reject),
            other => panic!("unexpected prompt: {other:?}"),
        }
    })
    .await;
    assert_eq!(prompts, 1, "the destination must not be asked about");
    assert!(matches!(
        reason,
        CloseReason::Failed(SessionError::JumpHost { hop: 0, source, .. })
            if matches!(*source, SessionError::HostKeyRejected { .. })
    ));
}

#[tokio::test]
async fn a_jump_host_requiring_a_key_the_user_lacks_fails_that_hop() {
    let Some((jump, _destination, request)) = chain("") else {
        return;
    };
    let transport = jump.transport("empty");
    let session = transport.open(request);
    let reason = closed(&session, accept).await;
    assert!(matches!(
        reason,
        CloseReason::Failed(SessionError::JumpHost { hop: 0, source, .. })
            if matches!(&*source, SessionError::Other(message) if message.contains("requires an SSH key"))
    ));
}

#[tokio::test]
async fn cancelling_at_a_later_hop_prompt_closes_earlier_hops() {
    let (Some(first), Some(second)) = (Sshd::start(), Sshd::start()) else {
        return;
    };
    second.authorize(&first);
    let (listener, proxy) = proxy_listener().await;
    let relay = relay_to(first.port(), listener);
    // The route is first, second, first: the second hop's key is the open question.
    let mut request = first.request();
    request.jumps = vec![jump_to(&first), jump_to(&second)];
    request.proxy = proxy;
    let transport = first.transport("plain");
    let session = transport.open(request);
    let pending = loop {
        match next_event(&session).await {
            Event::Prompt(Prompt::UnknownHostKey {
                hop: Some(PromptHop { index: 0, .. }),
                reply,
                ..
            }) => reply.send(HostKeyDecision::AcceptOnce),
            Event::Prompt(Prompt::UnknownHostKey {
                hop: Some(PromptHop { index: 1, .. }),
                reply,
                ..
            }) => break reply,
            Event::Closed(reason) => panic!("closed early: {reason:?}"),
            _ => {}
        }
    };
    session.close();
    assert_eq!(
        closed(&session, no_prompts).await,
        CloseReason::ClosedByUser
    );
    drop(pending);
    timeout(Duration::from_secs(2), relay)
        .await
        .expect("the earlier hops were left open")
        .unwrap();
}

#[tokio::test]
async fn a_host_listed_twice_is_asked_about_once_when_remembered() {
    let Some(sshd) = Sshd::start() else { return };
    let mut request = sshd.request();
    request.jumps = vec![jump_to(&sshd)];
    let transport = sshd.transport("plain");
    let session = transport.open(request);
    let mut asked = Vec::new();
    connect(&session, remember_all(&mut asked)).await;
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0.as_ref().map(|hop| hop.index), Some(0));
    session.close();
    closed(&session, no_prompts).await;
}

/// Lets the passphrase-protected key of `keys_of` sign in to `sshd` too.
fn authorize_encrypted(sshd: &Sshd, keys_of: &Sshd) {
    let key = fs::read_to_string(keys_of.root().join("encrypted/id_ed25519.pub")).unwrap();
    let path = sshd.root().join("authorized_keys");
    let mut keys = fs::read_to_string(&path).unwrap();
    keys.push_str(&key);
    fs::write(path, keys).unwrap();
}

fn hop_at(index: usize, sshd: &Sshd) -> Option<PromptHop> {
    Some(PromptHop {
        index,
        target: target_of(sshd),
    })
}

#[tokio::test]
async fn a_chain_of_two_jump_hosts_labels_each_hop_and_reaches_the_destination() {
    let (Some(first), Some(second), Some(destination)) =
        (Sshd::start(), Sshd::start(), Sshd::start())
    else {
        return;
    };
    second.authorize(&first);
    destination.authorize(&first);
    let (listener, proxy) = proxy_listener().await;
    let relay = relay_to(first.port(), listener);
    let mut request = destination.request();
    request.jumps = vec![jump_to(&first), jump_to(&second)];
    request.proxy = proxy;

    let transport = first.transport("plain");
    let session = transport.open(request);
    let mut asked = Vec::new();
    connect(&session, remember_all(&mut asked)).await;
    assert_eq!(
        asked,
        [
            (hop_at(0, &first), first.host_label()),
            (hop_at(1, &second), second.host_label()),
            (None, destination.host_label()),
        ]
    );

    session.input("printf 'conn=%s\\n' \"$SSH_CONNECTION\"\r");
    output_containing(&session, &format!("127.0.0.1 {}", destination.port())).await;
    session.close();
    assert_eq!(
        closed(&session, no_prompts).await,
        CloseReason::ClosedByUser
    );
    let connect_line = timeout(Duration::from_secs(2), relay)
        .await
        .expect("the connection to the first jump host was left open")
        .unwrap();
    assert_eq!(
        connect_line,
        format!("CONNECT 127.0.0.1:{} HTTP/1.1", first.port())
    );
}

#[tokio::test]
async fn forwarding_disabled_on_a_later_jump_host_is_reported_against_that_hop() {
    let (Some(first), Some(second), Some(destination)) = (
        Sshd::start(),
        Sshd::start_with("AllowTcpForwarding no\n"),
        Sshd::start(),
    ) else {
        return;
    };
    second.authorize(&first);
    destination.authorize(&first);
    let (listener, proxy) = proxy_listener().await;
    let relay = relay_to(first.port(), listener);
    let mut request = destination.request();
    request.jumps = vec![jump_to(&first), jump_to(&second)];
    request.proxy = proxy;
    let transport = first.transport("plain");
    let session = transport.open(request);
    let reason = closed(&session, accept).await;
    timeout(Duration::from_secs(2), relay)
        .await
        .expect("the first jump host was left connected after the route failed")
        .unwrap();
    let CloseReason::Failed(SessionError::JumpHost { hop, host, source }) = reason else {
        panic!("expected a jump host failure, got {reason:?}");
    };
    assert_eq!(
        hop, 1,
        "the second jump host refused to reach the destination"
    );
    assert_eq!(host, target_of(&second).to_string());
    assert!(matches!(*source, SessionError::Unreachable { .. }));
}

#[tokio::test]
async fn a_changed_jump_host_key_is_put_to_the_user_for_that_hop() {
    let Some((jump, destination, request)) = chain("") else {
        return;
    };
    keygen(&jump.root().join("impostor"), "");
    let stale = fs::read_to_string(jump.root().join("impostor.pub")).unwrap();
    fs::write(
        jump.root().join("known_hosts"),
        format!("{} {stale}", jump.host_label()),
    )
    .unwrap();
    let transport = jump.transport("plain");
    let session = transport.open(request);
    let mut changed_for = None;
    let reason = closed(&session, |prompt| match prompt {
        Prompt::ChangedHostKey { hop, reply, .. } => {
            changed_for = Some(hop);
            reply.send(HostKeyDecision::Reject);
        }
        other => panic!("unexpected prompt: {other:?}"),
    })
    .await;
    assert_eq!(changed_for, Some(hop_at(0, &jump)));
    assert!(matches!(
        reason,
        CloseReason::Failed(SessionError::JumpHost { hop: 0, source, .. })
            if matches!(*source, SessionError::HostKeyRejected { .. })
    ));
    assert!(!destination.root().join("known_hosts").exists());
}

#[tokio::test]
async fn rejecting_the_destination_key_after_the_jump_host_is_not_blamed_on_a_hop() {
    let Some((jump, destination, request)) = chain("") else {
        return;
    };
    let transport = jump.transport("plain");
    let session = transport.open(request);
    let reason = closed(&session, |prompt| match prompt {
        Prompt::UnknownHostKey {
            hop: Some(_),
            reply,
            ..
        } => reply.send(HostKeyDecision::AcceptOnce),
        Prompt::UnknownHostKey {
            hop: None, reply, ..
        } => reply.send(HostKeyDecision::Reject),
        other => panic!("unexpected prompt: {other:?}"),
    })
    .await;
    assert_eq!(
        reason,
        CloseReason::Failed(SessionError::HostKeyRejected {
            host: destination.request().target.host
        })
    );
}

#[tokio::test]
async fn key_passphrases_are_asked_for_each_host_and_labelled_with_its_hop() {
    let Some((jump, destination, request)) = chain("") else {
        return;
    };
    authorize_encrypted(&destination, &jump);
    let transport = jump.transport("encrypted");
    let session = transport.open(request);
    let mut passphrases = Vec::new();
    connect(&session, |prompt| match prompt {
        Prompt::UnknownHostKey { reply, .. } => reply.send(HostKeyDecision::AcceptAndRemember),
        Prompt::Secret {
            request: SecretRequest::KeyPassphrase { .. },
            hop,
            reply,
        } => {
            passphrases.push(hop);
            reply.send(Some(Secret::new(PASSPHRASE)));
        }
        other => panic!("unexpected prompt: {other:?}"),
    })
    .await;
    assert_eq!(passphrases, [hop_at(0, &jump), None]);
    session.close();
    closed(&session, no_prompts).await;
}

#[tokio::test]
async fn losing_the_connection_to_the_first_jump_host_ends_the_session() {
    let Some((jump, _destination, mut request)) = chain("") else {
        return;
    };
    let (listener, proxy) = proxy_listener().await;
    request.proxy = proxy;
    let relay = relay_to(jump.port(), listener);
    let transport = jump.transport("plain");
    let session = transport.open(request);
    connect(&session, accept).await;

    relay.abort();
    let reason = closed(&session, no_prompts).await;
    assert!(
        matches!(
            reason,
            CloseReason::Failed(SessionError::JumpHost { hop: 0, .. })
        ),
        "the first jump host must be blamed, got {reason:?}"
    );
}

#[tokio::test]
async fn cancelling_at_the_first_jump_host_prompt_disconnects_from_it() {
    let Some((jump, _destination, mut request)) = chain("") else {
        return;
    };
    let (listener, proxy) = proxy_listener().await;
    let relay = relay_to(jump.port(), listener);
    request.proxy = proxy;
    let transport = jump.transport("plain");
    let session = transport.open(request);
    let pending = loop {
        match next_event(&session).await {
            Event::Prompt(Prompt::UnknownHostKey {
                hop: Some(_),
                reply,
                ..
            }) => break reply,
            Event::Closed(reason) => panic!("closed early: {reason:?}"),
            _ => {}
        }
    };
    session.close();
    assert_eq!(
        closed(&session, no_prompts).await,
        CloseReason::ClosedByUser
    );
    drop(pending);
    timeout(Duration::from_secs(2), relay)
        .await
        .expect("the connection to the jump host was left open")
        .unwrap();
}
