//! Fixed workspace environment exports preserve original command arguments.
use nocterm_project_providers::{LocalTransportFactory, WorkspaceSessionFactory};
use nocterm_session::{
    ConnectRequest, ExecFuture, ExecOutput, ExecRequest, HostExec, Session, ShellLaunch,
    TerminalRequest, Transport,
};
use std::sync::Arc;

// Match the managed installer layout, without executing a mutable environment file.
const ENVIRONMENT: &str = r#"
if [ -n "${HOME:-}" ]; then
    nocterm_toolchains="$HOME/.local/share/nocterm/toolchains"
    if [ -d "$nocterm_toolchains" ] && [ ! -L "$nocterm_toolchains" ]; then
        export PATH="$nocterm_toolchains/bin:$nocterm_toolchains/rust/cargo/bin:$nocterm_toolchains/bun/current/bin:$nocterm_toolchains/node/current/bin:$nocterm_toolchains/java/current/bin:$PATH"
        if [ -x "$nocterm_toolchains/rust/cargo/bin/rustup" ]; then
            export CARGO_HOME="$nocterm_toolchains/rust/cargo"
            export RUSTUP_HOME="$nocterm_toolchains/rust/rustup"
        fi
        if [ -x "$nocterm_toolchains/bun/current/bin/bun" ]; then
            export BUN_INSTALL="$nocterm_toolchains/bun/current"
        fi
        if [ -x "$nocterm_toolchains/java/current/bin/java" ]; then
            export JAVA_HOME="$nocterm_toolchains/java/current"
        fi
    fi
    unset nocterm_toolchains
fi
"#;

pub(crate) fn activate_request(request: ExecRequest) -> ExecRequest {
    let mut activated = ExecRequest::new("sh")
        .args([
            "-c",
            &format!("{ENVIRONMENT}\nexec \"$@\""),
            "nocterm-workspace",
        ])
        .arg(request.program)
        .args(request.args);
    activated.stdin = request.stdin;
    activated
}

fn activate_launch(mut launch: ShellLaunch) -> ShellLaunch {
    let script = format!(
        "{ENVIRONMENT}\nif [ -n \"$1\" ]; then exec \"$@\"; else shift; exec \"${{SHELL:-/bin/sh}}\" \"$@\"; fi"
    );
    let mut args = vec!["-c".into(), script, "nocterm-workspace".into()];
    args.push(launch.program.take().unwrap_or_default());
    args.append(&mut launch.args);
    launch.program = Some("sh".into());
    launch.args = args;
    launch.integration = false;
    launch
}

#[derive(Clone, Copy, Default)]
pub struct NativeWorkspaceSessions;
impl WorkspaceSessionFactory for NativeWorkspaceSessions {
    fn wrap(&self, base: Arc<dyn Transport>) -> Arc<dyn Transport> {
        Arc::new(WorkspaceTransport {
            base: LaunchBase::Remote(base),
        })
    }
    fn wrap_local(&self, factory: LocalTransportFactory) -> Arc<dyn Transport> {
        Arc::new(WorkspaceTransport {
            base: LaunchBase::Local(factory),
        })
    }
}

enum LaunchBase {
    Remote(Arc<dyn Transport>),
    Local(LocalTransportFactory),
}
struct WorkspaceTransport {
    base: LaunchBase,
}
impl Transport for WorkspaceTransport {
    fn open(&self, mut request: ConnectRequest) -> Session {
        let session = match &self.base {
            LaunchBase::Remote(base) => {
                request.launch = activate_launch(request.launch);
                base.open(request)
            }
            LaunchBase::Local(factory) => {
                activate_local_environment(&mut request.launch);
                factory(request.launch.clone()).open(request)
            }
        };
        match session.exec() {
            Some(base) => session.with_exec(Arc::new(WorkspaceExec { base })),
            None => session,
        }
    }
}

fn activate_local_environment(launch: &mut ShellLaunch) {
    let home = launch
        .env
        .get("HOME")
        .cloned()
        .or_else(|| std::env::var("HOME").ok());
    let Some(home) = home else {
        return;
    };
    let root = std::path::Path::new(&home).join(".local/share/nocterm/toolchains");
    if !std::fs::symlink_metadata(&root).is_ok_and(|metadata| metadata.is_dir()) {
        return;
    }
    let inherited = launch
        .env
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    let mut paths: Vec<_> = [
        "bin",
        "rust/cargo/bin",
        "bun/current/bin",
        "node/current/bin",
        "java/current/bin",
    ]
    .into_iter()
    .map(|path| root.join(path))
    .collect();
    paths.extend(std::env::split_paths(&inherited));
    if let Ok(path) = std::env::join_paths(paths) {
        launch
            .env
            .insert("PATH".into(), path.to_string_lossy().into_owned());
    }
    if root.join("rust/cargo/bin/rustup").is_file() {
        for (name, relative) in [("CARGO_HOME", "rust/cargo"), ("RUSTUP_HOME", "rust/rustup")] {
            launch.env.insert(
                name.into(),
                root.join(relative).to_string_lossy().into_owned(),
            );
        }
    }
    if root.join("bun/current/bin/bun").is_file() {
        launch.env.insert(
            "BUN_INSTALL".into(),
            root.join("bun/current").to_string_lossy().into_owned(),
        );
    }
    if root.join("java/current/bin/java").is_file() {
        launch.env.insert(
            "JAVA_HOME".into(),
            root.join("java/current").to_string_lossy().into_owned(),
        );
    }
}

struct WorkspaceExec {
    base: Arc<dyn HostExec>,
}
impl HostExec for WorkspaceExec {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        self.base.exec(activate_request(request))
    }
    fn terminal(&self, mut request: TerminalRequest) -> Session {
        request.program = activate_request(request.program);
        let session = self.base.terminal(request);
        match session.exec() {
            Some(base) => session.with_exec(Arc::new(Self { base })),
            None => session,
        }
    }
}

#[cfg(test)]
mod tests;

impl nocterm_project_providers::WorkspaceCommandActivation for NativeWorkspaceSessions {
    fn activate_request(&self, request: ExecRequest) -> ExecRequest {
        activate_request(request)
    }
}
