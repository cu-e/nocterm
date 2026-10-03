//! Where nocterm keeps its files.
//!
//! Every crate that touches the disk asks [`Paths`] for the location instead
//! of assembling one, so the layout is defined in exactly one place.

use std::{
    env,
    path::{Path, PathBuf},
};

use directories::{BaseDirs, ProjectDirs};

/// Environment variable that relocates every nocterm file under one directory.
///
/// It exists for portable installs and for hermetic tests.
pub const HOME_ENV: &str = "NOCTERM_HOME";

const APPLICATION: &str = "nocterm";

/// The platform has no notion of a home directory for the current user.
#[derive(Debug, thiserror::Error)]
#[error("cannot determine the user's home directory")]
pub struct PathsError;

/// The directories and files nocterm reads and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config_dir: PathBuf,
    state_dir: PathBuf,
    user_home: Option<PathBuf>,
}

impl Paths {
    /// Resolves the platform's standard locations, honouring [`HOME_ENV`].
    pub fn discover() -> Result<Self, PathsError> {
        if let Some(root) = env::var_os(HOME_ENV).filter(|value| !value.is_empty()) {
            return Ok(Self::rooted_at(root));
        }

        let project = ProjectDirs::from("", "", APPLICATION).ok_or(PathsError)?;
        Ok(Self {
            config_dir: project.config_dir().to_path_buf(),
            // Only Linux has a dedicated state directory.
            state_dir: project
                .state_dir()
                .unwrap_or_else(|| project.data_local_dir())
                .to_path_buf(),
            user_home: BaseDirs::new().map(|base| base.home_dir().to_path_buf()),
        })
    }

    /// Puts every file under `root`, ignoring the platform conventions.
    pub fn rooted_at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_dir: root.join("config"),
            state_dir: root.join("state"),
            user_home: None,
        }
    }

    /// Directory of the files a user is expected to edit by hand.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Directory of the files nocterm maintains by itself.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// User settings.
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    /// User overrides of the design tokens.
    pub fn theme_file(&self) -> PathBuf {
        self.config_dir.join("theme.toml")
    }

    /// Saved connection profiles.
    pub fn connections_file(&self) -> PathBuf {
        self.config_dir.join("connections.toml")
    }

    /// Host keys the user accepted from inside nocterm.
    pub fn known_hosts_file(&self) -> PathBuf {
        self.config_dir.join("known_hosts")
    }

    /// Recently opened connections.
    pub fn recents_file(&self) -> PathBuf {
        self.state_dir.join("recent.toml")
    }

    /// The user's home directory, unless nocterm was relocated with [`HOME_ENV`].
    pub fn user_home(&self) -> Option<&Path> {
        self.user_home.as_deref()
    }

    /// The user's OpenSSH directory (`~/.ssh`), when a home directory exists.
    ///
    /// nocterm only ever reads from it.
    pub fn openssh_dir(&self) -> Option<PathBuf> {
        self.user_home.as_ref().map(|home| home.join(".ssh"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooted_layout_separates_config_from_state() {
        let paths = Paths::rooted_at("/tmp/nocterm-test");

        assert_eq!(
            paths.settings_file(),
            Path::new("/tmp/nocterm-test/config/settings.toml")
        );
        assert_eq!(
            paths.recents_file(),
            Path::new("/tmp/nocterm-test/state/recent.toml")
        );
        assert_eq!(paths.openssh_dir(), None);
    }
}
