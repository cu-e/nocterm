//! Operating-system isolation of agent processes (Linux, bubblewrap).
//!
//! The policy follows the "workspace write" model of Codex and Claude Code's
//! sandbox: the whole file system is readable but not writable; the working
//! directory and the agent's own state and caches are writable; credential
//! stores are replaced by empty directories; the process tree gets its own
//! PID namespace and session, so it dies with nocterm and cannot inject input
//! into the controlling terminal. The network stays available: agents talk
//! to their model provider.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Credential stores no agent needs, relative to the home directory.
const HIDDEN_IN_HOME: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".config/gcloud",
    ".config/gh",
    ".config/doctl",
    ".password-store",
    ".local/share/keyrings",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".netrc",
    ".git-credentials",
    ".pgpass",
    ".vault-token",
    ".npmrc",
    ".pypirc",
];

/// Where agents and their package managers keep state, relative to the home
/// directory. Writable so that sign-in, history and `npx` downloads work.
const AGENT_STATE_IN_HOME: &[&str] = &[
    ".claude",
    ".claude.json",
    ".codex",
    ".hermes",
    ".gemini",
    ".npm",
    ".cache",
    ".local/state",
    ".config/claude",
];

/// What the sandbox shows of the file system.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SandboxPolicy {
    /// Writable: the working directory and agent state.
    pub writable: Vec<PathBuf>,
    /// Replaced by empty directories or files.
    pub hidden: Vec<PathBuf>,
    /// Readable even though they lie under a hidden or temporary path, such
    /// as the directory of the terminal tools' socket.
    pub readable: Vec<PathBuf>,
    /// Credential roots cannot be used as a writable workspace.
    pub credentials: Vec<PathBuf>,
}

impl SandboxPolicy {
    /// The policy for an agent working in `workdir`. `private` lists
    /// nocterm's own directories (settings, vault, chats), hidden like
    /// credential stores; `shared` lists what must stay reachable inside them.
    pub fn new(
        workdir: &Path,
        home: Option<&Path>,
        private: &[PathBuf],
        shared: &[PathBuf],
    ) -> Self {
        let mut policy = Self {
            writable: vec![workdir.to_owned()],
            hidden: private.to_vec(),
            readable: shared.to_vec(),
            credentials: Vec::new(),
        };
        if let Some(home) = home {
            policy.credentials = HIDDEN_IN_HOME.iter().map(|path| home.join(path)).collect();
            policy.hidden.extend(policy.credentials.iter().cloned());
            policy
                .writable
                .extend(AGENT_STATE_IN_HOME.iter().map(|path| home.join(path)));
        }
        policy
    }

    /// Refuses direct and symlinked credential workspaces before mounting them.
    pub fn validate_workdir(&self, workdir: &Path) -> Result<(), String> {
        let canonical = std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_owned());
        if self.credentials.iter().any(|root| {
            let resolved = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
            workdir.starts_with(root) || canonical.starts_with(&resolved)
        }) {
            return Err(
                "The agent workspace cannot be inside a protected credential store.".into(),
            );
        }
        Ok(())
    }

    /// Arguments for `bwrap` that run `program` with `args` under this policy.
    /// Paths that do not exist are left out: bubblewrap cannot mount them.
    pub fn bubblewrap_args(
        &self,
        workdir: &Path,
        program: &Path,
        args: &[String],
        exists: impl Fn(&Path) -> Option<bool>,
    ) -> Vec<String> {
        // Compare and mount the same filesystem locations. HOME and private
        // state may themselves be symlinks; lexical aliases would otherwise
        // classify credential descendants as ancestors and reopen them later.
        let resolve = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        let resolved = resolve(workdir);
        let workdir = resolved.as_path();
        let writable: Vec<_> = self.writable.iter().map(|path| resolve(path)).collect();
        let hidden_paths: Vec<_> = self.hidden.iter().map(|path| resolve(path)).collect();
        let readable: Vec<_> = self.readable.iter().map(|path| resolve(path)).collect();
        let text = |path: &Path| path.to_string_lossy().into_owned();
        let mut out: Vec<String> = [
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--tmpfs",
            "/tmp",
            "--unshare-pid",
            "--die-with-parent",
            "--new-session",
            "--share-net",
        ]
        .map(str::to_owned)
        .to_vec();
        // Mount broad writable roots first. Hidden ancestors then mask private
        // state, while the dedicated workspace can still be reopened inside it.
        // Hidden descendants go last so choosing home never reveals credentials.
        let hidden = |path: &Path| hidden_paths.iter().any(|hidden| path.starts_with(hidden));
        for path in writable.iter().filter(|path| path.as_path() != workdir) {
            if exists(path).is_some() && !hidden(path) {
                out.extend(["--bind".into(), text(path), text(path)]);
            }
        }
        let mask = |out: &mut Vec<String>, path: &Path| match exists(path) {
            Some(true) => out.extend(["--tmpfs".into(), text(path)]),
            Some(false) => out.extend(["--ro-bind".into(), "/dev/null".into(), text(path)]),
            None => {}
        };
        for path in hidden_paths
            .iter()
            .filter(|path| !path.starts_with(workdir))
        {
            mask(&mut out, path);
        }
        if exists(workdir).is_some() {
            out.extend(["--bind".into(), text(workdir), text(workdir)]);
        }
        for path in hidden_paths.iter().filter(|path| path.starts_with(workdir)) {
            mask(&mut out, path);
        }
        for path in &readable {
            if exists(path).is_some() {
                out.extend(["--ro-bind".into(), text(path), text(path)]);
            }
        }
        out.extend(["--chdir".into(), text(workdir), "--".into(), text(program)]);
        out.extend(args.iter().cloned());
        out
    }
}

