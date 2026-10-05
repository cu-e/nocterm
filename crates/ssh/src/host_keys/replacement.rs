//! Conservative replacement of conflicting records owned by the application.
use super::*;
use russh::keys::HashAlg;
use std::{
    fs,
    io::{self, Write as _},
    path::Path,
};

#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
    stamp: Option<Stamp>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    modified: Option<std::time::SystemTime>,
    readonly: bool,
    #[cfg(unix)]
    identity: (u64, u64, u32),
}
impl Stamp {
    fn new(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;
        Self {
            modified: metadata.modified().ok(),
            readonly: metadata.permissions().readonly(),
            #[cfg(unix)]
            identity: (metadata.dev(), metadata.ino(), metadata.mode()),
        }
    }
}
impl Snapshot {
    pub(super) fn missing(path: PathBuf) -> Self {
        Self {
            path,
            bytes: None,
            stamp: None,
        }
    }
    pub(super) fn capture(path: &Path, bytes: Vec<u8>) -> io::Result<Self> {
        Ok(Self {
            path: path.to_owned(),
            bytes: Some(bytes),
            stamp: Some(Stamp::new(&fs::metadata(path)?)),
        })
    }
    fn unchanged(&self) -> bool {
        match crate::file::read(&self.path, MAX_STORE) {
            Ok(bytes) => {
                self.bytes.as_deref() == Some(bytes.as_slice())
                    && fs::metadata(&self.path)
                        .is_ok_and(|metadata| Some(Stamp::new(&metadata)) == self.stamp)
            }
            Err(error) => error.kind() == io::ErrorKind::NotFound && self.bytes.is_none(),
        }
    }
}
impl HostKeys {
    pub(crate) fn old_fingerprints(&self, host: &str, port: u16, key: &PublicKey) -> Vec<String> {
        let mut fingerprints: Vec<_> = self
            .conflicting(host, port, key)
            .map(|record| {
                record
                    .entry
                    .public_key()
                    .fingerprint(HashAlg::Sha256)
                    .to_string()
            })
            .collect();
        fingerprints.sort();
        fingerprints.dedup();
        fingerprints
    }
    fn conflicting<'a>(
        &'a self,
        host: &str,
        port: u16,
        key: &PublicKey,
    ) -> impl Iterator<Item = &'a Recorded> {
        let algorithm = key.algorithm();
        self.matching(host, port).filter(move |record| {
            record.entry.marker().is_none()
                && same_kind(&record.entry.public_key().algorithm(), &algorithm)
        })
    }
    pub(crate) fn replacement_error(
        &self,
        host: &str,
        port: u16,
        key: &PublicKey,
    ) -> Option<String> {
        let label = if port == 22 {
            host.to_owned()
        } else {
            format!("[{host}]:{port}")
        };
        let records: Vec<_> = self.conflicting(host, port, key).collect();
        if records.is_empty() {
            return Some("There are no conflicting records to replace.".into());
        }
        if records.iter().any(|record| record.file != self.writable) {
            return Some("A conflicting key is in a read-only SSH trust file. Update that file yourself after verifying the new fingerprint, or connect once.".into());
        }
        if records
            .iter()
            .any(|record| match record.entry.host_patterns() {
                HostPatterns::HashedName { .. } => false,
                HostPatterns::Patterns(patterns) => {
                    patterns.len() != 1
                        || !patterns[0].eq_ignore_ascii_case(&label)
                        || patterns[0].contains(['*', '?', '!'])
                }
            })
        {
            return Some("A conflicting record covers aliases or a wildcard. Update it yourself to avoid changing trust for other hosts, or connect once.".into());
        }
        #[cfg(unix)]
        if self.writable.parent().is_none_or(|parent| {
            rustix::fs::access(
                parent,
                rustix::fs::Access::WRITE_OK | rustix::fs::Access::EXEC_OK,
            )
            .is_err()
        }) {
            return Some("The application's SSH trust directory is not writable. Check its permissions, or connect once.".into());
        }
        if fs::symlink_metadata(&self.writable).is_err()
            || fs::symlink_metadata(&self.writable)
                .is_ok_and(|metadata| !metadata.is_file() || metadata.permissions().readonly())
            || fs::OpenOptions::new()
                .write(true)
                .open(&self.writable)
                .is_err()
        {
            return Some("The application's SSH trust file cannot be safely replaced. Check its permissions, or connect once.".into());
        }
        None
    }
    pub(crate) fn revalidate(&self) -> Result<(), String> {
        if self.snapshots.iter().all(Snapshot::unchanged) {
            Ok(())
        } else {
            Err("SSH trust records changed while awaiting confirmation. Reconnect and verify the key again.".into())
        }
    }
    pub(crate) fn replace(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), String> {
        let _guard = MUTATIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.revalidate()?;
        if let Some(error) = self.replacement_error(host, port, key) {
            return Err(error);
        }
        // Reparse the current stores as a second fail-closed check before writing.
        let current = Self::load(self.writable.clone(), self.read_only.clone())
            .map_err(|error| error.to_string())?;
        if !matches!(current.check(host, port, key), Verdict::Changed { .. }) {
            return Err(
                "The host key conflict changed. Reconnect and verify the key again.".into(),
            );
        }
        let lines: Vec<_> = self
            .conflicting(host, port, key)
            .map(|record| record.line)
            .collect();
        let snapshot = self
            .snapshots
            .iter()
            .find(|snapshot| snapshot.path == self.writable)
            .and_then(|snapshot| snapshot.bytes.as_ref())
            .ok_or("SSH trust file disappeared")?;
        let text = std::str::from_utf8(snapshot).map_err(|error| error.to_string())?;
        let key = key.to_openssh().map_err(|error| error.to_string())?;
        let mut output = String::new();
        for (index, raw) in text.split_inclusive('\n').enumerate() {
            if lines.contains(&(index + 1)) {
                // Retain the exact host pattern (including hashes), whitespace,
                // comments and line ending, replacing only algorithm and key data.
                output.push_str(&replace_fields(raw, &key)?);
            } else {
                output.push_str(raw);
            }
        }
        self.atomic_write(output.as_bytes())
            .map_err(|error| format!("Could not save the new host key: {error}"))
    }
    fn atomic_write(&self, bytes: &[u8]) -> io::Result<()> {
        let parent = self
            .writable
            .parent()
            .ok_or_else(|| io::Error::other("trust file has no parent"))?;
        let temporary = parent.join(format!(".nocterm-host-key-{}", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.set_permissions(fs::metadata(&self.writable)?.permissions())?;
            file.write_all(bytes)?;
            file.sync_all()?;
            self.revalidate().map_err(io::Error::other)?;
            fs::rename(&temporary, &self.writable)?;
            // Publication committed; match core persistence's best-effort
            // directory sync rather than reporting an already-saved key as failed.
            if let Ok(directory) = fs::File::open(parent) {
                let _ = directory.sync_all();
            }
            Ok(())
        })();
        let _ = fs::remove_file(&temporary);
        result
    }
}
fn replace_fields(raw: &str, key: &str) -> Result<String, String> {
    let start = raw
        .find(|c: char| !c.is_whitespace())
        .ok_or("invalid host record")?;
    let host_end = raw[start..]
        .find(char::is_whitespace)
        .map(|offset| start + offset)
        .ok_or("invalid host record")?;
    let algorithm_start = raw[host_end..]
        .find(|c: char| !c.is_whitespace())
        .map(|offset| host_end + offset)
        .ok_or("invalid host record")?;
    let mut split = raw[algorithm_start..].split_whitespace();
    let algorithm = split.next().ok_or("invalid host record")?;
    let data = split.next().ok_or("invalid host record")?;
    let data_start = algorithm_start
        + algorithm.len()
        + raw[algorithm_start + algorithm.len()..]
            .find(|c: char| !c.is_whitespace())
            .ok_or("invalid host record")?;
    let (new_algorithm, new_data) = key.split_once(' ').ok_or("invalid public key")?;
    Ok(format!(
        "{}{}{}{}{}",
        &raw[..algorithm_start],
        new_algorithm,
        &raw[algorithm_start + algorithm.len()..data_start],
        new_data,
        &raw[data_start + data.len()..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBSxX+Bd/OnfEhnzshFKaUSI8HShuznJQy8eDQiiA1TL";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOF7Xj+BmEOw0ifztT8akl//320j5D5Y+IWls0EkOjcy";
    fn key(value: &str) -> PublicKey {
        PublicKey::from_openssh(value).unwrap()
    }
    fn load(path: &Path) -> HostKeys {
        HostKeys::load(path.to_owned(), vec![]).unwrap()
    }

    #[test]
    fn replaces_all_stale_exact_records_and_preserves_unrelated_bytes_ports_and_mode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        let before = format!(
            "# unchanged\r\nexample.com\t{KEY_A} old comment\r\n\n[example.com]:2222 {KEY_A}\nother.com {KEY_A}\nexample.com {KEY_A}"
        );
        fs::write(&path, &before).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        }
        let hosts = load(&path);
        assert!(
            hosts
                .replacement_error("example.com", 22, &key(KEY_B))
                .is_none()
        );
        hosts.replace("example.com", 22, &key(KEY_B)).unwrap();
        let expected = before.replacen(
            &format!("example.com\t{KEY_A}"),
            &format!("example.com\t{KEY_B}"),
            1,
        );
        let expected = expected.replace(
            &format!("\nexample.com {KEY_A}"),
            &format!("\nexample.com {KEY_B}"),
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        let current = load(&path);
        assert_eq!(
            current.check("example.com", 22, &key(KEY_B)),
            Verdict::Known
        );
        assert!(matches!(
            current.check("example.com", 22, &key(KEY_A)),
            Verdict::Changed { .. }
        ));
        assert_eq!(
            current.check("example.com", 2222, &key(KEY_A)),
            Verdict::Known
        );
        assert_eq!(current.check("other.com", 22, &key(KEY_A)), Verdict::Known);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
    }
    #[test]
    fn hashed_conflicts_are_replaced_without_disclosing_host_or_leaving_old_trust() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        let pattern = "|1|YWFhYWFhYWFhYWFhYWFhYWFhYWE=|lMugv7ii5/R8LZHkQxdKDhSoGRY=";
        fs::write(&path, format!("{pattern} {KEY_A}\nexample.com {KEY_A}\n")).unwrap();
        load(&path).replace("example.com", 22, &key(KEY_B)).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("{pattern} {KEY_B}\nexample.com {KEY_B}\n")
        );
        assert_eq!(
            load(&path).check("example.com", 22, &key(KEY_B)),
            Verdict::Known
        );
        assert!(matches!(
            load(&path).check("example.com", 22, &key(KEY_A)),
            Verdict::Changed { .. }
        ));
    }
    #[test]
    fn wildcard_alias_and_mixed_stores_refuse_persistent_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        for pattern in [
            "*.com",
            "example.com,alias",
            "example.com,!other.com",
            "ex?mple.com",
        ] {
            let text = format!("example.com {KEY_A}\n{pattern} {KEY_A}\n");
            fs::write(&path, &text).unwrap();
            let hosts = load(&path);
            assert!(
                hosts
                    .replacement_error("example.com", 22, &key(KEY_B))
                    .unwrap()
                    .contains("aliases or a wildcard")
            );
            assert!(hosts.replace("example.com", 22, &key(KEY_B)).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
        fs::write(&path, format!("example.com {KEY_A}\n")).unwrap();
        let readonly = directory.path().join("external");
        fs::write(&readonly, format!("example.com {KEY_A}\n")).unwrap();
        let hosts = HostKeys::load(path.clone(), vec![readonly]).unwrap();
        assert!(
            hosts
                .replacement_error("example.com", 22, &key(KEY_B))
                .unwrap()
                .contains("read-only")
        );
        assert!(hosts.replace("example.com", 22, &key(KEY_B)).is_err());
    }
    #[test]
    fn changed_or_new_optional_store_cannot_be_overwritten_after_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        fs::write(&path, format!("example.com {KEY_A}\n")).unwrap();
        let external = directory.path().join("external");
        let hosts = HostKeys::load(path.clone(), vec![external.clone()]).unwrap();
        fs::write(&external, format!("@revoked example.com {KEY_B}\n")).unwrap();
        assert!(hosts.revalidate().is_err());
        assert!(
            hosts
                .replace("example.com", 22, &key(KEY_B))
                .unwrap_err()
                .contains("changed while awaiting")
        );
        assert!(!fs::read_to_string(&path).unwrap().contains(KEY_B));
        let hosts = load(&path);
        fs::write(&path, format!("example.com {KEY_A}\n# new data\n")).unwrap();
        assert!(hosts.replace("example.com", 22, &key(KEY_B)).is_err());
        assert!(fs::read_to_string(&path).unwrap().ends_with("# new data\n"));
    }
    #[cfg(unix)]
    #[test]
    fn symlink_and_readonly_app_file_cannot_be_replaced() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let directory = tempfile::tempdir().unwrap();
        let actual = directory.path().join("actual");
        let path = directory.path().join("known_hosts");
        fs::write(&actual, format!("example.com {KEY_A}\n")).unwrap();
        symlink(&actual, &path).unwrap();
        let hosts = load(&path);
        assert!(
            hosts
                .replacement_error("example.com", 22, &key(KEY_B))
                .is_some()
        );
        assert!(hosts.replace("example.com", 22, &key(KEY_B)).is_err());
        fs::remove_file(&path).unwrap();
        fs::rename(&actual, &path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(
            load(&path)
                .replacement_error("example.com", 22, &key(KEY_B))
                .is_some()
        );
    }
    #[cfg(unix)]
    #[test]
    fn save_failure_is_reported_and_keeps_existing_bytes() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        let text = format!("example.com {KEY_A}\n");
        fs::write(&path, &text).unwrap();
        let hosts = load(&path);
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let result = hosts.replace("example.com", 22, &key(KEY_B));
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            result
                .unwrap_err()
                .contains("trust directory is not writable")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
