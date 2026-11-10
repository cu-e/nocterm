//! Where a bare agent command such as `npx` is looked for when the inherited
//! search path does not contain it. Applications started from the Windows shell
//! or the macOS Finder often inherit a shorter search path than a terminal, so
//! the usual Node.js install locations are tried after it, in this order.
use std::{collections::BTreeMap, path::PathBuf};

/// Directories to search after `PATH`, built only from the sanitized environment.
pub(super) fn fallback(env: &BTreeMap<String, String>) -> Vec<PathBuf> {
    let var = |name: &str| {
        env.get(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let mut directories = Vec::new();
    if cfg!(windows) {
        directories.extend(var("NVM_SYMLINK"));
        directories.extend(var("VOLTA_HOME").map(|home| home.join("bin")));
        directories.extend(var("LOCALAPPDATA").map(|local| local.join("Volta").join("bin")));
        for program_files in ["PROGRAMFILES", "PROGRAMW6432"] {
            directories.extend(var(program_files).map(|root| root.join("nodejs")));
        }
        directories.extend(var("LOCALAPPDATA").map(|local| local.join("Programs").join("nodejs")));
        directories.extend(var("APPDATA").map(|roaming| roaming.join("npm")));
        directories.extend(var("USERPROFILE").map(|home| home.join("scoop").join("shims")));
        directories.extend(var("PROGRAMDATA").map(|data| data.join("chocolatey").join("bin")));
    } else if cfg!(target_os = "macos") {
        directories.push(PathBuf::from("/opt/homebrew/bin"));
        directories.push(PathBuf::from("/usr/local/bin"));
        directories.extend(var("HOME").map(|home| home.join(".volta").join("bin")));
    }
    // Only absolute locations: a relative one would search the working directory.
    directories.retain(|directory| directory.is_absolute());
    directories.dedup();
    directories
}

#[cfg(test)]
mod tests {
    use super::fallback;
    use std::collections::BTreeMap;

    #[test]
    fn relative_and_empty_locations_are_never_searched() {
        let env = BTreeMap::from([
            ("APPDATA".to_owned(), "relative".to_owned()),
            ("HOME".to_owned(), String::new()),
            ("PROGRAMFILES".to_owned(), String::new()),
        ]);
        assert!(
            fallback(&env)
                .iter()
                .all(|directory| directory.is_absolute())
        );
    }
}
