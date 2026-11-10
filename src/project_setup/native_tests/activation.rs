//! A real wrapped local session consumes the production activation path.
#![cfg(unix)]
use super::*;
use nocterm_project_providers::{LocalTransportFactory, WorkspaceSessionFactory as _};
use nocterm_session::{CloseReason, Event};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt as _};

fn executable(path: &std::path::Path, script: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn a_native_workspace_shell_gets_managed_exports_without_sourcing_mutable_files() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    let base = home.join(".local/share/nocterm/toolchains");
    for probe in [
        "rust/cargo/bin/rustup",
        "bun/current/bin/bun",
        "java/current/bin/java",
    ] {
        executable(&base.join(probe), "#!/bin/sh\nexit 0\n");
    }
    executable(
        &base.join("bin/node"),
        "#!/bin/sh\nprintf 'managed-node\\n'\n",
    );
    std::fs::write(base.join("env.sh"), "printf 'MUTABLE_FILE_EXECUTED\\n'\n").unwrap();
    let factory: LocalTransportFactory = Arc::new(nocterm_local::LocalTransport::prepared);
    let transport = nocterm_project_environment::NativeWorkspaceSessions.wrap_local(factory);
    let session = transport.open(ConnectRequest {
        target: Target::new("native-test", "this-computer", 22), auth: Auth::Auto,
        jumps: vec![], proxy: Default::default(), term: "xterm".into(), size: Default::default(),
        connect_timeout: Duration::from_secs(5), keepalive_interval: None,
        launch: ShellLaunch {
            program: Some("/bin/sh".into()), integration: false,
            args: vec!["-c".into(), "printf 'cargo=%s\\nrustup=%s\\nbun=%s\\njava=%s\\n' \"$CARGO_HOME\" \"$RUSTUP_HOME\" \"$BUN_INSTALL\" \"$JAVA_HOME\"; node".into()],
            env: BTreeMap::from([("HOME".into(), home.to_string_lossy().into_owned())]),
            ..Default::default()
        },
    });
    let collect = async {
        let mut output = Vec::new();
        loop {
            match session.next_event().await {
                Some(Event::Output(bytes)) => output.extend(bytes),
                Some(Event::Closed(reason)) => {
                    assert_eq!(reason, CloseReason::Exited(Some(0)));
                    break String::from_utf8_lossy(&output).into_owned();
                }
                Some(_) => {}
                None => panic!("native session ended without a close event"),
            }
        }
    };
    let timeout = async_io::Timer::after(Duration::from_secs(10));
    futures::pin_mut!(collect, timeout);
    let output = match block_on(futures::future::select(collect, timeout)) {
        futures::future::Either::Left((output, _)) => output,
        futures::future::Either::Right(_) => {
            session.close();
            panic!("native activation timed out");
        }
    };
    for (name, relative) in [
        ("cargo", "rust/cargo"),
        ("rustup", "rust/rustup"),
        ("bun", "bun/current"),
        ("java", "java/current"),
    ] {
        assert!(
            output.contains(&format!("{name}={}", base.join(relative).display())),
            "{output}"
        );
    }
    assert!(output.contains("managed-node"));
    assert!(!output.contains("MUTABLE_FILE_EXECUTED"));
}
