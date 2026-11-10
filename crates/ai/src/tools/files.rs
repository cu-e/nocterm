//! File tools of attached terminals that serve files but run no programs: arguments, bounds,
//! and the work over the terminal's filesystem.
//!
//! An agent reaches such a terminal's files only through these tools: it cannot run a program
//! there to read or change them. Paths are absolute without `.` or `..` components; files are
//! read and written whole as UTF-8 text within the bounds below, and an edit replaces exact
//! text, so an agent sends only what changes.
use super::TerminalCallError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod work;

pub use work::run;

pub const MAX_FILE_PATH_BYTES: usize = 4096;
/// The most text one `read_file` answer carries.
pub const MAX_FILE_READ_BYTES: usize = 64 * 1024;
/// The largest file read or edited whole.
pub const MAX_FILE_LOAD_BYTES: usize = 1024 * 1024;
/// The largest content `write_file` takes.
pub const MAX_FILE_WRITE_BYTES: usize = 512 * 1024;
/// The largest `old_text` or `new_text` of an edit.
pub const MAX_EDIT_BYTES: usize = 64 * 1024;
pub const MAX_LISTED_ENTRIES: usize = 1000;
pub const MAX_READ_FILE_LINES: usize = 2000;

/// List a folder of an attached terminal that has `file_tools`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListFiles {
    pub terminal_id: String,
    /// Absolute path of the folder; `/` is the top.
    pub path: String,
}

/// Read a text file, a range of its lines at a time.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadFile {
    pub terminal_id: String,
    pub path: String,
    /// First line to read, from 1 (default 1).
    pub start_line: Option<usize>,
    /// Lines to read, 1–2000 (default 2000); an answer also stops at 64 KiB.
    pub max_lines: Option<usize>,
}

/// Write a whole text file.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteFile {
    pub terminal_id: String,
    pub path: String,
    /// The whole new content, UTF-8 text of at most 512 KiB.
    pub content: String,
    /// Replace a file that exists; by default an existing file is refused.
    #[serde(default)]
    pub overwrite: bool,
}

/// Replace exact text in a file.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditFile {
    pub terminal_id: String,
    pub path: String,
    /// Text that occurs in the file exactly once, unless `replace_all` is true.
    pub old_text: String,
    /// The text it becomes.
    pub new_text: String,
    /// Replace every occurrence.
    #[serde(default)]
    pub replace_all: bool,
}

/// Create a folder; its parent must exist.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MakeDirectory {
    pub terminal_id: String,
    pub path: String,
}

/// Delete a file, or a folder as the terminal's host removes one.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeletePath {
    pub terminal_id: String,
    pub path: String,
}

/// Move or rename a file or folder.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MovePath {
    pub terminal_id: String,
    pub from: String,
    pub to: String,
}

/// An absolute path with no empty, `.` or `..` component and no control character, so a path
/// names one place and reads plainly on an approval card.
pub fn path_ok(path: &str) -> bool {
    path.len() <= MAX_FILE_PATH_BYTES
        && !path.chars().any(char::is_control)
        && (path == "/"
            || path.strip_prefix('/').is_some_and(|rest| {
                rest.split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..")
            }))
}

fn paths(paths: &[&str]) -> Result<(), TerminalCallError> {
    if paths.iter().all(|path| path_ok(path)) {
        Ok(())
    } else {
        Err(TerminalCallError::FilePathRejected)
    }
}

/// A path below the top, which a change may name.
fn below_top(path: &str) -> Result<(), TerminalCallError> {
    paths(&[path])?;
    if path == "/" {
        Err(TerminalCallError::FilePathRejected)
    } else {
        Ok(())
    }
}

impl ListFiles {
    pub(super) fn validate(&self) -> Result<(), TerminalCallError> {
        paths(&[&self.path])
    }
}

impl ReadFile {
    pub(super) fn validate(&self) -> Result<(), TerminalCallError> {
        below_top(&self.path)?;
        if self.start_line == Some(0)
            || self
                .max_lines
                .is_some_and(|n| n == 0 || n > MAX_READ_FILE_LINES)
        {
            return Err(TerminalCallError::ReadRangeRejected);
        }
        Ok(())
    }
}

impl WriteFile {
    pub(super) fn validate(&self) -> Result<(), TerminalCallError> {
        below_top(&self.path)?;
        if self.content.len() > MAX_FILE_WRITE_BYTES || self.content.contains('\0') {
            return Err(TerminalCallError::FileContentRejected);
        }
        Ok(())
    }
}

impl EditFile {
    pub(super) fn validate(&self) -> Result<(), TerminalCallError> {
        below_top(&self.path)?;
        if self.old_text.is_empty()
            || self.old_text.len() > MAX_EDIT_BYTES
            || self.new_text.len() > MAX_EDIT_BYTES
            || self.old_text == self.new_text
            || self.new_text.contains('\0')
        {
            return Err(TerminalCallError::EditRejected);
        }
        Ok(())
    }
}

impl MakeDirectory {
    pub(super) fn validate(&self) -> Result<(), TerminalCallError> {
        below_top(&self.path)
    }
}

impl DeletePath {
    pub(super) fn validate(&self) -> Result<(), TerminalCallError> {
        below_top(&self.path)
    }
}

impl MovePath {
    pub(super) fn validate(&self) -> Result<(), TerminalCallError> {
        below_top(&self.from)?;
        below_top(&self.to)?;
        let inside = self
            .to
            .strip_prefix(&self.from)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
        if inside {
            return Err(TerminalCallError::FilePathRejected);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
