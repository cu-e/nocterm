//! The keymap file format, shared by the defaults and the user's file, and
//! how the two combine.
use std::{collections::BTreeMap, env};

use gpui_kit::Keystroke;
use serde::{Deserialize, Serialize};

/// The action that unbinds keystrokes in a user keymap.
pub const UNBOUND: &str = "none";

/// Sections of bindings, each in one key context.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KeymapFile {
    #[serde(default, rename = "section", skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Section {
    /// The key context predicate; none binds everywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// The platforms the section applies to; empty means all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub os: Vec<String>,
    /// Keystrokes to action names.
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
}

impl Section {
    pub fn applies_here(&self) -> bool {
        self.os.is_empty() || self.os.iter().any(|os| os == env::consts::OS)
    }
}

/// Where a binding comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Default,
    User,
}

/// One keystroke sequence bound to an action in a context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub context: Option<String>,
    pub keystrokes: String,
    pub action: String,
    pub source: Source,
}

/// `keystrokes` written the one way the keymap writes them: `ctrl-shift-p`,
/// sequences separated by single spaces.
pub fn normalize(keystrokes: &str) -> Result<String, String> {
    let parts = keystrokes
        .split_whitespace()
        .map(|part| {
            Keystroke::parse(part)
                .map(|keystroke| keystroke.unparse())
                .map_err(|error| format!("{part}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.is_empty() {
        return Err("no keystroke".into());
    }
    Ok(parts.join(" "))
}

fn key(context: &Option<String>, keystrokes: &str) -> (Option<String>, String) {
    (
        context.clone(),
        normalize(keystrokes).unwrap_or_else(|_| keystrokes.to_owned()),
    )
}

impl KeymapFile {
    /// The bindings for this platform, in file order.
    fn here(&self) -> impl Iterator<Item = (&Option<String>, &String, &String)> {
        self.sections
            .iter()
            .filter(|section| section.applies_here())
            .flat_map(|section| {
                section
                    .bindings
                    .iter()
                    .map(move |(keys, action)| (&section.context, keys, action))
            })
    }

    /// The action `keystrokes` runs in `context`, if this file binds it.
    pub fn action_for(&self, context: &Option<String>, keystrokes: &str) -> Option<&str> {
        let wanted = key(context, keystrokes);
        self.here()
            .filter(|(c, k, _)| key(c, k) == wanted)
            .last()
            .map(|(_, _, action)| action.as_str())
    }

    fn bindings_mut(&mut self, context: &Option<String>) -> &mut BTreeMap<String, String> {
        let ix = match self
            .sections
            .iter()
            .position(|section| section.context == *context && section.os.is_empty())
        {
            Some(ix) => ix,
            None => {
                self.sections.push(Section {
                    context: context.clone(),
                    ..Section::default()
                });
                self.sections.len() - 1
            }
        };
        &mut self.sections[ix].bindings
    }

    /// Binds `keystrokes` to `action` in `context`, replacing what it ran.
    pub fn set(&mut self, context: &Option<String>, keystrokes: &str, action: &str) {
        self.remove(context, keystrokes);
        let keystrokes = key(context, keystrokes).1;
        self.bindings_mut(context)
            .insert(keystrokes, action.to_owned());
    }

    /// Forgets what this file says about `keystrokes` in `context`.
    pub fn remove(&mut self, context: &Option<String>, keystrokes: &str) {
        let wanted = key(context, keystrokes);
        for section in &mut self.sections {
            if section.context == wanted.0 {
                section
                    .bindings
                    .retain(|keys, _| key(context, keys).1 != wanted.1);
            }
        }
        self.sections.retain(|section| !section.bindings.is_empty());
    }

    /// Forgets every entry `keep` rejects.
    pub fn retain(&mut self, mut keep: impl FnMut(&Option<String>, &str, &str) -> bool) {
        for section in &mut self.sections {
            let context = section.context.clone();
            section
                .bindings
                .retain(|keys, action| keep(&context, keys, action));
        }
        self.sections.retain(|section| !section.bindings.is_empty());
    }
}

/// The bindings in effect: the defaults, with the user's entries replacing
/// or removing the ones with the same keystrokes and context.
pub fn merge(defaults: &KeymapFile, user: &KeymapFile) -> Vec<Binding> {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut apply = |context: &Option<String>, keys: &String, action: &String, source| {
        let wanted = key(context, keys);
        bindings.retain(|binding| key(&binding.context, &binding.keystrokes) != wanted);
        if action != UNBOUND {
            bindings.push(Binding {
                context: context.clone(),
                keystrokes: wanted.1,
                action: action.clone(),
                source,
            });
        }
    };
    for (context, keys, action) in defaults.here() {
        apply(context, keys, action, Source::Default);
    }
    for (context, keys, action) in user.here() {
        apply(context, keys, action, Source::User);
    }
    bindings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> KeymapFile {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn user_entries_replace_and_remove_defaults() {
        let defaults = file(
            r#"
[[section]]
context = "Workspace"
[section.bindings]
"ctrl-e" = "workspace::SwapSides"
"ctrl-j" = "workspace::ToggleLocalTerminal"
"#,
        );
        let user = file(
            r#"
[[section]]
context = "Workspace"
[section.bindings]
"ctrl-e" = "none"
"alt-e" = "workspace::SwapSides"
"#,
        );
        let merged = merge(&defaults, &user);
        let find = |keys: &str| merged.iter().find(|b| b.keystrokes == keys);
        assert!(find("ctrl-e").is_none());
        assert_eq!(find("alt-e").unwrap().source, Source::User);
        assert_eq!(find("ctrl-j").unwrap().source, Source::Default);
    }

    #[test]
    fn keystrokes_are_compared_as_written_by_the_keymap() {
        assert_eq!(
            normalize("shift-ctrl-P").unwrap(),
            normalize("ctrl-shift-P").unwrap()
        );
        let mut user = KeymapFile::default();
        let context = Some("Workspace".to_owned());
        user.set(&context, "shift-ctrl-k", "a::B");
        user.set(&context, "ctrl-shift-k", "a::C");
        assert_eq!(user.sections[0].bindings.len(), 1);
        assert_eq!(user.action_for(&context, "ctrl-shift-k"), Some("a::C"));
        user.remove(&context, "ctrl-shift-k");
        assert!(user.sections.is_empty());
    }
}
