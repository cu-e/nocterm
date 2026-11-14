use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use nocterm_ai::{AgentError, ConnectRequest};

mod lifecycle;
mod locations;
pub(crate) use lifecycle::{CleanupError, ProcessGroup};
#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
mod systemd;
#[cfg(target_os = "linux")]
pub use host::{AgentHostError, run_agent_host};

pub(crate) struct Spawned {
    pub child: async_process::Child,
    pub group: Arc<ProcessGroup>,
    pub secrets: Arc<Vec<String>>,
}

pub(crate) fn spawn(
    request: &ConnectRequest,
    helper: Option<&Path>,
) -> Result<Spawned, AgentError> {
    let mut env = nocterm_ai::env::sanitized_environment(
        &request.launch,
        std::env::vars_os()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?))),
    );
    let secrets = secret_values(request, &env);
    let executable = match resolve(&request.launch.command, &env, &request.working_directory) {
        Some(executable) => executable,
        None => {
            let (executable, directory) = resolve_fallback(&request.launch.command, &env)
                .ok_or_else(|| {
                    AgentError::Io(nocterm_ai::redact::redact(&format!(
                        "Cannot find agent command '{}' in PATH or the usual Node.js locations. \
                         Install it or configure an absolute executable path; inherited PATH is {}",
                        request.launch.command,
                        env.get("PATH").map_or("<unset>", String::as_str)
                    )))
                })?;
            // Scripts such as npx.cmd start `node` from the same search path.
            prepend_path(&mut env, directory);
            executable
        }
    };
    let (executable, process_args) = match &request.sandbox {
        None => (executable, request.launch.args.clone()),
        Some(policy) => crate::sandbox::SandboxLauncher::prepare(
            policy,
            &request.working_directory,
            &executable,
            &request.launch.args,
            &env,
        )?
        .into_parts(),
    };
    #[cfg(target_os = "linux")]
    let managed = helper
        .map(|helper| systemd::prepare(request, helper, &executable, &process_args, &env))
        .transpose()?;
    #[cfg(not(target_os = "linux"))]
    let _ = helper;
    #[cfg(target_os = "linux")]
    let mut command = managed.as_ref().map_or_else(
        || std::process::Command::new(&executable),
        |managed| managed.command(),
    );
    #[cfg(not(target_os = "linux"))]
    let mut command = std::process::Command::new(&executable);
    #[cfg(target_os = "linux")]
    if managed.is_none() {
        command.args(&process_args);
    }
    #[cfg(not(target_os = "linux"))]
    command.args(&process_args);
    let child = spawn_child(command, request, env)?;
    let group = Arc::new(ProcessGroup::new(
        child.id(),
        #[cfg(target_os = "linux")]
        managed,
    ));
    Ok(Spawned {
        child,
        group,
        secrets,
    })
}

fn spawn_child(
    mut command: std::process::Command,
    request: &ConnectRequest,
    env: BTreeMap<String, String>,
) -> Result<async_process::Child, AgentError> {
    command
        .env_clear()
        .envs(env)
        .current_dir(&request.working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // The application has no console of its own. Without this flag Windows opens
        // a console window for every console agent (npx.cmd, node) it starts. The
        // hidden console is inherited, so the agent's own tools open none either.
        command.creation_flags(CREATE_NO_WINDOW);
    }
    async_process::Command::from(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| match &request.sandbox {
            Some(_) => AgentError::Sandbox(nocterm_ai::sandbox::SandboxLaunchError::spawn(source)),
            None => AgentError::Io(format!("Cannot start agent: {source}")),
        })
}

fn secret_values(request: &ConnectRequest, env: &BTreeMap<String, String>) -> Arc<Vec<String>> {
    let mut values: Vec<_> = env
        .iter()
        .filter(|(name, _)| {
            ["KEY", "TOKEN", "PASSWORD", "SECRET", "PROXY"]
                .iter()
                .any(|term| name.to_uppercase().contains(term))
        })
        .map(|(_, value)| value.clone())
        .chain(request.launch.env.values().cloned())
        .filter(|value| value.len() >= 4)
        .collect();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    Arc::new(values)
}

