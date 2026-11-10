//! The paperclip: files of any kind for the next message. Images among them are attached as
//! images the agent sees; every other file as a link to where it is on this computer.
use nocterm_ai::prompt_files::{PromptFile, PromptFileError, is_image};
use std::path::PathBuf;

/// A pick delivered only to the chat and composer revision it was started for.
pub(super) struct AttachPick {
    chat: Option<gpui_kit::EntityId>,
    revision: u64,
}

impl AttachPick {
    pub(super) fn new(chat: Option<gpui_kit::EntityId>, revision: u64) -> Self {
        Self { chat, revision }
    }
    pub(super) fn current(&self, chat: Option<gpui_kit::EntityId>, revision: u64) -> bool {
        self.chat == chat && self.revision == revision
    }
}

/// `paths` split into the images (when the agent takes images) and the other files.
pub(super) fn split(paths: Vec<PathBuf>, images_taken: bool) -> (Vec<PathBuf>, Vec<PathBuf>) {
    paths
        .into_iter()
        .partition(|path| images_taken && is_image(path))
}

/// Links to `files` with their sizes. Reads file metadata: run it off the UI thread.
pub(super) fn link(files: &[PathBuf]) -> Result<Vec<PromptFile>, PromptFileError> {
    files
        .iter()
        .map(|path| {
            let size = std::fs::metadata(path).map_or(0, |meta| meta.len());
            PromptFile::link(path, size)
        })
        .collect()
}
