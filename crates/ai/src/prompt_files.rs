//! Files a person attaches to a prompt that are not images. A prompt carries a link to the
//! file where it is (`file://`): a local agent opens it there, and a hub chat uploads it.

use std::path::Path;
use std::sync::Arc;

use crate::model::content::{Content, ResourceReference};

/// The most files one prompt carries besides its images.
pub const MAX_PROMPT_FILES: usize = 16;

/// The extensions of files attached as images, which the agent sees inline instead.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Whether `path` is attached as an image rather than as a file.
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

/// Why a file cannot be attached.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PromptFileError {
    #[error("Only files given by their full path can be attached")]
    Relative,
    #[error("At most {MAX_PROMPT_FILES} files can be attached to a message")]
    TooMany,
}

/// A file of a prompt: its name and size for the chip, and the link the agent receives.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptFile {
    name: String,
    size: u64,
    content: Arc<Content>,
}

impl PromptFile {
    /// The file at the absolute `path`, of `size` bytes.
    pub fn link(path: &Path, size: u64) -> Result<Self, PromptFileError> {
        if !path.is_absolute() {
            return Err(PromptFileError::Relative);
        }
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into(),
        );
        let content = Content::ResourceReference(ResourceReference {
            annotations: None,
            description: None,
            mime_type: None,
            name: name.clone(),
            size: i64::try_from(size).ok(),
            title: None,
            uri: file_uri(path),
            meta: None,
        });
        Ok(Self {
            name,
            size,
            content: Arc::new(content),
        })
    }

    /// The file a prompt's `content` links to, when it is one.
    pub fn of(content: &Content) -> Option<Self> {
        let Content::ResourceReference(link) = content else {
            return None;
        };
        link.uri.starts_with("file://").then(|| Self {
            name: link.name.clone(),
            size: link
                .size
                .and_then(|size| u64::try_from(size).ok())
                .unwrap_or(0),
            content: Arc::new(content.clone()),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// The link the prompt carries.
    pub fn content(&self) -> Content {
        (*self.content).clone()
    }
}

/// The local path a `file://` link names, if it names one.
pub fn path_of(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let mut chars = rest.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let high = chars.next()?;
            let low = chars.next()?;
            let hex = [high, low];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    Some(std::path::PathBuf::from(String::from_utf8(bytes).ok()?))
}

/// `file://` and the path with every byte outside the unreserved set and `/` escaped.
fn file_uri(path: &Path) -> String {
    use std::fmt::Write as _;
    let text = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_linked_by_its_escaped_path_and_read_back_from_the_link() {
        let path = Path::new("/home/ops/notes 2026/план.txt");
        let file = PromptFile::link(path, 42).unwrap();
        assert_eq!(file.name(), "план.txt");
        let Content::ResourceReference(link) = file.content() else {
            panic!("a resource link");
        };
        assert!(link.uri.starts_with("file:///home/ops/notes%202026/"));
        assert_eq!(path_of(&link.uri).as_deref(), Some(path));
        assert_eq!(PromptFile::of(&file.content()), Some(file));
    }

    #[test]
    fn images_are_told_apart_by_extension_and_relative_paths_are_refused() {
        assert!(is_image(Path::new("/tmp/Shot.PNG")));
        assert!(!is_image(Path::new("/tmp/report.pdf")));
        assert_eq!(
            PromptFile::link(Path::new("notes.txt"), 1),
            Err(PromptFileError::Relative)
        );
    }
}
