//! The pipeline against a scripted reader, without D-Bus or fprintd.
use super::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex as StdMutex},
};

/// What the scripted reader does when asked.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Fault {
    #[default]
    None,
    ClaimFails,
    ReleaseHangs,
}

#[derive(Clone, Default)]
struct Reader {
    /// One script of `(status, done)` per scan; a finished script waits forever.
    scans: Arc<StdMutex<VecDeque<Vec<(&'static str, bool)>>>>,
    log: Arc<StdMutex<Vec<&'static str>>>,
    fault: Fault,
}

impl Reader {
    fn new(scans: Vec<Vec<(&'static str, bool)>>) -> Self {
        Self {
            scans: Arc::new(StdMutex::new(scans.into())),
            ..Self::default()
        }
    }

    fn with(mut self, fault: Fault) -> Self {
        self.fault = fault;
        self
    }

    fn log(&self) -> Vec<&'static str> {
        self.log.lock().unwrap().clone()
    }

    fn push(&self, call: &'static str) {
        self.log.lock().unwrap().push(call);
    }
}

struct ScriptedBackend {
    reader: Reader,
    client_present: AtomicBool,
}

impl ScriptedBackend {
    fn new(reader: &Reader) -> Self {
        Self {
            reader: reader.clone(),
            client_present: AtomicBool::new(true),
        }
    }
}

impl FingerprintBackend for ScriptedBackend {
    type Device = Reader;

    async fn device(&self, _owner: &Owner) -> fdo::Result<Reader> {
        Ok(self.reader.clone())
    }

