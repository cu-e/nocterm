//! Key bindings, read from data rather than written in code.
//!
//! The defaults come with the application in the keymap format
//! ([`KeymapFile`]); the user's changes live in a file of the same format,
//! `keymap.toml` in the configuration directory, which only lists what
//! differs: a keystroke bound to another action, or to `"none"` to unbind a
//! default. [`Keymap`] merges the two, installs the result in GPUI and
//! applies every change at once, without a restart. Bindings other crates
//! install in code (the component library's text fields, lists and menus)
//! are left alone.

mod file;
mod names;

use std::{path::PathBuf, rc::Rc};

use gpui_kit::{
    App, DummyKeyboardMapper, Global, KeyBinding, KeyBindingContextPredicate, KeyBindingMetaIndex,
    NoAction,
};

pub use file::{Binding, KeymapFile, Section, Source, UNBOUND, merge, normalize};
pub use names::{OFFERED_NAMESPACES, humanize, offered};

/// Marks the bindings this crate installs, so a reload replaces only them.
const DEFAULT_META: KeyBindingMetaIndex = KeyBindingMetaIndex(0x6e6f_6301);
const USER_META: KeyBindingMetaIndex = KeyBindingMetaIndex(0x6e6f_6302);

/// The defaults, the user's changes and the file they are kept in.
pub struct Keymap {
    defaults: KeymapFile,
    user: KeymapFile,
    file: Option<PathBuf>,
    /// Why the user's file could not be read or written, if it could not.
    pub error: Option<String>,
}

impl Global for Keymap {}

impl Keymap {
    /// Reads the defaults from `defaults` and the user's changes from
    /// `file`, then binds the merge. A malformed user file is reported in
    /// [`Keymap::error`] and ignored, rather than costing every binding.
    pub fn init(defaults: &str, file: Option<PathBuf>, cx: &mut App) {
        let defaults = toml::from_str(defaults).unwrap_or_else(|error| {
            tracing::error!(%error, "the default keymap is malformed");
            KeymapFile::default()
        });
        let (user, error) = match file.as_deref().map(nocterm_core::persist::load) {
            None | Some(Ok(None)) => (KeymapFile::default(), None),
            Some(Ok(Some(user))) => (user, None),
            Some(Err(error)) => {
                tracing::error!(%error, "the user keymap is malformed");
                (KeymapFile::default(), Some(error.to_string()))
            }
        };
        cx.set_global(Self {
            defaults,
            user,
            file,
            error,
        });
        Self::install(cx);
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Every binding in effect on this platform.
    pub fn bindings(&self) -> Vec<Binding> {
        merge(&self.defaults, &self.user)
    }

    /// The default bindings of this platform.
    pub fn defaults(&self) -> Vec<Binding> {
        merge(&self.defaults, &KeymapFile::default())
    }

    /// Whether the user changed how `action` is bound.
    pub fn is_customized(&self, action: &str) -> bool {
        let mut customized = false;
        let mut user = self.user.clone();
        user.retain(|context, keys, bound| {
            customized |= bound == action
                || (bound == UNBOUND && self.defaults.action_for(context, keys) == Some(action));
            true
        });
        customized
    }

    /// Binds `keystrokes` to `action` in `context`. With `replacing`, the
    /// binding being edited goes away.
    pub fn bind(
        action: &str,
        context: Option<String>,
        replacing: Option<&str>,
        keystrokes: &str,
        cx: &mut App,
    ) -> Result<(), String> {
        let keystrokes = normalize(keystrokes)?;
        Self::change(cx, |keymap| {
            if let Some(old) = replacing {
                keymap.unbind_entry(action, &context, old);
            }
            keymap.user.set(&context, &keystrokes, action);
        })
    }

    /// Removes the binding of `keystrokes` to `action` in `context`.
    pub fn unbind(
        action: &str,
        context: Option<String>,
        keystrokes: &str,
        cx: &mut App,
    ) -> Result<(), String> {
        Self::change(cx, |keymap| {
            keymap.unbind_entry(action, &context, keystrokes)
        })
    }

    /// Returns `action` to its default bindings.
    pub fn reset(action: &str, cx: &mut App) -> Result<(), String> {
        Self::change(cx, |keymap| {
            let defaults = keymap.defaults.clone();
            keymap.user.retain(|context, keys, bound| {
                bound != action
                    && !(bound == UNBOUND && defaults.action_for(context, keys) == Some(action))
            });
        })
    }

    /// The actions other than `action` that `keystrokes` runs in `context`.
    pub fn conflicts(
        &self,
        action: &str,
        context: &Option<String>,
        keystrokes: &str,
    ) -> Vec<String> {
        let Ok(keystrokes) = normalize(keystrokes) else {
            return Vec::new();
        };
        self.bindings()
            .into_iter()
            .filter(|binding| {
                binding.context == *context
                    && binding.keystrokes == keystrokes
                    && binding.action != action
            })
            .map(|binding| binding.action)
            .collect()
    }

    fn unbind_entry(&mut self, action: &str, context: &Option<String>, keystrokes: &str) {
        if self.defaults.action_for(context, keystrokes) == Some(action) {
            self.user.set(context, keystrokes, UNBOUND);
        } else {
            self.user.remove(context, keystrokes);
        }
    }

    /// Applies `edit` to the user's changes, saves them and rebinds.
    fn change(cx: &mut App, edit: impl FnOnce(&mut Self)) -> Result<(), String> {
        let keymap = cx.global_mut::<Self>();
        edit(keymap);
        let result = match &keymap.file {
            Some(file) => {
                nocterm_core::persist::save(file, &keymap.user).map_err(|error| error.to_string())
            }
            None => Ok(()),
        };
        keymap.error = result.clone().err();
        Self::install(cx);
        result
    }

    /// Replaces the bindings this crate installed with the current merge.
    fn install(cx: &mut App) {
        let keymap = cx.global::<Self>();
        let mut ours = Vec::new();
        for (file, meta) in [(&keymap.defaults, DEFAULT_META), (&keymap.user, USER_META)] {
            ours.extend(bindings_of(file, cx).into_iter().map(|b| b.with_meta(meta)));
        }
        let others: Vec<KeyBinding> = cx
            .key_bindings()
            .borrow()
            .bindings()
            .filter(|binding| !matches!(binding.meta(), Some(DEFAULT_META | USER_META)))
            .cloned()
            .collect();
        cx.clear_key_bindings();
        cx.bind_keys(others.into_iter().chain(ours));
    }
}

/// `file`'s bindings for this platform as GPUI bindings. Entries that do
/// not resolve are logged and skipped: one stale binding must not cost the
/// others.
#[expect(clippy::cognitive_complexity, reason = "predates the limit")]
fn bindings_of(file: &KeymapFile, cx: &App) -> Vec<KeyBinding> {
    let mut bindings = Vec::new();
    for section in file
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
            let action = if action == UNBOUND {
                Ok(Box::new(NoAction) as Box<dyn gpui_kit::Action>)
            } else {
                cx.build_action(action, None)
            };
            let action = match action {
                Ok(action) => action,
                Err(error) => {
                    tracing::error!(%keystrokes, %error, "unknown action in keymap");
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
    bindings
}

#[cfg(test)]
mod tests;
