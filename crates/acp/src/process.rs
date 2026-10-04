use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use nocterm_ai::{AgentError, ConnectRequest};

pub(crate) struct ProcessGroup {
    pid: u32,
    active: AtomicBool,
}
impl ProcessGroup {
    pub(crate) fn kill(&self) {
        if !self.active.swap(false, Ordering::AcqRel) {
            return;
        }
        #[cfg(unix)]
        {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(self.pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &self.pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    pub(crate) fn finish(&self) {
        self.kill();
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.kill();
    }
}

pub(crate) struct Spawned {
    pub child: async_process::Child,
    pub group: Arc<ProcessGroup>,
    pub secrets: Arc<Vec<String>>,
}

pub(crate) fn spawn(request: &ConnectRequest) -> Result<Spawned, AgentError> {
    let env = nocterm_ai::env::sanitized_environment(
        &request.launch,
        std::env::vars_os()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?))),
    );
    let mut secret_values: Vec<String> = env
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
    secret_values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    secret_values.dedup();
    let secrets = Arc::new(secret_values);
    let executable = resolve(&request.launch.command, &env, &request.working_directory)
        .ok_or_else(|| {
            AgentError::Io(nocterm_ai::redact::redact(&format!(
                "Cannot find agent command '{}'. Configure an absolute executable path; inherited PATH is {}",
                request.launch.command,
                env.get("PATH").map_or("<unset>", String::as_str)
            )))
        })?;
    let (executable, process_args) = match &request.sandbox {
        None => (executable, request.launch.args.clone()),
        Some(policy) => {
            // Fail closed: an agent the user asked to isolate never runs
            // without isolation.
            let bwrap = match nocterm_ai::sandbox::availability(&env) {
                nocterm_ai::sandbox::Availability::Available(bwrap) => bwrap,
                nocterm_ai::sandbox::Availability::Missing => {
                    return Err(AgentError::Io(
                        "Agent isolation is on, but bubblewrap (bwrap) was not found in PATH. Install bubblewrap or turn isolation off in AI settings."
                            .into(),
                    ));
                }
                nocterm_ai::sandbox::Availability::Unsupported => {
                    return Err(AgentError::Io(
                        "Agent isolation is only available on Linux. Turn it off in AI settings."
                            .into(),
                    ));
                }
            };
            let args = policy.bubblewrap_args(
                &request.working_directory,
                &executable,
                &request.launch.args,
                nocterm_ai::sandbox::file_kind,
            );
            (bwrap, args)
        }
    };
    let mut command = std::process::Command::new(executable);
    command
        .args(&process_args)
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
    let child = async_process::Command::from(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| AgentError::Io(format!("Cannot start agent: {e}")))?;
    let group = Arc::new(ProcessGroup {
        pid: child.id(),
        active: AtomicBool::new(true),
    });
    Ok(Spawned {
        child,
        group,
        secrets,
    })
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
        return executable(&path)
            .then(|| path.canonicalize().ok())
            .flatten();
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
        let candidate = directory.join(command);
        if executable(&candidate) {
            return candidate.canonicalize().ok();
        }
        #[cfg(windows)]
        for suffix in env
            .get("PATHEXT")
            .map_or(".EXE;.CMD;.BAT", String::as_str)
            .split(';')
        {
            let candidate = directory.join(format!("{command}{suffix}"));
            if executable(&candidate) {
                return candidate.canonicalize().ok();
            }
        }
    }
    None
}
fn executable(path: &Path) -> bool {
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
            launch: nocterm_ai::AgentLaunch {
                id: "test".into(),
                name: "test".into(),
                command: command.into(),
                args,
                env: BTreeMap::new(),
                inherit_env: Vec::new(),
            },
            working_directory: directory.to_owned(),
        }
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
        let mut spawned = spawn(&r).unwrap();
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
        let mut spawned = spawn(&request(dir.path(), "./agent", Vec::new())).unwrap();
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
        let mut spawned = spawn(&request(
            dir.path(),
            "/bin/sh",
            vec!["-c".into(), "sleep 60 & echo $!; wait".into()],
        ))
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
            nocterm_ai::sandbox::availability(&env),
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
        let mut spawned = spawn(&r).expect("Spawn with bwrap");
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
}
