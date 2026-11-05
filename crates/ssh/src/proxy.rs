//! Explicit routes for the first connection: a plain TCP connection, an HTTP
//! CONNECT or SOCKS5 proxy, or a stream handed over by a hub relay. The SSH
//! handshake and host-key policy stay unchanged: whatever the route, the host key
//! checked is the destination's, never the proxy's or the hub's.
use nocterm_session::{ProxyConfig, RelayDialer, RelayError, SessionError, Target};
use std::{io, net::IpAddr, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _},
    net::TcpStream,
};

#[cfg(test)]
mod hub_tests;
mod relay;

const MAX_HEADER: usize = 16 * 1024;

/// A byte stream an SSH handshake can run over: a socket, a relayed stream or a
/// forwarded channel.
pub(crate) trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Why the first connection could not be made.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConnectError {
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The route names a hub, but this transport was built without a relay.
    #[error("this transport has no hub relay")]
    NoRelay,
    #[error("hub {hub}: {source}")]
    Relay {
        hub: String,
        #[source]
        source: RelayError,
    },
}

impl ConnectError {
    /// The session error for a first connection to `host` that failed this way.
    pub(crate) fn into_session(self, host: &str) -> SessionError {
        match self {
            Self::Relay { hub, source } => SessionError::Relay { hub, source },
            other => SessionError::Unreachable {
                host: host.to_owned(),
                reason: other.to_string(),
            },
        }
    }
}

/// Connects to `target` along `route`. A hub route needs `relay`.
pub(crate) async fn connect(
    target: &Target,
    route: &ProxyConfig,
    relay: Option<&Arc<dyn RelayDialer>>,
) -> Result<Box<dyn Stream>, ConnectError> {
    validate_host(&target.host)?;
    Ok(match route {
        ProxyConfig::Direct => Box::new(socket(&target.host, target.port).await?),
        ProxyConfig::HttpConnect { host, port } => {
            let mut stream = proxy_socket(host, *port).await?;
            http(&mut stream, target).await?;
            Box::new(stream)
        }
        ProxyConfig::Socks5 {
            host,
            port,
            remote_dns,
        } => {
            let mut stream = proxy_socket(host, *port).await?;
            socks(&mut stream, target, *remote_dns).await?;
            Box::new(stream)
        }
        ProxyConfig::Hub { address, node } => {
            let relay = relay.ok_or(ConnectError::NoRelay)?;
            let stream = relay
                .dial(address, node.as_deref(), target)
                .await
                .map_err(|source| ConnectError::Relay {
                    hub: address.clone(),
                    source,
                })?;
            Box::new(relay::bridge(stream))
        }
    })
}

async fn proxy_socket(host: &str, port: u16) -> io::Result<TcpStream> {
    validate_host(host)?;
    if port == 0 {
        return Err(invalid("proxy port must be nonzero"));
    }
    socket(host, port).await
}

