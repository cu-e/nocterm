//! Key bindings, read from data rather than written in code.
//!
//! Owns: keymap format, default/user merge, override persistence and GPUI installation.
//! Does not own: action handlers, component bindings or the Keymap settings view.
//! Ports used: shared Persisted snapshot execution and ShutdownCoordinator. Provided: explicit
//! KeymapHandle with desired binding admission and publication receipt. Settings: none. Globals: none.
//! Native loading remains startup acquisition; accepted writes use the shared bounded snapshot worker.
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

use gpui_kit::{
    App, AppContext as _, DummyKeyboardMapper, Entity, KeyBinding, KeyBindingContextPredicate,
    KeyBindingMetaIndex, NoAction, Subscription, Task,
};
use nocterm_core::{SharedError, persist::PersistError};
use nocterm_store::{Persisted, Publication, ShutdownCoordinator, SnapshotWriter, StoreError};
use std::{path::PathBuf, rc::Rc, sync::Arc};

pub use error::KeymapError;
pub use file::{Binding, KeymapFile, Section, Source, UNBOUND, merge, normalize};
pub use names::{OFFERED_NAMESPACES, humanize, offered};
const DEFAULT_META: KeyBindingMetaIndex = KeyBindingMetaIndex(0x6e6f_6301);
const USER_META: KeyBindingMetaIndex = KeyBindingMetaIndex(0x6e6f_6302);

enum Failure {
    Load(SharedError<PersistError>),
    Write(StoreError),
}
struct Keymap {
    defaults: KeymapFile,
    user: Entity<Persisted<KeymapFile>>,
    failure: Option<Failure>,
    revision: u64,
    generation: u64,
    publication: Option<Task<Result<(), KeymapError>>>,
}

/// Original binding owner; its Persisted document is the only authoritative user keymap.
#[derive(Clone)]
pub struct KeymapHandle {
    state: Entity<Keymap>,
}

