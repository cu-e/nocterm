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
    path::PathBuf,
    sync::{Arc, Weak},
};

use gpui_kit::{App, AppContext as _, Context, Entity, Global, Image, ImageFormat, Task};
use nocterm_core::{Paths, persist};
use nocterm_session::RemoteFs;
use nocterm_workspace::Workspace;
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
    file: Option<PathBuf>,
    flags_dir: Option<PathBuf>,
    /// Whether GeoIP and flag requests may go over the network. Without a
    /// state directory (tests, `--no-config`) nothing is fetched.
    online: bool,
    known: Known,
    /// Absent: never asked for; `None`: loading, or no image exists.
    flags: HashMap<String, Option<Arc<Image>>>,
    /// Sessions already probed; each is probed once.
    probed: Vec<Weak<dyn RemoteFs>>,
    locating: HashSet<ProfileId>,
    saver: Option<Task<()>>,
    revision: u64,
    saved_revision: u64,
}

struct GlobalServerFacts(Entity<ServerFacts>);
impl Global for GlobalServerFacts {}

impl ServerFacts {
    pub fn load(paths: &Paths) -> Self {
        let file = paths.state_dir().join("servers.toml");
        let known = persist::load(&file)
            .unwrap_or_else(|error| {
                // Only observations: they are found out again.
                tracing::warn!(%error, "could not read detected server details");
                None
            })
            .unwrap_or_default();
        Self {
            file: Some(file),
            flags_dir: Some(paths.state_dir().join("flags")),
            online: true,
            known,
            ..Self::in_memory()
        }
    }

    pub fn in_memory() -> Self {
        Self {
            file: None,
            flags_dir: None,
            online: false,
            known: Known::default(),
            flags: HashMap::new(),
            probed: Vec::new(),
            locating: HashSet::new(),
            saver: None,
            revision: 0,
            saved_revision: 0,
        }
    }

    pub(crate) fn install(self, cx: &mut App) {
        let countries: HashSet<String> = self
            .known
            .servers
            .values()
            .filter_map(|detected| detected.country.clone())
            .collect();
        let entity = cx.new(|cx| {
            let mut this = self;
            for code in countries {
                this.ensure_flag(&code, cx);
            }
            this
        });
        // Forget servers whose connection was deleted, and load the flags of
        // countries chosen by hand.
        cx.observe(&Connections::global(cx), {
            let entity = entity.downgrade();
            move |connections, cx| {
                let profiles = connections.read(cx).profiles();
                let ids: HashSet<ProfileId> = profiles.iter().map(|profile| profile.id).collect();
                let chosen: HashSet<String> = profiles
                    .iter()
                    .filter_map(|profile| profile.country.clone())
                    .collect();
                let _ = entity.update(cx, |this, cx| {
                    for code in &chosen {
                        this.ensure_flag(code, cx);
                    }
                    let before = this.known.servers.len();
                    this.known.servers.retain(|id, _| ids.contains(id));
                    if this.known.servers.len() != before {
                        this.schedule_save(cx);
                    }
                });
            }
        })
        .detach();
        cx.set_global(GlobalServerFacts(entity));
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalServerFacts>().0.clone()
    }

