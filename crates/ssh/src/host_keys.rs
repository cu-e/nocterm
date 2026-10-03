//! Parsed, bounded OpenSSH trust records. Unreadable stores never become TOFU.
use hmac::{Hmac, KeyInit as _, Mac as _};
use russh::keys::{
    Algorithm, PublicKey,
    known_hosts::learn_known_hosts_path,
    ssh_key::known_hosts::{Entry, HostPatterns, Marker},
};
use sha1::Sha1;
use std::path::PathBuf;

const MAX_STORE: usize = 4 * 1024 * 1024;
const MAX_LINE: usize = 64 * 1024;
const MAX_RECORDS: usize = 16_384;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Known,
    Unknown,
    Changed { known_hosts: PathBuf, line: usize },
    Revoked { known_hosts: PathBuf, line: usize },
}

#[derive(Debug)]
pub(crate) struct TrustError {
    file: PathBuf,
    line: Option<usize>,
    reason: String,
}
impl std::fmt::Display for TrustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot verify host keys using {}", self.file.display())?;
        if let Some(line) = self.line {
            write!(f, " (line {line})")?;
        }
        write!(f, ": {}", self.reason)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct HostKeys {
    writable: PathBuf,
    records: Vec<Recorded>,
}
#[derive(Clone, Debug)]
struct Recorded {
    file: PathBuf,
    line: usize,
    entry: Entry,
}
impl HostKeys {
    pub(crate) fn load(writable: PathBuf, read_only: Vec<PathBuf>) -> Result<Self, TrustError> {
        let mut records = Vec::new();
        for file in std::iter::once(&writable).chain(read_only.iter()) {
            let bytes = match crate::file::read(file, MAX_STORE) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(TrustError {
                        file: file.clone(),
                        line: None,
                        reason: error.to_string(),
                    });
                }
            };
            let text = std::str::from_utf8(&bytes).map_err(|error| TrustError {
                file: file.clone(),
                line: None,
                reason: error.to_string(),
            })?;
            for (index, line) in text.lines().enumerate() {
                let number = index + 1;
                let invalid = |reason: &str| TrustError {
                    file: file.clone(),
                    line: Some(number),
                    reason: reason.into(),
                };
                if line.len() > MAX_LINE {
                    return Err(invalid("host record exceeds the supported size limit"));
                }
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if records.len() == MAX_RECORDS {
                    return Err(invalid("too many host records"));
                }
                // The upstream entry parser expects ordinary single spaces. OpenSSH
                // also accepts tabs and repeated whitespace between these fields.
                let fields: Vec<_> = line.split_whitespace().collect();
                let key_start = usize::from(fields[0].starts_with('@'));
                if fields.len() < key_start + 3 {
                    return Err(invalid("invalid host record"));
                }
                let normalized = fields[..key_start + 3].join(" ");
                let entry: Entry = normalized
                    .parse()
                    .map_err(|_| invalid("invalid host record"))?;
                records.push(Recorded {
                    file: file.clone(),
                    line: number,
                    entry,
                });
            }
        }
        Ok(Self { writable, records })
    }
    pub(crate) fn check(&self, host: &str, port: u16, key: &PublicKey) -> Verdict {
        let recorded: Vec<_> = self.matching(host, port).collect();
        if let Some(record) = recorded.iter().find(|record| {
            record.entry.marker() == Some(&Marker::Revoked)
                && record.entry.public_key().key_data() == key.key_data()
        }) {
            return Verdict::Revoked {
                known_hosts: record.file.clone(),
                line: record.line,
            };
        }
        if recorded.iter().any(|record| {
            record.entry.marker().is_none()
                && record.entry.public_key().key_data() == key.key_data()
        }) {
            return Verdict::Known;
        }
        recorded
            .into_iter()
            .find(|record| {
                record.entry.marker().is_none()
                    && same_kind(&record.entry.public_key().algorithm(), &key.algorithm())
            })
            .map_or(Verdict::Unknown, |record| Verdict::Changed {
                known_hosts: record.file.clone(),
                line: record.line,
            })
    }
    pub(crate) fn known_algorithms(&self, host: &str, port: u16) -> Vec<Algorithm> {
        self.matching(host, port)
            .filter(|record| record.entry.marker().is_none())
            .map(|record| record.entry.public_key().algorithm())
            .collect()
    }
    pub(crate) fn remember(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), String> {
        learn_known_hosts_path(host, port, key, &self.writable).map_err(|error| error.to_string())
    }
    fn matching<'a>(&'a self, host: &str, port: u16) -> impl Iterator<Item = &'a Recorded> {
        let label = if port == 22 {
            host.to_owned()
        } else {
            format!("[{host}]:{port}")
        };
        self.records
            .iter()
            .filter(move |record| host_matches(record.entry.host_patterns(), &label))
    }
}
fn host_matches(patterns: &HostPatterns, host: &str) -> bool {
    match patterns {
        HostPatterns::HashedName { salt, hash } => Hmac::<Sha1>::new_from_slice(salt)
            .is_ok_and(|mac| mac.chain_update(host.as_bytes()).verify_slice(hash).is_ok()),
        HostPatterns::Patterns(patterns) => {
            let mut positive = false;
            for pattern in patterns {
                if let Some(negative) = pattern.strip_prefix('!') {
                    if glob(negative.as_bytes(), host.as_bytes()) {
                        return false;
                    }
                } else if glob(pattern.as_bytes(), host.as_bytes()) {
                    positive = true;
                }
            }
            positive
        }
    }
}
// Greedy wildcard matching uses constant memory. Input is capped by MAX_LINE;
// no regex backtracking or unbounded recursive parser is involved.
fn glob(pattern: &[u8], host: &[u8]) -> bool {
    let (mut p, mut h) = (0, 0);
    let (mut star, mut retry) = (None, 0);
    while h < host.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p].eq_ignore_ascii_case(&host[h])) {
            p += 1;
            h += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = h;
        } else if let Some(s) = star {
            retry += 1;
            h = retry;
            p = s + 1;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}
