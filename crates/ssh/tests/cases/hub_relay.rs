//! Sessions whose first connection is a stream handed over by a hub relay.
//!
//! The relay supplies bytes only: host keys, `known_hosts` and sign-in are still the
//! transport's, against the destination. The stand-in hub is in `support/hub_relay.rs`.
use super::*;
use nocterm_session::{
    JumpHost, PtySize, RelayDialer, RelayError, RelayRefusal,
    testing::{RelayRequest, ScriptedRelay},
};
use std::sync::Arc;
#[path = "../support/hub_relay.rs"]
mod hub;
use hub::{HUB, TcpRelay, through_hub};

/// A request that needs no server: the relay never reaches the target.
fn offline_request() -> ConnectRequest {
    ConnectRequest {
        proxy: Default::default(),
        launch: Default::default(),
        target: Target::new("user", "10.0.0.7", 22),
        auth: Auth::Auto,
        jumps: Vec::new(),
        term: "xterm-256color".into(),
        size: PtySize::default(),
        connect_timeout: Duration::from_secs(10),
        keepalive_interval: None,
    }
}

/// A transport with its own empty trust store and the given relay, if any.
fn offline_transport(relay: Option<Arc<dyn RelayDialer>>) -> (tempfile::TempDir, SshTransport) {
    let dir = tempfile::tempdir().unwrap();
    let transport = SshTransport::new(nocterm_ssh::SshConfig {
        known_hosts: dir.path().join("known_hosts"),
        ..Default::default()
    })
    .unwrap();
    match relay {
        Some(relay) => (dir, transport.with_relay(relay)),
        None => (dir, transport),
    }
}

fn target_of(sshd: &Sshd) -> Target {
    Target::new(current_user(), "127.0.0.1", sshd.port())
}

#[tokio::test]
async fn a_session_through_a_hub_relay_remembers_the_destination_key_and_not_the_hub() {
    let Some(server) = Sshd::start() else { return };
    let hub = TcpRelay::new();
    let transport = server.transport("plain").with_relay(hub.clone());
    let session = transport.open(through_hub(server.request()));
    let mut asked = 0;
    connect(&session, |prompt| match prompt {
        Prompt::UnknownHostKey {
            host, hop, reply, ..
        } => {
            asked += 1;
            assert_eq!(host, server.host_label());
            assert_eq!(hop, None);
            reply.send(HostKeyDecision::AcceptAndRemember);
        }
        other => panic!("unexpected prompt: {other:?}"),
    })
    .await;
    assert_eq!(asked, 1);
    assert_eq!(
        hub.asked(),
        [RelayRequest {
            hub: HUB.into(),
            target: target_of(&server),
        }]
    );
    assert!(session.fs().unwrap().home().await.unwrap().starts_with('/'));
    session.input("printf 'relayed-%s\\n' $((40+2))\r");
    output_containing(&session, "relayed-42").await;
    let recorded = fs::read_to_string(server.root().join("known_hosts")).unwrap();
    assert!(recorded.contains(&server.host_label()));
    assert!(!recorded.contains("hub.test"));
    session.close();
    assert_eq!(
        closed(&session, no_prompts).await,
        CloseReason::ClosedByUser
    );
    hub.connection_ended().await;

    // A remembered key is trusted on the next connection without a prompt.
    let again = transport.open(through_hub(server.request()));
    connect(&again, no_prompts).await;
    again.close();
    closed(&again, no_prompts).await;
    hub.connection_ended().await;
}

#[tokio::test]
async fn a_relayed_destination_key_that_is_rejected_never_reaches_sign_in() {
    let Some(server) = Sshd::start() else { return };
    let hub = TcpRelay::new();
    let transport = server.transport("plain").with_relay(hub.clone());
    let session = transport.open(through_hub(server.request()));
    let mut prompts = 0;
    let reason = closed(&session, |prompt| {
        prompts += 1;
        match prompt {
            Prompt::UnknownHostKey { reply, .. } => reply.send(HostKeyDecision::Reject),
            other => panic!("unexpected prompt: {other:?}"),
        }
    })
    .await;
    assert_eq!(prompts, 1);
    assert!(matches!(
        reason,
        CloseReason::Failed(SessionError::HostKeyRejected { .. })
    ));
    assert!(!server.root().join("known_hosts").exists());
    hub.connection_ended().await;
}

#[tokio::test]
async fn a_revoked_destination_key_is_refused_over_a_relay() {
    let Some(server) = Sshd::start() else { return };
    let key = fs::read_to_string(server.root().join("host_key.pub")).unwrap();
    let label = server.host_label();
    fs::write(
        server.root().join("known_hosts"),
        format!("{label} {key}\n@revoked {label} {key}"),
    )
    .unwrap();
    let hub = TcpRelay::new();
    let transport = server.transport("plain").with_relay(hub.clone());
    let session = transport.open(through_hub(server.request()));
    let reason = closed(&session, no_prompts).await;
    assert!(
        matches!(reason, CloseReason::Failed(SessionError::Other(message)) if message.contains("revoked"))
    );
    hub.connection_ended().await;
}

