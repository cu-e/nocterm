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

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Weak},
};

use crate::{ObservationDeps, observation::LocationRequest};
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Image, ImageFormat};
use nocterm_connections_model::observation::CountryCode;
use nocterm_core::{Paths, persist};
use nocterm_session::RemoteFs;
use nocterm_store::{ShutdownCoordinator, StoreError};
use nocterm_workspace::Workspace;

mod persistence;
use persistence::FactsStore;
use serde::{Deserialize, Serialize};

use crate::{
    Connections, geo,
    os::{self, Os},
    store::{Profile, ProfileId},
};

const MAX_OS_RELEASE_BYTES: usize = 16 * 1024;
/// Files that only one system has, for systems without `os-release`.
/// Appliances built on another system, which their `os-release` names
/// instead: a file only the appliance has, the appliance, and the systems it
/// can be built on (empty: no `os-release` at all).
const PLATFORMS: [(&str, &str, &[&str]); 9] = [
    ("/usr/bin/pveversion", "proxmox", &["debian"]),
    ("/usr/bin/midclt", "truenas", &["debian", "freebsd", ""]),
    ("/etc/openmediavault", "openmediavault", &["debian"]),
    ("/opt/vyatta", "vyos", &["debian"]),
    ("/etc/unraid-version", "unraid", &["slackware", ""]),
    ("/usr/local/opnsense", "opnsense", &["freebsd", ""]),
    ("/etc/platform", "pfsense", &["freebsd", ""]),
    ("/etc/synoinfo.conf", "synology", &["linux", ""]),
    ("/etc/config/uLinux.conf", "qnap", &["linux", ""]),
];
const MARKERS: [(&str, &str); 4] = [
    ("/System/Library/CoreServices/SystemVersion.plist", "macos"),
    ("/bin/freebsd-version", "freebsd"),
    ("/bsd", "openbsd"),
    ("/netbsd", "netbsd"),
];

/// What was found out about one server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Detected {
    /// A [`crate::os`] id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    os: Option<String>,
    /// A lower-case ISO 3166-1 alpha-2 code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    /// The host the country belongs to; a changed host is located again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    located: Option<String>,
}

