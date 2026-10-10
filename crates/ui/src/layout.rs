//! Remembered column widths and layout choices.
//!
//! Owns: named layout values and their JSON encoding.
//! Does not own: persistence queues, write ordering or shutdown deadlines.
//! Ports used: `Persisted`, `ShutdownCoordinator`. Ports provided: layout values.
//! Settings section: none. Globals: existing `LayoutMemory` compatibility shell.
//! Invariants: valid user changes are immediately visible; failed persistence
//! leaves the desired arrangement intact.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use gpui_kit::{App, AppContext as _, Entity, Global, Pixels, px};
use nocterm_core::persist;
use nocterm_store::{Persisted, Publication, ShutdownCoordinator, SnapshotWriter, StoreError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
struct Saved {
    sizes: BTreeMap<String, f32>,
    flags: BTreeMap<String, bool>,
}

#[derive(Default)]
pub struct LayoutMemory {
    persisted: Option<Entity<Persisted<Saved>>>,
}

impl Global for LayoutMemory {}

impl LayoutMemory {
    /// Installs the arrangement and delegates the existing quit hook to the
    /// explicitly supplied coordinator. Other owners register with that handle.
    pub fn init(file: Option<PathBuf>, shutdown: &ShutdownCoordinator, cx: &mut App) {
        let saved = file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let persisted =
            cx.new(|_| Persisted::new(saved, writer(file), Publication::Optimistic, 4096));
        if let Err(error) = shutdown.register(&persisted) {
            tracing::warn!(%error, "layout shutdown participant was not registered");
        }
        cx.set_global(Self {
            persisted: Some(persisted),
        });
        let shutdown = shutdown.clone();
        cx.on_app_quit(move |cx| {
            let captured = shutdown.capture_all(cx);
            async move {
                let report = captured.await;
                for error in report.failures() {
                    tracing::warn!(%error, "application state shutdown persistence failed");
                }
            }
        })
        .detach();
    }

    pub fn get(key: &str, cx: &App) -> Option<Pixels> {
        cx.try_global::<Self>()?
            .persisted
            .as_ref()?
            .read(cx)
            .visible()
            .sizes
            .get(key)
            .filter(|size| size.is_finite() && **size > 0.)
            .map(|size| px(*size))
    }

    pub fn set(key: &str, size: Pixels, cx: &mut App) {
        let size = f32::from(size).round();
        if !size.is_finite() || size <= 0. {
            return;
        }
        let key = key.to_owned();
        Self::edit(cx, move |saved| {
            saved.sizes.insert(key, size);
        });
    }

    pub fn flag(key: &str, cx: &App) -> bool {
        cx.try_global::<Self>()
            .and_then(|memory| memory.persisted.as_ref())
            .and_then(|entity| entity.read(cx).visible().flags.get(key).copied())
            .unwrap_or(false)
    }

    pub fn set_flag(key: &str, value: bool, cx: &mut App) {
        if Self::flag(key, cx) == value {
            return;
        }
        let key = key.to_owned();
        Self::edit(cx, move |saved| {
            saved.flags.insert(key, value);
        });
    }

    fn edit(cx: &mut App, edit: impl FnOnce(&mut Saved) + Send + 'static) {
        let entity = cx
            .try_global::<Self>()
            .and_then(|memory| memory.persisted.clone());
        let entity = entity.unwrap_or_else(|| {
            let entity = cx.new(|_| {
                Persisted::new(
                    Saved::default(),
                    writer(None),
                    Publication::Optimistic,
                    4096,
                )
            });
            cx.default_global::<Self>().persisted = Some(entity.clone());
            entity
        });
        if let Err(error) = entity.update(cx, |state, cx| state.update_latest(edit, cx)) {
            tracing::warn!(%error, "layout change was not admitted");
        }
    }
}

fn writer(file: Option<PathBuf>) -> SnapshotWriter<Saved> {
    Arc::new(move |saved| {
        let file = file.clone();
        Box::pin(async move {
            let Some(file) = file else { return Ok(()) };
            let text = serde_json::to_string_pretty(&saved).map_err(StoreError::backend)?;
            persist::save_text(&file, &text).map_err(|error| {
                tracing::warn!(%error, file = %file.display(), "layout was not saved");
                StoreError::backend(error)
            })
        })
    })
}

#[cfg(test)]
mod tests;
