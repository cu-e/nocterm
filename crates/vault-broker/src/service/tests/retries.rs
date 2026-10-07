use super::*;

struct Fixture {
    _bus: Bus,
    _fake: Connection,
    _client: Connection,
    broker: Broker,
    owner: Owner,
    calls: Arc<AtomicUsize>,
    events: Arc<std::sync::Mutex<Vec<String>>>,
    cancel: Arc<AtomicBool>,
    binding: String,
    token: String,
}
impl Fixture {
    async fn new(scripts: Vec<Vec<(&str, bool)>>, mode: usize, timeout: Duration) -> Self {
        let bus = Bus::new();
        let service = connect(&bus).await;
        let fake = connect(&bus).await;
        let client = connect(&bus).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        fake.object_server()
            .at("/net/reactivated/Fprint/Manager", Manager)
            .await
            .unwrap();
        fake.object_server()
            .at(
                "/net/reactivated/Fprint/Device/0",
                Device {
                    calls: calls.clone(),
                    mode: Arc::new(AtomicUsize::new(mode)),
                    scripts: Arc::new(std::sync::Mutex::new(
                        scripts
                            .into_iter()
                            .map(|script| {
                                script
                                    .into_iter()
                                    .map(|(status, done)| (status.to_string(), done))
                                    .collect()
                            })
                            .collect(),
                    )),
                    events: events.clone(),
                    cancel: cancel.clone(),
                },
            )
            .await
            .unwrap();
        fake.request_name(FPRINT).await.unwrap();
        let owner = Owner {
            uid: nix::unistd::getuid().as_raw(),
            connection: client.unique_name().unwrap().to_string(),
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
            binding,
            token,
            broker: Broker {
                store: Arc::new(Mutex::new(store)),
                scanner: Mutex::new(()),
                connection: service,
                fingerprint_uid: nix::unistd::getuid().as_raw(),
                scan_timeout: timeout,
            },
            owner,
            calls,
            events,
            cancel,
            _bus: bus,
            _fake: fake,
            _client: client,
        }
    }
    async fn verify(&self) -> fdo::Result<()> {
        self.broker
            .verify(&self.owner, &self.binding, &self.token, self.cancel.clone())
            .await
    }
}

#[tokio::test]
async fn matches_on_each_attempt_stop_immediately_and_order_retries() {
    for matched_attempt in 1..=3 {
        let mut scripts = vec![vec![("verify-no-match", true)]; matched_attempt - 1];
        scripts.push(vec![("verify-match", true)]);
        let fixture = Fixture::new(scripts, 1, Duration::from_secs(30)).await;
        fixture.verify().await.unwrap();
        assert_eq!(fixture.calls.load(Ordering::SeqCst), matched_attempt);
        let mut expected = Vec::new();
        for _ in 0..matched_attempt {
            expected.extend(["start", "stop"]);
        }
        expected.push("release");
        assert_eq!(*fixture.events.lock().unwrap(), expected);
    }
}

#[tokio::test]
async fn three_non_matches_exhaust_the_limit_and_ignore_stale_success_on_stop() {
    let fixture = Fixture::new(vec![], 1, Duration::from_secs(30)).await;
    let error = fixture.verify().await.unwrap_err().to_string();
    assert!(
        error.contains("Fingerprint unlock is locked after 3 failed attempts"),
        "{error}"
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        *fixture.events.lock().unwrap(),
        ["start", "stop", "start", "stop", "start", "stop", "release"]
    );
}

