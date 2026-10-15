use gpui_kit::{KeyBinding, TestAppContext};

use super::*;

gpui_kit::actions!(test, [First, Second, Library]);

const DEFAULTS: &str = r#"
[[section]]
context = "Workspace"
[section.bindings]
"ctrl-e" = "test::First"
"ctrl-j" = "test::Second"
"#;

fn bound(keymap: &KeymapHandle, cx: &App, keys: &str) -> Vec<String> {
    keymap
        .bindings(cx)
        .into_iter()
        .filter(|binding| binding.keystrokes == keys)
        .map(|binding| binding.action)
        .collect()
}

#[gpui_kit::test]
fn rebinding_saves_only_the_change_and_reset_restores_the_default(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("keymap.toml");
    let keymap = cx.update(|cx| {
        cx.bind_keys([KeyBinding::new("ctrl-a", Library, None)]);
        KeymapHandle::install(DEFAULTS, Some(file.clone()), cx)
    });
    cx.update(|cx| {
        keymap
            .bind(
                "test::First",
                Some("Workspace".into()),
                Some("ctrl-e"),
                "alt-e",
                cx,
            )
            .unwrap();
        assert!(bound(&keymap, cx, "ctrl-e").is_empty());
        assert_eq!(bound(&keymap, cx, "alt-e"), ["test::First"]);
        assert!(keymap.is_customized("test::First", cx));
        assert!(!keymap.is_customized("test::Second", cx));
        assert_eq!(
            keymap.conflicts("test::First", &Some("Workspace".into()), "ctrl-j", cx),
            ["test::Second"]
        );
        // The library's own bindings survive every reload.
        let keymap = cx.key_bindings();
        assert!(
            keymap
                .borrow()
                .bindings()
                .any(|binding| binding.action().name() == "test::Library")
        );
    });
    let saved: KeymapFile = toml::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let workspace = Some("Workspace".to_owned());
    assert_eq!(saved.action_for(&workspace, "ctrl-e"), Some(UNBOUND));
    assert_eq!(saved.action_for(&workspace, "alt-e"), Some("test::First"));
    assert_eq!(saved.action_for(&workspace, "ctrl-j"), None);

    cx.update(|cx| {
        keymap.reset("test::First", cx).unwrap();
        assert_eq!(bound(&keymap, cx, "ctrl-e"), ["test::First"]);
        assert!(bound(&keymap, cx, "alt-e").is_empty());
        assert!(!keymap.is_customized("test::First", cx));
        // A user file is read back on the next start.
        keymap
            .unbind("test::Second", Some("Workspace".into()), "ctrl-j", cx)
            .unwrap();
        let keymap = KeymapHandle::install(DEFAULTS, Some(file.clone()), cx);
        assert!(bound(&keymap, cx, "ctrl-j").is_empty());
    });
}

#[gpui_kit::test]
fn a_malformed_user_file_keeps_the_defaults_and_says_why(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("keymap.toml");
    std::fs::write(&file, "this is not toml [").unwrap();
    cx.update(|cx| {
        let keymap = KeymapHandle::install(DEFAULTS, Some(file), cx);
        assert!(keymap.error(cx).is_some());
        assert_eq!(bound(&keymap, cx, "ctrl-e"), ["test::First"]);
    });
}

#[gpui_kit::test]
fn invalid_bind_returns_a_native_error_without_changing_installed_defaults(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        let keymap = KeymapHandle::install(DEFAULTS, None, cx);
        let error = keymap
            .bind(
                "test::First",
                Some("Workspace".into()),
                Some("ctrl-e"),
                "invalid-key",
                cx,
            )
            .unwrap_err();
        assert!(matches!(
            &error,
            KeymapError::InvalidStroke { index: 0, .. }
        ));
        assert!(
            std::error::Error::source(&error)
                .unwrap()
                .is::<gpui_kit::InvalidKeystrokeError>()
        );
        assert_eq!(bound(&keymap, cx, "ctrl-e"), ["test::First"]);
        assert!(bound(&keymap, cx, "invalid-key").is_empty());
        assert!(!keymap.is_customized("test::First", cx));
    });
}

fn assert_native_failure(
    keymap: &KeymapHandle,
    file: &std::path::Path,
    error: &KeymapError,
    cx: &mut App,
) {
    let KeymapError::Persistence(shared) = error else {
        panic!("expected persistence error")
    };
    let nocterm_core::persist::PersistError::Io { path, source } = shared.as_ref() else {
        panic!("expected actual file-publication failure")
    };
    assert_eq!(path, file);
    let returned = std::error::Error::source(error)
        .unwrap()
        .downcast_ref::<nocterm_core::persist::PersistError>()
        .unwrap();
    assert!(std::ptr::eq(returned, shared.as_ref()));
    assert!(std::ptr::eq(returned, keymap.error(cx).unwrap().as_ref()));
    let native = std::error::Error::source(returned).unwrap();
    assert!(std::ptr::eq(
        native.downcast_ref::<std::io::Error>().unwrap(),
        source
    ));
    let remembered = keymap.error(cx).unwrap().clone();
    assert!(
        keymap
            .bind(
                "test::First",
                Some("Workspace".into()),
                None,
                "invalid-key",
                cx
            )
            .is_err()
    );
    assert!(std::ptr::eq(
        remembered.as_ref(),
        keymap.error(cx).unwrap().as_ref()
    ));
    assert!(
        std::error::Error::source(error)
            .unwrap()
            .is::<nocterm_core::persist::PersistError>()
    );
    let has_os_cause =
        source.raw_os_error().is_some() || std::error::Error::source(source).is_some();
    assert!(has_os_cause);
}