    /// The system detected on `id`'s server.
    pub fn os(&self, id: ProfileId) -> Option<&'static Os> {
        self.known
            .servers
            .get(&id)?
            .os
            .as_deref()
            .and_then(os::find)
    }

    /// The country `id`'s server is in.
    pub fn country(&self, id: ProfileId) -> Option<&str> {
        self.known.servers.get(&id)?.country.as_deref()
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
                    let detected = this.known.servers.entry(id).or_default();
                    if detected.os.as_deref() != Some(os.id) {
                        detected.os = Some(os.id.to_owned());
                        this.schedule_save(cx);
                        cx.notify();
                    }
                });
            })
            .detach();
        }

        let located = self
            .known
            .servers
            .get(&id)
            .is_some_and(|detected| detected.located.as_deref() == Some(host));
        if self.online
            && profile.country.is_none()
            && country_detection_enabled(cx)
            && !located
            && self.locating.insert(id)
        {
            let host = host.to_owned();
            let cached = self.known.geoip.clone();
            let lookup = cx
                .background_spawn(async move { locate(&host, &cached).map(|found| (host, found)) });
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
                    if let Some((ip, code)) = &found {
                        this.known.geoip.insert(ip.clone(), code.clone());
                    }
                    let country = found.map(|(_, code)| code);
                    if let Some(code) = &country {
                        this.ensure_flag(code, cx);
                    }
                    let detected = this.known.servers.entry(id).or_default();
                    detected.country = country;
                    detected.located = Some(host);
                    this.schedule_save(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// The icon shown for `profile`: the system chosen by hand, else the
    /// detected one, in the chosen or brand colour. `None`: a generic server.
    pub fn icon(&self, profile: &Profile) -> Option<Arc<Image>> {
        let os = profile
            .icon
            .as_deref()
            .and_then(os::find)
            .or_else(|| self.os(profile.id))?;
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
    pub fn shown_country<'a>(&'a self, profile: &'a Profile, cx: &App) -> Option<&'a str> {
        profile.country.as_deref().or_else(|| {
            country_detection_enabled(cx)
                .then(|| self.country(profile.id))
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
        let path = self
            .flags_dir
            .as_ref()
            .map(|dir| dir.join(format!("{code}.svg")));
        let online = self.online;
        let load = cx.background_spawn({
            let code = code.clone();
            async move { load_flag(&code, path, online) }
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

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.revision = self.revision.wrapping_add(1);
        if self.file.is_none() || self.saver.is_some() {
            return;
        }
        self.saver = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok((path, known, revision)) = this.update(cx, |this, _| {
                    (this.file.clone(), this.known.clone(), this.revision)
                }) else {
                    return;
                };
                let result = cx
                    .background_spawn(async move {
                        path.map_or(Ok(()), |path| persist::save(&path, &known))
                    })
                    .await;
                if let Err(error) = result {
                    tracing::warn!(%error, "could not save detected server details");
                }
                let again = this.update(cx, |this, _| {
                    this.saved_revision = revision;
                    let again = this.revision != revision;
                    if !again {
                        this.saver = None;
                    }
                    again
                });
                if !matches!(again, Ok(true)) {
                    return;
                }
            }
        }));
    }
}

/// Whether the user lets nocterm look up server countries. Without settings
/// (tests) it is on; nothing goes over the network there anyway.
pub(crate) fn country_detection_enabled(cx: &App) -> bool {
    !cx.has_global::<nocterm_ui::SettingsStore>()
        || nocterm_ui::ActiveSettings::settings(cx)
            .appearance
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
            let id = item.terminal_access(cx)?.info(cx)?.profile?;
            let profile = connections
                .read(cx)
                .profiles()
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

/// The public address of `host` and its country. `Ok(None)` when the host is
/// private or the service does not know it; `Err` when asking failed.
fn locate(
    host: &str,
    cached: &BTreeMap<String, String>,
) -> Result<Option<(String, String)>, String> {
    let Some(ip) = geo::public_address(host) else {
        return Ok(None);
    };
    let key = ip.to_string();
    if let Some(code) = cached.get(&key) {
        return Ok(Some((key, code.clone())));
    }
    let code = geo::country_of(&geo::client()?, ip)?;
    Ok(code.map(|code| (key, code)))
}

fn load_flag(code: &str, path: Option<PathBuf>, online: bool) -> Option<Vec<u8>> {
    if let Some(path) = &path
        && let Ok(bytes) = std::fs::read(path)
        && geo::is_svg(&bytes)
    {
        return Some(bytes);
    }
    let downloaded = if online {
        geo::client()
            .and_then(|client| geo::download_flag(&client, code))
            .inspect_err(|error| tracing::debug!(%error, code, "could not download flag"))
            .ok()
    } else {
        None
    };
    match (downloaded, path) {
        (Some(bytes), Some(path)) => {
            let saved = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| persist_bytes(&path, &bytes));
            if let Err(error) = saved {
                tracing::warn!(%error, "could not cache flag");
            }
            Some(bytes)
        }
        (Some(bytes), None) => Some(bytes),
        (None, _) => geo::builtin_flag(code),
    }
}

/// Writes through a temporary file, so a cut-off write is never read back as
/// a flag.
fn persist_bytes(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("svg.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

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

    #[test]
    fn private_hosts_are_located_without_asking() {
        let cached = BTreeMap::new();
        assert_eq!(locate("192.168.1.5", &cached), Ok(None));
        assert_eq!(locate("pi.local", &cached), Ok(None));
    }

    #[test]
    fn cached_addresses_are_not_looked_up_again() {
        let cached = BTreeMap::from([("8.8.8.8".to_owned(), "us".to_owned())]);
        assert_eq!(
            locate("8.8.8.8", &cached),
            Ok(Some(("8.8.8.8".into(), "us".into())))
        );
    }

    #[test]
    fn flags_come_from_disk_before_the_network_and_fall_back_to_drawn_ones() {
        let dir = tempfile::tempdir().unwrap();
        let cached = dir.path().join("xx.svg");
        std::fs::write(&cached, b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
        assert_eq!(
            load_flag("xx", Some(cached), false).as_deref(),
            Some(&b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"[..])
        );

        let broken = dir.path().join("de.svg");
        std::fs::write(&broken, b"<html>").unwrap();
        assert_eq!(
            load_flag("de", Some(broken), false),
            geo::builtin_flag("de")
        );
        assert_eq!(load_flag("us", None, false), None);
    }

    #[gpui_kit::test]
    async fn sessions_are_probed_once_and_chosen_icons_skip_detection(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let facts = cx.new(|_| ServerFacts::in_memory());
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
            facts.read_with(cx, |facts, _| facts.os(profile.id).map(|os| os.id))
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
