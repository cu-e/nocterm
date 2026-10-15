//! Key bindings, read from data rather than written in code.
//!
//! Owns: keymap format, default/user merge, override persistence and GPUI installation.
//! Does not own: action handlers, component bindings or the Keymap settings view.
//! Ports used/provided: explicit `KeymapHandle`. Settings section: none. Globals: none.
//! Native loading and synchronous desired-file publication remain existing UI-layer debt.
//! Invariants: installation replaces only this crate's marked bindings; explicit
//! unbindings remove defaults without modifying unrelated toolkit bindings;
//! desired edits remain installed when typed persistence errors are reported.
//!
//! The defaults come with the application in the keymap format
//! ([`KeymapFile`]); the user's changes live in a file of the same format,
//! `keymap.toml` in the configuration directory, which only lists what
//! differs: a keystroke bound to another action, or to `"none"` to unbind a
//! default. [`KeymapHandle`] merges the two, installs the result in GPUI and
//! applies every change at once, without a restart. Bindings other crates
//! install in code (the component library's text fields, lists and menus)
//! are left alone.

mod error;
mod file;
mod names;

use std::{path::PathBuf, rc::Rc};

use gpui_kit::{
    App, AppContext as _, DummyKeyboardMapper, Entity, KeyBinding, KeyBindingContextPredicate,
    KeyBindingMetaIndex, NoAction, Subscription,
};

pub use error::KeymapError;
pub use file::{Binding, KeymapFile, Section, Source, UNBOUND, merge, normalize};
pub use names::{OFFERED_NAMESPACES, humanize, offered};

/// Marks the bindings this crate installs, so a reload replaces only them.
const DEFAULT_META: KeyBindingMetaIndex = KeyBindingMetaIndex(0x6e6f_6301);
const USER_META: KeyBindingMetaIndex = KeyBindingMetaIndex(0x6e6f_6302);

/// The defaults, the user's changes and the file they are kept in.
struct Keymap {
    defaults: KeymapFile,
    user: KeymapFile,
    file: Option<PathBuf>,
    /// Why the user's file could not be read or written, if it could not.
    error: Option<nocterm_core::SharedError<nocterm_core::persist::PersistError>>,
}

/// Explicit shared binding owner retained by the composition root and its pages.
#[derive(Clone)]
pub struct KeymapHandle {
    state: Entity<Keymap>,
}

impl KeymapHandle {
    /// Reads the defaults from `defaults` and the user's changes from
    /// `file`, then binds the merge. A malformed user file is reported in
    /// [`KeymapHandle::error`] and ignored, rather than costing every binding.
    pub fn install(defaults: &str, file: Option<PathBuf>, cx: &mut App) -> Self {
        let defaults = toml::from_str(defaults).unwrap_or_else(|error| {
            tracing::error!(%error, "the default keymap is malformed");
            KeymapFile::default()
        });
        let (user, error) = match file.as_deref().map(nocterm_core::persist::load) {
            None | Some(Ok(None)) => (KeymapFile::default(), None),
            Some(Ok(Some(user))) => (user, None),
            Some(Err(error)) => {
                tracing::error!(%error, "the user keymap is malformed");
                (
                    KeymapFile::default(),
                    Some(nocterm_core::SharedError::from_error(error)),
                )
            }
        };
        let state = cx.new(|_| Keymap {
            defaults,
            user,
            file,
            error,
        });
        let handle = Self { state };
        handle.replace_bindings(cx);
        handle
    }

