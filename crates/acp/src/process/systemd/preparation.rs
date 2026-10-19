//! Borrowed launch facts enter the original systemd containment engine.
use super::{Container, SystemctlError, control, host};
use nocterm_ai::{AgentResourceSettings, InvalidResourceLimits};
use std::{ffi::OsString, fs, io, os::unix::fs::DirBuilderExt as _, path::Path};
#[derive(Debug, thiserror::Error)]
pub(crate) enum PreparationError {
    #[error(transparent)]
    Limits(#[from] InvalidResourceLimits),
    #[error(
        "Agent process containment requires a running systemd user manager and systemd-run. Start the user manager before sending a message."
    )]
    Manager(#[source] SystemctlError),
    #[error(transparent)]
    Entropy(#[from] getrandom::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}
pub(super) struct Preparation<'a> {
    limits: &'a AgentResourceSettings,
    workdir: &'a Path,
}
impl<'a> Preparation<'a> {
    pub(super) fn new(limits: &'a AgentResourceSettings, workdir: &'a Path) -> Self {
        Self { limits, workdir }
    }
    pub(super) fn run<'b>(
        self,
        helper: &Path,
        executable: &Path,
        args: impl Iterator<Item = &'b str>,
        keys: impl Iterator<Item = &'b str> + Clone,
    ) -> Result<Container, PreparationError> {
        self.limits.validate()?;
        control(&["show", "--property=Version", "--value"]).map_err(PreparationError::Manager)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)?;
        let token: String = random.iter().map(|value| format!("{value:02x}")).collect();
        let unit = format!("nocterm-agent-{token}.service");
        let lease = std::env::temp_dir().join(format!("nocterm-agent-{token}"));
        fs::DirBuilder::new().mode(0o700).create(&lease)?;
        let mut container = Container {
            unit,
            lease,
            args: Vec::new(),
        };
        let owner = std::process::id();
        let identity = host::start_time(owner)?;
        self.options(&mut container);
        container.args.extend(
            keys.clone()
                .map(|key| OsString::from(format!("--setenv={key}"))),
        );
        container.args.extend([
            OsString::from("--"),
            helper.as_os_str().to_owned(),
            OsString::from("agent-host"),
            owner.to_string().into(),
            identity.to_string().into(),
            container.lease.as_os_str().to_owned(),
            keys.collect::<Vec<_>>().join(",").into(),
            executable.as_os_str().to_owned(),
        ]);
        container.args.extend(args.map(OsString::from));
        Ok(container)
    }
    fn options(&self, container: &mut Container) {
        let options = vec![
            "--user".to_owned(),
            "--quiet".into(),
            "--pipe".into(),
            "--wait".into(),
            "--collect".into(),
            "--service-type=exec".into(),
            "--expand-environment=no".into(),
            format!("--unit={}", container.unit),
            "--property=KillMode=control-group".into(),
            "--property=TimeoutStopSec=3s".into(),
            format!("--property=MemoryHigh={}M", self.limits.memory_high_mb),
            format!("--property=MemoryMax={}M", self.limits.memory_max_mb),
            format!(
                "--property=MemorySwapMax={}M",
                self.limits.memory_swap_max_mb
            ),
            format!("--property=TasksMax={}", self.limits.tasks_max),
            format!("--working-directory={}", self.workdir.display()),
        ];
        container
            .args
            .extend(options.into_iter().map(OsString::from));
    }
}
