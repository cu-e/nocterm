//! Host key verification against known-hosts files.

use std::path::{Path, PathBuf};

use russh::keys::{
    Algorithm, PublicKey,
    known_hosts::{known_host_keys_path, learn_known_hosts_path},
};

/// What the known-hosts files say about a key a host presented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The key is on record for this host.
    Known,
    /// Nothing is on record for this host and this kind of key.
    Unknown,
    /// A different key of the same kind is on record.
    Changed { known_hosts: PathBuf, line: usize },
}

/// The known-hosts files of one transport.
#[derive(Clone, Debug)]
pub(crate) struct HostKeys {
    writable: PathBuf,
    read_only: Vec<PathBuf>,
}

impl HostKeys {
    pub(crate) fn new(writable: PathBuf, read_only: Vec<PathBuf>) -> Self {
        Self {
            writable,
            read_only,
        }
    }

    pub(crate) fn check(&self, host: &str, port: u16, key: &PublicKey) -> Verdict {
        let recorded = self.recorded(host, port);
        if recorded
            .iter()
            .any(|entry| entry.key.key_data() == key.key_data())
        {
            return Verdict::Known;
        }

        recorded
            .into_iter()
            .find(|entry| same_kind(&entry.key.algorithm(), &key.algorithm()))
            .map_or(Verdict::Unknown, |entry| Verdict::Changed {
                known_hosts: entry.file,
                line: entry.line,
            })
    }

    /// The kinds of key on record for a host.
    ///
    /// Asking the host for one of these first avoids a spurious "unknown
    /// key" prompt when it also has keys of other kinds.
    pub(crate) fn known_algorithms(&self, host: &str, port: u16) -> Vec<Algorithm> {
        self.recorded(host, port)
            .into_iter()
            .map(|entry| entry.key.algorithm())
            .collect()
    }

    /// Puts a key on record in nocterm's own known-hosts file.
    pub(crate) fn remember(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), String> {
        learn_known_hosts_path(host, port, key, &self.writable).map_err(|error| error.to_string())
    }

    fn recorded(&self, host: &str, port: u16) -> Vec<Recorded> {
        self.files()
            .flat_map(|file| match known_host_keys_path(host, port, file) {
                Ok(keys) => {
                    let lines = LineNumbers::of(file);
                    keys.into_iter()
                        .map(|(line, key)| Recorded {
                            file: file.to_path_buf(),
                            line: lines.actual(line),
                            key,
                        })
                        .collect()
                }
                Err(error) => {
                    tracing::warn!(file = %file.display(), %error, "unreadable known-hosts file");
                    Vec::new()
                }
            })
            .collect()
    }

    fn files(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.writable.as_path()).chain(self.read_only.iter().map(PathBuf::as_path))
    }
}

struct Recorded {
    file: PathBuf,
    line: usize,
    key: PublicKey,
}

/// Translates russh's known-hosts line numbers, which skip comment lines,
/// into the numbers an editor shows.
struct LineNumbers {
    /// The editor line number of each line russh counts.
    counted: Vec<usize>,
}

impl LineNumbers {
    fn of(file: &Path) -> Self {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let counted = text
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.starts_with('#'))
            .map(|(index, _)| index + 1)
            .collect();
        Self { counted }
    }

    fn actual(&self, counted: usize) -> usize {
        counted
            .checked_sub(1)
            .and_then(|index| self.counted.get(index).copied())
            .unwrap_or(counted)
    }
}

/// Whether two algorithms name the same kind of key. RSA is one kind
/// whatever hash signs with it.
pub(crate) fn same_kind(left: &Algorithm, right: &Algorithm) -> bool {
    matches!(
        (left, right),
        (Algorithm::Rsa { .. }, Algorithm::Rsa { .. })
    ) || left == right
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBSxX+Bd/OnfEhnzshFKaUSI8HShuznJQy8eDQiiA1TL";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOF7Xj+BmEOw0ifztT8akl//320j5D5Y+IWls0EkOjcy";

    fn key(text: &str) -> PublicKey {
        PublicKey::from_openssh(text).expect("a valid public key")
    }

    fn host_keys(dir: &Path) -> HostKeys {
        HostKeys::new(
            dir.join("known_hosts"),
            vec![dir.join("system_known_hosts")],
        )
    }

    #[test]
    fn a_host_without_a_record_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = host_keys(dir.path());

        assert_eq!(
            hosts.check("example.com", 22, &key(KEY_A)),
            Verdict::Unknown
        );
        assert!(hosts.known_algorithms("example.com", 22).is_empty());
    }

    #[test]
    fn a_remembered_key_is_known_for_that_host_and_port_only() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = host_keys(dir.path());

        hosts.remember("example.com", 2222, &key(KEY_A)).unwrap();

        assert_eq!(
            hosts.check("example.com", 2222, &key(KEY_A)),
            Verdict::Known
        );
        assert_eq!(
            hosts.check("example.com", 22, &key(KEY_A)),
            Verdict::Unknown
        );
        assert_eq!(
            hosts.check("other.com", 2222, &key(KEY_A)),
            Verdict::Unknown
        );
        assert_eq!(
            hosts.known_algorithms("example.com", 2222),
            vec![Algorithm::Ed25519]
        );
    }

    #[test]
    fn a_different_key_of_the_same_kind_is_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = host_keys(dir.path());
        let system = dir.path().join("system_known_hosts");
        fs::write(&system, format!("# comment\nexample.com {KEY_A}\n")).unwrap();

        assert_eq!(hosts.check("example.com", 22, &key(KEY_A)), Verdict::Known);
        assert_eq!(
            hosts.check("example.com", 22, &key(KEY_B)),
            Verdict::Changed {
                known_hosts: system,
                line: 2,
            }
        );
    }

    #[test]
    fn a_match_in_any_file_outweighs_a_stale_record_in_another() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = host_keys(dir.path());
        fs::write(
            dir.path().join("system_known_hosts"),
            format!("example.com {KEY_A}\n"),
        )
        .unwrap();
        hosts.remember("example.com", 22, &key(KEY_B)).unwrap();

        assert_eq!(hosts.check("example.com", 22, &key(KEY_B)), Verdict::Known);
    }
}
