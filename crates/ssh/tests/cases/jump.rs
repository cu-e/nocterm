//! Sessions that reach the destination through one or more jump hosts.
//!
//! Hop `A` is the jump host and `B` the destination, both real `sshd`s. Every
//! hop is signed in with `A`'s plain key, so `B` authorizes it.
use super::*;
use nocterm_session::{ExecRequest, JumpHost, PromptHop, ProxyConfig};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener as AsyncListener, TcpStream as AsyncStream},
};

fn target_of(sshd: &Sshd) -> Target {
    Target::new(current_user(), "127.0.0.1", sshd.port)
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

/// An HTTP CONNECT proxy for one connection to `port`. It resolves with the
/// CONNECT line once that connection has ended.
fn relay_to(port: u16, listener: AsyncListener) -> tokio::task::JoinHandle<String> {
    tokio::spawn(async move {
        let (mut incoming, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(incoming.read_u8().await.unwrap());
        }
        incoming
            .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
            .await
            .unwrap();
        let mut outgoing = AsyncStream::connect(("127.0.0.1", port)).await.unwrap();
        let _ = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await;
        String::from_utf8(header)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned()
    })
}

async fn proxy_listener() -> (AsyncListener, ProxyConfig) {
    let listener = AsyncListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = ProxyConfig::HttpConnect {
        host: "127.0.0.1".into(),
        port,
    };
    (listener, proxy)
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
async fn chain_reaches_the_destination_for_terminal_files_and_programs() {
    let Some((jump, destination, mut request)) = chain("") else {
        return;
    };
    let (listener, proxy) = proxy_listener().await;
    request.proxy = proxy;
    let relay = relay_to(jump.port, listener);

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
    output_containing(&session, &format!("127.0.0.1 {}", destination.port)).await;
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
            .contains(&format!("127.0.0.1 {}", destination.port))
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
        format!("CONNECT 127.0.0.1:{} HTTP/1.1", jump.port)
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
    let relay = relay_to(jump.port, listener);
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
    let relay = relay_to(first.port, listener);
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
