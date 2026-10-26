use super::*;
use nocterm_session::{
    Auth, ConnectRequest, ExecRequest, ProxyConfig, PtySize, ShellLaunch, Target, TerminalRequest,
    testing::{InMemoryFs, ScriptedTransport},
};
use std::time::Duration;

fn connection() -> ConnectRequest {
    ConnectRequest {
        target: Target::new("user", "original", 22),
        auth: Auth::Auto,
        jumps: vec![],
        term: "xterm".into(),
        size: PtySize::default(),
        proxy: ProxyConfig::default(),
        launch: ShellLaunch::default(),
        connect_timeout: Duration::from_secs(5),
        keepalive_interval: None,
    }
}

#[test]
fn container_session_launch_and_nested_execution_keep_original_engine_and_scope_filesystem() {
    let base = ScriptedTransport::default();
    let exec = FakeExec::default();
    answer(&exec, "podman", String::new());
    base.queue(
        Some(Arc::new(InMemoryFs::default())),
        Some(Arc::new(exec.clone())),
    );
    let location = ContainerLocation::new(
        HostLocator::Local,
        Engine::Podman,
        id(),
        RootPath::parse("/project space").unwrap(),
        ContainerOrigin::Attached,
    )
    .unwrap();
    let transport = container_sessions().wrap(Arc::new(base.clone()), location);
    let session = transport.open(connection());
    assert!(session.fs().is_some());
    assert_eq!(base.requests()[0].launch.program.as_deref(), Some("podman"));
    assert!(
        base.requests()[0]
            .launch
            .args
            .contains(&"/project space".into())
    );
    let output = block_on(
        session.exec().unwrap().exec(
            ExecRequest::new("printf")
                .arg("$(literal)")
                .stdin(b"input".to_vec()),
        ),
    )
    .unwrap();
    drop(output);
    let actual = &exec.requests()[0];
    assert_eq!(actual.program, "podman");
    assert_eq!(
        &actual.args[..5],
        &[
            "exec",
            "--interactive",
            "--workdir",
            "/project space",
            &id()
        ]
    );
    assert_eq!(
        &actual.args[actual.args.len() - 2..],
        &["printf", "$(literal)"]
    );
    assert!(
        actual
            .args
            .iter()
            .any(|argument| argument.contains("nocterm/toolchains"))
    );
    assert_eq!(actual.stdin.as_deref(), Some(b"input".as_slice()));
    let nested = session.exec().unwrap().terminal(TerminalRequest {
        program: ExecRequest::new("sh"),
        term: "xterm".into(),
        size: PtySize::default(),
    });
    assert!(Arc::ptr_eq(&session.fs().unwrap(), &nested.fs().unwrap()));
    let output = block_on(
        nested
            .exec()
            .unwrap()
            .exec(ExecRequest::new("printf").arg("nested")),
    )
    .unwrap();
    drop(output);
    let requests = exec.requests();
    assert_eq!(
        requests[1]
            .args
            .iter()
            .filter(|argument| *argument == &id())
            .count(),
        1
    );
}

#[test]
fn local_container_factory_receives_the_container_launch_before_opening_native_transport() {
    let base = ScriptedTransport::default();
    base.queue(Some(Arc::new(InMemoryFs::default())), None);
    let launches = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = launches.clone();
    let factory = Arc::new(move |launch: ShellLaunch| {
        recorded.lock().unwrap().push(launch);
        Arc::new(base.clone()) as Arc<dyn nocterm_session::Transport>
    });
    let location = ContainerLocation::new(
        HostLocator::Local,
        Engine::Docker,
        id(),
        RootPath::parse("/app").unwrap(),
        ContainerOrigin::Attached,
    )
    .unwrap();
    let transport = container_sessions().wrap_local(factory, location);
    let session = transport.open(ConnectRequest {
        target: Target::new("user", "localhost", 22),
        auth: Auth::Auto,
        jumps: vec![],
        term: "xterm".into(),
        size: PtySize::default(),
        proxy: ProxyConfig::default(),
        launch: ShellLaunch {
            program: Some("host-shell".into()),
            ..Default::default()
        },
        connect_timeout: Duration::from_secs(5),
        keepalive_interval: None,
    });
    assert!(session.fs().is_none());
    let launches = launches.lock().unwrap();
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].program.as_deref(), Some("docker"));
    assert!(launches[0].args.contains(&id()));
    assert!(launches[0].cwd.is_none());
}
