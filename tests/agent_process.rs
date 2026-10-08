//! Real Linux containment tests. Set NOCTERM_REQUIRE_SYSTEMD=1 to forbid skips.
#![cfg(target_os = "linux")]

use nocterm_acp::AcpConnector;
use nocterm_ai::{AgentCommands, AgentConnector as _, AgentLaunch, ConnectRequest, acp};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

static PROCESS_TESTS: Mutex<()> = Mutex::new(());
struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct ServiceGuard {
    runner: ChildGuard,
    unit: String,
}
impl Drop for ServiceGuard {
    fn drop(&mut self) {
        let _ = Command::new("systemctl")
            .args([
                "--user",
                "kill",
                "--signal=KILL",
                "--kill-whom=all",
                &self.unit,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("systemctl")
            .args(["--user", "stop", "--no-block", &self.unit])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn available() -> bool {
    let available = Command::new("systemctl")
        .args(["--user", "show", "--property=Version", "--value"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !available {
        assert!(
            std::env::var_os("NOCTERM_REQUIRE_SYSTEMD").is_none(),
            "systemd user manager required for agent containment tests"
        );
        // The skip must be visible with the usual --nocapture test flag.
        #[expect(
            clippy::print_stderr,
            reason = "explicit integration prerequisite skip"
        )]
        {
            eprintln!("SKIP agent containment: systemd user manager unavailable");
        }
    }
    available
}

fn running(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(") ")
            .is_some_and(|(_, tail)| !tail.starts_with('Z'))
    })
}
fn assert_gone(pids: &[u32]) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while pids.iter().any(|pid| running(*pid)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        pids.iter().all(|pid| !running(*pid)),
        "Agent processes still execute: {pids:?}"
    );
}
fn fake(dir: &Path) -> PathBuf {
    let script = dir.join("agent.py");
    fs::write(&script,r#"import json, os, signal, subprocess, sys, time
child = subprocess.Popen([sys.executable, '-c', 'import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(120)'], start_new_session=True, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
with open('pids', 'w') as f:
    f.write(str(os.getpid()) + ' ' + str(child.pid))
with open('environment', 'w') as f:
    json.dump({'unapproved':os.environ.get('UNAPPROVED_SECRET'), 'expected':os.environ.get('EXPECTED'), 'argument':sys.argv[-1]}, f)
for line in sys.stdin:
    msg = json.loads(line)
    method = msg.get('method')
    if method == 'initialize':
        if os.path.exists('delay-initialize'): time.sleep(120)
        result = {'protocolVersion':1, 'agentCapabilities': {'sessionCapabilities': {'close':{}}}}
    elif method == 'session/new':
        result = {'sessionId':'test-session'}
    elif method == 'session/close':
        result = {}
    else:
        result = {}
    if 'id' in msg:
        print(json.dumps({'jsonrpc':'2.0', 'id':msg['id'], 'result':result}), flush=True)
"#).unwrap();
    script
}
fn request(dir: &Path) -> ConnectRequest {
    let script = fake(dir);
    ConnectRequest {
        launch: AgentLaunch {
            id: "test".into(),
            name: "Test".into(),
            command: "/usr/bin/python3".into(),
            args: vec![script.to_string_lossy().into_owned(), "$LITERAL".into()],
            env: BTreeMap::from([("EXPECTED".into(), "forwarded".into())]),
            inherit_env: Vec::new(),
        },
        working_directory: dir.to_owned(),
        terminal_auth: false,
        sandbox: None,
        resources: Default::default(),
        cancellation: Default::default(),
    }
}
fn connection(dir: &Path) -> (Arc<dyn AgentCommands>, std::thread::JoinHandle<()>) {
    let request = request(dir);
    futures::executor::block_on(async {
        let connecting =
            AcpConnector::managed(PathBuf::from(env!("CARGO_BIN_EXE_nocterm"))).connect(request);
        let connection = connecting.await.expect("managed connection");
        let events = std::thread::spawn(move || {
            futures::executor::block_on(async move {
                while let Ok(event) = connection.events.recv().await {
                    if let nocterm_ai::AgentEvent::Barrier(ack) = event {
                        let _ = ack.send(());
                    }
                }
            })
        });
        (connection.commands, events)
    })
}
fn pids(dir: &Path) -> Vec<u32> {
    fs::read_to_string(dir.join("pids"))
        .unwrap()
        .split_whitespace()
        .map(|pid| pid.parse().unwrap())
        .collect()
}

#[test]
fn repeated_managed_close_reaps_detached_descendants_and_fds() {
    let _serial = PROCESS_TESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !available() {
        return;
    }
    // No other tests share this process's connector or file descriptor count.
    let descriptors = || fs::read_dir("/proc/self/fd").unwrap().count();
    let mut baseline = None;
    for cycle in 0..6 {
        let dir = tempfile::tempdir().unwrap();
        let (commands, events) = connection(dir.path());
        let session = futures::executor::block_on(
            commands.new_session(acp::NewSessionRequest::new(dir.path())),
        )
        .unwrap();
        let children = pids(dir.path());
        let environment: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("environment")).unwrap())
                .unwrap();
        assert_eq!(environment["expected"], "forwarded");
        assert_eq!(environment["argument"], "$LITERAL");
        if cycle == 0 {
            let membership = fs::read_to_string(format!("/proc/{}/cgroup", children[0])).unwrap();
            let path = membership
                .lines()
                .find_map(|line| line.strip_prefix("0::"))
                .unwrap();
            let cgroup = PathBuf::from("/sys/fs/cgroup").join(path.trim_start_matches('/'));
            assert_eq!(
                fs::read_to_string(cgroup.join("memory.high"))
                    .unwrap()
                    .trim(),
                (2048u64 * 1024 * 1024).to_string()
            );
            assert_eq!(
                fs::read_to_string(cgroup.join("memory.max"))
                    .unwrap()
                    .trim(),
                (4096u64 * 1024 * 1024).to_string()
            );
            assert_eq!(
                fs::read_to_string(cgroup.join("memory.swap.max"))
                    .unwrap()
                    .trim(),
                (1024u64 * 1024 * 1024).to_string()
            );
            assert_eq!(
                fs::read_to_string(cgroup.join("pids.max")).unwrap().trim(),
                "512"
            );
        }
        let outcome =
            futures::executor::block_on(commands.close_session(session.session_id)).unwrap();
        assert_eq!(outcome, nocterm_ai::CloseSessionOutcome::Closed);
        futures::executor::block_on(commands.shutdown_gracefully()).unwrap();
        drop(commands);
        events.join().unwrap();
        assert_gone(&children);
        std::thread::sleep(Duration::from_millis(100));
        if cycle == 0 {
            baseline = Some(descriptors());
        }
    }
    assert!(
        descriptors() <= baseline.unwrap(),
        "Descriptor count grew across repeated cleanup cycles"
    );
}

#[test]
fn service_guardian_cleans_cgroup_when_owner_is_killed() {
    let _serial = PROCESS_TESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = fake(dir.path());
    let mut owner = ChildGuard(Command::new("/bin/sleep").arg("60").spawn().unwrap());
    let owner_stat = fs::read_to_string(format!("/proc/{}/stat", owner.0.id())).unwrap();
    let start = owner_stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap();
    let unit = format!("nocterm-test-owner-{}.service", owner.0.id());
    let service = Command::new("systemd-run")
        .args([
            "--user",
            "--quiet",
            "--pipe",
            "--wait",
            "--collect",
            "--service-type=exec",
            "--expand-environment=no",
            "--property=KillMode=control-group",
            "--property=TimeoutStopSec=1s",
        ])
        .arg("--setenv=UNAPPROVED_SECRET=must-not-inherit")
        .arg(format!("--unit={unit}"))
        .arg(format!("--working-directory={}", dir.path().display()))
        .arg(env!("CARGO_BIN_EXE_nocterm"))
        .arg("agent-host")
        .arg(owner.0.id().to_string())
        .arg(start)
        .arg(dir.path())
        .arg("")
        .arg("/usr/bin/python3")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut service = ServiceGuard {
        runner: ChildGuard(service),
        unit: unit.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(8);
    while !dir.path().join("pids").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    if !dir.path().join("pids").exists() {
        let _ = owner.0.kill();
        let _ = owner.0.wait();
        let _ = Command::new("systemctl")
            .args(["--user", "stop", &unit])
            .status();
        let _ = service.runner.0.kill();
        let _ = service.runner.0.wait();
        panic!("Service guardian did not launch the test agent");
    }
    let children = pids(dir.path());
    let environment: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("environment")).unwrap()).unwrap();
    assert!(
        environment["unapproved"].is_null(),
        "User manager environment escaped the helper whitelist"
    );
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    assert_gone(&children);
    service.runner.0.wait().unwrap();
}

