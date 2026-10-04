//! Key bindings, read from data rather than written in code.
//!
//! The defaults live in `assets/keymap.toml`, so a binding changes without
//! touching a view; `nocterm-keymap` layers the user's `keymap.toml` over
//! them.

use std::path::PathBuf;

use gpui_kit::App;

/// The default key bindings.
const DEFAULT_KEYMAP: &str = include_str!("../assets/keymap.toml");

/// Binds the default keymap and the user's changes from `user_file`.
pub(crate) fn load(user_file: Option<PathBuf>, cx: &mut App) {
    nocterm_keymap::Keymap::init(DEFAULT_KEYMAP, user_file, cx);
}

#[cfg(test)]
mod tests {
    use gpui_kit::{KeyBindingContextPredicate, Keystroke, TestAppContext};
    use nocterm_keymap::KeymapFile as Keymap;

    use super::*;

    #[test]
    fn the_default_keymap_is_well_formed() {
        let keymap: Keymap = toml::from_str(DEFAULT_KEYMAP).unwrap();

        assert!(!keymap.sections.is_empty());
        for section in &keymap.sections {
            if let Some(context) = &section.context {
                KeyBindingContextPredicate::parse(context).unwrap();
            }
            for os in &section.os {
                assert!(["linux", "macos", "windows"].contains(&os.as_str()), "{os}");
            }
            for (keystrokes, action) in &section.bindings {
                for keystroke in keystrokes.split_whitespace() {
                    Keystroke::parse(keystroke).unwrap();
                }
                assert!(action.contains("::"), "{action} has no namespace");
            }
        }
    }

    #[test]
    fn every_platform_gets_bindings() {
        let keymap: Keymap = toml::from_str(DEFAULT_KEYMAP).unwrap();

        for os in ["linux", "macos", "windows"] {
            assert!(
                keymap
                    .sections
                    .iter()
                    .any(|section| section.os.is_empty() || section.os.iter().any(|o| o == os)),
                "{os} has no bindings"
            );
        }
    }

    #[gpui_kit::test]
    fn every_default_binding_resolves_a_registered_action(cx: &mut TestAppContext) {
        let keymap: Keymap = toml::from_str(DEFAULT_KEYMAP).unwrap();
        cx.update(|cx| {
            for section in keymap.sections {
                for action in section.bindings.values() {
                    assert!(
                        cx.build_action(action, None).is_ok(),
                        "unknown action: {action}"
                    );
                }
            }
        });
    }
}
