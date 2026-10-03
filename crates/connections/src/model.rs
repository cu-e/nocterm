use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use gpui_kit::{App, AppContext as _, Context, Entity, Global, WeakEntity, Window};
use nocterm_core::{Paths, persist};
use nocterm_session::{Auth, CredentialId, Target};
use nocterm_workspace::{SessionSpec, Workspace};

use crate::store::{Profile, ProfileId, Profiles, Recent, Recents};

/// Saved connections and recent ones, shared by every view that lists them.
pub struct Connections {
    profiles: Profiles,
    recents: Recents,
    profiles_file: Option<PathBuf>,
    recents_file: Option<PathBuf>,
    /// Why `connections.toml` could not be read. While this is set the file
    /// is never written, so nothing the user wrote in it is lost.
    load_error: Option<String>,
}

struct GlobalConnections(Entity<Connections>);

impl Global for GlobalConnections {}

impl Connections {
    /// Reads both files; problems are logged and leave the lists empty.
    pub fn load(paths: &Paths) -> Self {
        let profiles_file = paths.connections_file();
        let recents_file = paths.recents_file();
        let (profiles, load_error) = load_profiles(&profiles_file);
        Self {
            profiles,
            recents: load_recents(&recents_file),
            profiles_file: Some(profiles_file),
            recents_file: Some(recents_file),
            load_error,
        }
    }

    /// Connections kept only for the life of the process.
    pub fn in_memory() -> Self {
        Self {
            profiles: Profiles::default(),
            recents: Recents::default(),
            profiles_file: None,
            recents_file: None,
            load_error: None,
        }
    }

    pub(crate) fn install(self, cx: &mut App) {
        let entity = cx.new(|_| self);
        cx.set_global(GlobalConnections(entity));
    }

    /// The application's connections. Panics before [`crate::init`].
    pub fn global(cx: &App) -> Entity<Connections> {
        cx.global::<GlobalConnections>().0.clone()
    }

    pub fn profiles(&self) -> &Profiles {
        &self.profiles
    }

