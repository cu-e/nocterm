//! Pure saved-profile format and checked collection transitions.
mod mutation;
pub use crate::recents::{MAX_RECENTS, Recent, Recents};
pub use mutation::{ProfileError, ProfileExpectation, ProfileMutation};
use std::fmt;

use nocterm_session::{Auth, CredentialId, ShellLaunch, Target};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifies a saved connection across renames and edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileId(Uuid);

impl ProfileId {
    /// Identifier bytes are supplied by an adapter or deterministic caller.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A saved connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(
        default,
        skip_serializing_if = "nocterm_session::SessionOptions::is_default"
    )]
    pub options: nocterm_session::SessionOptions,
    pub id: ProfileId,
    /// Shown in the sidebar and on the tab.
    pub name: String,
    #[serde(flatten)]
    pub target: Target,
    #[serde(default, skip_serializing_if = "is_auto")]
    pub auth: Auth,
    /// The sidebar folder it is filed under; none keeps it at the top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<ShellLaunch>,
    /// The sidebar icon, as an operating system id from the presentation catalogue; none
    /// shows the system detected on connecting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The icon's colour as `#RRGGBB`; none uses the system's brand colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    /// The flag shown, as a lower-case ISO 3166-1 alpha-2 code; none shows
    /// the detected country.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

fn is_auto(auth: &Auth) -> bool {
    *auth == Auth::Auto
}

/// The contents of `connections.toml`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profiles {
    /// Forward-compatible tables belong to the document, even when not interpreted.
    #[serde(flatten)]
    extra: std::collections::BTreeMap<String, toml::Value>,
    /// Remember folders even after their last connection is moved or deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    folders: Vec<String>,
    #[serde(default, rename = "connection")]
    list: Vec<Profile>,
}

