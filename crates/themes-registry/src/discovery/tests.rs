use super::*;
use nocterm_themes::Appearance;
use std::error::Error as _;
#[test]
fn precedence_duplicates_broken_files_and_appearances() {
    let root = tempfile::tempdir().unwrap();
    let dirs = ThemeDirs {
        user: root.path().join("user"),
        installed: root.path().join("installed"),
    };
    fs::create_dir(&dirs.user).unwrap();
    fs::write(dirs.user.join("user.json"),r#"{"name":"User","themes":[{"name":"Same","appearance":"light","style":{}},{"name":"Nocterm Default","appearance":"dark","style":{}}]}"#).unwrap();
    fs::write(dirs.user.join("broken.json"), "bad").unwrap();
    for id in ["a", "b"] {
        let path = dirs.installed.join(id);
        fs::create_dir_all(path.join("themes")).unwrap();
        fs::write(
            path.join("extension.toml"),
            format!("id = '{id}'\nname = 'Pack'\nversion = '1'\n"),
        )
        .unwrap();
        fs::write(path.join("themes/a.json"),r#"{"themes":[{"name":"Same","appearance":"dark","style":{}},{"name":"Other","appearance":"dark","style":{}}]}"#).unwrap();
    }
    let catalog = crate::discovery::scan(&dirs);
    assert_eq!(catalog.entries().len(), 2);
    assert_eq!(catalog.packs().len(), 2);
    assert_eq!(catalog.problems().len(), 5);
    assert!(matches!(
        catalog.find("Same", Appearance::Light).unwrap().source,
        ThemeSource::User(_)
    ));
    assert!(
        matches!(&catalog.find("Other",Appearance::Dark).unwrap().source,ThemeSource::Installed{id,..} if id=="a")
    );
    assert!(catalog.find("Same", Appearance::Dark).is_none());
}

#[test]
fn missing_libraries_are_empty_and_bad_roots_are_nonfatal_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let dirs = ThemeDirs {
        user: root.path().join("user"),
        installed: root.path().join("installed"),
    };
    assert!(scan(&dirs).problems().is_empty());
    fs::write(&dirs.user, b"not a directory").unwrap();
    let catalog = scan(&dirs);
    assert_eq!(catalog.problems().len(), 1);
    assert!(catalog.entries().is_empty());
    assert!(catalog.problems()[0].error.source().is_some());
}
#[test]
fn oversized_originals_are_rejected_with_a_typed_problem() {
    let root = tempfile::tempdir().unwrap();
    let dirs = ThemeDirs {
        user: root.path().join("user"),
        installed: root.path().join("installed"),
    };
    fs::create_dir(&dirs.user).unwrap();
    fs::write(dirs.user.join("large.json"), vec![b' '; FILE_LIMIT + 1]).unwrap();
    let catalog = scan(&dirs);
    assert_eq!(catalog.problems().len(), 1);
    assert!(catalog.problems()[0].error.to_string().contains("8 MiB"));
}
#[cfg(unix)]
#[test]
fn linked_originals_and_linked_pack_directories_are_not_followed() {
    let root = tempfile::tempdir().unwrap();
    let dirs = ThemeDirs {
        user: root.path().join("user"),
        installed: root.path().join("installed"),
    };
    fs::create_dir(&dirs.user).unwrap();
    fs::create_dir(&dirs.installed).unwrap();
    fs::write(
        root.path().join("original.json"),
        br#"{"themes":[{"name":"Hidden","appearance":"dark","style":{}}]}"#,
    )
    .unwrap();
    std::os::unix::fs::symlink(
        root.path().join("original.json"),
        dirs.user.join("link.json"),
    )
    .unwrap();
    std::os::unix::fs::symlink(root.path(), dirs.installed.join("pack")).unwrap();
    let catalog = scan(&dirs);
    assert!(catalog.entries().is_empty());
    assert!(catalog.packs().is_empty());
}
