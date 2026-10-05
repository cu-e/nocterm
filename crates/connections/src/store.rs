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
    /// The sidebar icon, as an operating system id from [`crate::os`]; none
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profiles {
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

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Profile> {
        self.list.iter_mut()
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
    ) -> Result<(), &'static str> {
        let ix = self
            .list
            .iter()
            .position(|profile| profile.id == id)
            .ok_or("Connection no longer exists")?;
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
            icon: None,
            icon_color: None,
            country: None,
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
    fn legacy_folders_survive_the_last_profile_moving_out_and_back() {
        let original = profile("Build", "build.local", Some("Work"));
        // Old versions only recorded folder names on profiles.
        let text = format!("[[connection]]\n{}", toml::to_string(&original).unwrap());
        let mut profiles: Profiles = toml::from_str(&text).unwrap();
        assert_eq!(profiles.groups(), ["Work"]);
        let mut moved = original.clone();
        moved.group = Some("Personal".into());
        profiles.upsert(moved);
        let text = toml::to_string(&profiles).unwrap();
        let mut restored: Profiles = toml::from_str(&text).unwrap();
        assert_eq!(restored.groups(), ["Personal", "Work"]);
        assert!(restored.in_group(Some("Work"), "").is_empty());
        restored.upsert(original.clone());
        assert_eq!(restored.get(original.id), Some(&original));
        restored.remove(original.id);
        assert_eq!(restored.groups(), ["Personal", "Work"]);
    }

    #[test]
    fn rename_group_moves_members_and_empty_folders() {
        let mut profiles = Profiles::default();
        let mut build = profile("Build", "build.local", Some("Work"));
        build.description = "ci".into();
        profiles.upsert(build.clone());
        profiles.upsert(profile("Pi", "pi.local", None));
        let gone = profile("Gone", "gone.local", Some("Empty"));
        profiles.upsert(gone.clone());
        profiles.remove(gone.id);

        profiles.rename_group("Work", "Team");
        profiles.rename_group("Empty", "Spare");

        let moved = profiles.get(build.id).unwrap();
        assert_eq!(moved.group.as_deref(), Some("Team"));
        assert_eq!(moved.description, "ci");
        assert_eq!(profiles.groups(), ["Spare", "Team"]);
        let text = toml::to_string(&profiles).unwrap();
        let restored: Profiles = toml::from_str(&text).unwrap();
        assert_eq!(restored.groups(), ["Spare", "Team"]);
        assert!(!restored.has_group("Work"));
    }

    #[test]
    fn rename_group_merges_and_is_case_sensitive() {
        let mut profiles = Profiles::default();
        profiles.upsert(profile("A", "a.local", Some("Work")));
        profiles.upsert(profile("B", "b.local", Some("Personal")));
        profiles.upsert(profile("C", "c.local", Some("work")));

        profiles.rename_group("Work", "Personal");
        assert_eq!(profiles.groups(), ["Personal", "work"]);
        assert_eq!(profiles.in_group(Some("Personal"), "").len(), 2);
        assert_eq!(profiles.in_group(Some("work"), "").len(), 1);
    }

    #[test]
    fn ungroup_files_members_at_top_level_and_forgets_folder() {
        let mut profiles = Profiles::default();
        profiles.upsert(profile("A", "a.local", Some("Work")));
        profiles.upsert(profile("B", "b.local", Some("Other")));
        let mut remembered = profile("C", "c.local", Some("Old"));
        profiles.upsert(remembered.clone());
        remembered.group = None;
        profiles.upsert(remembered);

        profiles.ungroup("Work");
        profiles.ungroup("Old");

        assert_eq!(profiles.in_group(None, "").len(), 2);
        assert_eq!(profiles.groups(), ["Other"]);
    }

    #[test]
    fn remove_group_drops_only_members_and_does_not_resurrect_folder() {
        let mut profiles = Profiles::default();
        profiles.upsert(profile("A", "a.local", Some("Work")));
        profiles.upsert(profile("B", "b.local", Some("Other")));
        profiles.upsert(profile("C", "c.local", None));

        let removed = profiles.remove_group("Work");

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].name, "A");
        assert_eq!(profiles.iter().count(), 2);
        assert_eq!(profiles.groups(), ["Other"]);
        assert!(!profiles.has_group("Work"));
    }

    #[test]
    fn group_actions_on_unknown_or_identical_names_change_nothing() {
        let mut profiles = Profiles::default();
        profiles.upsert(profile("A", "a.local", Some("Work")));
        profiles.upsert(profile("B", "b.local", None));
        let before = toml::to_string(&profiles).unwrap();

        profiles.rename_group("Work", "Work");
        profiles.rename_group("Missing", "Missing");
        profiles.ungroup("Missing");
        let removed = profiles.remove_group("Missing");

        assert!(removed.is_empty());
        assert_eq!(toml::to_string(&profiles).unwrap(), before);
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
    fn icon_choices_round_trip_and_old_files_have_none() {
        let mut item = profile("Web", "web.local", None);
        item.icon = Some("ubuntu".into());
        item.icon_color = Some("#E95420".into());
        let mut profiles = Profiles::default();
        profiles.upsert(item);
        let text = toml::to_string(&profiles).unwrap();
        assert!(text.contains("icon = \"ubuntu\""), "{text}");
        assert_eq!(toml::from_str::<Profiles>(&text).unwrap(), profiles);

        let plain = toml::to_string(&profile("Pi", "pi.local", None)).unwrap();
        assert!(!plain.contains("icon"), "{plain}");
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
        assert_eq!(
            names(Some("work"), ""),
            ["web", "api"],
            "in the user's order"
        );
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
    #[test]
    fn placing_reorders_within_and_across_folders() {
        let mut profiles = Profiles::default();
        let a = profile("a", "a.host", None);
        let b = profile("b", "b.host", None);
        let c = profile("c", "c.host", Some("Work"));
        for p in [&a, &b, &c] {
            profiles.upsert(p.clone());
        }
        let names = |profiles: &Profiles, group: Option<&str>| {
            profiles
                .in_group(group, "")
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
        };
        profiles.place(b.id, None, Some(a.id)).unwrap();
        assert_eq!(names(&profiles, None), ["b", "a"]);
        profiles
            .place(a.id, Some("Work".into()), Some(c.id))
            .unwrap();
        assert_eq!(names(&profiles, None), ["b"]);
        assert_eq!(names(&profiles, Some("Work")), ["a", "c"]);
        profiles.place(a.id, Some("Work".into()), None).unwrap();
        assert_eq!(names(&profiles, Some("Work")), ["c", "a"]);
        profiles.place(b.id, None, Some(b.id)).unwrap();
        assert_eq!(names(&profiles, Some("Work")), ["c", "a"]);
        profiles
            .place(a.id, Some("Work".into()), Some(a.id))
            .unwrap();
        assert_eq!(
            names(&profiles, Some("Work")),
            ["c", "a"],
            "dropped on itself"
        );
        profiles.place(a.id, None, Some(a.id)).unwrap();
        assert_eq!(names(&profiles, None), ["b", "a"]);
        assert!(
            profiles.has_group("Work"),
            "an emptied folder is remembered"
        );
        assert!(profiles.place(ProfileId::generate(), None, None).is_err());
    }
    #[test]
    fn one_pass_grouped_view_preserves_folders_filters_and_user_order() {
        let mut profiles = Profiles::default();
        let saved = profile("Archived", "old.host", Some("Empty"));
        profiles.upsert(saved.clone());
        profiles.remove(saved.id);
        profiles.upsert(profile("beta", "build.host", Some("Work")));
        profiles.upsert(profile("Alpha", "build.host", Some("Work")));
        profiles.upsert(profile("Standalone", "other.host", None));
        let (top, groups) = profiles.grouped("");
        assert_eq!(
            top.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["Standalone"]
        );
        assert_eq!(
            groups
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["Empty", "Work"]
        );
        assert!(groups[0].1.is_empty());
        assert_eq!(
            groups[1]
                .1
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["beta", "Alpha"]
        );
        let (top, groups) = profiles.grouped(" BUILD ");
        assert!(top.is_empty());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1, profiles.in_group(Some("Work"), "BUILD"));
    }
}
