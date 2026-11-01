//! Walks the source tree of a relay on the source filesystem and prepares the
//! matching directories on the destination filesystem.
use crate::{
    discovery::{send, skip},
    scheduler::{FileJob, RelayJob, Work},
};
use nocterm_session::{
    DirEntry, EntryKind, FsError, RemoteFs,
    fs::{path, valid_publication_name},
};
use nocterm_transfers::{
    DiscoveryBudget, LimitKind, RelayRequest, TransferCause, TransferEvent, TransferOperation,
    TransferPath,
};
use std::sync::Arc;
use tokio::sync::mpsc;

pub(crate) async fn run(work: &Work, sender: &mpsc::Sender<FileJob>, request: RelayRequest) {
    let mut discovery =
        RelayDiscovery::new(work.clone(), sender.clone(), request.source_fs, request.fs);
    for source in request.sources {
        if work.batch().cancelled() {
            break;
        }
        if let Err(cause) = discovery.root(source.clone(), &request.destination).await {
            work.fail(
                Some(TransferPath::Remote(source)),
                TransferOperation::Discover,
                cause,
            );
        }
    }
}
struct Frame {
    source: String,
    remote: String,
    entries: std::vec::IntoIter<DirEntry>,
    depth: usize,
}
struct Node {
    source: String,
    remote: String,
    entry: DirEntry,
    depth: usize,
}
struct RelayDiscovery {
    work: Work,
    sender: mpsc::Sender<FileJob>,
    source_fs: Arc<dyn RemoteFs>,
    destination_fs: Arc<dyn RemoteFs>,
    budget: DiscoveryBudget,
    stack: Vec<Frame>,
}
impl RelayDiscovery {
    fn new(
        work: Work,
        sender: mpsc::Sender<FileJob>,
        source_fs: Arc<dyn RemoteFs>,
        destination_fs: Arc<dyn RemoteFs>,
    ) -> Self {
        let budget = DiscoveryBudget::new(work.state().limits().discovery());
        Self {
            work,
            sender,
            source_fs,
            destination_fs,
            budget,
            stack: Vec::new(),
        }
    }
    async fn root(&mut self, source: String, destination: &str) -> Result<(), TransferCause> {
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
        let remote = path::join(destination, &entry.name);
        self.node(Node {
            source,
            remote,
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
        if !valid_publication_name(&node.entry.name) {
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
            let job = RelayJob::new(
                self.source_fs.clone(),
                node.source,
                node.remote,
                node.entry.size,
            );
            send(&self.work, &self.sender, FileJob::Relay(job)).await
        } else {
            self.directory(node).await
        }
    }
    async fn directory(&mut self, node: Node) -> Result<(), TransferCause> {
        self.work
            .remote(self.destination_fs.create_dir(&node.remote))
            .await?;
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
            remote: node.remote,
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
                    remote: path::join(&frame.remote, &entry.name),
                    entry,
                    depth: frame.depth,
                });
            }
            self.stack.pop();
        }
        None
    }
}
