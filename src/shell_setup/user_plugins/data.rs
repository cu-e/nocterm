//! A plugin's values and storage, one private file `<root>/<plugin-id>.json` each, written
//! through the keyed persistence service so a change is coalesced and flushed at shutdown.
use std::{num::NonZeroUsize, path::PathBuf, sync::Arc};

use gpui_kit::{App, AppContext as _, Entity};
use nocterm_core::fs::{BoundedReadError, read_regular_bounded, write_private_atomic};
use nocterm_plugin_api::Cause;
use nocterm_plugin_host::{
    DocumentError, PluginData, PluginDataChange, PluginDataDoc, PluginDataError,
};
use nocterm_plugin_model::PluginId;
use nocterm_store::{KeyedPersisted, ShutdownCoordinator, StoreError};

use super::is_missing;

/// Largest file read: the document bounds with room for JSON escapes.
pub(super) const MAX_FILE_BYTES: usize = 512 * 1024;
/// Plugins whose changes may wait for the worker at once.
const PENDING_PLUGINS: usize = 16;

type Writer = Entity<KeyedPersisted<PluginId, PluginDataChange, ()>>;

/// The files of all plugins' data under one directory.
pub(crate) struct PluginDataFiles {
    root: PathBuf,
    writer: Writer,
}

impl PluginDataFiles {
    pub(crate) fn new(root: PathBuf, cx: &mut App) -> Self {
        let target = root.clone();
        let writer = cx.new(|_| {
            KeyedPersisted::new(
                Arc::new(move |plugin: PluginId, change| {
                    let root = target.clone();
                    Box::pin(async move { publish(&root, &plugin, &change) })
                }),
                NonZeroUsize::new(PENDING_PLUGINS).expect("nonzero constant"),
            )
        });
        Self { root, writer }
    }

    /// Flushes the pending changes when the application closes.
    pub(crate) fn register_shutdown(
        &self,
        shutdown: &ShutdownCoordinator,
    ) -> Result<(), StoreError> {
        let writer = self.writer.clone();
        shutdown.register_capture(move |cx| {
            let captured = writer.update(cx, |writer, _| writer.capture());
            Box::pin(async move {
                let failure = captured
                    .run()
                    .await
                    .into_iter()
                    .filter_map(|outcome| outcome.result().err())
                    .reduce(|first, second| StoreError::joined(Arc::new(first), Arc::new(second)));
                failure.map_or(Ok(()), Err)
            })
        })
    }
}

/// `<root>/<plugin>.json`, for a user plugin id that cannot name anything else.
fn file_of(root: &std::path::Path, plugin: &PluginId) -> Option<PathBuf> {
    if !plugin.is_user() || plugin.check().is_err() {
        return None;
    }
    Some(root.join(format!("{plugin}.json")))
}

fn publish(
    root: &std::path::Path,
    plugin: &PluginId,
    change: &PluginDataChange,
) -> Result<(), StoreError> {
    let file = file_of(root, plugin).ok_or(StoreError::Closing)?;
    match change {
        PluginDataChange::Replace(doc) => {
            let bytes = doc.encode().map_err(StoreError::backend)?;
            write_private_atomic(&file, &bytes).map_err(StoreError::backend)
        }
        PluginDataChange::Remove => match std::fs::remove_file(&file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(StoreError::backend(error))
            }
            Ok(()) | Err(_) => Ok(()),
        },
    }
}

impl PluginData for PluginDataFiles {
    fn load(&self, plugin: &PluginId, _: &App) -> Result<PluginDataDoc, PluginDataError> {
        let file = file_of(&self.root, plugin)
            .ok_or_else(|| PluginDataError::NotAPluginId(plugin.clone()))?;
        match read_regular_bounded(&file, MAX_FILE_BYTES) {
            Ok(bytes) => PluginDataDoc::decode(&bytes).map_err(|error| match error {
                DocumentError::Bounds(bounds) => PluginDataError::Bounds(bounds),
                syntax => PluginDataError::Broken(Cause::new(syntax)),
            }),
            Err(error) if is_missing(&error) => Ok(PluginDataDoc::default()),
            Err(
                error @ (BoundedReadError::NotRegular { .. } | BoundedReadError::TooLarge { .. }),
            ) => Err(PluginDataError::Broken(Cause::new(error))),
            Err(error) => Err(PluginDataError::Unreadable(Cause::new(error))),
        }
    }

    fn change(
        &self,
        plugin: PluginId,
        change: PluginDataChange,
        cx: &mut App,
    ) -> Result<(), PluginDataError> {
        if file_of(&self.root, &plugin).is_none() {
            return Err(PluginDataError::NotAPluginId(plugin));
        }
        self.writer
            .update(cx, |writer, cx| {
                writer.update_latest(plugin.clone(), change, (), cx)
            })
            .map(|_| ())
            .map_err(|error| PluginDataError::NotSaved(Cause::new(error)))
    }
}
