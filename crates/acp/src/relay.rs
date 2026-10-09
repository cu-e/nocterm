use std::io::{self, BufReader, Read, Write};

use crate::bridge::{ENDPOINT_VARIABLE, Socket, TOKEN_VARIABLE, bounded_line};

fn connect(endpoint: &str) -> io::Result<Socket> {
    #[cfg(unix)]
    {
        let path = endpoint.strip_prefix("unix:").ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Expected Unix bridge endpoint")
        })?;
        Socket::connect(path)
    }
    #[cfg(not(unix))]
    {
        let address = endpoint.strip_prefix("tcp:").ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Expected TCP bridge endpoint")
        })?;
        let address: std::net::SocketAddr = address
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid bridge endpoint"))?;
        if !address.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Bridge endpoint must be loopback",
            ));
        }
        Socket::connect(address)
    }
}

/// Runs the relay a [`crate::BridgeServer`] registration launched, with the
/// credentials it was handed.
pub fn run_relay_from_environment() -> io::Result<()> {
    let variable = |name| {
        std::env::var(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("Missing {name}")))
    };
    run_relay(
        io::stdin(),
        io::stdout(),
        &variable(ENDPOINT_VARIABLE)?,
        &variable(TOKEN_VARIABLE)?,
    )
}

/// Runs the stdio relay without GUI initialization or protocol output on stderr.
/// Both streams are bounded line-by-line. EOF closes the corresponding socket half.
pub fn run_relay<R: Read + Send + 'static, W: Write>(
    stdin: R,
    stdout: W,
    endpoint: &str,
    token: &str,
) -> io::Result<()> {
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid bridge token",
        ));
    }
    let mut socket = connect(endpoint)?;
    let hello = serde_json::json!({"token":token});
    serde_json::to_writer(&mut socket, &hello)?;
    socket.write_all(b"\n")?;
    socket.flush()?;
    let writer = socket.try_clone()?;
    std::thread::Builder::new()
        .name("nocterm-relay-input".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdin);
            let mut writer = writer;
            let result = (|| {
                while let Some(line) = bounded_line(&mut reader, nocterm_ai::MAX_MCP_LINE)? {
                    writer.write_all(line.as_bytes())?;
                    writer.write_all(b"\n")?;
                    writer.flush()?;
                }
                Ok::<_, io::Error>(())
            })();
            let _ = writer.shutdown(if result.is_ok() {
                std::net::Shutdown::Write
            } else {
                std::net::Shutdown::Both
            });
        })?;
    let mut reader = BufReader::new(&socket);
    let mut stdout = stdout;
    let result = (|| {
        while let Some(line) = bounded_line(&mut reader, nocterm_ai::MAX_MCP_LINE)? {
            stdout.write_all(line.as_bytes())?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
        Ok::<_, io::Error>(())
    })();
    let _ = socket.shutdown(std::net::Shutdown::Both);
    // Return immediately on revoke/EOF even if the agent still holds stdin.
    // The CLI owns this process; main exits and terminates its stdin worker.
    result
}
