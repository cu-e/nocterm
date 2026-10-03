//! What is saved about connections, with no UI attached.
//!
//! Two files, because they have different owners:
//!
//! - `connections.toml` (config directory) holds the profiles. People edit it
//!   by hand and keep it in dotfile repositories, so it is written in place,
//!   preserving comments.
//! - `recents.toml` (state directory) is nocterm's own bookkeeping.

use std::fmt;

use nocterm_session::{Auth, CredentialId, ShellLaunch, Target};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How many recent connections are remembered.
pub const MAX_RECENTS: usize = 8;

/// Identifies a saved connection across renames and edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileId(Uuid);

impl ProfileId {
    /// A new, unique id.
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
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
}

fn is_auto(auth: &Auth) -> bool {
    *auth == Auth::Auto
}

/// The contents of `connections.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profiles {
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

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Profile> {
        self.list.iter_mut()
    }

    pub fn get(&self, id: ProfileId) -> Option<&Profile> {
        self.list.iter().find(|profile| profile.id == id)
    }

    /// Replaces the profile with the same id, or adds it at the end.
    pub fn upsert(&mut self, profile: Profile) {
        match self
            .list
            .iter_mut()
            .find(|existing| existing.id == profile.id)
        {
            Some(existing) => *existing = profile,
            None => self.list.push(profile),
        }
    }

    pub fn remove(&mut self, id: ProfileId) -> Option<Profile> {
        let ix = self.list.iter().position(|profile| profile.id == id)?;
        Some(self.list.remove(ix))
    }

    /// The folders in use, sorted case-insensitively.
    pub fn groups(&self) -> Vec<&str> {
        let mut groups: Vec<&str> = self
            .list
            .iter()
            .filter_map(|profile| profile.group.as_deref())
            .collect();
        groups.sort_by(|left, right| {
            left.to_lowercase()
                .cmp(&right.to_lowercase())
                .then(left.cmp(right))
        });
        groups.dedup();
        groups
    }

    /// The profiles filed under `group` (`None`: at the top level) that match
    /// `filter`, sorted by name.
    pub fn in_group<'a>(&'a self, group: Option<&str>, filter: &str) -> Vec<&'a Profile> {
        let mut profiles: Vec<&Profile> = self
            .list
            .iter()
            .filter(|profile| profile.group.as_deref() == group && profile.matches(filter))
            .collect();
        profiles.sort_by_key(|profile| profile.name.to_lowercase());
        profiles
    }
}

impl Profile {
    /// Whether `filter` occurs, ignoring case, in the name, the destination or
    /// the folder. An empty filter matches everything.
    pub fn matches(&self, filter: &str) -> bool {
        let filter = filter.trim().to_lowercase();
        filter.is_empty()
            || [
                self.name.as_str(),
                self.description.as_str(),
                self.target.host.as_str(),
                self.target.user.as_str(),
                self.group.as_deref().unwrap_or_default(),
            ]
            .iter()
            .any(|field| field.to_lowercase().contains(&filter))
    }
}

/// A connection that was opened, saved or not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    #[serde(
        default,
        skip_serializing_if = "nocterm_session::SessionOptions::is_default"
    )]
    pub options: nocterm_session::SessionOptions,
    #[serde(flatten)]
    pub target: Target,
    #[serde(default, skip_serializing_if = "is_auto")]
    pub auth: Auth,
    /// The saved connection it was opened from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileId>,
    /// Seconds since the Unix epoch.
    pub last_used: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<ShellLaunch>,
}

impl Recent {
    /// Whether the two would open the same thing.
    fn same_as(&self, other: &Self) -> bool {
        match (self.profile, other.profile) {
            (Some(left), Some(right)) => left == right,
            (None, None) => {
                self.target == other.target
                    && self.auth == other.auth
                    && self.credential == other.credential
                    && self.launch == other.launch
                    && self.options == other.options
            }
            _ => false,
        }
    }
}

