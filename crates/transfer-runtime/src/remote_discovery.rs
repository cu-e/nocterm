//! One bounded DFS over a remote source, preserving each destination's authority.
use crate::{
    discovery::{send, skip},
    scheduler::{DownloadJob, FileJob, QueuedJob, RelayJob, Work},
};
use nocterm_session::{
    DirEntry, EntryKind, FsError, RemoteFs,
    fs::{path, valid_publication_name},
    local_fs::LocalDirectory,
};
use nocterm_transfers::{
    DiscoveryBudget, LimitKind, TransferCause, TransferEvent, TransferOperation, TransferPath,
    valid_download_name,
};
use std::{ffi::OsString, sync::Arc};
use tokio::sync::mpsc;

/// The already admitted parent, never recovered from a display path.
pub(crate) enum Destination {
    Local(Arc<dyn LocalDirectory>),
    Remote { fs: Arc<dyn RemoteFs>, path: String },
}
impl Destination {
    fn for_entry(&self, name: &str) -> Self {
        match self {
            Self::Local(directory) => Self::Local(directory.clone()),
            Self::Remote { fs, path: parent } => Self::Remote {
                fs: fs.clone(),
                path: path::join(parent, name),
            },
        }
    }
    fn valid_name(&self, name: &str) -> bool {
        match self {
            Self::Local(_) => valid_download_name(name),
            Self::Remote { .. } => valid_publication_name(name),
        }
    }
    async fn child(self, work: &Work, name: String) -> Result<Self, TransferCause> {
        match self {
            Self::Local(directory) => work
                .state()
                .blocking()
                .call(work.cancel(), move || {
                    directory.create_child(&OsString::from(name))
                })
                .await
                .map(Self::Local),
            Self::Remote { fs, path } => {
                work.remote(fs.create_dir(&path)).await?;
                Ok(Self::Remote { fs, path })
            }
        }
    }
    fn job(self, source_fs: Arc<dyn RemoteFs>, source: String, entry: DirEntry) -> FileJob {
        match self {
            Self::Local(directory) => FileJob::Download(DownloadJob::new(
                source,
                directory,
                entry.name.into(),
                entry.size,
            )),
            Self::Remote { path, .. } => {
                FileJob::Relay(RelayJob::new(source_fs, source, path, entry.size))
            }
        }
    }
}
pub(crate) async fn run(
    work: Work,
    sender: mpsc::Sender<QueuedJob>,
    source_fs: Arc<dyn RemoteFs>,
    sources: Vec<String>,
    destination: Destination,
) {
    let budget = DiscoveryBudget::new(work.state().limits().discovery());
    let mut discovery = RemoteDiscovery {
        work,
        sender,
        source_fs,
        budget,
        stack: Vec::new(),
    };
    for source in sources {
        if discovery.work.batch().cancelled() {
            break;
        }
        if let Err(cause) = discovery.root(source.clone(), &destination).await {
            discovery.work.fail(
                Some(TransferPath::Remote(source)),
                TransferOperation::Discover,
                cause,
            );
        }
    }
}
struct Frame {
    source: String,
    destination: Destination,
    entries: std::vec::IntoIter<DirEntry>,
    depth: usize,
}
struct Node {
    source: String,
    destination: Destination,
    entry: DirEntry,
    depth: usize,
}
struct RemoteDiscovery {
    work: Work,
    sender: mpsc::Sender<QueuedJob>,
    source_fs: Arc<dyn RemoteFs>,
    budget: DiscoveryBudget,
    stack: Vec<Frame>,
}
impl RemoteDiscovery {
    async fn root(
        &mut self,
        source: String,
        destination: &Destination,
    ) -> Result<(), TransferCause> {
        let entry = self
            .work
            .remote(self.source_fs.stat(&source))
            .await?
            .ok_or_else(|| {
                TransferCause::Remote(FsError::NotFound {
                    path: source.clone(),
                })
            })?;
        let entry = DirEntry {
            name: path::file_name(&source).to_owned(),
            ..entry
        };
        let destination = destination.for_entry(&entry.name);
        self.node(Node {
            source,
            destination,
            entry,
            depth: 0,
        })
        .await?;
        while let Some(node) = self.next() {
            let display = node.source.clone();
            if let Err(cause) = self.node(node).await {
                if matches!(cause, TransferCause::Limit(LimitKind::Visited)) {
                    self.stack.clear();
                    return Err(cause);
                }
                self.work.fail(
                    Some(TransferPath::Remote(display)),
                    TransferOperation::Discover,
                    cause,
                );
            }
        }
        Ok(())
    }
    async fn node(&mut self, node: Node) -> Result<(), TransferCause> {
        self.work.check()?;
        self.budget.visit(node.depth)?;
        if !node.destination.valid_name(&node.entry.name) {
            return Err(TransferCause::InvalidName);
        }
        if node.entry.is_symlink || node.entry.kind == EntryKind::Other {
            skip(&self.work, TransferPath::Remote(node.source));
            return Ok(());
        }
        if node.entry.kind == EntryKind::File {
            self.work.batch().event(TransferEvent::DiscoveredFile {
                bytes: node.entry.size.unwrap_or(0),
            });
            let job = node
                .destination
                .job(self.source_fs.clone(), node.source, node.entry);
            send(&self.work, &self.sender, job).await
        } else {
            self.directory(node).await
        }
    }
    async fn directory(&mut self, node: Node) -> Result<(), TransferCause> {
        let destination = node.destination.child(&self.work, node.entry.name).await?;
        self.work.batch().event(TransferEvent::DirectoryPrepared);
        let entries = self
            .work
            .remote(self.source_fs.read_dir(&node.source))
            .await?;
        let bytes = entries.iter().fold(0usize, |total, entry| {
            total.saturating_add(entry.name.len())
        });
        self.budget.hold_listing(entries.len(), bytes)?;
        self.stack.push(Frame {
            source: node.source,
            destination,
            entries: entries.into_iter(),
            depth: node.depth + 1,
        });
        Ok(())
    }
    fn next(&mut self) -> Option<Node> {
        while let Some(frame) = self.stack.last_mut() {
            if let Some(entry) = frame.entries.next() {
                self.budget.consumed_name(entry.name.len());
                return Some(Node {
                    source: path::join(&frame.source, &entry.name),
                    destination: frame.destination.for_entry(&entry.name),
                    entry,
                    depth: frame.depth,
                });
            }
            self.stack.pop();
        }
        None
    }
}
