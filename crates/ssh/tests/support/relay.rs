//! The existing HTTP CONNECT relay used by real SSH route tests.
use nocterm_session::ProxyConfig;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
};

/// Resolves with the CONNECT line once the original routed connection has ended.
pub(crate) fn relay_to(port: u16, listener: TcpListener) -> tokio::task::JoinHandle<String> {
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
        let mut outgoing = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let _ = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await;
        String::from_utf8(header)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned()
    })
}

pub(crate) async fn proxy_listener() -> (TcpListener, ProxyConfig) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = ProxyConfig::HttpConnect {
        host: "127.0.0.1".into(),
        port,
    };
    (listener, proxy)
}
