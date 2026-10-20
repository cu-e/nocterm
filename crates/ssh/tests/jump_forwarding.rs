//! Jump routes against an in-process server, so they run on every platform.
use nocterm_session::{
    Auth, CloseReason, ConnectRequest, Event, HostKeyDecision, JumpHost, MAX_JUMP_HOSTS, Prompt,
    PromptHop, PtySize, SessionError, Target, Transport,
};
use nocterm_ssh::{SshConfig, SshTransport};
use russh::{
    Channel,
    server::{self, ChannelOpenHandle, Msg, Session},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::timeout;

/// A server that signs everyone in and refuses every forward, remembering what was asked for.
struct RefusingServer(Arc<Mutex<Vec<(String, u32)>>>);
impl server::Handler for RefusingServer {
    type Error = russh::Error;
    async fn auth_none(&mut self, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        _: Channel<Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.0.lock().unwrap().push((host.to_owned(), port));
        drop(reply);
        Ok(())
    }
}

fn request(port: u16, jumps: Vec<JumpHost>) -> ConnectRequest {
    ConnectRequest {
        target: Target::new("test", "destination.invalid", 22),
        auth: Auth::Auto,
        launch: Default::default(),
        proxy: Default::default(),
        term: "xterm".into(),
        size: PtySize::default(),
        connect_timeout: Duration::from_secs(5),
        keepalive_interval: None,
        jumps: if jumps.is_empty() {
            vec![hop(port)]
        } else {
            jumps
        },
    }
}

fn hop(port: u16) -> JumpHost {
    JumpHost {
        target: Target::new("test", "127.0.0.1", port),
        auth: Auth::Auto,
    }
}

#[tokio::test]
async fn a_jump_host_that_refuses_the_forward_is_blamed_for_the_destination() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("host_key");
    assert!(
        std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key)
            .status()
            .unwrap()
            .success()
    );
    let config = Arc::new(server::Config {
        keys: vec![russh::keys::load_secret_key(key, None).unwrap()],
        ..Default::default()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let remote = asked.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _ = server::run_stream(config, stream, RefusingServer(remote))
            .await
            .unwrap()
            .await;
    });
    let transport = SshTransport::new(SshConfig {
        known_hosts: dir.path().join("known_hosts"),
        ..Default::default()
    })
    .unwrap();
    let session = transport.open(request(port, Vec::new()));
    let reason = timeout(Duration::from_secs(10), async {
        loop {
            match session.next_event().await.unwrap() {
                Event::Prompt(Prompt::UnknownHostKey { hop, reply, .. }) => {
                    assert_eq!(
                        hop,
                        Some(PromptHop {
                            index: 0,
                            target: Target::new("test", "127.0.0.1", port)
                        })
                    );
                    reply.send(HostKeyDecision::AcceptOnce);
                }
                Event::Closed(reason) => break reason,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    let CloseReason::Failed(SessionError::JumpHost { hop, source, .. }) = reason else {
        panic!("expected a jump host failure, got {reason:?}");
    };
    assert_eq!(hop, 0);
    assert!(
        matches!(*source, SessionError::Unreachable { ref host, .. } if host == "destination.invalid")
    );
    assert_eq!(
        *asked.lock().unwrap(),
        [("destination.invalid".to_owned(), 22)]
    );
    server.abort();
}

#[tokio::test]
async fn a_route_over_the_limit_fails_without_any_network_attempt() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let transport = SshTransport::new(SshConfig::default()).unwrap();
    let jumps = vec![hop(port); MAX_JUMP_HOSTS + 1];
    let session = transport.open(request(port, jumps));
    let event = timeout(Duration::from_secs(1), session.next_event())
        .await
        .unwrap();
    assert!(matches!(
        event,
        Some(Event::Closed(CloseReason::Failed(
            SessionError::TooManyJumpHosts { count: 9, limit: 8 }
        )))
    ));
    assert!(
        timeout(Duration::from_millis(25), listener.accept())
            .await
            .is_err()
    );
}
