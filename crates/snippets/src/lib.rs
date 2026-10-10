//! Saved code snippets and exact connection bindings, independent of the UI.
//!
//! Owns: snippet/library formats, validation and checked record mutations.
//! Does not own: identity generation, filesystem persistence or editor views.
//! Ports used/provided: none. Settings section: none. Globals: none.
//! Invariants: identity is supplied by the caller; stale expected records cannot
//! overwrite or delete a newer saved record; validation performs no I/O.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Languages whose grammars are bundled by the snippet editor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Bash,
    Json,
    Python,
    Yaml,
    Toml,
    Plaintext,
}
impl Language {
    pub const ALL: [Self; 6] = [
        Self::Bash,
        Self::Json,
        Self::Python,
        Self::Yaml,
        Self::Toml,
        Self::Plaintext,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Json => "json",
            Self::Python => "python",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::Plaintext => "plaintext",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Bash => "Shell",
            Self::Json => "JSON",
            Self::Python => "Python",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Plaintext => "Plain text",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub content: String,
    #[serde(default)]
    pub language: Language,
    /// Stable saved-profile identifiers, never inferred from host names.
    #[serde(default)]
    pub profiles: Vec<String>,
    /// Connection groups currently use exact names as their identities.
    #[serde(default)]
    pub groups: Vec<String>,
}
impl Snippet {
    /// Create a draft with the identity provided by the application boundary.
    pub fn new(id: Uuid) -> Self {
        Self {
            id,
            name: String::new(),
            description: String::new(),
            content: String::new(),
            language: Language::default(),
            profiles: Vec::new(),
            groups: Vec::new(),
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::Name);
        }
        if self.content.trim().is_empty() {
            return Err(ValidationError::Content);
        }
        let bytes = self
            .name
            .len()
            .saturating_add(self.description.len())
            .saturating_add(self.content.len())
            .saturating_add(
                self.profiles
                    .iter()
                    .chain(&self.groups)
                    .map(String::len)
                    .sum::<usize>(),
            );
        if bytes > 256 * 1024 {
            return Err(ValidationError::Size);
        }
        if self
            .profiles
            .iter()
            .chain(&self.groups)
            .any(|id| id.trim().is_empty())
        {
            return Err(ValidationError::Binding);
        }
        Ok(())
    }
    /// No active profile means no contextual match. Missing bindings remain scoped.
    pub fn matches(&self, profile: Option<&str>, group: Option<&str>) -> bool {
        let Some(profile) = profile else {
            return false;
        };
        self.profiles.iter().any(|id| id == profile)
            || group.is_some_and(|group| self.groups.iter().any(|name| name == group))
    }
}
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("Enter a snippet name.")]
    Name,
    #[error("Enter snippet content.")]
    Content,
    #[error("Snippet fields exceed the 256 KiB size limit.")]
    Size,
    #[error("Connection bindings cannot be empty.")]
    Binding,
}

/// Checked library mutations retain semantic errors for application policy.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum LibraryError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("Duplicate snippet identifier: {0}.")]
    DuplicateId(Uuid),
    #[error("This snippet changed or was deleted. Close the editor and open it again.")]
    SaveConflict(Uuid),
    #[error("This snippet was already deleted.")]
    Deleted(Uuid),
    #[error("This snippet changed. Review it before deleting.")]
    DeleteConflict(Uuid),
}

/// File format and checked mutations. Snapshots prevent stale dialogs overwriting changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub snippets: Vec<Snippet>,
}
impl Library {
    pub fn validate(&self) -> Result<(), LibraryError> {
        let mut ids = std::collections::HashSet::new();
        for snippet in &self.snippets {
            snippet.validate()?;
            if !ids.insert(snippet.id) {
                return Err(LibraryError::DuplicateId(snippet.id));
            }
        }
        Ok(())
    }
    pub fn save(
        &mut self,
        snippet: Snippet,
        expected: Option<&Snippet>,
    ) -> Result<(), LibraryError> {
        snippet.validate()?;
        let index = self
            .snippets
            .iter()
            .position(|saved| saved.id == snippet.id);
        if index.map(|index| &self.snippets[index]) != expected {
            return Err(LibraryError::SaveConflict(snippet.id));
        }
        if let Some(index) = index {
            self.snippets[index] = snippet;
        } else {
            self.snippets.push(snippet);
        }
        Ok(())
    }
    pub fn delete(&mut self, expected: &Snippet) -> Result<(), LibraryError> {
        let index = self
            .snippets
            .iter()
            .position(|saved| saved.id == expected.id)
            .ok_or(LibraryError::Deleted(expected.id))?;
        if &self.snippets[index] != expected {
            return Err(LibraryError::DeleteConflict(expected.id));
        }
        self.snippets.remove(index);
        Ok(())
    }
    pub fn partition(
        &self,
        profile: Option<&str>,
        group: Option<&str>,
        query: &str,
    ) -> (Vec<&Snippet>, Vec<&Snippet>) {
        let query = query.trim().to_lowercase();
        self.snippets
            .iter()
            .filter(|snippet| {
                query.is_empty()
                    || snippet.name.to_lowercase().contains(&query)
                    || snippet.description.to_lowercase().contains(&query)
                    || snippet.content.to_lowercase().contains(&query)
            })
            .partition(|snippet| snippet.matches(profile, group))
    }
}
#[cfg(test)]
mod tests;
