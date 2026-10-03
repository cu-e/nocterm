//! Explicit TCP routes. The SSH handshake and host-key policy stay unchanged.
use nocterm_session::{ProxyConfig, Target};
use std::{io, net::IpAddr};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};

const MAX_HEADER: usize = 16 * 1024;

pub(crate) async fn connect(target: &Target, route: &ProxyConfig) -> io::Result<TcpStream> {
    let (host, port) = match route {
        ProxyConfig::Direct => (target.host.as_str(), target.port),
        ProxyConfig::HttpConnect { host, port } | ProxyConfig::Socks5 { host, port, .. } => {
            validate_host(host)?;
            if *port == 0 {
                return Err(invalid("proxy port must be nonzero"));
            }
            (host.as_str(), *port)
        }
    };
    validate_host(&target.host)?;
    let mut stream = TcpStream::connect((host, port)).await?;
    stream.set_nodelay(true)?;
    match route {
        ProxyConfig::Direct => {}
        ProxyConfig::HttpConnect { .. } => http(&mut stream, target).await?,
        ProxyConfig::Socks5 { remote_dns, .. } => socks(&mut stream, target, *remote_dns).await?,
    }
    Ok(stream)
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn failure(message: &str) -> io::Error {
    io::Error::other(message)
}
fn validate_host(host: &str) -> io::Result<()> {
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
    stream.write_all(&[5, 1, 0]).await?;
    let mut greeting = [0; 2];
    stream.read_exact(&mut greeting).await?;
    if greeting != [5, 0] {
        return Err(failure(
            "SOCKS5 proxy did not accept authentication-free access",
        ));
    }
    let mut request = vec![5, 1, 0];
    let address = match target.host.parse::<IpAddr>() {
        Ok(ip) => Some(ip),
        Err(_) if !remote_dns => Some(
            tokio::net::lookup_host((target.host.as_str(), target.port))
                .await?
                .next()
                .ok_or_else(|| failure("target DNS returned no addresses"))?
                .ip(),
        ),
        Err(_) => None,
    };
    match address {
        Some(IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Some(IpAddr::V6(ip)) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
        None => {
            request.extend_from_slice(&[3, target.host.len() as u8]);
            request.extend_from_slice(target.host.as_bytes());
        }
    }
    request.extend_from_slice(&target.port.to_be_bytes());
    stream.write_all(&request).await?;
    let mut reply = [0; 4];
    stream.read_exact(&mut reply).await?;
    if reply[0] != 5 || reply[2] != 0 {
        return Err(failure("invalid SOCKS5 CONNECT reply"));
    }
    if reply[1] != 0 {
        return Err(failure(&format!(
            "SOCKS5 proxy rejected CONNECT (status {})",
            reply[1]
        )));
    }
    let count = match reply[3] {
        1 => 4,
        4 => 16,
        3 => stream.read_u8().await? as usize,
        _ => return Err(failure("invalid SOCKS5 address type")),
    };
    let mut address = vec![0; count + 2];
    stream.read_exact(&mut address).await?;
    Ok(())
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
                    }
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
                    }
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
        )
        .await
        .unwrap();
        let mut b = Vec::new();
        socket.read_to_end(&mut b).await.unwrap();
        assert_eq!(b, b"SSH");
        job.await.unwrap();
    }
    #[test]
    fn host_validation_prevents_http_header_injection() {
        for h in ["host\r\nX:evil", "user@host", "host/path", "host:22", ""] {
            assert!(validate_host(h).is_err());
        }
        assert!(validate_host("::1").is_ok());
    }
}