    async fn client_present(&self, _owner: &Owner) -> bool {
        self.client_present.load(Ordering::SeqCst)
    }
}

impl FingerprintDevice for Reader {
    type Statuses = VecDeque<(&'static str, bool)>;

    async fn claim(&self) -> fdo::Result<()> {
        self.push("claim");
        if self.fault == Fault::ClaimFails {
            return Err(denied("Fingerprint reader is claimed elsewhere"));
        }
        Ok(())
    }

    async fn verify_start(&self) -> fdo::Result<Self::Statuses> {
        self.push("start");
        Ok(self
            .scans
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default()
            .into())
    }

    async fn verify_stop(&self) -> fdo::Result<()> {
        self.push("stop");
        Ok(())
    }

    async fn release(&self) -> fdo::Result<()> {
        self.push("release");
        if self.fault == Fault::ReleaseHangs {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}

impl VerifyStatuses for VecDeque<(&'static str, bool)> {
    async fn next(&mut self) -> fdo::Result<(String, bool)> {
        match self.pop_front() {
            Some((status, done)) => Ok((status.into(), done)),
            None => std::future::pending().await,
        }
    }
}

struct Fixture {
    store: Mutex<Store>,
    scanner: Mutex<()>,
    cancel: AtomicBool,
    owner: Owner,
    binding: String,
    token: String,
    scan_timeout: Duration,
}

impl Fixture {
    fn new() -> Self {
        let owner = Owner {
            uid: 1000,
            connection: ":1.42".into(),
        };
        let binding = "a".repeat(64);
        let mut store = Store::default();
        let token = store
            .enroll(
                owner.clone(),
                binding.clone(),
                zeroize::Zeroizing::new([42; 32]),
            )
            .unwrap();
        Self {
            store: Mutex::new(store),
            scanner: Mutex::new(()),
            cancel: AtomicBool::new(false),
            owner,
            binding,
            token,
            scan_timeout: Duration::from_secs(30),
        }
    }

    fn request(&self) -> Request<'_> {
        Request {
            attempts: Attempts {
                store: &self.store,
                owner: &self.owner,
                binding: &self.binding,
                token: &self.token,
            },
            scanner: &self.scanner,
            cancel: &self.cancel,
            scan_timeout: self.scan_timeout,
        }
    }

    async fn verify(&self, backend: &ScriptedBackend) -> fdo::Result<()> {
        verify(backend, &self.request()).await
    }

    async fn remaining(&self) -> u8 {
        self.store
            .lock()
            .await
            .entry(&self.owner, &self.binding, &self.token)
            .unwrap()
            .attempts_remaining
    }
}

fn message(result: fdo::Result<()>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn completed_scans_spend_and_restore_attempts() {
    assert_eq!(after_scan(3, false), (2, Next::Retry));
    assert_eq!(after_scan(2, false), (1, Next::Retry));
    assert_eq!(after_scan(1, false), (0, Next::Locked));
    assert_eq!(after_scan(0, false), (0, Next::Locked));
    assert_eq!(after_scan(1, true), (MAX_ATTEMPTS, Next::Accept));
}

#[test]
fn statuses_decide_a_scan_only_when_done_or_faulted() {
    assert_eq!(
        verification_status("verify-match", true).unwrap(),
        ScanStatus::Match
    );
    assert_eq!(
        verification_status("verify-no-match", true).unwrap(),
        ScanStatus::NoMatch
    );
    for hint in ["verify-match", "verify-no-match", "verify-retry-scan"] {
        assert_eq!(
            verification_status(hint, false).unwrap(),
            ScanStatus::Pending
        );
    }
    for fault in ["verify-disconnected", "verify-unknown-error"] {
        assert!(verification_status(fault, false).is_err());
    }
    assert!(verification_status("verify-retry-scan", true).is_err());
}

#[tokio::test]
async fn a_match_stops_and_releases_the_reader() {
    let reader = Reader::new(vec![vec![("verify-match", true)]]);
    let fixture = Fixture::new();
    fixture
        .verify(&ScriptedBackend::new(&reader))
        .await
        .unwrap();
    assert_eq!(reader.log(), ["claim", "start", "stop", "release"]);
    assert_eq!(fixture.remaining().await, MAX_ATTEMPTS);
}

#[tokio::test]
async fn mismatches_lock_the_registration_after_the_limit() {
    let reader = Reader::new(vec![vec![("verify-no-match", true)]; 3]);
    let fixture = Fixture::new();
    let backend = ScriptedBackend::new(&reader);
    assert!(message(fixture.verify(&backend).await).contains(LOCKED));
    assert_eq!(
        reader.log(),
        [
            "claim", "start", "stop", "start", "stop", "start", "stop", "release"
        ]
    );
    assert_eq!(fixture.remaining().await, 0);
    // A locked registration never reaches the reader again.
    assert!(message(fixture.verify(&backend).await).contains(LOCKED));
    assert_eq!(reader.log().len(), 8);
}

#[tokio::test]
async fn a_disconnected_reader_is_left_alone() {
    let reader = Reader::new(vec![vec![("verify-disconnected", false)]]);
    let fixture = Fixture::new();
    let error = message(fixture.verify(&ScriptedBackend::new(&reader)).await);
    assert!(error.contains("disconnected"), "{error}");
    assert_eq!(reader.log(), ["claim", "start"]);
    assert_eq!(fixture.remaining().await, MAX_ATTEMPTS);
}

#[tokio::test]
async fn the_deadline_ends_a_scan_and_still_cleans_up() {
    let reader = Reader::new(vec![vec![("verify-retry-scan", false)]]);
    let mut fixture = Fixture::new();
    fixture.scan_timeout = Duration::from_millis(100);
    let error = message(fixture.verify(&ScriptedBackend::new(&reader)).await);
    assert!(error.contains("timed out"), "{error}");
    assert_eq!(reader.log(), ["claim", "start", "stop", "release"]);
    assert_eq!(fixture.remaining().await, MAX_ATTEMPTS);
}

#[tokio::test]
async fn cancelling_during_a_scan_ends_it() {
    let reader = Reader::new(vec![vec![]]);
    let fixture = Fixture::new();
    let backend = ScriptedBackend::new(&reader);
    let cancel = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        fixture.cancel.store(true, Ordering::SeqCst);
    };
    let (result, ()) = tokio::join!(fixture.verify(&backend), cancel);
    assert!(message(result).contains("Authentication cancelled"));
    assert_eq!(reader.log(), ["claim", "start", "stop", "release"]);
}

#[tokio::test]
async fn a_departed_client_ends_the_scan() {
    let reader = Reader::new(vec![vec![]]);
    let fixture = Fixture::new();
    let backend = ScriptedBackend::new(&reader);
    backend.client_present.store(false, Ordering::SeqCst);
    assert!(message(fixture.verify(&backend).await).contains("Client disconnected"));
    assert_eq!(reader.log(), ["claim", "start", "stop", "release"]);
}

#[tokio::test]
async fn a_hung_release_is_bounded_by_the_cleanup_limit() {
    let reader = Reader::new(vec![vec![("verify-match", true)]]).with(Fault::ReleaseHangs);
    let fixture = Fixture::new();
    let started = std::time::Instant::now();
    fixture
        .verify(&ScriptedBackend::new(&reader))
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert!(elapsed >= CLEANUP_LIMIT, "{elapsed:?}");
    assert!(elapsed < CLEANUP_LIMIT * 3, "{elapsed:?}");
    assert_eq!(reader.log(), ["claim", "start", "stop", "release"]);
}

#[tokio::test]
async fn a_failed_claim_has_nothing_to_clean_up() {
    let reader = Reader::new(vec![vec![("verify-match", true)]]).with(Fault::ClaimFails);
    let fixture = Fixture::new();
    let error = message(fixture.verify(&ScriptedBackend::new(&reader)).await);
    assert!(error.contains("claimed elsewhere"), "{error}");
    assert_eq!(reader.log(), ["claim"]);
}

#[tokio::test]
async fn a_second_verification_finds_the_reader_busy() {
    let reader = Reader::new(vec![vec![], vec![("verify-match", true)]]);
    let fixture = Fixture::new();
    let backend = ScriptedBackend::new(&reader);
    let second = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let result = fixture.verify(&backend).await;
        fixture.cancel.store(true, Ordering::SeqCst);
        result
    };
    let (first, second) = tokio::join!(fixture.verify(&backend), second);
    assert!(message(second).contains("busy"));
    assert!(message(first).contains("Authentication cancelled"));
    // The rejected request never touched the reader.
    assert_eq!(reader.log(), ["claim", "start", "stop", "release"]);
}
