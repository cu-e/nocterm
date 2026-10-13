//! What nocterm learns about saved servers by connecting to them: which
//! operating system they run and which country they are in.
//!
//! These are observations, not configuration, so they live in the state
//! directory (`servers.toml`, `flags/`) rather than in `connections.toml`:
//! detecting a system never rewrites a hand-edited file, and never conflicts
//! with a connection being edited. What the user chose in the editor
//! ([`Profile::icon`]) always wins over what was detected.
//!
//! Flags are cached twice: decoded in memory, shared by every server in the
//! same country, and as SVG files on disk, so each flag is downloaded once.

use nocterm_connections_model::profiles::{Profile, ProfileId};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};

use crate::{ObservationDeps, observation::LocationRequest};
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Image, ImageFormat};
use nocterm_connections_model::{facts::Known, observation::CountryCode};
use nocterm_session::RemoteFs;
use nocterm_store::{ShutdownCoordinator, StoreError};
use nocterm_workspace::Workspace;

mod persistence;
use persistence::FactsStore;

use crate::{
    Connections, geo,
    os::{self, Os},
};

/// Detected systems and countries of saved connections, and flag images.
pub struct ServerFacts {
    observations: ObservationDeps,
    known: FactsStore,
    /// Absent: never asked for; `None`: loading, or no image exists.
    flags: HashMap<String, Option<Arc<Image>>>,
    /// Sessions already probed; each is probed once.
    probed: Vec<Weak<dyn RemoteFs>>,
    locating: HashSet<ProfileId>,
}

struct GlobalServerFacts(Entity<ServerFacts>);
impl Global for GlobalServerFacts {}

impl ServerFacts {
    pub fn with_backend(
        known: Known,
        writer: nocterm_store::SnapshotWriter<Known>,
        observations: ObservationDeps,
    ) -> Self {
        Self {
            known: FactsStore::new(known, writer),
            ..Self::in_memory(observations)
        }
    }
    pub fn in_memory(observations: ObservationDeps) -> Self {
        Self {
            observations,
            known: FactsStore::new(Known::default(), Arc::new(|_| Box::pin(async { Ok(()) }))),
            flags: HashMap::new(),
            probed: Vec::new(),
            locating: HashSet::new(),
        }
    }

    pub(crate) fn install(
        mut self,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<(), StoreError> {
        self.known.register(shutdown, cx)?;
        let countries: HashSet<String> =
            self.known.read(cx).countries().map(str::to_owned).collect();
        let storage = self.known.install(cx);
        let entity = cx.new(|cx| {
            let mut this = self;
            for code in countries {
                this.ensure_flag(&code, cx);
            }
            this
        });
        cx.observe(&storage, {
            let entity = entity.downgrade();
            move |storage, cx| {
                if let Some(error) = storage.read(cx).error() {
                    tracing::warn!(%error, "could not save detected server details");
                }
                let _ = entity.update(cx, |_, cx| cx.notify());
            }
        })
        .detach();
        // Forget servers whose connection was deleted, and load the flags of
        // countries chosen by hand.
        cx.observe(&Connections::global(cx), {
            let entity = entity.downgrade();
            move |connections, cx| {
                let profiles = connections.read(cx).profiles(cx);
                let ids: HashSet<ProfileId> = profiles.iter().map(|profile| profile.id).collect();
                let chosen: HashSet<String> = profiles
                    .iter()
                    .filter_map(|profile| profile.country.clone())
                    .collect();
                let _ = entity.update(cx, |this, cx| {
                    for code in &chosen {
                        this.ensure_flag(code, cx);
                    }
                    this.known.update(|known| known.retain_profiles(&ids), cx);
                });
            }
        })
        .detach();
        cx.set_global(GlobalServerFacts(entity));
        Ok(())
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalServerFacts>().0.clone()
    }

    /// The system detected on `id`'s server.
    pub fn os(&self, id: ProfileId, cx: &App) -> Option<&'static Os> {
        self.known.read(cx).os(id).and_then(os::find)
    }

    /// The country observed on this profile's server.
    pub fn country<'a>(&'a self, id: ProfileId, cx: &'a App) -> Option<&'a str> {
        self.known.read(cx).country(id)
    }

    /// The flag of `code`, once it has loaded.
    pub fn flag(&self, code: &str) -> Option<Arc<Image>> {
        self.flags.get(code).cloned().flatten()
    }

