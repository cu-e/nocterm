//! How the user arranged the window — widths of resizable columns and
//! choices such as which side a panel is on — remembered across restarts.
//!
//! Views read a value by key when they lay out and record it when the user
//! changes it. The values live in a small JSON file in the state directory;
//! without a file (tests, a read-only home) they last for the session.

use std::{collections::BTreeMap, path::PathBuf};

use gpui_kit::{App, AppContext as _, Global, Pixels, px};
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
        cx.set_global(Self { saved, file });
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
        let memory = cx.global::<Self>();
        let Some(file) = memory.file.clone() else {
            return;
        };
        let Ok(text) = serde_json::to_string_pretty(&memory.saved) else {
            return;
        };
        cx.background_spawn(async move {
            if let Some(parent) = file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let temporary = file.with_extension("json.tmp");
            if let Err(error) =
                std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, &file))
            {
                tracing::warn!(%error, file = %file.display(), "layout was not saved");
            }
        })
        .detach();
    }
}