async fn socket(host: &str, port: u16) -> io::Result<TcpStream> {
    let stream = TcpStream::connect((host, port)).await?;
    // Keystrokes are tiny packets; batching them only adds latency.
    stream.set_nodelay(true)?;
    Ok(stream)
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn failure(message: &str) -> io::Error {
    io::Error::other(message)
}
pub(crate) fn validate_host(host: &str) -> io::Result<()> {
    if host.is_empty()
        || host.len() > 255
        || !host.is_ascii()
        || host.bytes().any(|b| {
            b.is_ascii_control()
                || b.is_ascii_whitespace()
                || matches!(b, b'/' | b'\\' | b'@' | b'?' | b'#')
        })
    {
        return Err(invalid(
            "proxy/target host must be an ASCII hostname or IP address",
        ));
    }
    if host.contains(':') && host.parse::<std::net::Ipv6Addr>().is_err() {
        return Err(invalid("invalid IPv6 address"));
    }
    Ok(())
}
fn authority(target: &Target) -> String {
    if target.host.contains(':') {
        format!("[{}]:{}", target.host, target.port)
    } else {
        format!("{}:{}", target.host, target.port)
    }
}
async fn http(stream: &mut TcpStream, target: &Target) -> io::Result<()> {
    let authority = authority(target);
    stream
        .write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await?;
    // Read exactly the header, never consuming a coalesced SSH banner.
    let mut header = Vec::with_capacity(256);
    loop {
        if header.len() == MAX_HEADER {
            return Err(failure("HTTP proxy response header exceeds 16 KiB"));
        }
        header.push(stream.read_u8().await?);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let first = header.split(|b| *b == b'\n').next().unwrap_or_default();
    let first =
        std::str::from_utf8(first).map_err(|_| failure("invalid HTTP proxy status line"))?;
    let mut parts = first.trim_end_matches('\r').split_ascii_whitespace();
    if !matches!(parts.next(), Some("HTTP/1.1" | "HTTP/1.0")) {
        return Err(failure("invalid HTTP proxy protocol"));
    }
    let code = parts
        .next()
        .filter(|s| s.len() == 3)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| failure("invalid HTTP proxy status"))?;
    if code == 407 {
        return Err(failure(
            "HTTP proxy requires authentication; authenticated proxies are not supported yet",
        ));
    }
    if !(200..300).contains(&code) {
        return Err(failure(&format!(
            "HTTP proxy rejected CONNECT (status {code})"
        )));
    }
    Ok(())
}
async fn socks(stream: &mut TcpStream, target: &Target, remote_dns: bool) -> io::Result<()> {
    use tokio_socks::{TargetAddr, tcp::Socks5Stream};
    // Resolve here only for local DNS; the library receives an explicit address policy.
    let address = match target.host.parse::<IpAddr>() {
        Ok(ip) => TargetAddr::Ip((ip, target.port).into()),
        Err(_) if !remote_dns => TargetAddr::Ip(
            tokio::net::lookup_host((target.host.as_str(), target.port))
                .await?
                .next()
                .ok_or_else(|| failure("target DNS returned no addresses"))?,
        ),
        Err(_) => TargetAddr::Domain(target.host.as_str().into(), target.port),
    };
    Socks5Stream::connect_with_socket(stream, address)
        .await
        .map(|_| ())
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        net::TcpListener,
        time::{Duration, timeout},
    };
    fn target(host: &str) -> Target {
        Target {
            host: host.into(),
            port: 22,
            user: "test".into(),
        }
    }
    #[tokio::test]
    async fn http_fragmentation_preserves_coalesced_banner() {
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        let job = tokio::spawn(async move {
            let (mut s, _) = server.accept().await.unwrap();
            let mut h = Vec::new();
            while !h.ends_with(b"\r\n\r\n") {
                h.push(s.read_u8().await.unwrap());
            }
            assert_eq!(h, b"CONNECT [::1]:22 HTTP/1.1\r\nHost: [::1]:22\r\n\r\n");
            for chunk in [b"HTTP/1.1 2".as_slice(), b"00 OK\r\nX: 1\r\n\r\nSSH-banner"] {
                s.write_all(chunk).await.unwrap();
            }
        });
        let mut socket = connect(
            &target("::1"),
            &ProxyConfig::HttpConnect {
                host: "127.0.0.1".into(),
                port,
            },
            None,
        )
        .await
        .unwrap();
        let mut banner = String::new();
        socket.read_to_string(&mut banner).await.unwrap();
        assert_eq!(banner, "SSH-banner");
        job.await.unwrap();
    }
    #[tokio::test]
    async fn http_rejection_bounds_and_timeout() {
        for response in [
            b"HTTP/1.1 407 Auth\r\n\r\n".to_vec(),
            b"HTTP/1.1 503 Down\r\n\r\n".to_vec(),
            b"HTTP/9 200 OK\r\n\r\n".to_vec(),
            vec![b'X'; MAX_HEADER + 1],
        ] {
            let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = server.local_addr().unwrap().port();
            let job = tokio::spawn(async move {
                let (mut s, _) = server.accept().await.unwrap();
                let mut q = [0; 512];
                let _ = s.read(&mut q).await;
                let _ = s.write_all(&response).await;
            });
            assert!(
                connect(
                    &target("example.invalid"),
                    &ProxyConfig::HttpConnect {
                        host: "127.0.0.1".into(),
                        port
                    },
                    None
                )
                .await
                .is_err()
            );
            job.await.unwrap();
        }
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        let _idle = server;
        assert!(
            timeout(
                Duration::from_millis(20),
                connect(
                    &target("host"),
                    &ProxyConfig::Socks5 {
                        host: "127.0.0.1".into(),
                        port,
                        remote_dns: true
                    },
                    None
                )
            )
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn socks_remote_dns_ipv6_reply_and_banner() {
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        let job = tokio::spawn(async move {
            let (mut s, _) = server.accept().await.unwrap();
            let mut g = [0; 3];
            s.read_exact(&mut g).await.unwrap();
            assert_eq!(g, [5, 1, 0]);
            s.write_all(&[5, 0]).await.unwrap();
            let mut header = [0; 5];
            s.read_exact(&mut header).await.unwrap();
            assert_eq!(&header[..4], &[5, 1, 0, 3]);
            let mut rest = vec![0; header[4] as usize + 2];
            s.read_exact(&mut rest).await.unwrap();
            assert_eq!(&rest[..rest.len() - 2], b"remote.invalid");
            assert_eq!(&rest[rest.len() - 2..], &22u16.to_be_bytes());
            let mut response = vec![5, 0, 0, 4];
            response.extend_from_slice(&[0; 18]);
            response.extend_from_slice(b"SSH");
            s.write_all(&response).await.unwrap();
        });
        let mut socket = connect(
            &target("remote.invalid"),
            &ProxyConfig::Socks5 {
                host: "127.0.0.1".into(),
                port,
                remote_dns: true,
            },
            None,
        )
        .await
        .unwrap();
        let mut b = Vec::new();
        socket.read_to_end(&mut b).await.unwrap();
        assert_eq!(b, b"SSH");
        job.await.unwrap();
    }
    #[tokio::test]
    async fn socks_local_dns_uses_ip_and_preserves_fragmented_reply_banner() {
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        let job = tokio::spawn(async move {
            let (mut stream, _) = server.accept().await.unwrap();
            let mut greeting = [0; 3];
            stream.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            stream.write_all(&[5, 0]).await.unwrap();
            let mut header = [0; 4];
            stream.read_exact(&mut header).await.unwrap();
            assert_eq!(&header[..3], &[5, 1, 0]);
            let size = match header[3] {
                1 => 6,
                4 => 18,
                _ => panic!("local DNS must send an IP"),
            };
            let mut address = vec![0; size];
            stream.read_exact(&mut address).await.unwrap();
            for bytes in [&[5, 0, 0][..], &[1, 127, 0], &[0, 1, 0, 22], b"SSH"] {
                stream.write_all(bytes).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        let mut socket = connect(
            &target("localhost"),
            &ProxyConfig::Socks5 {
                host: "127.0.0.1".into(),
                port,
                remote_dns: false,
            },
            None,
        )
        .await
        .unwrap();
        let mut banner = Vec::new();
        socket.read_to_end(&mut banner).await.unwrap();
        assert_eq!(banner, b"SSH");
        job.await.unwrap();
    }

    #[tokio::test]
    async fn socks_refuses_authentication_and_malformed_or_rejected_replies() {
        for (greeting, reply) in [
            ([5, 2], vec![]),
            ([4, 0], vec![]),
            ([5, 0], vec![5, 5, 0, 1]),
            ([5, 0], vec![5, 0, 1, 1]),
            ([5, 0], vec![5, 0, 0, 9]),
        ] {
            let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = server.local_addr().unwrap().port();
            let job = tokio::spawn(async move {
                let (mut stream, _) = server.accept().await.unwrap();
                let mut hello = [0; 3];
                stream.read_exact(&mut hello).await.unwrap();
                stream.write_all(&greeting).await.unwrap();
                if !reply.is_empty() {
                    let mut request = [0; 10];
                    stream.read_exact(&mut request).await.unwrap();
                    stream.write_all(&reply).await.unwrap();
                }
            });
            assert!(
                connect(
                    &target("127.0.0.1"),
                    &ProxyConfig::Socks5 {
                        host: "127.0.0.1".into(),
                        port,
                        remote_dns: true,
                    },
                    None
                )
                .await
                .is_err()
            );
            job.await.unwrap();
        }
    }

    #[test]
    fn host_validation_prevents_http_header_injection() {
        for h in ["host\r\nX:evil", "user@host", "host/path", "host:22", ""] {
            assert!(validate_host(h).is_err());
        }
        assert!(validate_host("::1").is_ok());
    }
}