/// The contents of `recents.toml`, most recent first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recents {
    #[serde(default, rename = "recent")]
    list: Vec<Recent>,
}

impl Recents {
    pub fn iter(&self) -> impl Iterator<Item = &Recent> {
        self.list.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Recent> {
        self.list.iter_mut()
    }

    /// Puts `recent` first, dropping an older entry for the same thing and
    /// whatever falls off the end.
    pub fn record(&mut self, recent: Recent) {
        self.list.retain(|existing| !existing.same_as(&recent));
        self.list.insert(0, recent);
        self.list.truncate(MAX_RECENTS);
    }

    /// Forgets that entries came from a profile that is gone; they stay as
    /// plain destinations.
    pub fn unlink(&mut self, id: ProfileId) {
        for recent in &mut self.list {
            if recent.profile == Some(id) {
                recent.profile = None;
            }
        }
        // Unlinking can leave two entries for the same destination.
        let mut seen: Vec<Recent> = Vec::new();
        self.list.retain(|recent| {
            let duplicate = seen.iter().any(|earlier| earlier.same_as(recent));
            seen.push(recent.clone());
            !duplicate
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn profile(name: &str, host: &str, group: Option<&str>) -> Profile {
        Profile {
            description: String::new(),
            options: Default::default(),
            id: ProfileId::generate(),
            name: name.to_owned(),
            target: Target::new("root", host, 22),
            auth: Auth::Auto,
            group: group.map(str::to_owned),
            credential: None,
            launch: None,
        }
    }

    fn recent(host: &str, profile: Option<ProfileId>, last_used: u64) -> Recent {
        Recent {
            options: Default::default(),
            target: Target::new("root", host, 22),
            auth: Auth::Auto,
            profile,
            last_used,
            credential: None,
            launch: None,
        }
    }

    #[test]
    fn profiles_read_from_hand_written_toml() {
        let text = r#"
            # Work machines
            [[connection]]
            id = "6f1c1f3e-8f6e-4b8a-9d5e-0a1b2c3d4e5f"
            name = "Build box"
            host = "build.example.com"
            port = 2222
            user = "ci"
            group = "Work"
            auth = { method = "key", path = "/home/me/.ssh/ci" }

            [[connection]]
            id = "0b9f3a51-2d64-4f1e-8c7a-5e6d7c8b9a0f"
            name = "Pi"
            host = "pi.local"
            user = "pi"
        "#;

        let profiles: Profiles = toml::from_str(text).unwrap();
        let all: Vec<&Profile> = profiles.iter().collect();

        assert_eq!(all.len(), 2);
        assert_eq!(all[0].target, Target::new("ci", "build.example.com", 2222));
        assert_eq!(
            all[0].auth,
            Auth::Key {
                path: PathBuf::from("/home/me/.ssh/ci")
            }
        );
        assert_eq!(all[0].group.as_deref(), Some("Work"));
        assert_eq!(all[1].target.port, 22);
        assert_eq!(all[1].auth, Auth::Auto);
    }

    #[test]
    fn profiles_round_trip_and_omit_defaults() {
        let mut profiles = Profiles::default();
        profiles.upsert(profile("Pi", "pi.local", None));

        let text = toml::to_string(&profiles).unwrap();

        assert!(text.contains("[[connection]]"), "{text}");
        assert!(!text.contains("port"), "{text}");
        assert!(!text.contains("auth"), "{text}");
        assert_eq!(toml::from_str::<Profiles>(&text).unwrap(), profiles);
    }

    #[test]
    fn credential_references_and_launch_round_trip_without_secret_fields() {
        let id = CredentialId::generate().unwrap();
        let mut item = profile("Build", "build.local", None);
        item.credential = Some(id);
        item.launch = Some(ShellLaunch {
            program: Some("/bin/bash".into()),
            args: vec!["-l".into()],
            ..ShellLaunch::default()
        });
        let mut profiles = Profiles::default();
        profiles.upsert(item);
        let text = toml::to_string(&profiles).unwrap();
        assert!(text.contains(&id.to_string()));
        for forbidden in ["password", "passphrase", "master", "secret"] {
            assert!(!text.contains(forbidden));
        }
        assert_eq!(toml::from_str::<Profiles>(&text).unwrap(), profiles);
        let mut item = recent("build.local", None, 1);
        item.credential = Some(id);
        item.launch = Some(ShellLaunch::default());
        let mut recents = Recents::default();
        recents.record(item);
        let text = toml::to_string(&recents).unwrap();
        assert!(text.contains(&id.to_string()));
        assert_eq!(toml::from_str::<Recents>(&text).unwrap(), recents);
    }

    #[test]
    fn upsert_replaces_by_id() {
        let mut profiles = Profiles::default();
        let mut pi = profile("Pi", "pi.local", None);
        profiles.upsert(pi.clone());

        pi.name = "Raspberry".to_owned();
        profiles.upsert(pi.clone());

        assert_eq!(profiles.iter().count(), 1);
        assert_eq!(profiles.get(pi.id).unwrap().name, "Raspberry");
        assert_eq!(profiles.remove(pi.id), Some(pi));
        assert!(profiles.is_empty());
    }

    #[test]
    fn groups_and_filtering() {
        let mut profiles = Profiles::default();
        profiles.upsert(profile("web", "web.example.com", Some("work")));
        profiles.upsert(profile("db", "db.example.com", Some("Work")));
        profiles.upsert(profile("Pi", "pi.local", None));
        profiles.upsert(profile("api", "api.example.com", Some("work")));

        assert_eq!(profiles.groups(), ["Work", "work"]);

        let names = |group, filter| {
            profiles
                .in_group(group, filter)
                .iter()
                .map(|profile| profile.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(Some("work"), ""), ["api", "web"]);
        assert_eq!(names(Some("work"), "WEB"), ["web"]);
        assert_eq!(names(None, ""), ["Pi"]);
        assert_eq!(names(None, "example"), Vec::<String>::new());
    }

    #[test]
    fn recents_keep_the_latest_use_of_each_destination() {
        let mut recents = Recents::default();
        let saved = ProfileId::generate();

        recents.record(recent("a", None, 1));
        recents.record(recent("b", Some(saved), 2));
        recents.record(recent("a", None, 3));
        // The same profile, even if its destination changed since.
        recents.record(recent("b2", Some(saved), 4));

        let hosts: Vec<&str> = recents.iter().map(|r| r.target.host.as_str()).collect();
        assert_eq!(hosts, ["b2", "a"]);
    }

    #[test]
    fn recents_are_bounded() {
        let mut recents = Recents::default();
        for ix in 0..(MAX_RECENTS as u64 + 3) {
            recents.record(recent(&format!("host{ix}"), None, ix));
        }

        assert_eq!(recents.iter().count(), MAX_RECENTS);
        assert_eq!(recents.iter().next().unwrap().target.host, "host10");
    }

    #[test]
    fn unlinking_a_deleted_profile_keeps_its_destination() {
        let mut recents = Recents::default();
        let gone = ProfileId::generate();
        recents.record(recent("a", None, 1));
        recents.record(recent("a", Some(gone), 2));

        recents.unlink(gone);

        let all: Vec<&Recent> = recents.iter().collect();
        assert_eq!(all.len(), 1, "the two entries are now the same destination");
        assert_eq!(all[0].profile, None);
        assert_eq!(all[0].last_used, 2);
    }

    #[test]
    fn recents_round_trip() {
        let mut recents = Recents::default();
        recents.record(recent("a", Some(ProfileId::generate()), 7));

        let text = toml::to_string(&recents).unwrap();

        assert_eq!(toml::from_str::<Recents>(&text).unwrap(), recents);
    }
}