#[tokio::test]
async fn scan_hints_do_not_consume_attempts() {
    let fixture = Fixture::new(
        vec![vec![
            ("verify-retry-scan", false),
            ("verify-swipe-too-short", false),
            ("verify-finger-not-centered", false),
            ("verify-remove-and-retry", false),
            ("verify-too-fast", false),
            ("verify-no-match", false),
            ("verify-match", false),
            ("verify-match", true),
        ]],
        1,
        Duration::from_secs(30),
    )
    .await;
    fixture.verify().await.unwrap();
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn terminal_faults_report_the_cause_without_retrying() {
    for (status, description) in [
        ("verify-disconnected", "Fingerprint reader disconnected"),
        (
            "verify-unknown-error",
            "Fingerprint reader reported an unknown error",
        ),
        (
            "verify-future-fault",
            "Fingerprint verification failed: verify-future-fault",
        ),
    ] {
        let fixture = Fixture::new(vec![vec![(status, true)]], 1, Duration::from_secs(30)).await;
        let error = fixture.verify().await.unwrap_err().to_string();
        assert!(error.contains(description), "{error}");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        if status == "verify-disconnected" {
            assert_eq!(*fixture.events.lock().unwrap(), ["start"]);
        }
    }
}

#[tokio::test]
async fn start_and_stop_errors_are_preserved_without_retrying() {
    for (mode, description) in [
        (4, "could not start scanning"),
        (5, "could not stop scanning"),
    ] {
        let fixture = Fixture::new(
            vec![vec![("verify-no-match", true)]],
            mode,
            Duration::from_secs(30),
        )
        .await;
        let error = fixture.verify().await.unwrap_err().to_string();
        assert!(error.contains(description), "{error}");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            fixture
                .broker
                .store
                .lock()
                .await
                .entry(&fixture.owner, &fixture.binding, &fixture.token)
                .unwrap()
                .attempts_remaining,
            if mode == 4 { 3 } else { 2 },
            "a completed mismatch consumes a try even when stopping the reader fails"
        );
    }
}

#[tokio::test]
async fn cancellation_between_attempts_prevents_a_new_scan() {
    let fixture = Fixture::new(
        vec![vec![("verify-no-match", true)]],
        7,
        Duration::from_secs(30),
    )
    .await;
    let error = fixture.verify().await.unwrap_err().to_string();
    assert!(error.contains("Authentication cancelled"), "{error}");
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retries_share_one_deadline_including_scan_setup() {
    let fixture = Fixture::new(
        vec![
            vec![("verify-no-match", true)],
            vec![("verify-match", true)],
        ],
        6,
        Duration::from_millis(250),
    )
    .await;
    let error = fixture.verify().await.unwrap_err().to_string();
    assert!(
        error.contains("Fingerprint authentication timed out"),
        "{error}"
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn remaining_attempts_are_published_and_lockout_prevents_another_scan() {
    let fixture = Fixture::new(vec![], 6, Duration::from_secs(30)).await;
    // Delay each scan's completion so each persisted decrement can be observed.
    fixture
        ._fake
        .object_server()
        .interface::<_, Device>("/net/reactivated/Fprint/Device/0")
        .await
        .unwrap()
        .get()
        .await
        .scripts
        .lock()
        .unwrap()
        .extend(vec![vec![("verify-no-match".into(), true)]; 3]);
    let remaining = async {
        let mut observed = vec![3];
        loop {
            let value = fixture
                .broker
                .store
                .lock()
                .await
                .entry(&fixture.owner, &fixture.binding, &fixture.token)
                .unwrap()
                .attempts_remaining;
            if observed.last() != Some(&value) {
                observed.push(value);
            }
            if value == 0 {
                break observed;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    let (result, observed) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(fixture.verify(), remaining)
    })
    .await
    .expect("completed mismatches must publish all remaining attempts before lockout");
    assert!(result.is_err());
    assert_eq!(observed, [3, 2, 1, 0]);
    assert!(fixture.verify().await.is_err());
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn failures_survive_cancellation_faults_and_timeout_but_match_resets() {
    for (mode, second, description) in [
        (7, "verify-match", "Authentication cancelled"),
        (1, "verify-unknown-error", "unknown error"),
        (2, "verify-retry-scan", "timed out"),
    ] {
        let fixture = Fixture::new(
            vec![vec![("verify-no-match", true)], vec![(second, mode == 1)]],
            mode,
            Duration::from_millis(250),
        )
        .await;
        assert!(
            fixture
                .verify()
                .await
                .unwrap_err()
                .to_string()
                .contains(description)
        );
        assert_eq!(
            fixture
                .broker
                .store
                .lock()
                .await
                .entry(&fixture.owner, &fixture.binding, &fixture.token)
                .unwrap()
                .attempts_remaining,
            2
        );
        fixture.cancel.store(false, Ordering::SeqCst);
        let interface = fixture
            ._fake
            .object_server()
            .interface::<_, Device>("/net/reactivated/Fprint/Device/0")
            .await
            .unwrap();
        interface.get().await.mode.store(0, Ordering::SeqCst);
        interface.get().await.scripts.lock().unwrap().clear();
        fixture.verify().await.unwrap();
        assert_eq!(
            fixture
                .broker
                .store
                .lock()
                .await
                .entry(&fixture.owner, &fixture.binding, &fixture.token)
                .unwrap()
                .attempts_remaining,
            3
        );
    }
}