/// The contents of `servers.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Known {
    #[serde(default, rename = "server")]
    servers: HashMap<ProfileId, Detected>,
    /// Countries of public addresses, shared by servers on the same address.
    #[serde(default)]
    geoip: BTreeMap<String, String>,
}

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
    pub fn load(paths: &Paths, observations: ObservationDeps) -> Self {
        let file = paths.state_dir().join("servers.toml");
        let known = persist::load(&file)
            .unwrap_or_else(|error| {
                // Only observations: they are found out again.
                tracing::warn!(%error, "could not read detected server details");
                None
            })
            .unwrap_or_default();
        Self {
            known: FactsStore::new(known, Some(file)),
            ..Self::in_memory(observations)
        }
    }

    pub fn in_memory(observations: ObservationDeps) -> Self {
        Self {
            observations,
            known: FactsStore::new(Known::default(), None),
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
        let countries: HashSet<String> = self
            .known
            .read(cx)
            .servers
            .values()
            .filter_map(|detected| detected.country.clone())
            .collect();
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
                    this.known
                        .update(|known| known.servers.retain(|id, _| ids.contains(id)), cx);
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
        self.known
            .read(cx)
            .servers
            .get(&id)?
            .os
            .as_deref()
            .and_then(os::find)
    }

    /// The country `id`'s server is in.
    pub fn country<'a>(&'a self, id: ProfileId, cx: &'a App) -> Option<&'a str> {
        self.known.read(cx).servers.get(&id)?.country.as_deref()
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
            let detection = cx.background_spawn(detect_os(fs));
            cx.spawn(async move |this, cx| {
                let Some(os) = detection.await else {
                    return;
                };
                let _ = this.update(cx, |this, cx| {
                    this.known.update(
                        |known| {
                            let detected = known.servers.entry(id).or_default();
                            detected.os = Some(os.id.to_owned());
                        },
                        cx,
                    );
                    cx.notify();
                });
            })
            .detach();
        }

        let located = self
            .known
            .read(cx)
            .servers
            .get(&id)
            .is_some_and(|detected| detected.located.as_deref() == Some(host));
        if self.observations.network_enabled()
            && profile.country.is_none()
            && country_detection_enabled(cx)
            && !located
            && self.locating.insert(id)
        {
            let request = LocationRequest::new(
                host,
                self.known.read(cx).geoip.clone(),
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
                    let country = found.as_ref().map(|(_, code)| code.clone());
                    if let Some(code) = &country {
                        this.ensure_flag(code, cx);
                    }
                    this.known.update(
                        |known| {
                            if let Some((ip, code)) = found {
                                known.geoip.insert(ip, code);
                            }
                            let detected = known.servers.entry(id).or_default();
                            detected.country = country;
                            detected.located = Some(host);
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

async fn detect_os(fs: Arc<dyn RemoteFs>) -> Option<&'static Os> {
    let mut base = None;
    // `/etc/os-release` is usually a link, which downloads refuse; its
    // target is the second choice.
    for path in ["/etc/os-release", "/usr/lib/os-release"] {
        if let Some(text) = read_small(fs.as_ref(), path).await
            && let Some(os) = os::from_os_release(&text)
        {
            base = Some(os);
            break;
        }
    }
    let base_id = base.map_or("", |os| os.id);
    for (path, id, bases) in PLATFORMS {
        if bases.contains(&base_id) && matches!(fs.stat(path).await, Ok(Some(_))) {
            return os::find(id);
        }
    }
    if base.is_some() {
        return base;
    }
    for (path, id) in MARKERS {
        if matches!(fs.stat(path).await, Ok(Some(_))) {
            return os::find(id);
        }
    }
    // Windows' OpenSSH shows drives as `/C:/Users/…`.
    let home = fs.home().await.ok()?;
    let home = home.strip_prefix('/').unwrap_or(&home).as_bytes();
    (home.len() >= 2 && home[0].is_ascii_alphabetic() && home[1] == b':')
        .then(|| os::find("windows"))
        .flatten()
}

/// The text of a small remote file; `None` if it is missing, large or binary.
async fn read_small(fs: &dyn RemoteFs, path: &str) -> Option<String> {
    let mut file = fs.download(path).await.ok()?;
    let mut bytes = Vec::new();
    loop {
        let chunk = file
            .read(MAX_OS_RELEASE_BYTES + 1 - bytes.len())
            .await
            .ok()?;
        if chunk.is_empty() {
            break;
        }
        bytes.extend(chunk);
        if bytes.len() > MAX_OS_RELEASE_BYTES {
            return None;
        }
    }
    let _ = file.close().await;
    String::from_utf8(bytes).ok()
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nocterm_session::{DirEntry, FsError, FsFuture, fs::RemoteDownload};

    use super::*;

    #[test]
    fn known_servers_round_trip() {
        let mut known = Known::default();
        known.servers.insert(
            ProfileId::generate(),
            Detected {
                os: Some("ubuntu".into()),
                country: Some("de".into()),
                located: Some("example.com".into()),
            },
        );
        known
            .servers
            .insert(ProfileId::generate(), Detected::default());
        known.geoip.insert("2a01:4f8::1".into(), "de".into());
        known.geoip.insert("1.1.1.1".into(), "au".into());

        let text = toml::to_string(&known).unwrap();

        assert_eq!(toml::from_str::<Known>(&text).unwrap(), known);
        assert_eq!(toml::from_str::<Known>("").unwrap(), Known::default());
    }

    #[gpui_kit::test]
    async fn sessions_are_probed_once_and_chosen_icons_skip_detection(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let facts = cx.new(|_| ServerFacts::in_memory(ObservationDeps::offline()));
        let profile = Profile {
            description: String::new(),
            options: Default::default(),
            id: ProfileId::generate(),
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
        let downloads = Arc::new(AtomicUsize::new(0));
        let fs: Arc<dyn RemoteFs> = Arc::new(FakeFs("ID=debian\n", downloads.clone(), &[]));
        let detected = |facts: &Entity<ServerFacts>, cx: &mut gpui_kit::TestAppContext| {
            facts.read_with(cx, |facts, cx| facts.os(profile.id, cx).map(|os| os.id))
        };
        facts.update(cx, |facts, cx| facts.probe(&profile, "web", fs.clone(), cx));
        cx.run_until_parked();
        assert_eq!(detected(&facts, cx), Some("debian"));
        assert_eq!(downloads.load(Ordering::SeqCst), 1);

        facts.update(cx, |facts, cx| facts.probe(&profile, "web", fs.clone(), cx));
        cx.run_until_parked();
        assert_eq!(downloads.load(Ordering::SeqCst), 1);

        let mut chosen = profile.clone();
        chosen.icon = Some("arch".into());
        let other: Arc<dyn RemoteFs> = Arc::new(FakeFs("ID=ubuntu\n", downloads.clone(), &[]));
        facts.update(cx, |facts, cx| facts.probe(&chosen, "web", other, cx));
        cx.run_until_parked();
        assert_eq!(downloads.load(Ordering::SeqCst), 1);
        assert_eq!(detected(&facts, cx), Some("debian"));
    }

    #[gpui_kit::test]
    async fn visible_observations_retry_at_quit_after_a_normal_write_failure(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(directory.path());
        let profile = Profile {
            description: String::new(),
            options: Default::default(),
            id: ProfileId::generate(),
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
        let mut profiles = crate::store::Profiles::default();
        profiles.upsert(profile.clone());
        persist::save(&paths.connections_file(), &profiles).unwrap();
        std::fs::write(paths.state_dir(), "blocks informational writes").unwrap();
        let shutdown = ShutdownCoordinator::default();
        cx.update(|cx| {
            crate::init(Some(&paths), &shutdown, ObservationDeps::offline(), cx).unwrap()
        });
        let facts = cx.read(ServerFacts::global);
        let fs: Arc<dyn RemoteFs> = Arc::new(FakeFs("ID=debian\n", Default::default(), &[]));
        facts.update(cx, |facts, cx| facts.probe(&profile, "web", fs, cx));
        cx.run_until_parked();
        assert_eq!(
            facts.read_with(cx, |facts, cx| facts.os(profile.id, cx).map(|os| os.id)),
            Some("debian")
        );
        std::fs::remove_file(paths.state_dir()).unwrap();
        std::fs::create_dir(paths.state_dir()).unwrap();
        let work = cx.update(|cx| shutdown.capture_all(cx));
        let report = cx.background_executor.spawn(work).await;
        assert!(report.failures().is_empty());
        let saved: Known = persist::load(&paths.state_dir().join("servers.toml"))
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.servers.get(&profile.id).unwrap().os.as_deref(),
            Some("debian")
        );
    }

    #[test]
    fn appliances_are_told_apart_from_the_system_they_are_built_on() {
        let detect = |release: &'static str, files: &'static [&'static str]| {
            let fs: Arc<dyn RemoteFs> = Arc::new(FakeFs(release, Default::default(), files));
            futures::executor::block_on(detect_os(fs)).map(|os| os.id)
        };
        assert_eq!(
            detect("ID=debian\n", &["/usr/bin/pveversion"]),
            Some("proxmox")
        );
        assert_eq!(
            detect("ID=debian\n", &["/etc/openmediavault"]),
            Some("openmediavault")
        );
        assert_eq!(detect("ID=debian\n", &[]), Some("debian"));
        // A marker on an unrelated system is a coincidence.
        assert_eq!(
            detect("ID=ubuntu\n", &["/usr/bin/pveversion"]),
            Some("ubuntu")
        );
        assert_eq!(detect("", &["/usr/local/opnsense"]), Some("opnsense"));
        assert_eq!(detect("", &["/bin/freebsd-version"]), Some("freebsd"));
        assert_eq!(detect("", &[]), None);
    }

    /// Serves `/etc/os-release`, has the files in the third field, and counts downloads.
    struct FakeFs(&'static str, Arc<AtomicUsize>, &'static [&'static str]);

    impl RemoteFs for FakeFs {
        fn home(&self) -> FsFuture<String> {
            Box::pin(async { Ok("/root".to_owned()) })
        }
        fn read_dir(&self, _: &str) -> FsFuture<Vec<DirEntry>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn stat(&self, path: &str) -> FsFuture<Option<DirEntry>> {
            let entry = self.2.contains(&path).then(|| DirEntry {
                name: path.rsplit('/').next().unwrap_or_default().to_owned(),
                kind: nocterm_session::EntryKind::File,
                is_symlink: false,
                size: None,
            });
            Box::pin(async move { Ok(entry) })
        }
        fn download(&self, path: &str) -> FsFuture<Box<dyn RemoteDownload>> {
            self.1.fetch_add(1, Ordering::SeqCst);
            let found = (path == "/etc/os-release").then(|| self.0.as_bytes().to_vec());
            let path = path.to_owned();
            Box::pin(async move {
                found
                    .map(|bytes| Box::new(FakeDownload(Some(bytes))) as Box<dyn RemoteDownload>)
                    .ok_or(FsError::NotFound { path })
            })
        }
    }

    struct FakeDownload(Option<Vec<u8>>);

    impl RemoteDownload for FakeDownload {
        fn read(&mut self, _: usize) -> FsFuture<Vec<u8>> {
            let bytes = self.0.take().unwrap_or_default();
            Box::pin(async move { Ok(bytes) })
        }
        fn close(self: Box<Self>) -> FsFuture<()> {
            Box::pin(async { Ok(()) })
        }
    }
}
