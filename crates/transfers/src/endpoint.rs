//! Pairing two endpoints into the request kind that moves files between them.
use crate::{
    CollisionPolicy, DiscoveryLimits, DownloadRequest, RelayRequest, Request, TransferError,
    UploadRequest,
};
use nocterm_session::{HostAddress, RemoteFs};
use std::{path::PathBuf, sync::Arc};

/// One side of a transfer.
#[derive(Clone)]
pub enum TransferEndpoint {
    /// This computer; the executing runtime owns local file access.
    Local,
    Remote {
        host: HostAddress,
        fs: Arc<dyn RemoteFs>,
    },
}
impl Request {
    /// Local to remote is an upload, remote to local a download and remote to
    /// remote a relay. The result is validated with the default discovery limits.
    pub fn between(
        source: TransferEndpoint,
        sources: Vec<String>,
        destination: TransferEndpoint,
        directory: String,
        collisions: CollisionPolicy,
    ) -> Result<Self, TransferError> {
        let request = match (source, destination) {
            (TransferEndpoint::Local, TransferEndpoint::Local) => {
                return Err(TransferError::SameLocalEndpoints);
            }
            (TransferEndpoint::Local, TransferEndpoint::Remote { host, fs }) => {
                Self::Upload(UploadRequest {
                    sources: sources.into_iter().map(PathBuf::from).collect(),
                    host,
                    destination: directory,
                    fs,
                    collisions,
                })
            }
            (TransferEndpoint::Remote { host, fs }, TransferEndpoint::Local) => {
                Self::Download(DownloadRequest {
                    sources,
                    host,
                    local_destination: PathBuf::from(directory),
                    fs,
                    collisions,
                })
            }
            (
                TransferEndpoint::Remote {
                    host: source_host,
                    fs: source_fs,
                },
                TransferEndpoint::Remote { host, fs },
            ) => {
                let same_host = source_host == host || Arc::ptr_eq(&source_fs, &fs);
                if same_host && sources.iter().any(|source| contains(source, &directory)) {
                    return Err(TransferError::DestinationInsideSource);
                }
                Self::Relay(RelayRequest {
                    source_host,
                    source_fs,
                    sources,
                    host,
                    destination: directory,
                    fs,
                    collisions,
                })
            }
        };
        request.validate(DiscoveryLimits::default())?;
        Ok(request)
    }
}
/// True when `directory` is `source` itself or lies below it.
fn contains(source: &str, directory: &str) -> bool {
    let source = source.trim_end_matches('/');
    let directory = directory.trim_end_matches('/');
    source.is_empty()
        || directory == source
        || directory
            .strip_prefix(source)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TransferDirection;
    use nocterm_session::{Target, testing::InMemoryFs};

    fn remote(name: &str) -> TransferEndpoint {
        TransferEndpoint::Remote {
            host: HostAddress::direct(Target {
                host: name.into(),
                port: 22,
                user: "user".into(),
            }),
            fs: Arc::new(InMemoryFs::default()),
        }
    }
    fn between(
        source: TransferEndpoint,
        sources: &[&str],
        destination: TransferEndpoint,
        directory: &str,
    ) -> Result<Request, TransferError> {
        Request::between(
            source,
            sources.iter().map(|source| (*source).to_owned()).collect(),
            destination,
            directory.to_owned(),
            CollisionPolicy::Skip,
        )
    }

    #[test]
    fn endpoint_pairs_choose_upload_download_or_relay() {
        let upload = between(TransferEndpoint::Local, &["/tmp/a"], remote("h"), "/srv").unwrap();
        assert_eq!(upload.direction(), TransferDirection::Upload);
        assert!(upload.source_fs().is_none());
        let download = between(remote("h"), &["/srv/a"], TransferEndpoint::Local, "/tmp").unwrap();
        assert_eq!(download.direction(), TransferDirection::Download);
        assert!(download.source_fs().is_some());
        let relay = between(remote("a"), &["/srv/a"], remote("b"), "/srv").unwrap();
        assert_eq!(relay.direction(), TransferDirection::Relay);
        assert_eq!(relay.destination(), "/srv");
        assert!(relay.source_fs().is_some());
    }

    #[test]
    fn two_local_endpoints_are_refused() {
        let result = between(
            TransferEndpoint::Local,
            &["/tmp/a"],
            TransferEndpoint::Local,
            "/srv",
        );
        assert!(matches!(result, Err(TransferError::SameLocalEndpoints)));
    }

    #[test]
    fn a_destination_equal_to_or_inside_a_source_on_one_host_is_refused() {
        for directory in ["/srv/data", "/srv/data/", "/srv/data/nested/deeper"] {
            let host = remote("same");
            let result = between(host.clone(), &["/srv/data"], host, directory);
            assert!(
                matches!(result, Err(TransferError::DestinationInsideSource)),
                "{directory}"
            );
        }
    }

    #[test]
    fn a_sibling_or_another_host_may_receive_a_source_folder() {
        let host = remote("same");
        assert!(between(host.clone(), &["/srv/data"], host.clone(), "/srv/database").is_ok());
        assert!(between(remote("a"), &["/srv/data"], remote("b"), "/srv/data").is_ok());
    }

    #[test]
    fn relay_paths_must_be_absolute_and_plain() {
        for source in ["relative", "/srv/../etc", "/srv/./a", "/", "/srv/a\0"] {
            assert!(
                matches!(
                    between(remote("a"), &[source], remote("b"), "/srv"),
                    Err(TransferError::InvalidDestination)
                ),
                "{source:?}"
            );
        }
        assert!(matches!(
            between(remote("a"), &["/srv/a"], remote("b"), "../in"),
            Err(TransferError::InvalidDestination)
        ));
        assert!(matches!(
            between(remote("a"), &[], remote("b"), "/srv"),
            Err(TransferError::Empty)
        ));
    }
}