    pub fn recents(&self) -> &Recents {
        &self.recents
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Adds or updates a saved connection.
    pub fn save_profile(&mut self, profile: Profile, cx: &mut Context<Self>) -> Result<(), String> {
        let mut profiles = self.profiles.clone();
        profiles.upsert(profile);
        self.write_profiles(profiles, cx)
    }

    /// Move the current saved profile; a drag carries identity, never stale settings.
    pub fn move_profile(
        &mut self,
        id: ProfileId,
        group: Option<String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let mut profile = self
            .profiles
            .get(id)
            .cloned()
            .ok_or("Connection no longer exists")?;
        if profile.group == group {
            return Ok(());
        }
        profile.group = group;
        self.save_profile(profile, cx)
    }

    pub fn delete_profile(&mut self, id: ProfileId, cx: &mut Context<Self>) -> Result<(), String> {
        let mut profiles = self.profiles.clone();
        if profiles.remove(id).is_none() {
            return Ok(());
        }
        self.write_profiles(profiles, cx)?;
        self.recents.unlink(id);
        self.write_recents();
        Ok(())
    }

    fn write_profiles(&mut self, profiles: Profiles, cx: &mut Context<Self>) -> Result<(), String> {
        if let Some(error) = &self.load_error {
            return Err(format!(
                "Saved connections are read-only until the file is fixed: {error}"
            ));
        }
        if let Some(path) = &self.profiles_file {
            persist::save_preserving(path, &profiles).map_err(|error| error.to_string())?;
        }
        self.profiles = profiles;
        cx.notify();
        Ok(())
    }

    /// Remembers that a connection was opened.
    pub fn record_use(
        &mut self,
        spec: &SessionSpec,
        profile: Option<ProfileId>,
        cx: &mut Context<Self>,
    ) {
        self.recents.record(Recent {
            options: spec.options.clone(),
            target: spec.target.clone(),
            auth: spec.auth.clone(),
            profile,
            last_used: now(),
            credential: spec.credential,
            launch: spec.launch.clone(),
        });
        self.write_recents();
        cx.notify();
    }

    /// Links an explicitly saved, authenticated secret without storing its value.
    pub fn associate_credential(
        &mut self,
        target: &Target,
        auth: &Auth,
        id: CredentialId,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let mut profiles = self.profiles.clone();
        for profile in profiles
            .iter_mut()
            .filter(|profile| &profile.target == target && &profile.auth == auth)
        {
            profile.credential = Some(id);
        }
        self.write_profiles(profiles, cx)?;
        let mut recents = self.recents.clone();
        for recent in recents
            .iter_mut()
            .filter(|recent| &recent.target == target && &recent.auth == auth)
        {
            recent.credential = Some(id);
        }
        if let Some(path) = &self.recents_file {
            persist::save(path, &recents).map_err(|error| error.to_string())?;
        }
        self.recents = recents;
        cx.notify();
        Ok(())
    }

    fn write_recents(&self) {
        if let Some(path) = &self.recents_file
            && let Err(error) = persist::save(path, &self.recents)
        {
            tracing::warn!(%error, "could not save recent connections");
        }
    }

    /// What opening `recent` opens: the saved connection as it is now, if it
    /// came from one that still exists.
    pub fn spec_for_recent(&self, recent: &Recent) -> SessionSpec {
        match recent.profile.and_then(|id| self.profiles.get(id)) {
            Some(profile) => spec_for_profile(profile),
            None => SessionSpec {
                options: recent.options.clone(),
                title: recent.target.to_string().into(),
                target: recent.target.clone(),
                auth: recent.auth.clone(),
                credential: recent.credential,
                launch: recent.launch.clone(),
            },
        }
    }
}

pub fn spec_for_profile(profile: &Profile) -> SessionSpec {
    SessionSpec {
        options: profile.options.clone(),
        title: profile.name.clone().into(),
        target: profile.target.clone(),
        auth: profile.auth.clone(),
        credential: profile.credential,
        launch: profile.launch.clone(),
    }
}

/// Opens `spec` in a new tab and remembers it among the recent connections.
pub fn connect(
    workspace: &WeakEntity<Workspace>,
    spec: SessionSpec,
    profile: Option<ProfileId>,
    window: &mut Window,
    cx: &mut App,
) {
    Connections::global(cx).update(cx, |connections, cx| {
        connections.record_use(&spec, profile, cx);
    });
    let _ = workspace.update(cx, |workspace, cx| workspace.open_session(spec, window, cx));
}

fn load_profiles(path: &Path) -> (Profiles, Option<String>) {
    match persist::load::<Profiles>(path) {
        Ok(profiles) => (profiles.unwrap_or_default(), None),
        Err(error) => {
            tracing::error!(%error, "could not read saved connections");
            (Profiles::default(), Some(error.to_string()))
        }
    }
}

fn load_recents(path: &Path) -> Recents {
    persist::load(path)
        .unwrap_or_else(|error| {
            // Only bookkeeping: start afresh.
            tracing::warn!(%error, "could not read recent connections");
            None
        })
        .unwrap_or_default()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[gpui_kit::test]
    fn moves_preserve_latest_attributes_and_roll_back_failed_storage(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let profile = Profile {
            id: ProfileId::generate(),
            name: "Build".into(),
            description: "Retained".into(),
            options: Default::default(),
            target: Target::new("ci", "build", 2222),
            auth: Auth::Password,
            credential: Some(CredentialId::generate().unwrap()),
            launch: Some(Default::default()),
            group: Some("Work".into()),
        };
        let entity = cx.new(|_| Connections::in_memory());
        entity.update(cx, |connections, cx| {
            connections.save_profile(profile.clone(), cx).unwrap();
            connections
                .move_profile(profile.id, Some("Personal".into()), cx)
                .unwrap();
            let mut expected = profile.clone();
            expected.group = Some("Personal".into());
            assert_eq!(connections.profiles.get(profile.id), Some(&expected));
            assert_eq!(connections.profiles.groups(), ["Personal", "Work"]);
            let before = connections.profiles.clone();
            // A directory cannot be atomically replaced by the TOML file.
            connections.profiles_file = Some(dir.path().into());
            assert!(connections.move_profile(profile.id, None, cx).is_err());
            assert_eq!(connections.profiles, before);
            connections.profiles_file = None;
            connections.load_error = Some("Invalid TOML".into());
            assert!(connections.move_profile(profile.id, None, cx).is_err());
            assert_eq!(connections.profiles, before);
            // Dropping on the existing folder does not attempt a write.
            connections
                .move_profile(profile.id, Some("Personal".into()), cx)
                .unwrap();
        });
    }

    #[test]
    fn saved_profile_keeps_description_and_session_overrides_when_opened() {
        let profile = Profile {
            id: ProfileId::generate(),
            name: "Legacy build".into(),
            description: "Windows-1251 machine\nUses a private proxy".into(),
            target: nocterm_session::Target::new("ci", "build.example", 22),
            auth: nocterm_session::Auth::Auto,
            group: None,
            credential: None,
            launch: None,
            options: nocterm_session::SessionOptions {
                term: Some("screen-256color".into()),
                charset: Some(nocterm_session::Charset::Windows1251),
                proxy: Some(nocterm_session::ProxyConfig::HttpConnect {
                    host: "127.0.0.1".into(),
                    port: 8080,
                }),
                logging: Some(nocterm_session::LoggingOptions::default()),
            },
        };
        let serialized = toml::to_string(&profile).unwrap();
        let restored: Profile = toml::from_str(&serialized).unwrap();
        assert_eq!(restored.description, profile.description);
        assert_eq!(spec_for_profile(&restored).options, profile.options);
    }
    #[test]
    fn a_missing_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();

        let (profiles, error) = load_profiles(&dir.path().join("connections.toml"));

        assert!(profiles.is_empty());
        assert_eq!(error, None);
        assert_eq!(
            load_recents(&dir.path().join("recents.toml")),
            Recents::default()
        );
    }

    #[test]
    fn a_broken_file_is_reported_not_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.toml");
        fs::write(&path, "[[connection]]\nname = 3\n").unwrap();

        let (profiles, error) = load_profiles(&path);

        assert!(profiles.is_empty());
        assert!(error.unwrap().contains("connections.toml"));
    }

    #[test]
    fn broken_recents_start_afresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recents.toml");
        fs::write(&path, "not toml at all [").unwrap();

        assert_eq!(load_recents(&path), Recents::default());
    }
}
