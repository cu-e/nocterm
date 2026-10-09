//! How the user arranged the window — widths of resizable columns and
//! choices such as which side a panel is on — remembered across restarts.
//!
//! Views read a value by key when they lay out and record it when the user
//! changes it. The values live in a small JSON file in the state directory;
//! without a file (tests, a read-only home) they last for the session.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use gpui_kit::{App, Global, Pixels, Task, px};
use nocterm_core::persist::{self, ShutdownDeadline, WriteGate};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct Saved {
    sizes: BTreeMap<String, f32>,
    flags: BTreeMap<String, bool>,
}

#[derive(Default)]
pub struct LayoutMemory {
    saved: Saved,
    file: Option<PathBuf>,
    gate: WriteGate,
    revision: Arc<AtomicU64>,
    writer: Option<Task<()>>,
    #[cfg(test)]
    captures: Arc<AtomicU64>,
}

impl Global for LayoutMemory {}

impl LayoutMemory {
    /// Installs the arrangement read from `file`.
    pub fn init(file: Option<PathBuf>, cx: &mut App) {
        let saved = file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        cx.set_global(Self {
            saved,
            file,
            ..Self::default()
        });
        cx.on_app_quit(|cx| {
            let memory = cx.global::<Self>();
            memory.gate.freeze();
            let saved = memory.saved.clone();
            let file = memory
                .file
                .clone()
                .filter(|_| memory.revision.load(Ordering::Acquire) > 0);
            let gate = memory.gate.clone();
            let executor = cx.background_executor().clone();
            let deadline = ShutdownDeadline::new(Duration::from_millis(180));
            async move {
                let Some(file) = file else { return };
                let work = executor.spawn(async move {
                    let _guard = gate.final_write().await;
                    write(file, saved);
                });
                if matches!(
                    futures::future::select(work, executor.timer(deadline.remaining())).await,
                    futures::future::Either::Right(_)
                ) {
                    tracing::warn!("timed out saving final layout during shutdown");
                }
            }
        })
        .detach();
    }

    /// The remembered size for `key`, if the user ever set one.
    pub fn get(key: &str, cx: &App) -> Option<Pixels> {
        cx.try_global::<Self>()?
            .saved
            .sizes
            .get(key)
            .filter(|size| size.is_finite() && **size > 0.)
            .map(|size| px(*size))
    }

    /// Remembers `size` for `key`.
    pub fn set(key: &str, size: Pixels, cx: &mut App) {
        let size = f32::from(size).round();
        if !size.is_finite() || size <= 0. {
            return;
        }
        let memory = cx.default_global::<Self>();
        if memory.saved.sizes.get(key) == Some(&size) {
            return;
        }
        memory.saved.sizes.insert(key.to_owned(), size);
        Self::save(cx);
    }

    /// The remembered choice `key`; off until the user turns it on.
    pub fn flag(key: &str, cx: &App) -> bool {
        cx.try_global::<Self>()
            .and_then(|memory| memory.saved.flags.get(key).copied())
            .unwrap_or(false)
    }

    /// Remembers choice `key`.
    pub fn set_flag(key: &str, value: bool, cx: &mut App) {
        let memory = cx.default_global::<Self>();
        if memory.saved.flags.get(key).copied().unwrap_or(false) == value {
            return;
        }
        memory.saved.flags.insert(key.to_owned(), value);
        Self::save(cx);
    }

    /// Writes the file in the background.
    fn save(cx: &mut App) {
        let memory = cx.global_mut::<Self>();
        if memory.file.is_none() {
            return;
        }
        if memory.gate.is_frozen() {
            return;
        }
        memory.revision.fetch_add(1, Ordering::AcqRel);
        if memory.writer.is_some() {
            return;
        }
        let gate = memory.gate.clone();
        let latest = memory.revision.clone();
        let task = cx.spawn(async move |cx| {
            loop {
                if gate.is_frozen() {
                    return;
                }
                let (file, saved, revision) = cx.update(|cx| {
                    let memory = cx.global::<Self>();
                    #[cfg(test)]
                    memory.captures.fetch_add(1, Ordering::Relaxed);
                    (
                        memory.file.clone(),
                        memory.saved.clone(),
                        latest.load(Ordering::Acquire),
                    )
                });
                let disk_gate = gate.clone();
                let disk_latest = latest.clone();
                cx.background_executor()
                    .spawn(async move {
                        let Some(_guard) = disk_gate.normal().await else {
                            return;
                        };
                        if disk_latest.load(Ordering::Acquire) == revision
                            && let Some(file) = file
                        {
                            write(file, saved);
                        }
                    })
                    .await;
                if gate.is_frozen() {
                    return;
                }
                let again = cx.update(|cx| {
                    let memory = cx.global_mut::<Self>();
                    if latest.load(Ordering::Acquire) == revision {
                        memory.writer = None;
                        false
                    } else {
                        true
                    }
                });
                if !again {
                    return;
                }
            }
        });
        cx.global_mut::<Self>().writer = Some(task);
    }
}

fn write(file: PathBuf, saved: Saved) {
    let result = serde_json::to_string_pretty(&saved)
        .map_err(|error| error.to_string())
        .and_then(|text| persist::save_text(&file, &text).map_err(|error| error.to_string()));
    if let Err(error) = result {
        tracing::warn!(%error, file = %file.display(), "layout was not saved");
    }
}

#[cfg(test)]
mod tests;