/// `CREATE_NO_WINDOW` from the Windows process creation flags.
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Look for a bare command in the usual install locations once `PATH` failed.
fn resolve_fallback(command: &str, env: &BTreeMap<String, String>) -> Option<(PathBuf, PathBuf)> {
    if Path::new(command).components().count() != 1 || Path::new(command).is_absolute() {
        return None;
    }
    locations::fallback(env)
        .into_iter()
        .find_map(|directory| Some((find_in(&directory, command, env)?, directory)))
}
fn prepend_path(env: &mut BTreeMap<String, String>, directory: PathBuf) {
    let inherited = env.get("PATH").cloned().unwrap_or_default();
    let paths = std::iter::once(directory).chain(std::env::split_paths(&inherited));
    if let Ok(path) = std::env::join_paths(paths) {
        env.insert("PATH".into(), path.to_string_lossy().into_owned());
    }
}
fn find_in(directory: &Path, command: &str, env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let candidate = directory.join(command);
    if executable(&candidate) {
        return canonical(&candidate);
    }
    #[cfg(windows)]
    for suffix in env
        .get("PATHEXT")
        .map_or(".EXE;.CMD;.BAT", String::as_str)
        .split(';')
        .filter(|suffix| !suffix.is_empty())
    {
        let candidate = directory.join(format!("{command}{suffix}"));
        if executable(&candidate) {
            return canonical(&candidate);
        }
    }
    #[cfg(not(windows))]
    let _ = env;
    None
}

/// The resolved program path. On Windows it keeps the ordinary `C:\` form where
/// possible: `cmd.exe`, which runs `.cmd` scripts such as npx.cmd, does not accept
/// verbatim `\\?\` paths.
fn canonical(path: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        dunce::canonicalize(path).ok()
    }
    #[cfg(not(windows))]
    {
        path.canonicalize().ok()
    }
}

