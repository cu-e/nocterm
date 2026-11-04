
use super::*;
#[test]
fn literals_and_local_names_never_use_dns() {
    for host in [
        "192.168.1.10",
        "[::1]",
        "pi.local",
        "localhost",
        "",
        "8.8.8.8",
    ] {
        let result =
            resolve_with::<Vec<IpAddr>>(host, |_| panic!("unexpected DNS for {host}")).unwrap();
        assert_eq!(result.is_some(), host == "8.8.8.8");
    }
}
#[test]
fn resolution_selects_first_public_address_only() {
    let found = resolve_with("example.com", |_| {
        Ok(vec![
            "10.0.0.1".parse().unwrap(),
            "8.8.8.8".parse().unwrap(),
        ])
    })
    .unwrap()
    .unwrap();
    assert_eq!(found.as_ip(), "8.8.8.8".parse::<IpAddr>().unwrap());
    assert!(matches!(
        resolve_with::<Vec<IpAddr>>("example.com", |_| Err(std::io::Error::other("DNS failed"))),
        Err(ObservationError::Resolution(_))
    ));
}

#[test]
fn stalled_dns_keeps_only_four_workers_and_expired_queued_queries_are_skipped() {
    use std::sync::{
        Condvar,
        atomic::{AtomicUsize, Ordering},
    };
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let started = Arc::new(AtomicUsize::new(0));
    let (began, began_rx) = mpsc::channel();
    let lookup: Lookup = {
        let gate = gate.clone();
        let started = started.clone();
        Arc::new(move |_| {
            started.fetch_add(1, Ordering::SeqCst);
            began.send(()).unwrap();
            let (lock, ready) = &*gate;
            let _ = ready
                .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(5), |open| !*open)
                .unwrap();
            Ok(vec!["8.8.8.8".parse().unwrap()])
        })
    };
    let resolver = Arc::new(NativeResolver {
        requests: OnceLock::from(Ok(workers(lookup).unwrap())),
        deadline: Duration::from_millis(100),
    });
    let calls: Vec<_> = (0..DNS_WORKERS)
        .map(|_| {
            let resolver = resolver.clone();
            thread::spawn(move || resolver.resolve("stalled.example"))
        })
        .collect();
    for _ in 0..DNS_WORKERS {
        began_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }
    for call in calls {
        assert!(call.join().unwrap().is_err());
    }
    assert!(resolver.resolve("expires.example").is_err());
    assert_eq!(started.load(Ordering::SeqCst), DNS_WORKERS);
    {
        let (lock, ready) = &*gate;
        *lock.lock().unwrap() = true;
        ready.notify_all();
    }
    let resolved = resolver.resolve("ready.example").unwrap().unwrap();
    assert_eq!(resolved.as_ip(), "8.8.8.8".parse::<IpAddr>().unwrap());
    assert_eq!(started.load(Ordering::SeqCst), DNS_WORKERS + 1);
}