pub(crate) fn same_kind(left: &Algorithm, right: &Algorithm) -> bool {
    matches!(
        (left, right),
        (Algorithm::Rsa { .. }, Algorithm::Rsa { .. })
    ) || left == right
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};
    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBSxX+Bd/OnfEhnzshFKaUSI8HShuznJQy8eDQiiA1TL";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOF7Xj+BmEOw0ifztT8akl//320j5D5Y+IWls0EkOjcy";
    fn key(text: &str) -> PublicKey {
        PublicKey::from_openssh(text).unwrap()
    }
    fn load(dir: &Path) -> HostKeys {
        HostKeys::load(
            dir.join("known_hosts"),
            vec![dir.join("system_known_hosts")],
        )
        .unwrap()
    }
    #[test]
    fn missing_optional_stores_are_unknown_and_remember_round_trips_ports() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = load(dir.path());
        assert_eq!(
            hosts.check("example.com", 22, &key(KEY_A)),
            Verdict::Unknown
        );
        hosts.remember("example.com", 2222, &key(KEY_A)).unwrap();
        let hosts = load(dir.path());
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
    }
    #[test]
    fn changed_keys_report_actual_editor_line_and_positive_match_wins_stale_records() {
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("system_known_hosts");
        fs::write(&system, format!("# comment\n\nexample.com {KEY_A}\n")).unwrap();
        assert_eq!(
            load(dir.path()).check("example.com", 22, &key(KEY_B)),
            Verdict::Changed {
                known_hosts: system,
                line: 3
            }
        );
        load(dir.path())
            .remember("example.com", 22, &key(KEY_B))
            .unwrap();
        assert_eq!(
            load(dir.path()).check("example.com", 22, &key(KEY_B)),
            Verdict::Known
        );
    }
    #[test]
    fn revoked_key_always_overrides_matching_trusted_key_in_other_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("known_hosts"),
            format!("example.com {KEY_A}\n"),
        )
        .unwrap();
        let system = dir.path().join("system_known_hosts");
        fs::write(&system, format!("@revoked *.com,!safe.com {KEY_A}\n")).unwrap();
        assert_eq!(
            load(dir.path()).check("example.com", 22, &key(KEY_A)),
            Verdict::Revoked {
                known_hosts: system,
                line: 1
            }
        );
        assert_eq!(
            load(dir.path()).check("safe.com", 22, &key(KEY_A)),
            Verdict::Unknown
        );
    }
    #[test]
    fn aliases_wildcards_negations_tabs_and_ports_follow_openssh_patterns() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("known_hosts"),
            format!(
                "*.example.com,alias,!bad.example.com\t{KEY_A}\n[*.example.com]:2222 {KEY_B}\n"
            ),
        )
        .unwrap();
        let hosts = load(dir.path());
        for host in ["a.example.com", "A.EXAMPLE.COM", "alias"] {
            assert_eq!(hosts.check(host, 22, &key(KEY_A)), Verdict::Known);
        }
        assert_eq!(
            hosts.check("bad.example.com", 22, &key(KEY_A)),
            Verdict::Unknown
        );
        assert_eq!(
            hosts.check("a.example.com", 2222, &key(KEY_B)),
            Verdict::Known
        );
        assert!(glob(b"ab?*d", b"abczzzD"));
        assert!(!glob(b"ab?d", b"abd"));
    }
    #[test]
    fn hashed_hosts_revocations_and_ca_markers_are_not_plain_trust() {
        let dir = tempfile::tempdir().unwrap();
        // OpenSSH |1| fixtures generated independently with HMAC-SHA1;
        // this unit test needs no platform ssh-keygen executable.
        let hashed =
            format!("|1|YWFhYWFhYWFhYWFhYWFhYWFhYWE=|lMugv7ii5/R8LZHkQxdKDhSoGRY= {KEY_A}\n");
        fs::write(dir.path().join("known_hosts"), &hashed).unwrap();
        assert_eq!(
            load(dir.path()).check("example.com", 22, &key(KEY_A)),
            Verdict::Known
        );
        assert_eq!(
            load(dir.path()).check("other.com", 22, &key(KEY_A)),
            Verdict::Unknown
        );
        fs::write(dir.path().join("known_hosts"), format!("@revoked {hashed}")).unwrap();
        assert!(matches!(
            load(dir.path()).check("example.com", 22, &key(KEY_A)),
            Verdict::Revoked { .. }
        ));
        fs::write(
            dir.path().join("known_hosts"),
            format!("|1|YWFhYWFhYWFhYWFhYWFhYWFhYWE=|NvwHB6/fmVliUynzWUgT79ML35I= {KEY_A}\n"),
        )
        .unwrap();
        assert_eq!(
            load(dir.path()).check("example.com", 2222, &key(KEY_A)),
            Verdict::Known
        );
        assert_eq!(
            load(dir.path()).check("example.com", 22, &key(KEY_A)),
            Verdict::Unknown
        );
        fs::write(
            dir.path().join("known_hosts"),
            format!("@cert-authority example.com {KEY_A}\n"),
        )
        .unwrap();
        assert_eq!(
            load(dir.path()).check("example.com", 22, &key(KEY_A)),
            Verdict::Unknown
        );
    }
    #[test]
    fn damaged_nonregular_or_oversized_stores_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        for text in [
            "example.com ssh-ed25519 not-base64",
            "@unknown example.com key",
            "unparseable",
        ] {
            fs::write(&path, text).unwrap();
            assert!(
                HostKeys::load(path.clone(), vec![])
                    .unwrap_err()
                    .to_string()
                    .contains("cannot verify")
            );
        }
        fs::write(&path, vec![b'x'; MAX_LINE + 1]).unwrap();
        assert!(HostKeys::load(path.clone(), vec![]).is_err());
        fs::write(&path, vec![b'x'; MAX_STORE + 1]).unwrap();
        assert!(HostKeys::load(path.clone(), vec![]).is_err());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(HostKeys::load(path, vec![]).is_err());
    }
}
