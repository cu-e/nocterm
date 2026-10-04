use super::*;
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
    let catalog = ThemeCatalog::load(&dirs);
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
