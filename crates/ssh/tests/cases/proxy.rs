//! The ordinary SSH/SFTP path tunneled through private byte-exact test proxies.
use super::*;
use nocterm_session::ProxyConfig;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener as AsyncListener, TcpStream as AsyncStream},
};

#[tokio::test]
async fn http_and_socks_tunnels_preserve_target_host_keys_and_sftp() {
    let Some(server) = Sshd::start() else {
        return;
    };
    for socks in [false, true] {
        let listener = AsyncListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = listener.local_addr().unwrap().port();
        let target_port = server.port;
        let relay = tokio::spawn(async move {
            let (mut incoming, _) = listener.accept().await.unwrap();
            if socks {
                let mut greeting = [0; 3];
                incoming.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting, [5, 1, 0]);
                incoming.write_all(&[5, 0]).await.unwrap();
                let mut header = [0; 4];
                incoming.read_exact(&mut header).await.unwrap();
                assert_eq!(header, [5, 1, 0, 1]);
                let mut address = [0; 6];
                incoming.read_exact(&mut address).await.unwrap();
                assert_eq!(&address[..4], &[127, 0, 0, 1]);
                assert_eq!(&address[4..], &target_port.to_be_bytes());
                incoming
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                    .await
                    .unwrap();
            } else {
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    header.push(incoming.read_u8().await.unwrap());
                }
                assert!(
                    String::from_utf8(header)
                        .unwrap()
                        .starts_with(&format!("CONNECT 127.0.0.1:{target_port} HTTP/1.1\r\n"))
                );
                incoming
                    .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
                    .await
                    .unwrap();
            }
            let mut outgoing = AsyncStream::connect(("127.0.0.1", target_port))
                .await
                .unwrap();
            let _ = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await;
        });
        let transport = server.transport("plain");
        let mut request = server.request();
        request.proxy = if socks {
            ProxyConfig::Socks5 {
                host: "127.0.0.1".into(),
                port: proxy_port,
                remote_dns: true,
            }
        } else {
            ProxyConfig::HttpConnect {
                host: "127.0.0.1".into(),
                port: proxy_port,
            }
        };
        let session = transport.open(request);
        connect(&session, |prompt| match prompt {
            Prompt::UnknownHostKey { reply, .. } => reply.send(HostKeyDecision::AcceptAndRemember),
            other => panic!("unexpected prompt: {other:?}"),
        })
        .await;
        let fs = session.fs().unwrap();
        assert!(fs.home().await.unwrap().starts_with('/'));
        let recorded = fs::read_to_string(server.root().join("known_hosts")).unwrap();
        assert!(recorded.contains(&server.host_label()));
        assert!(!recorded.contains(&format!("[127.0.0.1]:{proxy_port}")));
        session.close();
        closed(&session, no_prompts).await;
        timeout(EVENT_TIMEOUT, relay).await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn cancelling_a_stalled_proxy_closes_the_tunnel() {
    let Some(server) = Sshd::start() else {
        return;
    };
    let listener = AsyncListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_port = listener.local_addr().unwrap().port();
    let (seen, waiting) = tokio::sync::oneshot::channel();
    let relay = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(s.read_u8().await.unwrap());
        }
        seen.send(()).unwrap();
        let mut byte = [0];
        assert_eq!(s.read(&mut byte).await.unwrap(), 0);
    });
    let transport = server.transport("plain");
    let mut request = server.request();
    request.proxy = ProxyConfig::HttpConnect {
        host: "127.0.0.1".into(),
        port: proxy_port,
    };
    let session = transport.open(request);
    timeout(EVENT_TIMEOUT, waiting).await.unwrap().unwrap();
    session.close();
    assert_eq!(
        timeout(Duration::from_secs(2), closed(&session, no_prompts))
            .await
            .unwrap(),
        CloseReason::ClosedByUser
    );
    timeout(Duration::from_secs(2), relay)
        .await
        .unwrap()
        .unwrap();
}
