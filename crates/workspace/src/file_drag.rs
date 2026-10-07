//! File paths carried between independent Explorer and terminal features.
use nocterm_session::{RemoteFs, Target};
use std::{path::PathBuf, sync::Arc};

/// An internal file drag. Remote payloads retain their original connection so
/// switching tabs during a drag cannot redirect a transfer.
#[derive(Clone)]
pub enum FileDrag {
    Local(Vec<PathBuf>),
    Remote(RemoteFileDrag),
}

/// Remote paths and the filesystem on which they exist.
#[derive(Clone)]
pub struct RemoteFileDrag {
    pub sources: Vec<String>,
    pub target: Target,
    pub fs: Arc<dyn RemoteFs>,
}
