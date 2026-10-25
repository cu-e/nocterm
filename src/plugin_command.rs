//! `nocterm plugin check <dir>` and `nocterm plugin dev <file>`: the author's tools for user
//! plugin packages. `check` reports on a directory and exits; `dev` starts the application in a
//! developer session.
use std::{ffi::OsString, io, path::PathBuf};

use thiserror::Error;

const USAGE: &str = "usage: nocterm plugin check <directory>\n       nocterm plugin dev <file>";

/// What the subcommand asks the process to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// Leave with this exit status.
    Exit(i32),
    /// Start the application in a developer session that loads this file.
    Dev(PathBuf),
}

/// Runs the subcommand with the arguments after `plugin`.
pub(crate) fn run(mut args: impl Iterator<Item = OsString>) -> Command {
    match (args.next(), args.next(), args.next()) {
        (Some(command), Some(directory), None) if command == "check" => {
            Command::Exit(check(directory.into()))
        }
        (Some(command), Some(file), None) if command == "dev" => dev(file.into()),
        _ => usage(),
    }
}

fn usage() -> Command {
    say(&mut io::stderr(), USAGE);
    Command::Exit(2)
}

/// Why a developer session cannot load the path it was given.
#[derive(Debug, Error)]
pub(crate) enum DevFileError {
    #[error("`{}` cannot be read", .0.display())]
    Unreadable(PathBuf, #[source] io::Error),
    /// The session reads the file without following links, so a link would fail at the load.
    #[error("`{}` is a link; name the file itself", .0.display())]
    Link(PathBuf),
    #[error("`{}` is not a file", .0.display())]
    NotAFile(PathBuf),
    /// The path is handed to the settings page as text and would name another file.
    #[error("`{}` is not valid UTF-8", .0.display())]
    NotUtf8(PathBuf),
}

/// The absolute path of the one regular file a developer session loads, checked the way the
/// session will read it: links are not followed and the path must be text.
fn dev_file(file: PathBuf) -> Result<PathBuf, DevFileError> {
    let metadata = std::fs::symlink_metadata(&file)
        .map_err(|error| DevFileError::Unreadable(file.clone(), error))?;
    if metadata.file_type().is_symlink() {
        return Err(DevFileError::Link(file));
    }
    if !metadata.is_file() {
        return Err(DevFileError::NotAFile(file));
    }
    let absolute = std::path::absolute(&file)
        .map_err(|error| DevFileError::Unreadable(file.clone(), error))?;
    if absolute.to_str().is_none() {
        return Err(DevFileError::NotUtf8(absolute));
    }
    Ok(absolute)
}

/// A developer session loads one regular file; anything else is a usage error before a window
/// opens.
fn dev(file: PathBuf) -> Command {
    match dev_file(file) {
        Ok(file) => Command::Dev(file),
        Err(error) => {
            say(&mut io::stderr(), &error.to_string());
            usage()
        }
    }
}

fn check(directory: PathBuf) -> i32 {
    let catalog = match crate::shell_setup::plugin_catalog() {
        Ok(catalog) => catalog,
        Err(error) => {
            let text = format!("the application's plugin catalog is invalid: {error}");
            say(&mut io::stderr(), &text);
            return 1;
        }
    };
    let report = crate::shell_setup::check_directory(&directory, &catalog);
    say(&mut io::stdout(), &report.to_string());
    if report.passed() { 0 } else { 1 }
}

/// Writes a line to the terminal. A closed pipe has nobody to tell, so a failure is ignored.
fn say(out: &mut impl io::Write, text: &str) {
    let _ = writeln!(out, "{}", text.trim_end());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> impl Iterator<Item = OsString> {
        words
            .iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn dev_takes_one_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("clock.wasm");
        std::fs::write(&file, b"\0asm").unwrap();
        assert_eq!(
            run(args(&["dev", file.to_str().unwrap()])),
            Command::Dev(file)
        );
    }

    #[cfg(unix)]
    #[test]
    fn dev_refuses_a_link_and_a_path_that_is_not_text() {
        use std::os::unix::ffi::OsStringExt as _;
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("clock.wasm");
        std::fs::write(&file, b"\0asm").unwrap();
        let link = directory.path().join("link.wasm");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(matches!(dev_file(link.clone()), Err(DevFileError::Link(path)) if path == link));
        assert_eq!(
            run(args(&["dev", link.to_str().unwrap()])),
            Command::Exit(2)
        );

        let odd = directory
            .path()
            .join(OsString::from_vec(b"odd\xff.wasm".to_vec()));
        std::fs::write(&odd, b"\0asm").unwrap();
        assert!(matches!(dev_file(odd), Err(DevFileError::NotUtf8(_))));
    }

    #[test]
    fn dev_refuses_a_directory_a_missing_file_and_the_wrong_arity() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("nope.wasm");
        for words in [
            vec!["dev", directory.path().to_str().unwrap()],
            vec!["dev", missing.to_str().unwrap()],
            vec!["dev"],
            vec!["dev", "a", "b"],
        ] {
            assert_eq!(run(args(&words)), Command::Exit(2), "{words:?}");
        }
    }

    #[test]
    fn check_reports_through_the_exit_status() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            run(args(&["check", directory.path().to_str().unwrap()])),
            Command::Exit(1),
            "an empty directory has nothing to pass"
        );
        assert_eq!(run(args(&["check"])), Command::Exit(2));
        assert_eq!(run(args(&["unknown", "x"])), Command::Exit(2));
        assert_eq!(run(args(&[])), Command::Exit(2));
    }
}
