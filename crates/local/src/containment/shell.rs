//! One private bundled asset per prepared principal; never a host startup directory.
use super::policy::ContainmentError;
use std::io::Write as _;

pub(super) const BASHRC: &str = "/run/nocterm/bashrc";

pub(super) struct ShellAsset {
    path: String,
    _directory: tempfile::TempDir,
}
impl ShellAsset {
    pub(super) fn new() -> Result<Self, ContainmentError> {
        let (name, script) = crate::integration::bundled("bash").ok_or(ContainmentError::Shell)?;
        if script.len() > 64 * 1024 {
            return Err(ContainmentError::Limits);
        }
        let directory = tempfile::tempdir().map_err(ContainmentError::ShellIo)?;
        let path = directory.path().join(name);
        let mut file =
            nocterm_core::fs::create_private(&path).map_err(ContainmentError::ShellFile)?;
        file.write_all(script.as_bytes())
            .map_err(ContainmentError::ShellIo)?;
        let path = path.to_str().ok_or(ContainmentError::Workspace)?.to_owned();
        Ok(Self {
            path,
            _directory: directory,
        })
    }
    pub(super) fn mount(&self, arguments: &mut Vec<String>) {
        arguments.extend([
            "--dir".into(),
            "/run/nocterm".into(),
            "--ro-bind".into(),
            self.path.clone(),
            BASHRC.into(),
        ]);
    }
}

pub(super) fn validate_terminal(
    term: &str,
    size: nocterm_session::PtySize,
) -> Result<(), ContainmentError> {
    if term.is_empty()
        || term.len() > 128
        || !term
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
        || !(2..=240).contains(&size.cols)
        || !(1..=120).contains(&size.rows)
    {
        return Err(ContainmentError::Limits);
    }
    Ok(())
}
