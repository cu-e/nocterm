//! `terminals.files`: the files of a plugin's terminal tab.
//!
//! A plugin that opens a terminal tab with `files` set serves that tab's files: the
//! application's file browser shows them while the tab is active, and an AI chat the tab is
//! attached to reads and changes them with its usual file tools. The host asks the plugin with
//! a `file-request` event and the plugin answers with this capability's effect, under the
//! request's number. Paths are absolute POSIX paths under `/`; nothing the plugin answers is
//! ever a file of this computer. File contents travel as base64, so binary files keep their
//! bytes, and are at most [`MAX_FILE_BYTES`] long.

use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

use super::{Capability, sealed, terminals::MAX_TERMINAL_KEY};
use crate::{LimitError, error::key_syntax};

/// Most bytes of one file read or written through a plugin.
pub const MAX_FILE_BYTES: usize = 160 * 1024;
/// Most entries of one directory listing.
pub const MAX_FILE_ENTRIES: usize = 1000;
/// Longest path the host asks about, in bytes.
pub const MAX_FILE_PATH: usize = 1024;
/// Longest entry name, in bytes.
pub const MAX_FILE_NAME: usize = 255;
/// Longest failure reason, in characters.
pub const MAX_FILE_REASON: usize = 200;
/// Most file requests of one tab waiting for the plugin's answer.
pub const MAX_FILE_REQUESTS: usize = 8;
/// How long the host waits for an answer before the request fails.
pub const FILE_TIMEOUT_MS: u32 = 30_000;

/// What the host asks of a plugin's file system.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum FileOp {
    /// The entries of a directory, answered with [`FileAnswer::Listed`].
    List { path: String },
    /// One entry, answered with [`FileAnswer::Found`].
    Stat { path: String },
    /// A file's contents, answered with [`FileAnswer::Content`].
    Read { path: String },
    /// Writes a file. With `create` it must not exist yet. `data` is base64.
    Write {
        path: String,
        data: String,
        create: bool,
    },
    /// Creates a directory, answered with [`FileAnswer::Done`].
    CreateDir { path: String },
    /// Removes a file, or an empty directory when `directory` is set.
    Remove { path: String, directory: bool },
    /// Renames an entry; the destination must not exist.
    Rename { from: String, to: String },
}

impl FileOp {
    /// The paths the request names.
    pub fn paths(&self) -> [Option<&str>; 2] {
        match self {
            Self::List { path }
            | Self::Stat { path }
            | Self::Read { path }
            | Self::Write { path, .. }
            | Self::CreateDir { path }
            | Self::Remove { path, .. } => [Some(path), None],
            Self::Rename { from, to } => [Some(from), Some(to)],
        }
    }

    /// Paths are absolute, without `.` or `..` steps, NUL or doubled slashes, and bounded;
    /// written data is base64 of at most [`MAX_FILE_BYTES`].
    pub fn check(&self) -> Result<(), LimitError> {
        for path in self.paths().into_iter().flatten() {
            check_path(path)?;
        }
        if let Self::Write { data, .. } = self
            && crate::base64::decoded_len(data).is_none_or(|len| len > MAX_FILE_BYTES)
        {
            return Err(LimitError::TooLong {
                field: "data",
                max: MAX_FILE_BYTES,
            });
        }
        Ok(())
    }
}

/// An absolute POSIX path the host may ask about.
pub fn check_path(path: &str) -> Result<(), LimitError> {
    let unusual = || LimitError::Syntax { field: "path" };
    if path.len() > MAX_FILE_PATH {
        return Err(LimitError::TooLong {
            field: "path",
            max: MAX_FILE_PATH,
        });
    }
    if !path.starts_with('/') || path.contains('\0') {
        return Err(unusual());
    }
    if path == "/" {
        return Ok(());
    }
    let steps = path.trim_end_matches('/').split('/').skip(1);
    for step in steps {
        if step.is_empty() || step == "." || step == ".." {
            return Err(unusual());
        }
    }
    Ok(())
}

/// One entry of a plugin's file system.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct FileEntry {
    pub name: String,
    pub directory: bool,
    /// Size in bytes, for a file.
    pub size: Option<u64>,
    /// Last change, in seconds since the Unix epoch.
    pub modified: Option<u64>,
}

impl FileEntry {
    /// A name of one path step: not empty, not `.` or `..`, without `/`, NUL, control,
    /// invisible or direction characters, at most [`MAX_FILE_NAME`] bytes.
    pub fn check(&self) -> Result<(), LimitError> {
        let name = &self.name;
        if name.len() > MAX_FILE_NAME {
            return Err(LimitError::TooLong {
                field: "name",
                max: MAX_FILE_NAME,
            });
        }
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.chars().any(|c| c == '/' || c.is_control())
        {
            return Err(LimitError::Syntax { field: "name" });
        }
        // Invisible and direction characters could disguise a name, as they could view text.
        if crate::sanitize_display(name, MAX_FILE_NAME) != *name {
            return Err(LimitError::Unsanitized { field: "name" });
        }
        Ok(())
    }
}

/// Why a file request failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum FileFailure {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    /// The plugin's server does not offer this operation.
    Unsupported,
    /// Anything else, in at most [`MAX_FILE_REASON`] characters.
    Other(String),
}

/// A plugin's answer to one file request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum FileAnswer {
    Listed(Vec<FileEntry>),
    Found(Option<FileEntry>),
    /// A file's contents as base64.
    Content(String),
    Done,
    Failed(FileFailure),
}

/// The effect of `terminals.files`: the answer to the file request `request` of `terminal`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct FileReply {
    pub terminal: String,
    pub request: u32,
    pub answer: FileAnswer,
}

impl FileReply {
    pub fn check(&self) -> Result<(), LimitError> {
        key_syntax(&self.terminal, MAX_TERMINAL_KEY, "terminal")?;
        match &self.answer {
            FileAnswer::Listed(entries) => {
                if entries.len() > MAX_FILE_ENTRIES {
                    return Err(LimitError::TooLong {
                        field: "entries",
                        max: MAX_FILE_ENTRIES,
                    });
                }
                entries.iter().try_for_each(FileEntry::check)
            }
            FileAnswer::Found(entry) => entry.as_ref().map_or(Ok(()), FileEntry::check),
            FileAnswer::Content(data) => match crate::base64::decoded_len(data) {
                Some(len) if len <= MAX_FILE_BYTES => Ok(()),
                Some(_) => Err(LimitError::TooLong {
                    field: "content",
                    max: MAX_FILE_BYTES,
                }),
                None => Err(LimitError::Syntax { field: "content" }),
            },
            FileAnswer::Failed(FileFailure::Other(reason))
                if reason.chars().count() > MAX_FILE_REASON =>
            {
                Err(LimitError::TooLong {
                    field: "reason",
                    max: MAX_FILE_REASON,
                })
            }
            FileAnswer::Done | FileAnswer::Failed(_) => Ok(()),
        }
    }
}

/// Marker of `terminals.files`.
pub struct ServeFiles;

impl sealed::Sealed for ServeFiles {}

impl Capability for ServeFiles {
    const ID: &'static str = "terminals.files";
    type Value = FileReply;
}