/// Whether `path` is a directory (`Some(true)`), another file (`Some(false)`)
/// or absent (`None`).
pub fn file_kind(path: &Path) -> Option<bool> {
    std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.is_dir())
}

/// Whether agents can be isolated on this system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Available(PathBuf),
    /// Linux without `bwrap` in PATH.
    Missing,
    /// Not Linux.
    Unsupported,
}

/// Looks for bubblewrap in `env`'s PATH.
pub fn availability(env: &BTreeMap<String, String>) -> Availability {
    if !cfg!(target_os = "linux") {
        return Availability::Unsupported;
    }
    env.get("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(path).collect::<Vec<_>>())
        .map(|directory| directory.join("bwrap"))
        .find(|candidate| candidate.is_file())
        .map_or(Availability::Missing, Availability::Available)
}

/// [`availability`] for this process's environment.
pub fn current_availability() -> Availability {
    availability(&std::env::vars().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_credentials_and_opens_only_the_workspace_and_agent_state() {
        let home = Path::new("/home/me");
        let workdir = Path::new("/home/me/project");
        let policy = SandboxPolicy::new(
            workdir,
            Some(home),
            &[PathBuf::from("/home/me/.config/nocterm")],
            &[PathBuf::from("/tmp/nocterm-1000")],
        );
        let present = [
            "/home/me/.ssh",
            "/home/me/.config/nocterm",
            "/home/me/.claude",
            "/home/me/project",
            "/tmp/nocterm-1000",
        ];
        let args = policy.bubblewrap_args(
            workdir,
            Path::new("/usr/bin/agent"),
            &["acp".into()],
            |path| {
                if path == Path::new("/home/me/.netrc") {
                    Some(false)
                } else {
                    present.contains(&path.to_str().unwrap()).then_some(true)
                }
            },
        );
        let joined = args.join(" ");
        assert!(joined.starts_with("--ro-bind / / "), "{joined}");
        assert!(joined.contains("--tmpfs /home/me/.ssh"));
        assert!(joined.contains("--ro-bind /dev/null /home/me/.netrc"));
        assert!(joined.contains("--tmpfs /home/me/.config/nocterm"));
        assert!(joined.contains("--bind /home/me/project /home/me/project"));
        assert!(joined.contains("--bind /home/me/.claude /home/me/.claude"));
        assert!(joined.contains("--ro-bind /tmp/nocterm-1000 /tmp/nocterm-1000"));
        assert!(joined.contains("--unshare-pid"));
        assert!(!joined.contains(".aws"), "absent paths are not mounted");
        assert!(joined.ends_with("--chdir /home/me/project -- /usr/bin/agent acp"));
        let claude = joined.find("--bind /home/me/.claude").unwrap();
        let nocterm = joined.find("--tmpfs /home/me/.config/nocterm").unwrap();
        let work = joined.find("--bind /home/me/project").unwrap();
        assert!(
            claude < nocterm,
            "agent state is mounted before hidden paths"
        );
        assert!(
            nocterm < work,
            "the workspace is mounted after hidden paths"
        );
    }

    #[test]
    fn writable_paths_inside_hidden_ones_stay_hidden_except_the_workspace() {
        let policy = SandboxPolicy {
            writable: vec![
                PathBuf::from("/state/agent-workspace"),
                PathBuf::from("/state/cache"),
            ],
            hidden: vec![PathBuf::from("/state")],
            readable: Vec::new(),
            credentials: Vec::new(),
        };
        let args = policy
            .bubblewrap_args(
                Path::new("/state/agent-workspace"),
                Path::new("/bin/agent"),
                &[],
                |_| Some(true),
            )
            .join(" ");
        assert!(args.contains("--bind /state/agent-workspace /state/agent-workspace"));
        assert!(!args.contains("--bind /state/cache"));
    }

    #[test]
    fn home_workspace_keeps_credentials_masked() {
        let home = Path::new("/home/me");
        let policy = SandboxPolicy::new(home, Some(home), &[], &[]);
        let args = policy
            .bubblewrap_args(home, Path::new("/bin/agent"), &[], |_| Some(true))
            .join(" ");
        assert!(
            args.find("--bind /home/me /home/me").unwrap()
                < args.find("--tmpfs /home/me/.ssh").unwrap()
        );
        assert!(policy.validate_workdir(&home.join(".aws/project")).is_err());
        assert!(policy.validate_workdir(&home.join("project")).is_ok());
    }

    #[test]
    fn availability_searches_path() {
        let env = BTreeMap::from([("PATH".to_owned(), "/nonexistent".to_owned())]);
        let expected = if cfg!(target_os = "linux") {
            Availability::Missing
        } else {
            Availability::Unsupported
        };
        assert_eq!(availability(&env), expected);
    }
}
