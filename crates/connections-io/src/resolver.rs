//! Local resolution filters literals and names before calling native DNS.
use nocterm_connections_model::observation::{
    ObservationError, PublicAddress, PublicAddressResolver,
};
use nocterm_core::SharedError;
use std::{
    io,
    net::{IpAddr, ToSocketAddrs as _},
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

const DNS_WORKERS: usize = 4;
const DNS_PENDING: usize = 4;
const DNS_DEADLINE: Duration = Duration::from_secs(8);

struct Request {
    host: String,
    deadline: Instant,
    answer: SyncSender<io::Result<Vec<IpAddr>>>,
}

/// Optional country resolution has its own finite physical budget. An OS lookup
/// that ignores cancellation retains one of these workers, not a shared fact slot.
/// Dropping the last resolver closes idle workers; queued expired work is skipped.
pub struct NativeResolver {
    requests: OnceLock<Result<SyncSender<Request>, SharedError<io::Error>>>,
    deadline: Duration,
}

impl Default for NativeResolver {
    fn default() -> Self {
        Self {
            requests: OnceLock::new(),
            deadline: DNS_DEADLINE,
        }
    }
}

type Lookup = Arc<dyn Fn(&str) -> io::Result<Vec<IpAddr>> + Send + Sync>;
fn workers(lookup: Lookup) -> io::Result<SyncSender<Request>> {
    let (sender, receiver) = mpsc::sync_channel::<Request>(DNS_PENDING);
    let receiver = Arc::new(Mutex::new(receiver));
    for index in 0..DNS_WORKERS {
        let receiver = receiver.clone();
        let lookup = lookup.clone();
        thread::Builder::new()
            .name(format!("nocterm-dns-{index}"))
            .spawn(move || {
                loop {
                    let next = receiver
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .recv();
                    let Ok(request) = next else { break };
                    if Instant::now() >= request.deadline {
                        continue;
                    }
                    let resolved = lookup(&request.host);
                    let _ = request.answer.send(resolved);
                }
            })?;
    }
    Ok(sender)
}

impl NativeResolver {
    fn lookup(&self, host: &str) -> io::Result<Vec<IpAddr>> {
        let sender = self
            .requests
            .get_or_init(|| {
                workers(Arc::new(|host| {
                    (host, 0)
                        .to_socket_addrs()
                        .map(|addresses| addresses.map(|address| address.ip()).collect())
                }))
                .map_err(SharedError::from_error)
            })
            .as_ref()
            .map_err(|error| io::Error::other(error.clone()))?;
        let (answer, result) = mpsc::sync_channel(1);
        sender
            .try_send(Request {
                host: host.to_owned(),
                deadline: Instant::now() + self.deadline,
                answer,
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => io::ErrorKind::WouldBlock,
                mpsc::TrySendError::Disconnected(_) => io::ErrorKind::BrokenPipe,
            })?;
        result
            .recv_timeout(self.deadline)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => io::ErrorKind::TimedOut,
                mpsc::RecvTimeoutError::Disconnected => io::ErrorKind::BrokenPipe,
            })?
    }
}

impl PublicAddressResolver for NativeResolver {
    fn resolve(&self, host: &str) -> Result<Option<PublicAddress>, ObservationError> {
        resolve_with(host, |host| self.lookup(host))
    }
}

fn resolve_with<I: IntoIterator<Item = IpAddr>>(
    host: &str,
    resolve: impl FnOnce(&str) -> std::io::Result<I>,
) -> Result<Option<PublicAddress>, ObservationError> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(PublicAddress::new(ip));
    }
    if host.is_empty() || host.ends_with(".local") || !host.contains('.') {
        return Ok(None);
    }
    let addresses =
        resolve(host).map_err(|source| ObservationError::Resolution(Box::new(source)))?;
    Ok(addresses.into_iter().find_map(PublicAddress::new))
}

#[cfg(test)]
mod tests;
