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

fn bound(cx: &App, keys: &str) -> Vec<String> {
    Keymap::global(cx)
        .bindings()
        .into_iter()
        .filter(|binding| binding.keystrokes == keys)
        .map(|binding| binding.action)
        .collect()
}

#[gpui_kit::test]
fn rebinding_saves_only_the_change_and_reset_restores_the_default(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("keymap.toml");
    cx.update(|cx| {
        cx.bind_keys([KeyBinding::new("ctrl-a", Library, None)]);
        Keymap::init(DEFAULTS, Some(file.clone()), cx);
        Keymap::bind(
            "test::First",
            Some("Workspace".into()),
            Some("ctrl-e"),
            "alt-e",
            cx,
        )
        .unwrap();
        assert!(bound(cx, "ctrl-e").is_empty());
        assert_eq!(bound(cx, "alt-e"), ["test::First"]);
        assert!(Keymap::global(cx).is_customized("test::First"));
        assert!(!Keymap::global(cx).is_customized("test::Second"));
        assert_eq!(
            Keymap::global(cx).conflicts("test::First", &Some("Workspace".into()), "ctrl-j"),
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
        Keymap::reset("test::First", cx).unwrap();
        assert_eq!(bound(cx, "ctrl-e"), ["test::First"]);
        assert!(bound(cx, "alt-e").is_empty());
        assert!(!Keymap::global(cx).is_customized("test::First"));
        // A user file is read back on the next start.
        Keymap::unbind("test::Second", Some("Workspace".into()), "ctrl-j", cx).unwrap();
        Keymap::init(DEFAULTS, Some(file.clone()), cx);
        assert!(bound(cx, "ctrl-j").is_empty());
    });
}

#[gpui_kit::test]
fn a_malformed_user_file_keeps_the_defaults_and_says_why(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("keymap.toml");
    std::fs::write(&file, "this is not toml [").unwrap();
    cx.update(|cx| {
        Keymap::init(DEFAULTS, Some(file), cx);
        assert!(Keymap::global(cx).error.is_some());
        assert_eq!(bound(cx, "ctrl-e"), ["test::First"]);
    });
}
