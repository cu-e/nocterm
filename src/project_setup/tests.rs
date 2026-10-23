use futures::executor::block_on;
use nocterm_containers::Engine;
use nocterm_project_model::{HostLocator, ProjectId, RootPath};
use nocterm_project_providers::*;
use nocterm_session::{
    local_fs::CancelFlag,
    testing::{ExecResponse, FakeExec},
};
use std::sync::Arc;

fn container_sessions() -> nocterm_project_container_fs::NativeContainerSessions {
    nocterm_project_container_fs::NativeContainerSessions::new(Arc::new(
        nocterm_project_environment::NativeWorkspaceSessions,
    ))
}
fn id() -> String {
    "a".repeat(64)
}
fn answer(exec: &FakeExec, prefix: &str, stdout: String) {
    if let Some(program) = prefix
        .split_whitespace()
        .next()
        .filter(|name| matches!(*name, "docker" | "podman"))
    {
        exec.respond(
            format!("{program} exec --interactive {} sh -c", id()),
            Ok(ExecResponse::text(probe_fixture(), 0, "")),
        );
        if !prefix.contains(' ') || prefix.ends_with(" pull") {
            exec.respond(
                format!(r#"sh -c 'exec "$@" 2>&1' nocterm-pull {program} pull"#),
                Ok(ExecResponse::text(stdout.clone(), 0, "")),
            );
        }
    }
    exec.respond(prefix, Ok(ExecResponse::text(stdout, 0, "")));
}

mod session;

fn runtime() -> nocterm_project_runtime::NativeProjectPreparation {
    nocterm_project_runtime::NativeProjectPreparation::new(Arc::new(
        nocterm_project_environment::NativeWorkspaceEnvironment::new(Arc::new(
            nocterm_project_runtime::NativeEnvironmentCommands,
        )),
    ))
}

fn request(mode: SetupMode) -> ProjectSetupRequest {
    ProjectSetupRequest {
        project: ProjectId::from_bytes([1; 16]),
        host: HostLocator::Local,
        mode,
        repository: None,
    }
}

fn inspect(status: &str, owned: bool) -> String {
    serde_json::json!([{"Id":id(), "State":{"Status":status},"Config":{"WorkingDir":"/app", "Labels": if owned { serde_json::json!({"io.nocterm.project":ProjectId::from_bytes([1;16]).to_string()}) } else { serde_json::json!({}) }}}]).to_string()
}

fn prepare(
    exec: FakeExec,
    request: ProjectSetupRequest,
) -> Result<PreparedProject, ProjectSetupError> {
    block_on(runtime().prepare(
        Arc::new(exec),
        request,
        CancelFlag::default(),
        Arc::new(|_| {}),
    ))
}

fn probe_fixture() -> String {
    "kernel\x00Linux\x00architecture\x00x86_64\x00os_release\x00ID=ubuntu\nPRETTY_NAME=Ubuntu\x00libc\x00glibc 2.39\x00uid\x000\x00user\x00root\x00home\x00/root\x00writable\x00yes\x00program\x00apt-get\x00program\x00curl\x00program\x00bash\x00program\x00tar\x00program\x00gzip\x00program\x00xz\x00program\x00unzip\x00program\x00sha256sum\x00program\x00awk\x00program\x00mktemp\x00program\x00cc\x00python\x00Python 3.13.2\x00rust\x00rustc 1.90.0\x00bun\x001.3.3\x00node\x00v22.1.0\x00java\x00openjdk version 21.0.1\x00".into()
}

mod environment;
mod preparation;