fn resolve(
    command: &str,
    env: &BTreeMap<String, String>,
    working_directory: &Path,
) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.is_absolute() || path.components().count() > 1 {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            working_directory.join(path)
        };
        return executable(&path).then(|| canonical(&path)).flatten();
    }
    let paths = env.get("PATH")?;
    for directory in std::env::split_paths(paths) {
        // An empty PATH segment must not search the application's cwd.
        if directory.as_os_str().is_empty() {
            continue;
        }
        let directory = if directory.is_absolute() {
            directory
        } else {
            working_directory.join(directory)
        };
        if let Some(found) = find_in(&directory, command, env) {
            return Some(found);
        }
    }
    None
}
pub(crate) fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use futures::AsyncReadExt as _;
    use std::{os::unix::fs::PermissionsExt as _, time::Duration};
    fn request(directory: &Path, command: &str, args: Vec<String>) -> ConnectRequest {
        ConnectRequest {
            terminal_auth: false,
            sandbox: None,
            resources: Default::default(),
            cancellation: Default::default(),
            launch: nocterm_ai::AgentLaunch {
                id: "test".into(),
                name: "test".into(),
                command: command.into(),
                args,
                env: BTreeMap::new(),
                inherit_env: Vec::new(),
                hub: None,
            },
            working_directory: directory.to_owned(),
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn isolated_launch_never_executes_the_plain_program_when_bubblewrap_disappears() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let marker = root.join("plain-agent-ran");
        let mut input = request(
            &root,
            "/bin/sh",
            vec![
                "-c".into(),
                format!("printf leaked > '{}'", marker.display()),
            ],
        );
        input.sandbox = Some(nocterm_ai::sandbox::SandboxPolicy::new(
            &root,
            None,
            &[],
            &[],
        ));
        input
            .launch
            .env
            .insert("PATH".into(), root.to_string_lossy().into_owned());
        let result = spawn(&input, None);
        match result {
            Err(AgentError::Sandbox(nocterm_ai::sandbox::SandboxLaunchError::Missing)) => {}
            Err(error) => panic!("unexpected refusal: {error}"),
            Ok(mut process) => {
                let _ = futures::executor::block_on(process.child.status());
                process.group.finish();
                panic!("an isolated agent was spawned without bubblewrap");
            }
        }
        assert!(!marker.exists());
    }
    #[test]
    fn clears_environment_and_blocks_explicitly_requested_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = request(
            dir.path(),
            "/bin/sh",
            vec![
                "-c".into(),
                "printf '%s|%s|%s' \"$SSH_AUTH_SOCK\" \"$NOCTERM_SECRET\" \"$EXPECTED\"".into(),
            ],
        );
        r.launch.env = BTreeMap::from([
            ("SSH_AUTH_SOCK".into(), "secret_socket".into()),
            ("NOCTERM_SECRET".into(), "secret_value".into()),
            ("EXPECTED".into(), "present".into()),
        ]);
        r.launch.inherit_env = vec!["SSH_AUTH_SOCK".into(), "NOCTERM_SECRET".into()];
        let mut spawned = spawn(&r, None).unwrap();
        let mut stdout = spawned.child.stdout.take().unwrap();
        let text = futures::executor::block_on(async {
            let mut text = String::new();
            stdout.read_to_string(&mut text).await.unwrap();
            spawned.child.status().await.unwrap();
            text
        });
        spawned.group.finish();
        assert_eq!(text, "||present");
    }
    #[test]
    fn resolves_relative_executable_against_agent_workdir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent");
        std::fs::write(&path, "#!/bin/sh\nprintf correct").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut spawned = spawn(&request(dir.path(), "./agent", Vec::new()), None).unwrap();
        let text = futures::executor::block_on(async {
            let mut text = String::new();
            spawned
                .child
                .stdout
                .take()
                .unwrap()
                .read_to_string(&mut text)
                .await
                .unwrap();
            spawned.child.status().await.unwrap();
            text
        });
        spawned.group.finish();
        assert_eq!(text, "correct");
    }
    #[test]
    fn kills_the_process_group_including_grandchildren() {
        let dir = tempfile::tempdir().unwrap();
        let mut spawned = spawn(
            &request(
                dir.path(),
                "/bin/sh",
                vec!["-c".into(), "sleep 60 & echo $!; wait".into()],
            ),
            None,
        )
        .unwrap();
        let mut reader = futures::io::BufReader::new(spawned.child.stdout.take().unwrap());
        let pid: i32 = futures::executor::block_on(crate::lines::read_line(&mut reader, 128))
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        spawned.group.finish();
        assert!(
            !futures::executor::block_on(spawned.child.status())
                .unwrap()
                .success()
        );
        for _ in 0..100 {
            if !running(pid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !running(pid),
            "Grandchild kept executing after group shutdown"
        );
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn workspace_sandbox_runs_inside_bwrap_if_available() {
        let env: BTreeMap<String, String> = std::env::vars().collect();
        if !matches!(
            crate::sandbox::availability(&env, std::env::current_dir().ok().as_deref()),
            nocterm_ai::sandbox::Availability::Available(_)
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut r = request(
            dir.path(),
            "/bin/sh",
            vec!["-c".into(), "echo sandbox-ok".into()],
        );
        r.sandbox = Some(nocterm_ai::sandbox::SandboxPolicy::new(
            dir.path(),
            None,
            &[],
            &[],
        ));
        let mut spawned = spawn(&r, None).expect("Spawn with bwrap");
        let text = futures::executor::block_on(async {
            let mut text = String::new();
            spawned
                .child
                .stdout
                .take()
                .unwrap()
                .read_to_string(&mut text)
                .await
                .unwrap();
            text
        });
        assert_eq!(text.trim(), "sandbox-ok");
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

    #[cfg(target_os = "linux")]
    #[test]
    fn invalid_native_sandbox_executable_preserves_spawn_cause_without_plain_fallback() {
        use std::error::Error;
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let bwrap = root.join("bwrap");
        std::fs::write(&bwrap, b"invalid native executable format").unwrap();
        std::fs::set_permissions(&bwrap, std::fs::Permissions::from_mode(0o700)).unwrap();
        let marker = root.join("plain-ran");
        let mut input = request(
            &root,
            "/bin/sh",
            vec![
                "-c".into(),
                format!("printf leaked > '{}'", marker.display()),
            ],
        );
        input.sandbox = Some(nocterm_ai::sandbox::SandboxPolicy::new(
            &root,
            None,
            &[],
            &[],
        ));
        input
            .launch
            .env
            .insert("PATH".into(), root.to_string_lossy().into_owned());
        let error = spawn(&input, None)
            .err()
            .expect("native spawn must refuse invalid bubblewrap");
        let AgentError::Sandbox(error) = error else {
            panic!("native refusal remains a sandbox launch error")
        };
        assert!(matches!(
            error,
            nocterm_ai::sandbox::SandboxLaunchError::Spawn(_)
        ));
        let native = error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(native.raw_os_error(), Some(8));
        let clone = error.clone();
        let shared = clone
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert!(std::ptr::eq(native, shared));
        assert_eq!(native.kind(), shared.kind());
        assert!(!marker.exists());
    }
}