    /// Finds out what is not yet known about `profile`'s server through its
    /// connected session.
    pub(crate) fn probe(
        &mut self,
        profile: &Profile,
        host: &str,
        fs: Arc<dyn RemoteFs>,
        cx: &mut Context<Self>,
    ) {
        self.probed.retain(|seen| seen.strong_count() > 0);
        if self
            .probed
            .iter()
            .filter_map(Weak::upgrade)
            .any(|seen| Arc::ptr_eq(&seen, &fs))
        {
            return;
        }
        self.probed.push(Arc::downgrade(&fs));
        let id = profile.id;

        // The system is looked at on every new session: it is one small read,
        // and catches a reinstalled server. A chosen icon needs no detection.
        if profile.icon.is_none() {
            let detection = cx.background_spawn(self.observations.os().detect(fs));
            cx.spawn(async move |this, cx| {
                let os = match detection.await {
                    Ok(Some(os)) => os,
                    Ok(None) => return,
                    Err(error) => {
                        tracing::debug!(%error, "could not observe remote operating system");
                        return;
                    }
                };
                let _ = this.update(cx, |this, cx| {
                    this.known.update(|known| known.set_os(id, os), cx);
                    cx.notify();
                });
            })
            .detach();
        }

        let located = self.known.read(cx).located_host(id) == Some(host);
        if self.observations.network_enabled()
            && profile.country.is_none()
            && country_detection_enabled(cx)
            && !located
            && self.locating.insert(id)
        {
            let request = LocationRequest::new(
                host,
                self.known.read(cx).geoip().clone(),
                self.observations.clone(),
            );
            let lookup = cx.background_spawn(async move { request.locate() });
            cx.spawn(async move |this, cx| {
                let result = lookup.await;
                let _ = this.update(cx, |this, cx| {
                    this.locating.remove(&id);
                    let (host, found) = match result {
                        Ok(found) => found,
                        Err(error) => {
                            // Tried again on the next connection.
                            tracing::debug!(%error, "could not locate server");
                            return;
                        }
                    };
                    let country = found.as_ref().map(|(_, code)| code.as_str().to_owned());
                    if let Some(code) = &country {
                        this.ensure_flag(code, cx);
                    }
                    this.known.update(
                        |known| {
                            known.accept_country(id, host, found);
                        },
                        cx,
                    );
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// The icon shown for `profile`: the system chosen by hand, else the
    /// detected one, in the chosen or brand colour. `None`: a generic server.
    pub fn icon(&self, profile: &Profile, cx: &App) -> Option<Arc<Image>> {
        let os = profile
            .icon
            .as_deref()
            .and_then(os::find)
            .or_else(|| self.os(profile.id, cx))?;
        let color = profile
            .icon_color
            .as_deref()
            .filter(|color| os::is_valid_color(color))
            .unwrap_or(os.color);
        Some(Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            os::svg(os, color),
        )))
    }

    /// The country shown for `profile`: the one chosen by hand, else the
    /// detected one while detection is on.
    pub fn shown_country<'a>(&'a self, profile: &'a Profile, cx: &'a App) -> Option<&'a str> {
        profile.country.as_deref().or_else(|| {
            country_detection_enabled(cx)
                .then(|| self.country(profile.id, cx))
                .flatten()
        })
    }

    /// Loads the flag of `code` unless it is loaded or loading: from memory,
    /// then disk, then the network, then the few drawn in.
    pub(crate) fn ensure_flag(&mut self, code: &str, cx: &mut Context<Self>) {
        if !geo::is_country_code(code) || self.flags.contains_key(code) {
            return;
        }
        self.flags.insert(code.to_owned(), None);
        let code = code.to_owned();
        let observations = self.observations.clone();
        let load = cx.background_spawn({
            let code = code.clone();
            async move { load_flag(&code, &observations) }
        });
        cx.spawn(async move |this, cx| {
            let Some(bytes) = load.await else {
                return;
            };
            let image = Arc::new(Image::from_bytes(ImageFormat::Svg, bytes));
            let _ = this.update(cx, |this, cx| {
                this.flags.insert(code, Some(image));
                cx.notify();
            });
        })
        .detach();
    }
}

/// Whether the user lets nocterm look up server countries. Without settings
/// (tests) it is on; nothing goes over the network there anyway.
pub(crate) fn country_detection_enabled(cx: &App) -> bool {
    !cx.has_global::<nocterm_ui::SettingsStore>()
        || nocterm_ui::SettingsExt::setting::<nocterm_ui::AppearanceSettings>(cx)
            .detect_server_country
}

/// Probes the connected sessions of saved connections that are open in
/// `workspace`. Each session is probed once, however often this runs.
pub(crate) fn probe_sessions(workspace: &Workspace, cx: &mut App) {
    let connections = Connections::global(cx);
    let found: Vec<_> = workspace
        .items()
        .filter_map(|item| {
            let session = item.session(cx)?;
            if !session.connected {
                return None;
            }
            let fs = session.fs?;
            let id = item.terminal_capabilities(cx)?.state().info(cx)?.profile?;
            let profile = connections
                .read(cx)
                .profiles(cx)
                .iter()
                .find(|profile| profile.id.to_string() == *id)?
                .clone();
            Some((profile, session.target.host, fs))
        })
        .collect();
    if found.is_empty() {
        return;
    }
    ServerFacts::global(cx).update(cx, |facts, cx| {
        for (profile, host, fs) in found {
            facts.probe(&profile, &host, fs, cx);
        }
    });
}

fn load_flag(code: &str, observations: &ObservationDeps) -> Option<Vec<u8>> {
    let checked = CountryCode::new(code).ok()?;
    observations
        .flags()
        .load(checked)
        .inspect_err(|error| tracing::debug!(%error, code, "could not load flag"))
        .ok()
        .flatten()
        .or_else(|| geo::builtin_flag(code))
}

#[cfg(test)]
mod observation_tests;

#[cfg(test)]
mod tests {
    use nocterm_connections_model::{os::OsId, test_support::ScriptedOsDetector};
    use nocterm_session::testing::InMemoryFs;

    fn observations(detector: Arc<ScriptedOsDetector>) -> ObservationDeps {
        let offline = Arc::new(nocterm_connections_model::observation::OfflineObservations);
        ObservationDeps::new(offline.clone(), offline.clone(), offline, false, detector)
    }

    use super::*;

    #[gpui_kit::test]
    async fn sessions_are_probed_once_and_chosen_icons_skip_detection(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let detector = Arc::new(ScriptedOsDetector::default());
        detector.answer_next(Ok(OsId::new("debian")));
        let facts = cx.new(|_| ServerFacts::in_memory(observations(detector.clone())));
        let profile = Profile {
            description: String::new(),
            options: Default::default(),
            id: nocterm_connections_model::test_support::profile_id_for(
                "crates/connections/src/facts.rs:3",
            ),
            name: "web".into(),
            target: nocterm_session::Target::new("root", "web", 22),
            auth: Default::default(),
            group: None,
            credential: None,
            launch: None,
            icon: None,
            icon_color: None,
            country: None,
        };
        let fs: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
        let detected = |facts: &Entity<ServerFacts>, cx: &mut gpui_kit::TestAppContext| {
            facts.read_with(cx, |facts, cx| facts.os(profile.id, cx).map(|os| os.id))
        };
        facts.update(cx, |facts, cx| facts.probe(&profile, "web", fs.clone(), cx));
        cx.run_until_parked();
        assert_eq!(detected(&facts, cx), Some("debian"));
        assert_eq!(detector.calls(), 1);

        facts.update(cx, |facts, cx| facts.probe(&profile, "web", fs.clone(), cx));
        cx.run_until_parked();
        assert_eq!(detector.calls(), 1);

        let mut chosen = profile.clone();
        chosen.icon = Some("arch".into());
        let other: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
        facts.update(cx, |facts, cx| facts.probe(&chosen, "web", other, cx));
        cx.run_until_parked();
        assert_eq!(detector.calls(), 1);
        assert_eq!(detected(&facts, cx), Some("debian"));
        detector.answer_next(Err(
            nocterm_connections_model::os_observation::OsObservationError::Home(
                nocterm_session::FsError::NotConnected,
            ),
        ));
        let failed: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
        facts.update(cx, |facts, cx| facts.probe(&profile, "web", failed, cx));
        cx.run_until_parked();
        assert_eq!(detector.calls(), 2);
        assert_eq!(detected(&facts, cx), Some("debian"));
        detector.answer_next(Ok(None));
        let absent: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
        facts.update(cx, |facts, cx| facts.probe(&profile, "web", absent, cx));
        cx.run_until_parked();
        assert_eq!(detector.calls(), 3);
        assert_eq!(detected(&facts, cx), Some("debian"));
    }

    #[gpui_kit::test]
    async fn visible_observations_retry_at_quit_after_a_normal_write_failure(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let profile = Profile {
            description: String::new(),
            options: Default::default(),
            id: nocterm_connections_model::test_support::profile_id_for(
                "crates/connections/src/facts.rs:4",
            ),
            name: "web".into(),
            target: nocterm_session::Target::new("root", "web", 22),
            auth: Default::default(),
            group: None,
            credential: None,
            launch: None,
            icon: None,
            icon_color: None,
            country: Some("de".into()),
        };
        let mut profiles = nocterm_connections_model::profiles::Profiles::default();
        profiles.upsert(profile.clone());
        use nocterm_connections_model::{
            repository::{
                ConnectionDocument, ConnectionStorageError, LoadedConnections, StorageOperation,
            },
            test_support::InMemoryConnectionRepository,
        };
        let repository = Arc::new(InMemoryConnectionRepository::new(LoadedConnections {
            profiles,
            ..Default::default()
        }));
        repository.fail_next(ConnectionStorageError::new(
            ConnectionDocument::Facts,
            StorageOperation::Write,
            std::io::Error::from(std::io::ErrorKind::StorageFull),
        ));
        let shutdown = ShutdownCoordinator::default();
        let detector = Arc::new(ScriptedOsDetector::default());
        detector.answer_next(Ok(OsId::new("debian")));
        let prepared =
            crate::test_support::prepared_repository(repository.clone(), observations(detector));
        cx.update(|cx| {
            crate::init(prepared, &shutdown, cx).unwrap();
        });
        let facts = cx.read(ServerFacts::global);
        let fs: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
        facts.update(cx, |facts, cx| facts.probe(&profile, "web", fs, cx));
        cx.run_until_parked();
        assert_eq!(
            facts.read_with(cx, |facts, cx| facts.os(profile.id, cx).map(|os| os.id)),
            Some("debian")
        );
        let work = cx.update(|cx| shutdown.capture_all(cx));
        let report = cx.background_executor.spawn(work).await;
        assert!(report.failures().is_empty());
        assert_eq!(repository.snapshot().known.os(profile.id), Some("debian"));
    }
}
