use super::*;
use futures::executor::block_on;
use nocterm_session::{
    Auth, ProxyConfig, PtySize, Target,
    testing::{ExecResponse, FakeExec, InMemoryFs, ScriptedTransport},
};
use std::{collections::BTreeMap, time::Duration};

fn connection(launch: ShellLaunch) -> ConnectRequest {
    ConnectRequest {
        target: Target::new("original-user", "original-server", 2222),
        auth: Auth::Auto,
        jumps: vec![],
        term: "xterm".into(),
        size: PtySize::default(),
        proxy: ProxyConfig::default(),
        launch,
        connect_timeout: Duration::from_secs(9),
        keepalive_interval: None,
    }
}

#[test]
fn workspace_activation_keeps_original_server_filesystem_and_literal_execution() {
    let base = ScriptedTransport::default();
    let exec = FakeExec::default();
    exec.respond("sh", Ok(ExecResponse::text("literal", 0, "")));
    base.queue(
        Some(Arc::new(InMemoryFs::default())),
        Some(Arc::new(exec.clone())),
    );
    let wrapped = NativeWorkspaceSessions.wrap(Arc::new(base.clone()));
    let session = wrapped.open(connection(ShellLaunch {
        program: Some("/bin/zsh".into()),
        args: vec!["-l".into()],
        cwd: Some("/project with spaces".into()),
        env: BTreeMap::from([("KEEP_ORIGINAL".into(), "value".into())]),
        ..Default::default()
    }));
    assert!(session.fs().is_some());
    let launches = base.requests();
    assert_eq!(
        launches[0].target,
        Target::new("original-user", "original-server", 2222)
    );
    assert_eq!(
        launches[0].launch.cwd.as_deref(),
        Some("/project with spaces")
    );
    assert_eq!(
        launches[0]
            .launch
            .env
            .get("KEEP_ORIGINAL")
            .map(String::as_str),
        Some("value")
    );
    assert_eq!(&launches[0].launch.args[3..], ["/bin/zsh", "-l"]);
    block_on(
        session.exec().unwrap().exec(
            ExecRequest::new("printf")
                .args(["%s", "$(literal); still literal"])
                .stdin(b"input".to_vec()),
        ),
    )
    .unwrap();
    let requests = exec.requests();
    assert_eq!(
        &requests[0].args[3..],
        ["printf", "%s", "$(literal); still literal"]
    );
    assert_eq!(requests[0].stdin.as_deref(), Some(b"input".as_slice()));
    assert!(!requests[0].args[1].contains(". \""));
}

#[test]
fn local_workspace_activation_passes_the_activated_launch_into_native_factory() {
    let base = ScriptedTransport::default();
    base.queue(None, None);
    let recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let launches = recorded.clone();
    let native = Arc::new(move |launch: ShellLaunch| {
        launches.lock().unwrap().push(launch);
        Arc::new(base.clone()) as Arc<dyn Transport>
    });
    let session = NativeWorkspaceSessions
        .wrap_local(native)
        .open(connection(ShellLaunch {
            program: Some("/bin/bash".into()),
            args: vec!["--noprofile".into()],
            ..Default::default()
        }));
    drop(session);
    let recorded = recorded.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].program.as_deref(), Some("/bin/bash"));
    assert_eq!(recorded[0].args, ["--noprofile"]);
    assert!(recorded[0].integration);
}

#[test]
fn default_workspace_shell_is_resolved_on_its_original_host() {
    let base = ScriptedTransport::default();
    base.queue(None, None);
    NativeWorkspaceSessions
        .wrap(Arc::new(base.clone()))
        .open(connection(ShellLaunch::default()));
    let requests = base.requests();
    assert_eq!(requests[0].launch.args[3], "");
    assert!(requests[0].launch.args[1].contains("${SHELL:-/bin/sh}"));
    assert!(!requests[0].launch.args[1].contains("/etc/profile"));
}
