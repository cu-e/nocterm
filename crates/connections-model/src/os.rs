//! Checked OS identity and release/marker policy, independent of presentation and I/O.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OsId(&'static str);
impl OsId {
    pub fn new(id: &str) -> Option<Self> {
        SUPPORTED_IDS
            .iter()
            .copied()
            .find(|known| *known == id)
            .map(Self)
    }
    pub fn as_str(self) -> &'static str {
        self.0
    }
}
pub const SUPPORTED_IDS: &[&str] = &[
    "ubuntu",
    "debian",
    "arch",
    "manjaro",
    "alpine",
    "fedora",
    "centos",
    "rocky",
    "almalinux",
    "rhel",
    "opensuse",
    "suse",
    "nixos",
    "gentoo",
    "kali",
    "linuxmint",
    "pop",
    "elementary",
    "void",
    "raspbian",
    "ol",
    "amzn",
    "slackware",
    "linux",
    "freebsd",
    "openbsd",
    "netbsd",
    "proxmox",
    "truenas",
    "openmediavault",
    "unraid",
    "synology",
    "qnap",
    "pfsense",
    "opnsense",
    "vyos",
    "openwrt",
    "haos",
    "docker",
    "kubernetes",
    "endeavouros",
    "garuda",
    "artix",
    "cachyos",
    "zorin",
    "neon",
    "kubuntu",
    "deepin",
    "devuan",
    "mx",
    "pureos",
    "parrot",
    "tails",
    "solus",
    "mageia",
    "pclinuxos",
    "nobara",
    "bazzite",
    "ultramarine",
    "fedora-coreos",
    "flatcar",
    "talos",
    "bottlerocket",
    "photon",
    "azurelinux",
    "cos",
    "wolfi",
    "clear-linux-os",
    "cloudlinux",
    "eurolinux",
    "scientific",
    "openeuler",
    "anolis",
    "kylin",
    "uos",
    "astra",
    "altlinux",
    "redos",
    "rosa",
    "guix",
    "chimera",
    "postmarketos",
    "steamos",
    "chromeos",
    "dragonfly",
    "omnios",
    "openindiana",
    "solaris",
    "macos",
    "windows",
];
/// The system an `os-release` file describes: its `ID` when the catalog knows
/// it, else the first `ID_LIKE` it knows, else plain Linux.
pub fn from_os_release(text: &str) -> Option<OsId> {
    let mut id = None;
    let mut like = None;
    let mut variant = None;
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
        match key.trim() {
            "ID" => id = Some(value),
            "ID_LIKE" => like = Some(value),
            "VARIANT_ID" => variant = Some(value),
            _ => {}
        }
    }
    let mut id = id?;
    if id == "fedora" && variant.as_deref() == Some("coreos") {
        id = "fedora-coreos".to_owned();
    }
    std::iter::once(id.as_str())
        .chain(like.iter().flat_map(|like| like.split_whitespace()))
        .find_map(by_release_id)
        .or_else(|| OsId::new("linux"))
}

/// Maps `os-release` IDs onto catalog entries; distributions with several IDs
/// share one entry.
fn by_release_id(id: &str) -> Option<OsId> {
    let id = match id {
        "opensuse-leap" | "opensuse-tumbleweed" | "opensuse-microos" | "opensuse-slowroll" => {
            "opensuse"
        }
        "sles" | "sled" | "sle-micro" => "suse",
        "archarm" | "arch32" => "arch",
        "manjaro-arm" => "manjaro",
        "redhat" | "rhel" | "rhcos" => "rhel",
        "centos-stream" => "centos",
        "raspberrypi" => "raspbian",
        "fedora-asahi-remix" => "fedora",
        "mariner" => "azurelinux",
        "alt" => "altlinux",
        "hassos" => "haos",
        "cachyos-linux" => "cachyos",
        "chromiumos" => "chromeos",
        id => id,
    };
    OsId::new(id)
}

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

/// Eligible appliance markers in detection priority order.
pub fn platform_markers(base: Option<OsId>) -> impl Iterator<Item = (&'static str, OsId)> {
    let base_id = base.map_or("", OsId::as_str);
    PLATFORMS
        .into_iter()
        .filter_map(move |(path, id, bases)| bases.contains(&base_id).then_some((path, OsId(id))))
}
/// Generic host markers in detection priority order.
pub fn host_markers() -> impl Iterator<Item = (&'static str, OsId)> {
    MARKERS.into_iter().map(|(path, id)| (path, OsId(id)))
}
/// Windows OpenSSH represents drive paths with an optional leading slash.
pub fn is_windows_home(home: &str) -> bool {
    let bytes = home.strip_prefix('/').unwrap_or(home).as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}
#[cfg(test)]
mod tests;
