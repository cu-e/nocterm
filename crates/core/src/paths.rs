//! Where nocterm keeps its files.
//!
//! Every crate that touches the disk asks [`Paths`] for the location instead
//! of assembling one, so the layout is defined in exactly one place.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use directories::{BaseDirs, ProjectDirs};

/// Environment variable that relocates every nocterm file under one directory.
///
/// It exists for portable installs and for hermetic tests.
pub const HOME_ENV: &str = "NOCTERM_HOME";

const APPLICATION: &str = "nocterm";

/// Longest runtime directory that still leaves room for a Unix socket name:
/// `sun_path` holds 104 bytes on macOS and the BSDs.
const RUNTIME_DIR_BUDGET: usize = 80;

/// The platform has no notion of a home directory for the current user.
#[derive(Debug, thiserror::Error)]
#[error("cannot determine the user's home directory")]
pub struct PathsError;

/// The directories and files nocterm reads and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config_dir: PathBuf,
    state_dir: PathBuf,
    runtime_dir: PathBuf,
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
            runtime_dir: project
                .runtime_dir()
                .map_or_else(fallback_runtime_dir, Path::to_path_buf),
            user_home: BaseDirs::new().map(|base| base.home_dir().to_path_buf()),
        })
    }

    /// Puts every file under `root`, ignoring the platform conventions.
    pub fn rooted_at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_dir: root.join("config"),
            state_dir: root.join("state"),
            runtime_dir: root.join("run"),
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

    /// Directory of files that only matter while nocterm runs, such as sockets.
    ///
    /// The path is only a location: use [`Paths::ensure_runtime_dir`] before
    /// putting anything in it.
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// Creates the runtime directory, private to this user, and returns it.
    ///
    /// A location too long for a Unix socket name falls back to a short
    /// private directory under the system temporary directory.
    pub fn ensure_runtime_dir(&self) -> io::Result<PathBuf> {
        let directory = self.effective_runtime_dir();
        ensure_private_dir(&directory)?;
        Ok(directory)
    }

    /// The directory [`Paths::ensure_runtime_dir`] creates, without creating it.
    pub fn effective_runtime_dir(&self) -> PathBuf {
        short_enough(&self.runtime_dir, fallback_runtime_dir())
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

/// Creates `path` readable and writable by this user only.
///
/// Refuses a symbolic link or a directory that belongs to somebody else, so
/// another user cannot plant a directory where sockets and tokens will go. A
/// directory of ours with looser permissions is tightened.
pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    create_private(path)?;
    verify_private(path)
}

#[cfg(unix)]
fn create_private(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;

    match fs::DirBuilder::new().mode(0o700).create(path) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        result => result,
    }
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn verify_private(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a plain directory", path.display()),
        ));
    }
    if metadata.uid() != nix::unistd::Uid::effective().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} belongs to another user", path.display()),
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_private(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a plain directory", path.display()),
        ));
    }
    Ok(())
}

/// `preferred`, unless it is too long for a socket name inside it.
fn short_enough(preferred: &Path, fallback: PathBuf) -> PathBuf {
    if preferred.as_os_str().len() <= RUNTIME_DIR_BUDGET {
        preferred.to_path_buf()
    } else {
        fallback
    }
}

/// A per-user directory under the system temporary directory.
fn fallback_runtime_dir() -> PathBuf {
    env::temp_dir().join(format!("{APPLICATION}-{}", user_tag()))
}

#[cfg(unix)]
fn user_tag() -> String {
    nix::unistd::Uid::effective().as_raw().to_string()
}

#[cfg(not(unix))]
fn user_tag() -> String {
    env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
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

    #[test]
    fn rooted_layout_keeps_runtime_files_apart() {
        let paths = Paths::rooted_at("/tmp/nocterm-test");

        assert_eq!(paths.runtime_dir(), Path::new("/tmp/nocterm-test/run"));
    }

    #[test]
    fn overlong_runtime_directory_falls_back_to_a_short_one() {
        let long = Path::new("/").join("x".repeat(RUNTIME_DIR_BUDGET));
        let fallback = PathBuf::from("/tmp/nocterm-1");

        assert_eq!(short_enough(&long, fallback.clone()), fallback);
        assert_eq!(
            short_enough(Path::new("/run/user/1000/nocterm"), fallback),
            Path::new("/run/user/1000/nocterm")
        );
    }

    #[test]
    fn runtime_directory_is_created_and_returned() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(root.path());

        let directory = paths.ensure_runtime_dir().unwrap();

        assert_eq!(directory, root.path().join("run"));
        assert!(directory.is_dir());
        // Asking again is fine.
        assert_eq!(paths.ensure_runtime_dir().unwrap(), directory);
    }

    #[cfg(unix)]
    mod private {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        use super::*;

        fn mode(path: &Path) -> u32 {
            fs::metadata(path).unwrap().permissions().mode() & 0o777
        }

        #[test]
        fn new_directory_is_private() {
            let root = tempfile::tempdir().unwrap();
            let directory = root.path().join("a/b");

            ensure_private_dir(&directory).unwrap();

            assert_eq!(mode(&directory), 0o700);
        }

        #[test]
        fn loose_permissions_are_tightened() {
            let root = tempfile::tempdir().unwrap();
            let directory = root.path().join("run");
            fs::create_dir(&directory).unwrap();
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();

            ensure_private_dir(&directory).unwrap();

            assert_eq!(mode(&directory), 0o700);
        }

        #[test]
        fn symlink_is_refused() {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("target");
            fs::create_dir(&target).unwrap();
            let link = root.path().join("run");
            symlink(&target, &link).unwrap();

            let error = ensure_private_dir(&link).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }

        #[test]
        fn plain_file_is_refused() {
            let root = tempfile::tempdir().unwrap();
            let file = root.path().join("run");
            fs::write(&file, "").unwrap();

            assert!(ensure_private_dir(&file).is_err());
        }
    }
}
