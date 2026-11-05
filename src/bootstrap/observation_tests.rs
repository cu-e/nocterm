//! Connection observations are read from the supplied cache only.
use super::*;

#[gpui_kit::test]
fn configured_observations_use_supplied_disk_cache(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(directory.path());
    let id = nocterm_connections_model::test_support::profile_id_for("src/bootstrap.rs:1");
    std::fs::create_dir_all(paths.state_dir().join("flags")).unwrap();
    std::fs::write(
        paths.state_dir().join("servers.toml"),
        format!("[server.\"{id}\"]\ncountry = \"xx\"\n"),
    )
    .unwrap();
    std::fs::write(paths.state_dir().join("flags/xx.svg"), b"<svg/>").unwrap();
    let prepared = crate::connection_setup::native(&paths);
    let installed = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            DesignTokens::builtin(),
            SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        );
        nocterm_connections::install(
            prepared,
            &ready,
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap()
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| installed.flag("xx", cx).is_some()));
}

#[gpui_kit::test]
fn memory_observations_ignore_even_an_existing_native_cache(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(directory.path());
    let id = nocterm_connections_model::test_support::profile_id_for("src/bootstrap.rs:2");
    std::fs::create_dir_all(paths.state_dir().join("flags")).unwrap();
    std::fs::write(
        paths.state_dir().join("servers.toml"),
        format!("[server.\"{id}\"]\ncountry = \"xx\"\n"),
    )
    .unwrap();
    std::fs::write(paths.state_dir().join("flags/xx.svg"), b"<svg/>").unwrap();
    let installed = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            DesignTokens::builtin(),
            SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        );
        // The observations are deliberately composed with no configured
        // paths, although old observations are present in the fixture.
        nocterm_connections::install(
            crate::connection_setup::memory(),
            &ready,
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap()
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| installed.flag("xx", cx).is_none()));
}
