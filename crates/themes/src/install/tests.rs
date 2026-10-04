use super::*;
use flate2::{Compression, write::GzEncoder};
fn info() -> ExtensionInfo {
    serde_json::from_str(r#"{"id":"test","name":"Test","version":"1"}"#).unwrap()
}
fn theme(name: &str) -> Vec<u8> {
    format!(r#"{{"themes":[{{"name":"{name}","appearance":"dark","style":{{}}}}]}}"#).into_bytes()
}
fn archive(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (path, data) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, data.as_slice())
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}
#[test]
fn installs_replaces_and_failure_preserves_old_pack() {
    let dir = tempfile::tempdir().unwrap();
    install_archive(
        dir.path(),
        &info(),
        &archive(&[("./themes/a.json", theme("A"))]),
    )
    .unwrap();
    assert!(dir.path().join("test/extension.toml").is_file());
    install_archive(
        dir.path(),
        &info(),
        &archive(&[("themes/b.json", theme("B"))]),
    )
    .unwrap();
    assert!(!dir.path().join("test/themes/a.json").exists());
    assert!(
        install_archive(
            dir.path(),
            &info(),
            &archive(&[("themes/broken.json", b"bad".to_vec())])
        )
        .is_err()
    );
    assert!(dir.path().join("test/themes/b.json").is_file());
    std::fs::create_dir(dir.path().join("other")).unwrap();
    uninstall(dir.path(), "test").unwrap();
    assert!(dir.path().join("other").exists());
}
#[test]
fn irrelevant_and_nested_files_are_ignored_and_empty_is_error() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = archive(&[
        ("themes/a/b.json", theme("A")),
        ("themes/x.txt", theme("A")),
        ("icons/x.json", theme("A")),
    ]);
    assert!(install_archive(dir.path(), &info(), &bytes).is_err());
    assert!(!dir.path().join("test").exists());
    for path in [
        "../x.json",
        "/themes/x.json",
        "themes/a/b.json",
        "icons/x.json",
        "themes/a.txt",
    ] {
        assert!(theme_filename(Path::new(path)).is_none());
    }
    for id in ["", "../x", "A", "a/b", "-a", &"a".repeat(65)] {
        assert!(!valid_id(id));
        assert!(uninstall(dir.path(), id).is_err());
    }
}
#[test]
fn caps_files_and_the_entire_decompressed_stream() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        install_archive(
            dir.path(),
            &info(),
            &archive(&[("themes/big.json", vec![b' '; FILE_LIMIT + 1])])
        )
        .is_err()
    );
    assert!(
        install_archive(
            dir.path(),
            &info(),
            &archive(&[("icons/bomb", vec![0; DECOMPRESSED_LIMIT as usize + 1])])
        )
        .is_err()
    );
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    use std::io::Write as _;
    let mut tar_bytes = tar::Builder::new(Vec::new());
    let data = theme("A");
    let mut h = tar::Header::new_gnu();
    h.set_size(data.len() as u64);
    h.set_mode(0o644);
    h.set_cksum();
    tar_bytes
        .append_data(&mut h, "themes/a.json", data.as_slice())
        .unwrap();
    encoder.write_all(&tar_bytes.into_inner().unwrap()).unwrap();
    encoder
        .write_all(&vec![0; DECOMPRESSED_LIMIT as usize])
        .unwrap();
    assert!(install_archive(dir.path(), &info(), &encoder.finish().unwrap()).is_err());
}
#[cfg(unix)]
#[test]
fn links_and_link_destinations_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for kind in [tar::EntryType::Symlink, tar::EntryType::Link] {
        let mut h = tar::Header::new_gnu();
        h.set_size(0);
        h.set_mode(0o644);
        h.set_entry_type(kind);
        h.set_link_name("/tmp/escape").unwrap();
        h.set_cksum();
        builder
            .append_data(&mut h, "themes/evil.json", io::empty())
            .unwrap();
    }
    assert!(
        install_archive(
            dir.path(),
            &info(),
            &builder.into_inner().unwrap().finish().unwrap()
        )
        .is_err()
    );
    let target = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(target.path(), dir.path().join("test")).unwrap();
    assert!(
        install_archive(
            dir.path(),
            &info(),
            &archive(&[("themes/a.json", theme("A"))])
        )
        .is_err()
    );
    assert!(uninstall(dir.path(), "test").is_err());
    assert!(target.path().exists());
}

