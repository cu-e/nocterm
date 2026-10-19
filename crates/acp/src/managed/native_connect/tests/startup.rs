//! Native startup refuses oversized protocol lines and stops the process tree.
use super::*;
use std::time::Duration;
#[test]
fn oversized_subprocess_line_fails_initialization_and_stops_processes() {
    let directory = tempfile::tempdir().unwrap();
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let connector = AcpConnector::try_unmanaged().unwrap();
    let (_producer, request) = request(
        &authority,
        directory.path(),
        "head -c 16777217 /dev/zero | tr '\\000' x; printf '\\n'; sleep 60",
    );
    let started = connector.connect_managed(request).unwrap();
    match block_on(futures::future::select(
        started,
        Box::pin(async_io::Timer::after(Duration::from_secs(5))),
    )) {
        futures::future::Either::Left((result, _)) => assert!(result.is_err()),
        futures::future::Either::Right(_) => {
            panic!("oversized protocol line did not terminate initialization")
        }
    }
}
