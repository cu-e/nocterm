//! A server that receives exec but deliberately withholds startup confirmation.
use nocterm_session::{
    Auth, ConnectRequest, Event, ExecRequest, HostKeyDecision, Prompt, PtySize, Target, Transport,
};
use nocterm_ssh::{SshConfig, SshTransport};
use russh::{
    Channel, ChannelId, Pty,
    server::{self, ChannelOpenHandle, Msg, Session},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Notify, time::timeout};

#[derive(Default)]
struct Observed {
    exec: Mutex<Option<ChannelId>>,
    started: Notify,
    closed: Notify,
}
struct Server(Arc<Observed>);
impl server::Handler for Server {
    type Error = russh::Error;
    async fn auth_none(&mut self, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }
    async fn channel_open_session(
        &mut self,
        _: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }
    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _: &[u8],
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        *self.0.exec.lock().unwrap() = Some(channel);
        self.0.started.notify_one();
        // No ChannelSuccess: the client's program is still starting.
        Ok(())
    }
    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        if *self.0.exec.lock().unwrap() == Some(channel) {
            self.0.closed.notify_one();
        }
        Ok(())
    }
}
#[tokio::test]
async fn cancelled_startup_closes_the_unconfirmed_exec_channel() {
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
    let observed = Arc::new(Observed::default());
    let remote = observed.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        server::run_stream(config, stream, Server(remote))
            .await
            .unwrap()
            .await
            .unwrap();
    });
    let transport = SshTransport::new(SshConfig {
        known_hosts: dir.path().join("known_hosts"),
        ..Default::default()
    })
    .unwrap();
    let session = transport.open(ConnectRequest {
        target: Target::new("test", "127.0.0.1", port),
        auth: Auth::Auto,
        launch: Default::default(),
        proxy: Default::default(),
        term: "xterm".into(),
        size: PtySize::default(),
        connect_timeout: Duration::from_secs(5),
        keepalive_interval: None,
    });
    timeout(Duration::from_secs(5), async {
        loop {
            match session.next_event().await.unwrap() {
                Event::Connected => break,
                Event::Prompt(Prompt::UnknownHostKey { reply, .. }) => {
                    reply.send(HostKeyDecision::AcceptOnce);
                }
                Event::Closed(reason) => panic!("connection failed: {reason:?}"),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    let exec = session.exec().unwrap();
    let startup = tokio::spawn(async move { exec.exec(ExecRequest::new("never-confirmed")).await });
    timeout(Duration::from_secs(5), observed.started.notified())
        .await
        .unwrap();
    startup.abort();
    let _ = startup.await;
    timeout(Duration::from_secs(5), observed.closed.notified())
        .await
        .expect("cancelled startup must close its owned channel");
    session.close();
    server.abort();
}