    /// Last native load/publication failure, retained without reconstructing its cause.
    pub fn error<'a>(
        &self,
        cx: &'a App,
    ) -> Option<&'a nocterm_core::SharedError<nocterm_core::persist::PersistError>> {
        self.state.read(cx).error.as_ref()
    }

    /// Observe completed installation/error transitions; callers retain the subscription.
    pub fn observe(
        &self,
        cx: &mut App,
        mut on_change: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        cx.observe(&self.state, move |_, cx| on_change(cx))
    }

    pub fn bindings(&self, cx: &App) -> Vec<Binding> {
        self.state.read(cx).bindings()
    }
    pub fn defaults(&self, cx: &App) -> Vec<Binding> {
        self.state.read(cx).defaults()
    }
    pub fn is_customized(&self, action: &str, cx: &App) -> bool {
        self.state.read(cx).is_customized(action)
    }
    pub fn conflicts(
        &self,
        action: &str,
        context: &Option<String>,
        keys: &str,
        cx: &App,
    ) -> Vec<String> {
        self.state.read(cx).conflicts(action, context, keys)
    }

    /// Binds `keystrokes` to `action` in `context`. With `replacing`, the
    /// binding being edited goes away.
    pub fn bind(
        &self,
        action: &str,
        context: Option<String>,
        replacing: Option<&str>,
        keystrokes: &str,
        cx: &mut App,
    ) -> Result<(), KeymapError> {
        let keystrokes = normalize(keystrokes)?;
        self.change(cx, |keymap| {
            if let Some(old) = replacing {
                keymap.unbind_entry(action, &context, old);
            }
            keymap.user.set(&context, &keystrokes, action);
        })
    }

    /// Removes the binding of `keystrokes` to `action` in `context`.
    pub fn unbind(
        &self,
        action: &str,
        context: Option<String>,
        keystrokes: &str,
        cx: &mut App,
    ) -> Result<(), KeymapError> {
        self.change(cx, |keymap| {
            keymap.unbind_entry(action, &context, keystrokes)
        })
    }

    /// Returns `action` to its default bindings.
    pub fn reset(&self, action: &str, cx: &mut App) -> Result<(), KeymapError> {
        self.change(cx, |keymap| {
            let defaults = keymap.defaults.clone();
            keymap.user.retain(|context, keys, bound| {
                bound != action
                    && !(bound == UNBOUND && defaults.action_for(context, keys) == Some(action))
            });
        })
    }

    /// Publish the desired edit even when its synchronous native save fails.
    fn change(&self, cx: &mut App, edit: impl FnOnce(&mut Keymap)) -> Result<(), KeymapError> {
        let result = self.state.update(cx, |keymap, _| {
            edit(keymap);
            let result = match &keymap.file {
                Some(file) => nocterm_core::persist::save(file, &keymap.user)
                    .map_err(nocterm_core::SharedError::from_error),
                None => Ok(()),
            };
            keymap.error = result.as_ref().err().cloned();
            result.map_err(KeymapError::Persistence)
        });
        self.replace_bindings(cx);
        self.state.update(cx, |_, cx| cx.notify());
        result
    }

    fn replace_bindings(&self, cx: &mut App) {
        let ours = {
            let keymap = self.state.read(cx);
            let mut ours = Vec::new();
            for (file, meta) in [(&keymap.defaults, DEFAULT_META), (&keymap.user, USER_META)] {
                ours.extend(
                    bindings_of(file, cx)
                        .into_iter()
                        .map(|binding| binding.with_meta(meta)),
                );
            }
            ours
        };
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

impl Keymap {
    /// Every binding in effect on this platform.
    fn bindings(&self) -> Vec<Binding> {
        merge(&self.defaults, &self.user)
    }

    /// The default bindings of this platform.
    fn defaults(&self) -> Vec<Binding> {
        merge(&self.defaults, &KeymapFile::default())
    }

    /// Whether the user changed how `action` is bound.
    fn is_customized(&self, action: &str) -> bool {
        let mut customized = false;
        let mut user = self.user.clone();
        user.retain(|context, keys, bound| {
            customized |= bound == action
                || (bound == UNBOUND && self.defaults.action_for(context, keys) == Some(action));
            true
        });
        customized
    }

    /// The actions other than `action` that `keystrokes` runs in `context`.
    fn conflicts(&self, action: &str, context: &Option<String>, keystrokes: &str) -> Vec<String> {
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