impl Profiles {
    pub fn iter(&self) -> impl Iterator<Item = &Profile> {
        self.list.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn associate_credential(&mut self, target: &Target, auth: &Auth, id: CredentialId) {
        for profile in self
            .list
            .iter_mut()
            .filter(|profile| &profile.target == target && &profile.auth == auth)
        {
            profile.credential = Some(id);
        }
    }

    pub fn get(&self, id: ProfileId) -> Option<&Profile> {
        self.list.iter().find(|profile| profile.id == id)
    }

    /// Replaces the profile with the same id, or adds it at the end.
    pub fn upsert(&mut self, profile: Profile) {
        let previous = self.get(profile.id).and_then(|item| item.group.clone());
        self.remember_group(previous.as_deref());
        self.remember_group(profile.group.as_deref());
        match self
            .list
            .iter_mut()
            .find(|existing| existing.id == profile.id)
        {
            Some(existing) => *existing = profile,
            None => self.list.push(profile),
        }
    }

    /// Files the profile under `group` and places it just before `before`,
    /// or at the end when `before` is absent, the profile itself or gone.
    /// The list order is the order the sidebar shows.
    pub fn place(
        &mut self,
        id: ProfileId,
        group: Option<String>,
        before: Option<ProfileId>,
    ) -> Result<(), ProfileError> {
        let ix = self
            .list
            .iter()
            .position(|profile| profile.id == id)
            .ok_or(ProfileError::MissingProfile(id))?;
        if before == Some(id) {
            // Dropped on its own row: it stays where it is.
            self.remember_group(self.list[ix].group.clone().as_deref());
            self.remember_group(group.as_deref());
            self.list[ix].group = group;
            return Ok(());
        }
        let mut profile = self.list.remove(ix);
        self.remember_group(profile.group.as_deref());
        self.remember_group(group.as_deref());
        profile.group = group;
        let at = before
            .filter(|before| *before != id)
            .and_then(|before| self.list.iter().position(|p| p.id == before))
            .unwrap_or(self.list.len());
        self.list.insert(at, profile);
        Ok(())
    }

    pub fn remove(&mut self, id: ProfileId) -> Option<Profile> {
        let ix = self.list.iter().position(|profile| profile.id == id)?;
        let profile = self.list.remove(ix);
        self.remember_group(profile.group.as_deref());
        Some(profile)
    }

    /// Whether `name` is an existing or remembered folder.
    pub fn has_group(&self, name: &str) -> bool {
        self.folders.iter().any(|folder| folder == name)
            || self
                .list
                .iter()
                .any(|profile| profile.group.as_deref() == Some(name))
    }

    /// Renames a folder; renaming onto an existing folder merges the two.
    pub fn rename_group(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        for profile in &mut self.list {
            if profile.group.as_deref() == Some(from) {
                profile.group = Some(to.to_owned());
            }
        }
        self.forget_group(from);
        self.remember_group(Some(to));
    }

    /// Files the folder's connections at the top level and forgets the folder.
    pub fn ungroup(&mut self, group: &str) {
        for profile in &mut self.list {
            if profile.group.as_deref() == Some(group) {
                profile.group = None;
            }
        }
        self.forget_group(group);
    }

    /// Deletes the folder and its connections, returning the removed ones.
    pub fn remove_group(&mut self, group: &str) -> Vec<Profile> {
        let (removed, kept) = std::mem::take(&mut self.list)
            .into_iter()
            .partition(|profile| profile.group.as_deref() == Some(group));
        self.list = kept;
        self.forget_group(group);
        removed
    }

    fn forget_group(&mut self, group: &str) {
        self.folders.retain(|existing| existing != group);
    }

    fn remember_group(&mut self, group: Option<&str>) {
        if let Some(group) = group
            && !self.folders.iter().any(|existing| existing == group)
        {
            self.folders.push(group.to_owned());
            self.folders.sort();
        }
    }

    /// Existing and remembered folders, sorted case-insensitively.
    pub fn groups(&self) -> Vec<&str> {
        let mut groups: Vec<&str> = self
            .folders
            .iter()
            .map(String::as_str)
            .chain(
                self.list
                    .iter()
                    .filter_map(|profile| profile.group.as_deref()),
            )
            .collect();
        groups.sort_by_cached_key(|name| (name.to_lowercase(), *name));
        groups.dedup();
        groups
    }

    /// Group and filter once, rather than rescanning every profile for every
    /// folder. Members keep the user's order, which drag and drop changes.
    pub fn grouped(&self, filter: &str) -> (Vec<&Profile>, Vec<(String, Vec<&Profile>)>) {
        let filter = filter.trim().to_lowercase();
        let mut groups: Vec<_> = self
            .groups()
            .into_iter()
            .map(|name| (name.to_owned(), Vec::new()))
            .collect();
        let indices: std::collections::HashMap<_, _> = groups
            .iter()
            .enumerate()
            .map(|(index, (name, _))| (name.clone(), index))
            .collect();
        let mut top = Vec::new();
        for profile in &self.list {
            if !profile.matches_normalized(&filter) {
                continue;
            }
            if let Some(group) = &profile.group {
                groups[indices[group]].1.push(profile);
            } else {
                top.push(profile);
            }
        }
        if !filter.is_empty() {
            groups.retain(|(_, members)| !members.is_empty());
        }
        (top, groups)
    }

    /// The profiles filed under `group` (`None`: at the top level) that match
    /// `filter`, in the user's order.
    pub fn in_group<'a>(&'a self, group: Option<&str>, filter: &str) -> Vec<&'a Profile> {
        self.list
            .iter()
            .filter(|profile| profile.group.as_deref() == group && profile.matches(filter))
            .collect()
    }
}

impl Profile {
    /// Whether `filter` occurs, ignoring case, in the name, the destination or
    /// the folder. An empty filter matches everything.
    pub fn matches(&self, filter: &str) -> bool {
        self.matches_normalized(&filter.trim().to_lowercase())
    }
    fn matches_normalized(&self, filter: &str) -> bool {
        filter.is_empty()
            || [
                self.name.as_str(),
                self.description.as_str(),
                self.target.host.as_str(),
                self.target.user.as_str(),
                self.group.as_deref().unwrap_or_default(),
            ]
            .iter()
            .any(|field| field.to_lowercase().contains(filter))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ProfileIdParseError(#[from] uuid::Error);
impl std::str::FromStr for ProfileId {
    type Err = ProfileIdParseError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self(text.parse()?))
    }
}
#[cfg(test)]
mod tests;
