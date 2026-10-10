//! Failures while validating a shortcut or persisting the desired keymap.
use gpui_kit::InvalidKeystrokeError;
use nocterm_core::persist::PersistError;

#[derive(Debug, thiserror::Error)]
pub enum KeymapError {
    #[error("no keystroke")]
    EmptySequence,
    #[error("{}: {source}", .source.keystroke)]
    InvalidStroke {
        index: usize,
        #[source]
        source: InvalidKeystrokeError,
    },
    #[error("{0}")]
    Persistence(#[from] PersistError),
}
