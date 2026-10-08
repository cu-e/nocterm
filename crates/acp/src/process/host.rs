//! The service leader ties the whole cgroup to the original application process.
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    process::{Pid, PidfdFlags, pidfd_open},
};
use std::{
    ffi::OsString,
    fs, io,
    path::PathBuf,
    process::{Command, Stdio},
};

pub(super) fn start_time(pid: u32) -> io::Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| io::Error::other("Invalid owner process identity"))
}

/// Internal CLI; stdin/stdout/stderr belong exclusively to the ACP agent.
pub fn run_agent_host(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    run(args).map_err(|error| format!("Agent process container: {error}"))
}

fn run(args: impl IntoIterator<Item = OsString>) -> io::Result<()> {
    let mut args = args.into_iter();
    let invalid = || io::Error::other("Invalid agent-host arguments");
    let owner: u32 = args
        .next()
        .and_then(|v| v.to_str()?.parse().ok())
        .ok_or_else(invalid)?;
    let expected: u64 = args
        .next()
        .and_then(|v| v.to_str()?.parse().ok())
        .ok_or_else(invalid)?;
    let lease = PathBuf::from(args.next().ok_or_else(invalid)?);
    let keys = args
        .next()
        .and_then(|v| v.into_string().ok())
        .ok_or_else(invalid)?;
    let executable = args.next().ok_or_else(invalid)?;
    if start_time(owner)? != expected || !lease.is_dir() {
        return Err(io::Error::other("Agent owner is no longer available"));
    }
    let pid = i32::try_from(owner)
        .ok()
        .and_then(Pid::from_raw)
        .ok_or_else(invalid)?;
    let owner_fd = pidfd_open(pid, PidfdFlags::empty())?;
    if start_time(owner)? != expected {
        return Err(io::Error::other("Agent owner identity changed"));
    }
    let mut owner_poll = [PollFd::new(&owner_fd, PollFlags::IN)];
    if poll(
        &mut owner_poll,
        Some(&Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }),
    )? != 0
    {
        return Err(io::Error::other("Agent owner has exited"));
    }
    // The user manager has its own environment. Only the names explicitly
    // forwarded by the application are permitted to reach the agent.
    let env = keys
        .split(',')
        .filter_map(|key| std::env::var_os(key).map(|value| (key, value)));
    let mut child = Command::new(executable)
        .args(args)
        .env_clear()
        .envs(env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    let result = loop {
        if let Some(status) = child.try_wait()? {
            break if status.success() {
                Ok(())
            } else {
                Err(io::Error::other(format!("Agent exited: {status}")))
            };
        }
        if !lease.is_dir() {
            break Ok(());
        }
        match poll(
            &mut owner_poll,
            Some(&Timespec {
                tv_sec: 0,
                tv_nsec: 100_000_000,
            }),
        ) {
            Ok(0) => {}
            Ok(_) => break Ok(()),
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => break Err(error.into()),
        }
    };
    // Exiting the service leader invokes systemd's control-group cleanup.
    // Do not wait here for an uncooperative child or its detached descendants.
    let _ = fs::remove_dir(&lease);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_wrong_owner_identity_before_spawning() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("spawned");
        let args = [
            std::process::id().to_string(),
            (start_time(std::process::id()).unwrap() + 1).to_string(),
            dir.path().to_string_lossy().into_owned(),
            String::new(),
            "/usr/bin/touch".into(),
            marker.to_string_lossy().into_owned(),
        ]
        .map(OsString::from);
        assert!(run_agent_host(args).is_err());
        assert!(!marker.exists());
    }
}