#[test]
fn cancelling_initialization_cleans_an_already_launched_tree() {
    let _serial = PROCESS_TESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let request = request(dir.path());
    fs::write(dir.path().join("delay-initialize"), "").unwrap();
    let (cancel, cancelled) = futures::channel::oneshot::channel::<()>();
    let worker = std::thread::spawn(move || {
        futures::executor::block_on(async {
            let connecting = AcpConnector::managed(PathBuf::from(env!("CARGO_BIN_EXE_nocterm")))
                .connect(request);
            match futures::future::select(connecting, cancelled).await {
                futures::future::Either::Right(_) => {}
                futures::future::Either::Left((result, _)) => panic!(
                    "Initialization did not wait for cancellation: {}",
                    result
                        .err()
                        .map_or("connected".into(), |error| error.to_string())
                ),
            }
        })
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    while !dir.path().join("pids").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    let launched = dir.path().join("pids").exists();
    cancel.send(()).unwrap();
    worker.join().unwrap();
    assert!(launched, "Delayed agent never started");
    assert_gone(&pids(dir.path()));
}

#[test]
fn startup_cancellation_is_acknowledged_after_cgroup_cleanup() {
    let _serial = PROCESS_TESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let request = request(dir.path());
    let cancellation = request.cancellation.clone();
    fs::write(dir.path().join("delay-initialize"), "").unwrap();
    let worker = std::thread::spawn(move || {
        futures::executor::block_on(
            AcpConnector::managed(PathBuf::from(env!("CARGO_BIN_EXE_nocterm"))).connect(request),
        )
        .err()
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    while !dir.path().join("pids").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    let launched = dir.path().join("pids").exists();
    cancellation.cancel();
    let error = worker
        .join()
        .unwrap()
        .expect("cancelled connection must not become ready");
    assert!(launched, "Delayed agent never started");
    assert!(error.to_string().contains("cancelled"), "{error}");
    // Unlike dropping the connecting future, this is an acknowledgement:
    // no eventual wait is permitted after the cancellation returns.
    assert!(
        pids(dir.path()).iter().all(|pid| !running(*pid)),
        "Cancellation returned before the process tree was stopped"
    );
}
