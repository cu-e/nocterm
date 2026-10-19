//! Native launch clears the environment, resolves the program against the
//! workdir, contains its process group and never falls back from isolation.
use super::*;
use nocterm_ai::history::source::ManagedCommandError;
use std::{os::unix::fs::PermissionsExt as _, path::PathBuf, time::Duration};
/// Answers initialization and every later request with an empty result.
const AGENT: &str = r#"
while IFS= read -r line; do
    id=${line#*'"id":'}; id=${id%%,*}
    case "$line" in
        *'"method":"initialize"'*)
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[]}}\n' "$id";;
        *'"id":'*) printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id";;
    esac
done
"#;
fn root(directory: &tempfile::TempDir) -> PathBuf {
    std::fs::canonicalize(directory.path()).unwrap()
}
fn connect(
    root: &Path,
    launch: &AgentLaunch,
    isolated: bool,
) -> Result<Connected, ManagedCommandError> {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let connector = AcpConnector::try_unmanaged().unwrap();
    let (producer, request) = prepared_request(&authority, root, launch, isolated);
    let (_, commands, events) = block_on(connector.connect_managed(request).unwrap())?.into_parts();
    Ok(Connected {
        _producer: producer,
        commands,
        events,
    })
}
#[test]
fn clears_environment_and_blocks_explicitly_requested_secrets() {
    let directory = tempfile::tempdir().unwrap();
    let root = root(&directory);
    let script = format!(
        "printf '%s|%s|%s' \"$SSH_AUTH_SOCK\" \"$NOCTERM_SECRET\" \"$EXPECTED\" > observed\n{AGENT}"
    );
    let mut launch = shell(&script);
    launch.env = [
        ("SSH_AUTH_SOCK".into(), "secret_socket".into()),
        ("NOCTERM_SECRET".into(), "secret_value".into()),
        ("EXPECTED".into(), "present".into()),
    ]
    .into_iter()
    .collect();
    launch.inherit_env = vec!["SSH_AUTH_SOCK".into(), "NOCTERM_SECRET".into()];
    let agent = connect(&root, &launch, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("observed")).unwrap(),
        "||present"
    );
    agent.finish();
}
#[test]
fn resolves_relative_executable_against_agent_workdir() {
    let directory = tempfile::tempdir().unwrap();
    let root = root(&directory);
    let program = root.join("agent");
    std::fs::write(
        &program,
        format!("#!/bin/sh\nprintf correct > observed\n{AGENT}"),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let agent = connect(&root, &launch("./agent", []), false).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("observed")).unwrap(),
        "correct"
    );
    agent.finish();
}
fn running(pid: i32) -> bool {
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        && stat
            .rsplit_once(") ")
            .is_some_and(|(_, tail)| tail.starts_with('Z'))
    {
        return false;
    }
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}
#[test]
fn shutdown_kills_the_process_group_including_grandchildren() {
    let directory = tempfile::tempdir().unwrap();
    let root = root(&directory);
    let script = format!("sleep 60 & echo $! > grandchild\n{AGENT}");
    let agent = connect(&root, &shell(&script), false).unwrap();
    let pid: i32 = std::fs::read_to_string(root.join("grandchild"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(running(pid));
    agent.finish();
    for _ in 0..100 {
        if !running(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !running(pid),
        "grandchild kept executing after group shutdown"
    );
}
#[cfg(target_os = "linux")]
fn plain_marker_launch(root: &Path) -> (AgentLaunch, PathBuf) {
    let marker = root.join("plain-agent-ran");
    let mut launch = shell(&format!("printf leaked > '{}'", marker.display()));
    launch
        .env
        .insert("PATH".into(), root.to_string_lossy().into_owned());
    (launch, marker)
}
#[cfg(target_os = "linux")]
#[test]
fn isolated_launch_never_executes_the_plain_program_when_bubblewrap_disappears() {
    let directory = tempfile::tempdir().unwrap();
    let root = root(&directory);
    let (launch, marker) = plain_marker_launch(&root);
    let Err(error) = connect(&root, &launch, true) else {
        panic!("an isolated agent was spawned without bubblewrap")
    };
    assert!(
        error
            .to_string()
            .contains("bubblewrap (bwrap) was not found")
    );
    assert!(!marker.exists());
}
#[cfg(target_os = "linux")]
#[test]
fn invalid_native_sandbox_executable_reports_spawn_cause_without_plain_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let root = root(&directory);
    let bwrap = root.join("bwrap");
    std::fs::write(&bwrap, b"invalid native executable format").unwrap();
    std::fs::set_permissions(&bwrap, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (launch, marker) = plain_marker_launch(&root);
    let Err(error) = connect(&root, &launch, true) else {
        panic!("native spawn must refuse invalid bubblewrap")
    };
    let message = error.to_string();
    assert!(message.contains("Cannot start agent"), "{message}");
    assert!(message.contains("os error 8"), "{message}");
    assert!(!marker.exists());
}
#[cfg(target_os = "linux")]
#[test]
fn unexecutable_bubblewrap_never_falls_back_to_the_plain_program() {
    let directory = tempfile::tempdir().unwrap();
    let root = root(&directory);
    let bwrap = root.join("bwrap");
    std::fs::write(&bwrap, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&bwrap, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (launch, marker) = plain_marker_launch(&root);
    let Err(error) = connect(&root, &launch, true) else {
        panic!("an isolated agent was spawned with unexecutable bubblewrap")
    };
    assert!(
        error.to_string().contains("Cannot execute bubblewrap"),
        "{error}"
    );
    assert!(!marker.exists());
}
#[cfg(target_os = "linux")]
#[test]
fn workspace_sandbox_runs_inside_bwrap_if_available() {
    use nocterm_ai::sandbox::{Availability, SandboxProbe as _};
    if !matches!(
        crate::NativeSandboxProbe.availability(),
        Availability::Available(_)
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let root = root(&directory);
    let mut launch = shell(&format!("printf sandbox-ok > observed\n{AGENT}"));
    launch.inherit_env = vec!["PATH".into()];
    let agent = connect(&root, &launch, true).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("observed")).unwrap(),
        "sandbox-ok"
    );
    agent.finish();
}
