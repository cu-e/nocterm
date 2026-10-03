//! Key bindings, read from data rather than written in code.
//!
//! The defaults live in `assets/keymap.toml`, so a binding changes without
//! touching a view, and a user keymap can later be layered on the same
//! format.

use std::{collections::BTreeMap, env, rc::Rc};

use gpui_kit::{App, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate};
use serde::Deserialize;

/// The default key bindings.
const DEFAULT_KEYMAP: &str = include_str!("../assets/keymap.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Keymap {
    #[serde(default, rename = "section")]
    sections: Vec<Section>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
    /// The key context predicate; none binds everywhere.
    context: Option<String>,
    /// The platforms the section applies to; empty means all.
    #[serde(default)]
    os: Vec<String>,
    /// Keystrokes to action names.
    bindings: BTreeMap<String, String>,
}

impl Section {
    fn applies_here(&self) -> bool {
        self.os.is_empty() || self.os.iter().any(|os| os == env::consts::OS)
    }
}

/// Binds the default keymap. Entries that do not resolve are logged and
/// skipped: one stale binding must not cost the others.
pub(crate) fn load(cx: &mut App) {
    let keymap: Keymap = match toml::from_str(DEFAULT_KEYMAP) {
        Ok(keymap) => keymap,
        Err(error) => {
            tracing::error!(%error, "the default keymap is malformed");
            return;
        }
    };

    let mut bindings = Vec::new();
    for section in keymap
        .sections
        .iter()
        .filter(|section| section.applies_here())
    {
        let context = match section
            .context
            .as_deref()
            .map(KeyBindingContextPredicate::parse)
        {
            None => None,
            Some(Ok(predicate)) => Some(Rc::new(predicate)),
            Some(Err(error)) => {
                tracing::error!(context = ?section.context, %error, "invalid key context");
                continue;
            }
        };
        for (keystrokes, action) in &section.bindings {
            let action = match cx.build_action(action, None) {
                Ok(action) => action,
                Err(error) => {
                    tracing::error!(%keystrokes, %action, %error, "unknown action in keymap");
                    continue;
                }
            };
            match KeyBinding::load(
                keystrokes,
                action,
                context.clone(),
                false,
                None,
                &DummyKeyboardMapper,
            ) {
                Ok(binding) => bindings.push(binding),
                Err(error) => tracing::error!(%keystrokes, %error, "invalid keystroke in keymap"),
            }
        }
    }
    cx.bind_keys(bindings);
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Keystroke, TestAppContext};

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
