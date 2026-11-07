use crate::{
    remote_discovery::{self, Destination},
    scheduler::{FileJob, QueuedJob, UploadJob, Work},
};
use nocterm_session::{
    EntryKind, FileMetadata,
    fs::path,
    local_fs::{LocalCursor, LocalDirectory, LocalFsError},
};
use nocterm_transfers::{
    DiscoveryBudget, Request, TransferCause, TransferEvent, TransferIssue, TransferOperation,
    TransferPath,
};
use std::{ffi::OsString, path::PathBuf, sync::Arc};
use tokio::sync::mpsc;

pub(crate) async fn run(work: Work, sender: mpsc::Sender<QueuedJob>) {
    match work.batch().request().clone() {
        Request::Upload(request) => {
            let mut discovery = UploadDiscovery::new(work.clone(), sender);
            for source in request.sources {
                if work.batch().cancelled() {
                    break;
                }
                if let Err(cause) = discovery
                    .root(source.clone(), request.destination.clone())
                    .await
                {
                    work.fail(
                        Some(TransferPath::Local(source)),
                        TransferOperation::Discover,
                        cause,
                    );
                }
            }
        }
        Request::Relay(request) => {
            remote_discovery::run(
                work.clone(),
                sender,
                request.source_fs,
                request.sources,
                Destination::Remote {
                    fs: request.fs,
                    path: request.destination,
                },
            )
            .await
        }
        Request::Download(request) => {
            let destination = request.local_destination;
            let local = work.state().local().clone();
            let opened = work
                .state()
                .blocking()
                .call(work.cancel(), move || local.open_directory(&destination))
                .await;
            match opened {
                Ok(directory) => {
                    remote_discovery::run(
                        work.clone(),
                        sender,
                        request.fs,
                        request.sources,
                        Destination::Local(directory),
                    )
                    .await
                }
                Err(cause) => work.fail(None, TransferOperation::Discover, cause),
            }
        }
    }
    work.batch().event(TransferEvent::DiscoveryComplete);
}
struct LocalFrame {
    directory: Arc<dyn LocalDirectory>,
    cursor: Box<dyn LocalCursor>,
    remote: String,
    depth: usize,
}
struct LocalNode {
    directory: Arc<dyn LocalDirectory>,
    name: OsString,
    remote: String,
    depth: usize,
}
struct UploadDiscovery {
    work: Work,
    sender: mpsc::Sender<QueuedJob>,
    budget: DiscoveryBudget,
    stack: Vec<LocalFrame>,
}
impl UploadDiscovery {
    fn new(work: Work, sender: mpsc::Sender<QueuedJob>) -> Self {
        let budget = DiscoveryBudget::new(work.state().limits().discovery());
        Self {
            work,
            sender,
            budget,
            stack: Vec::new(),
        }
    }
    async fn root(&mut self, source: PathBuf, destination: String) -> Result<(), TransferCause> {
        let local = self.work.state().local().clone();
        let (directory, name) = self
            .work
            .state()
            .blocking()
            .call(self.work.cancel(), move || {
                let source = local.resolve_source(&source)?;
                let name = source
                    .file_name()
                    .ok_or_else(|| LocalFsError::InvalidPath(source.clone()))?
                    .to_owned();
                let parent = source
                    .parent()
                    .ok_or_else(|| LocalFsError::InvalidPath(source.clone()))?;
                Ok((local.open_directory(parent)?, name))
            })
            .await?;
        let name_text = name.to_str().ok_or(TransferCause::InvalidName)?;
        let remote = path::join(&destination, name_text);
        self.node(LocalNode {
            directory,
            name,
            remote,
            depth: 0,
        })
        .await?;
        while let Some(node) = self.next().await? {
            let display = node.directory.location().join(&node.name);
            if let Err(cause) = self.node(node).await {
                if matches!(
                    cause,
                    TransferCause::Limit(nocterm_transfers::LimitKind::Visited)
                ) {
                    self.stack.clear();
                    return Err(cause);
                }
                self.work.fail(
                    Some(TransferPath::Local(display)),
                    TransferOperation::Discover,
                    cause,
                );
            }
        }
        Ok(())
    }
    async fn node(&mut self, node: LocalNode) -> Result<(), TransferCause> {
        self.work.check()?;
        self.budget.visit(node.depth)?;
        let text = node.name.to_str().ok_or(TransferCause::InvalidName)?;
        if !nocterm_session::fs::valid_publication_name(text) {
            return Err(TransferCause::InvalidName);
        }
        let parent = node.directory.clone();
        let name = node.name.clone();
        let metadata = self
            .work
            .state()
            .blocking()
            .call(self.work.cancel(), move || parent.metadata(&name))
            .await?
            .ok_or_else(|| {
                TransferCause::Local(LocalFsError::NotFound(
                    node.directory.location().join(&node.name),
                ))
            })?;
        if special(&metadata) {
            self.skip(TransferPath::Local(
                node.directory.location().join(&node.name),
            ));
            return Ok(());
        }
        match metadata.kind {
            EntryKind::File => self.file(node, metadata).await,
            EntryKind::Directory => self.directory(node).await,
            EntryKind::Other => Ok(()),
        }
    }
    async fn file(&mut self, node: LocalNode, metadata: FileMetadata) -> Result<(), TransferCause> {
        let directory = node.directory;
        let name = node.name;
        let source = self
            .work
            .state()
            .blocking()
            .call(self.work.cancel(), move || directory.open_source(&name))
            .await?;
        self.work.batch().event(TransferEvent::DiscoveredFile {
            bytes: metadata.size.unwrap_or(0),
        });
        send(
            &self.work,
            &self.sender,
            FileJob::Upload(UploadJob::new(source, node.remote, metadata.size)),
        )
        .await
    }
    async fn directory(&mut self, node: LocalNode) -> Result<(), TransferCause> {
        self.work
            .remote(self.work.batch().request().fs().create_dir(&node.remote))
            .await?;
        self.work.batch().event(TransferEvent::DirectoryPrepared);
        let parent = node.directory;
        let name = node.name;
        let directory = self
            .work
            .state()
            .blocking()
            .call(self.work.cancel(), move || parent.child(&name))
            .await?;
        let for_cursor = directory.clone();
        let cursor = self
            .work
            .state()
            .blocking()
            .call(self.work.cancel(), move || for_cursor.entries())
            .await?;
        self.stack.push(LocalFrame {
            directory,
            cursor,
            remote: node.remote,
            depth: node.depth + 1,
        });
        Ok(())
    }
    async fn next(&mut self) -> Result<Option<LocalNode>, TransferCause> {
        while let Some(mut frame) = self.stack.pop() {
            let mut cursor = frame.cursor;
            let (returned, result) = self
                .work
                .state()
                .blocking()
                .call(self.work.cancel(), move || {
                    let result = cursor.next();
                    Ok((cursor, result))
                })
                .await?;
            frame.cursor = returned;
            match result {
                Ok(Some(name)) => {
                    let Some(text) = name.to_str() else {
                        self.budget.visit(frame.depth)?;
                        self.work.fail(
                            Some(TransferPath::Local(frame.directory.location().join(&name))),
                            TransferOperation::Discover,
                            TransferCause::InvalidName,
                        );
                        self.stack.push(frame);
                        continue;
                    };
                    let node = LocalNode {
                        directory: frame.directory.clone(),
                        remote: path::join(&frame.remote, text),
                        name,
                        depth: frame.depth,
                    };
                    self.stack.push(frame);
                    return Ok(Some(node));
                }
                Ok(None) => {}
                Err(cause) => self.work.fail(
                    Some(TransferPath::Local(frame.directory.location())),
                    TransferOperation::Discover,
                    cause.into(),
                ),
            }
        }
        Ok(None)
    }
    fn skip(&self, path: TransferPath) {
        skip(&self.work, path);
    }
}
fn special(metadata: &FileMetadata) -> bool {
    metadata.is_symlink || metadata.kind == EntryKind::Other
}
pub(crate) fn skip(work: &Work, path: TransferPath) {
    work.batch()
        .event(TransferEvent::FileSkipped(Some(TransferIssue::new(
            Some(path),
            TransferOperation::Discover,
            TransferCause::SpecialFile,
        ))));
}
pub(crate) async fn send(
    work: &Work,
    sender: &mpsc::Sender<QueuedJob>,
    job: FileJob,
) -> Result<(), TransferCause> {
    let queued = work.queued(job).await?;
    tokio::select! {result=sender.send(queued)=>result.map_err(|_|TransferCause::Cancelled),()=work.cancelled()=>Err(TransferCause::Cancelled)}
}