impl KeymapHandle {
    pub fn install(defaults: &str, file: Option<PathBuf>, cx: &mut App) -> Self {
        let defaults = toml::from_str(defaults).unwrap_or_else(|error| {
            tracing::error!(%error, "the default keymap is malformed");
            KeymapFile::default()
        });
        let (initial, failure) = match file.as_deref().map(nocterm_core::persist::load) {
            None | Some(Ok(None)) => (KeymapFile::default(), None),
            Some(Ok(Some(user))) => (user, None),
            Some(Err(error)) => (
                KeymapFile::default(),
                Some(Failure::Load(SharedError::from_error(error))),
            ),
        };
        let user = cx.new(|_| Persisted::new(initial, writer(file), Publication::Optimistic, 8));
        let state = cx.new(|_| Keymap {
            defaults,
            user,
            failure,
            revision: 0,
            generation: 0,
            publication: None,
        });
        let handle = Self { state };
        handle.replace_bindings(cx);
        handle
    }
    /// Registers the existing snapshot owner, including accepted writes, with application quit.
    pub fn register_shutdown(
        &self,
        shutdown: &ShutdownCoordinator,
        cx: &App,
    ) -> Result<(), StoreError> {
        shutdown.register(&self.state.read(cx).user)
    }
    /// Last original native load/write cause, preserving its concrete allocation.
    pub fn error<'a>(&self, cx: &'a App) -> Option<&'a SharedError<PersistError>> {
        match self.state.read(cx).failure.as_ref()? {
            Failure::Load(error) => Some(error),
            Failure::Write(error) => native_error(error),
        }
    }
    pub fn publication_error<'a>(&self, cx: &'a App) -> Option<&'a StoreError> {
        match self.state.read(cx).failure.as_ref()? {
            Failure::Write(error) => Some(error),
            _ => None,
        }
    }
    pub fn revision(&self, cx: &App) -> u64 {
        self.state.read(cx).revision
    }
    pub fn observe(
        &self,
        cx: &mut App,
        mut on_change: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        cx.observe(&self.state, move |_, cx| on_change(cx))
    }
    pub fn bindings(&self, cx: &App) -> Vec<Binding> {
        let state = self.state.read(cx);
        merge(&state.defaults, state.user.read(cx).visible())
    }
    pub fn defaults(&self, cx: &App) -> Vec<Binding> {
        merge(&self.state.read(cx).defaults, &KeymapFile::default())
    }
    pub fn customized_actions(&self, cx: &App) -> std::collections::BTreeSet<String> {
        let state = self.state.read(cx);
        state
            .user
            .read(cx)
            .visible()
            .here()
            .filter_map(|(context, keys, action)| {
                if action == UNBOUND {
                    state.defaults.action_for(context, keys).map(str::to_owned)
                } else {
                    Some(action.clone())
                }
            })
            .collect()
    }
    pub fn is_customized(&self, action: &str, cx: &App) -> bool {
        self.customized_actions(cx).contains(action)
    }
    pub fn conflicts(
        &self,
        action: &str,
        context: &Option<String>,
        keys: &str,
        cx: &App,
    ) -> Vec<String> {
        let Ok(keys) = normalize(keys) else {
            return Vec::new();
        };
        self.bindings(cx)
            .into_iter()
            .filter(|binding| {
                binding.context == *context
                    && binding.keystrokes == keys
                    && binding.action != action
            })
            .map(|binding| binding.action)
            .collect()
    }
    /// Installs the desired edit immediately; the returned result reports admission.
    pub fn bind(
        &self,
        action: &str,
        context: Option<String>,
        replacing: Option<&str>,
        keystrokes: &str,
        cx: &mut App,
    ) -> Result<(), KeymapError> {
        let keys = normalize(keystrokes)?;
        self.change(cx, |user, defaults| {
            if let Some(old) = replacing {
                unbind_entry(user, defaults, action, &context, old);
            }
            user.set(&context, &keys, action);
        })
    }
    pub fn unbind(
        &self,
        action: &str,
        context: Option<String>,
        keys: &str,
        cx: &mut App,
    ) -> Result<(), KeymapError> {
        self.change(cx, |user, defaults| {
            unbind_entry(user, defaults, action, &context, keys)
        })
    }
    pub fn reset(&self, action: &str, cx: &mut App) -> Result<(), KeymapError> {
        self.change(cx, |user, defaults| {
            user.retain(|context, keys, bound| {
                bound != action
                    && !(bound == UNBOUND && defaults.action_for(context, keys) == Some(action))
            })
        })
    }
    /// The latest accepted publication; dropping this waiter leaves the store's work accepted.
    pub fn flush(&self, cx: &mut App) -> Task<Result<(), KeymapError>> {
        self.state
            .update(cx, |state, _| state.publication.take())
            .unwrap_or_else(|| Task::ready(Ok(())))
    }
    fn change(
        &self,
        cx: &mut App,
        edit: impl FnOnce(&mut KeymapFile, &KeymapFile),
    ) -> Result<(), KeymapError> {
        let state = self.state.read(cx);
        let user = state.user.clone();
        let mut next = user.read(cx).visible().clone();
        edit(&mut next, &state.defaults);
        let changed = next != *user.read(cx).visible();
        let receipt = user.update(cx, |user, cx| {
            user.update(move |current| *current = next, cx)
        })?;
        self.state.update(cx, |state, cx| {
            state.revision = state.revision.wrapping_add(u64::from(changed));
            state.generation = state.generation.wrapping_add(1);
            let generation = state.generation;
            state.publication = Some(cx.spawn(async move |state, cx| {
                let result = receipt.wait().await;
                let _ = state.update(cx, |state, cx| {
                    if state.generation == generation {
                        let next = result.as_ref().err().cloned().map(Failure::Write);
                        let changed = state.failure.is_some() || next.is_some();
                        state.failure = next;
                        if changed {
                            cx.notify();
                        }
                    }
                });
                result.map(drop).map_err(publication_error)
            }));
            cx.notify();
        });
        if changed {
            self.replace_bindings(cx);
        }
        Ok(())
    }
    fn replace_bindings(&self, cx: &mut App) {
        let ours = {
            let state = self.state.read(cx);
            let mut ours = Vec::new();
            for (file, meta) in [
                (&state.defaults, DEFAULT_META),
                (state.user.read(cx).visible(), USER_META),
            ] {
                ours.extend(
                    bindings_of(file, cx)
                        .into_iter()
                        .map(|binding| binding.with_meta(meta)),
                );
            }
            ours
        };
        let others: Vec<_> = cx
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
fn unbind_entry(
    user: &mut KeymapFile,
    defaults: &KeymapFile,
    action: &str,
    context: &Option<String>,
    keys: &str,
) {
    if defaults.action_for(context, keys) == Some(action) {
        user.set(context, keys, UNBOUND);
    } else {
        user.remove(context, keys);
    }
}
fn writer(file: Option<PathBuf>) -> SnapshotWriter<KeymapFile> {
    Arc::new(move |user| {
        let file = file.clone();
        Box::pin(async move {
            if let Some(file) = file {
                nocterm_core::persist::save(&file, &user)
                    .map_err(SharedError::from_error)
                    .map_err(StoreError::backend)?;
            }
            Ok(())
        })
    })
}
fn native_error(error: &StoreError) -> Option<&SharedError<PersistError>> {
    if let StoreError::Backend(cause) = error {
        cause.downcast_ref()
    } else {
        None
    }
}
fn publication_error(error: StoreError) -> KeymapError {
    native_error(&error)
        .cloned()
        .map(KeymapError::Persistence)
        .unwrap_or(KeymapError::Store(error))
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