#[tokio::test]
async fn a_jump_host_route_continues_behind_the_relay_with_every_key_checked() {
    let Some(jump) = Sshd::start() else { return };
    let Some(destination) = Sshd::start() else {
        return;
    };
    destination.authorize(&jump);
    let hub = TcpRelay::new();
    let transport = jump.transport("plain").with_relay(hub.clone());
    let mut request = through_hub(destination.request());
    request.jumps = vec![JumpHost {
        target: target_of(&jump),
        auth: Auth::Auto,
    }];
    let session = transport.open(request);
    let mut hosts = Vec::new();
    connect(&session, |prompt| match prompt {
        Prompt::UnknownHostKey {
            host, hop, reply, ..
        } => {
            hosts.push((hop.map(|hop| hop.index), host));
            reply.send(HostKeyDecision::AcceptAndRemember);
        }
        other => panic!("unexpected prompt: {other:?}"),
    })
    .await;
    assert_eq!(
        hosts,
        [
            (Some(0), jump.host_label()),
            (None, destination.host_label())
        ]
    );
    assert_eq!(
        hub.asked(),
        [RelayRequest {
            hub: HUB.into(),
            target: target_of(&jump),
        }],
        "only the first hop is dialled through the hub; the destination is forwarded by the jump host"
    );
    session.input("printf 'destination=%s\\n' \"$SSH_CONNECTION\"\r");
    // Wait for the printed address itself: the terminal also echoes the typed command,
    // which contains `destination=` before the shell has answered.
    output_containing(&session, &format!("127.0.0.1 {}", destination.port())).await;
    session.close();
    closed(&session, no_prompts).await;
    hub.connection_ended().await;
}

#[tokio::test]
async fn a_relay_refusal_fails_the_session_naming_the_hub() {
    let refusal = RelayError::Refused {
        reason: RelayRefusal::PrivateNetwork,
    };
    let hub = Arc::new(ScriptedRelay::refusing(refusal.clone()));
    let (_dir, transport) = offline_transport(Some(hub));
    let session = transport.open(through_hub(offline_request()));
    assert_eq!(
        closed(&session, no_prompts).await,
        CloseReason::Failed(SessionError::Relay {
            hub: HUB.into(),
            source: refusal,
        })
    );
}

#[tokio::test]
async fn a_relay_refusal_for_a_jump_host_is_reported_against_that_hop() {
    let hub = Arc::new(ScriptedRelay::refusing(RelayError::NotSignedIn {
        hub: HUB.into(),
    }));
    let (_dir, transport) = offline_transport(Some(hub));
    let mut request = through_hub(offline_request());
    request.jumps = vec![JumpHost {
        target: Target::new("jump", "10.0.0.1", 22),
        auth: Auth::Auto,
    }];
    let session = transport.open(request);
    let reason = closed(&session, no_prompts).await;
    assert!(matches!(
        reason,
        CloseReason::Failed(SessionError::JumpHost { hop: 0, source, .. })
            if matches!(*source, SessionError::Relay { .. })
    ));
}

#[tokio::test]
async fn a_hub_route_on_a_transport_without_a_relay_cannot_connect() {
    let (_dir, transport) = offline_transport(None);
    let session = transport.open(through_hub(offline_request()));
    assert!(matches!(
        closed(&session, no_prompts).await,
        CloseReason::Failed(SessionError::Unreachable { host, .. }) if host == "10.0.0.7"
    ));
}

#[tokio::test]
async fn cancelling_while_the_relay_is_silent_closes_both_directions() {
    let hub = ScriptedRelay::default();
    let (_dir, transport) = offline_transport(Some(Arc::new(hub.clone())));
    let session = transport.open(through_hub(offline_request()));
    let (from_transport, to_transport) = timeout(Duration::from_secs(5), hub.next_peer())
        .await
        .unwrap()
        .into_parts();
    // The handshake has started: the client's version line reaches the relay.
    let banner = timeout(Duration::from_secs(5), from_transport.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(banner.starts_with(b"SSH-2.0-"));
    session.close();
    assert_eq!(
        timeout(Duration::from_secs(2), closed(&session, no_prompts))
            .await
            .unwrap(),
        CloseReason::ClosedByUser
    );
    assert!(
        timeout(Duration::from_secs(2), from_transport.recv())
            .await
            .unwrap()
            .is_err(),
        "the relay sees the end of the stream"
    );
    timeout(Duration::from_secs(2), async {
        while to_transport.send(vec![0]).await.is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the relay's sends fail once the session is gone");
}

#[tokio::test]
async fn the_relay_going_away_mid_handshake_ends_the_session_with_a_lost_connection() {
    let hub = ScriptedRelay::default();
    let (_dir, transport) = offline_transport(Some(Arc::new(hub.clone())));
    let session = transport.open(through_hub(offline_request()));
    drop(
        timeout(Duration::from_secs(5), hub.next_peer())
            .await
            .unwrap(),
    );
    let reason = timeout(Duration::from_secs(5), closed(&session, no_prompts))
        .await
        .unwrap();
    assert!(matches!(reason, CloseReason::Failed(_)), "{reason:?}");
}