#[gpui_kit::test]
fn failed_save_keeps_the_desired_binding_installed_and_retry_persists_it(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("keymap.toml");
    // A directory at the destination deterministically refuses atomic file publication.
    std::fs::create_dir(&file).unwrap();
    let keymap = cx.update(|cx| {
        cx.bind_keys([KeyBinding::new("ctrl-a", Library, None)]);
        KeymapHandle::install(DEFAULTS, Some(file.clone()), cx)
    });
    cx.update(|cx| {
        let error = keymap
            .bind(
                "test::First",
                Some("Workspace".into()),
                Some("ctrl-e"),
                "alt-e",
                cx,
            )
            .unwrap_err();
        assert_native_failure(&keymap, &file, &error, cx);
        assert!(bound(&keymap, cx, "ctrl-e").is_empty());
        assert_eq!(bound(&keymap, cx, "alt-e"), ["test::First"]);
        assert!(keymap.error(cx).is_some());
        assert!(keymap.is_customized("test::First", cx));
        let installed = cx.key_bindings();
        let installed = installed.borrow();
        assert!(installed.bindings().any(|binding| {
            binding.action().name() == "test::First"
                && binding
                    .keystrokes()
                    .iter()
                    .map(|key| key.unparse())
                    .collect::<Vec<_>>()
                    == ["alt-e"]
        }));
        assert!(
            installed
                .bindings()
                .any(|binding| binding.action().name() == "test::Library")
        );
    });
    assert!(
        file.is_dir(),
        "failed save replaced the blocking destination"
    );
    std::fs::remove_dir(&file).unwrap();
    cx.update(|cx| {
        keymap
            .bind(
                "test::First",
                Some("Workspace".into()),
                Some("ctrl-e"),
                "alt-e",
                cx,
            )
            .unwrap();
        assert!(keymap.error(cx).is_none());
        assert_eq!(bound(&keymap, cx, "alt-e"), ["test::First"]);
    });
    let saved = nocterm_core::persist::load::<KeymapFile>(&file)
        .unwrap()
        .unwrap();
    let workspace = Some("Workspace".to_owned());
    assert_eq!(saved.action_for(&workspace, "ctrl-e"), Some(UNBOUND));
    assert_eq!(saved.action_for(&workspace, "alt-e"), Some("test::First"));
    cx.update(|cx| {
        let keymap = KeymapHandle::install(DEFAULTS, Some(file), cx);
        assert_eq!(bound(&keymap, cx, "alt-e"), ["test::First"]);
        assert!(bound(&keymap, cx, "ctrl-e").is_empty());
    });
}

#[gpui_kit::test]
fn shared_owner_notifies_after_installation_and_keeps_toolkit_metadata(cx: &mut TestAppContext) {
    use std::{cell::Cell, rc::Rc};
    let keymap = cx.update(|cx| {
        cx.bind_keys([KeyBinding::new("ctrl-a", Library, None).with_meta(KeyBindingMetaIndex(99))]);
        KeymapHandle::install(DEFAULTS, None, cx)
    });
    let clone = keymap.clone();
    let notifications = Rc::new(Cell::new(0));
    let count = notifications.clone();
    let observed = clone.clone();
    let _subscription = cx.update(|cx| {
        keymap.observe(cx, move |cx| {
            count.set(count.get() + 1);
            assert!(observed.is_customized("test::First", cx));
            let keys = cx.key_bindings();
            let keys = keys.borrow();
            assert!(
                keys.bindings()
                    .any(|b| b.meta() == Some(USER_META) && b.action().name() == "test::First")
            );
            assert!(
                keys.bindings()
                    .any(|b| b.meta() == Some(DEFAULT_META) && b.action().name() == "test::Second")
            );
            assert!(
                keys.bindings()
                    .any(|b| b.meta() == Some(KeyBindingMetaIndex(99))
                        && b.action().name() == "test::Library")
            );
        })
    });
    cx.update(|cx| {
        clone
            .bind(
                "test::First",
                Some("Workspace".into()),
                Some("ctrl-e"),
                "alt-e",
                cx,
            )
            .unwrap()
    });
    cx.run_until_parked();
    assert_eq!(notifications.get(), 1);
    cx.update(|cx| {
        assert_eq!(bound(&keymap, cx, "alt-e"), ["test::First"]);
        assert!(
            clone
                .bind("test::First", None, None, "invalid-key", cx)
                .is_err()
        );
    });
    cx.run_until_parked();
    assert_eq!(
        notifications.get(),
        1,
        "invalid input is not an accepted transition"
    );
}

#[test]
fn transparent_persistence_keeps_even_a_source_less_native_error() {
    use std::error::Error as _;
    let shared = nocterm_core::SharedError::from_error(nocterm_core::persist::PersistError::Io {
        path: "keymap.toml".into(),
        source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    });
    let error = KeymapError::Persistence(shared.clone());
    let source = error
        .source()
        .unwrap()
        .downcast_ref::<nocterm_core::persist::PersistError>()
        .unwrap();
    assert!(std::ptr::eq(source, shared.as_ref()));
    let native = source
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(native.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(native.source().is_none());
}