#[test]
fn rejects_non_normal_raw_paths_and_enforces_entry_and_file_counts() {
    for path in [
        "themes/./x.json",
        "themes//x.json",
        "././themes/x.json",
        "themes/../x.json",
    ] {
        assert!(theme_filename(Path::new(path)).is_none(), "{path}");
    }
    let directory = tempfile::tempdir().unwrap();
    let files: Vec<_> = (0..257)
        .map(|i| (format!("themes/{i}.json"), theme("A")))
        .collect();
    let borrowed: Vec<_> = files.iter().map(|(p, b)| (p.as_str(), b.clone())).collect();
    assert!(
        install_archive(directory.path(), &info(), &archive(&borrowed))
            .unwrap_err()
            .to_string()
            .contains("256")
    );
    let files: Vec<_> = (0..10001)
        .map(|i| (format!("ignored/{i}"), vec![]))
        .collect();
    let borrowed: Vec<_> = files.iter().map(|(p, b)| (p.as_str(), b.clone())).collect();
    assert!(
        install_archive(directory.path(), &info(), &archive(&borrowed))
            .unwrap_err()
            .to_string()
            .contains("10000")
    );
}
#[test]
fn concatenated_gzip_members_are_bounded() {
    use std::io::Write as _;
    let directory = tempfile::tempdir().unwrap();
    let mut bytes = archive(&[("themes/a.json", theme("A"))]);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(&vec![0; DECOMPRESSED_LIMIT as usize])
        .unwrap();
    bytes.extend(encoder.finish().unwrap());
    assert!(
        install_archive(directory.path(), &info(), &bytes)
            .unwrap_err()
            .to_string()
            .contains("64 MiB")
    );
    assert!(!directory.path().join("test").exists());
}
#[cfg(unix)]
#[test]
fn managed_root_symlink_never_deletes_external_files() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(outside.path().join("test")).unwrap();
    let link = root.path().join("installed");
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    assert!(uninstall(&link, "test").is_err());
    assert!(install_archive(&link, &info(), &archive(&[("themes/a.json", theme("A"))])).is_err());
    assert!(outside.path().join("test").exists());
}

#[test]
fn hostile_entries_are_skipped_while_valid_pack_installs() {
    let root = tempfile::tempdir().unwrap();
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for path in [
        "../escape.json",
        "/themes/absolute.json",
        "themes/a/b.json",
        "themes/./dot.json",
        "themes//slash.json",
        "icons/a.json",
        "themes/a.txt",
    ] {
        let data = theme("Hostile");
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        // Set raw name bytes: tar::Builder rejects traversal for callers creating
        // archives, but the installer must handle bytes sent by an attacker.
        h.as_mut_bytes()[..100].fill(0);
        h.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
        h.set_cksum();
        builder.append(&h, data.as_slice()).unwrap();
    }
    for kind in [
        tar::EntryType::Symlink,
        tar::EntryType::Link,
        tar::EntryType::Char,
        tar::EntryType::Block,
    ] {
        let mut h = tar::Header::new_gnu();
        h.set_size(0);
        h.set_mode(0o644);
        h.set_entry_type(kind);
        h.set_path("themes/link.json").unwrap();
        h.set_link_name("../escape.json").unwrap();
        h.set_cksum();
        builder.append(&h, io::empty()).unwrap();
    }
    let data = theme("Good");
    let mut h = tar::Header::new_gnu();
    h.set_size(data.len() as u64);
    h.set_mode(0o644);
    h.set_cksum();
    builder
        .append_data(&mut h, "themes/good.json", data.as_slice())
        .unwrap();
    install_archive(
        root.path(),
        &info(),
        &builder.into_inner().unwrap().finish().unwrap(),
    )
    .unwrap();
    let files: Vec<_> = fs::read_dir(root.path().join("test/themes"))
        .unwrap()
        .map(|f| f.unwrap().file_name())
        .collect();
    assert_eq!(files, [std::ffi::OsString::from("good.json")]);
    assert!(!root.path().join("escape.json").exists());
}
