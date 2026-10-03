//! Download discovery is depth-first; the existing RemoteFs listing contract
//! materializes one directory per level, capped at 256 levels. File data is
//! streamed separately with the same bounded worker pool as uploads.
mod local;

use crate::{Batch, CHUNK_SIZE, CollisionPolicy, Job, MAX_ERROR_DETAILS, Request, local_error};
use local::LocalDirectory;
use nocterm_session::{DirEntry, EntryKind, FsError, fs::path};
use std::{collections::HashSet, path::PathBuf, sync::Arc, time::Duration};
use tokio::{io::AsyncWriteExt, sync::mpsc};

const MAX_DEPTH: usize = 256;

pub(super) struct FileJob {
    remote: String,
    directory: LocalDirectory,
    name: String,
    expected_size: Option<u64>,
}

pub(super) fn valid_source(source: &str) -> bool {
    source.starts_with('/')
        && !source.contains('\0')
        && !source.split('/').any(|p| p == "." || p == "..")
        && valid_name(path::file_name(source))
}
fn valid_name(name: &str) -> bool {
    // Reject Windows prefixes/separators as well, even when running on Unix.
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0', ':'])
        && !name.ends_with(['.', ' '])
        && !matches!(
            name.split('.')
                .next()
                .unwrap_or("")
                .to_ascii_uppercase()
                .as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        )
}
fn skipped(batch: &Batch, message: String) {
    let mut p = batch.progress.lock();
    p.skipped_files += 1;
    if p.errors.len() < MAX_ERROR_DETAILS {
        p.errors.push(message);
    }
}

pub(super) async fn discover(batch: Arc<Batch>, send: mpsc::Sender<Job>) {
    let Request::Download(request) = &batch.request else {
        return;
    };
    let root = match LocalDirectory::open(&request.local_destination) {
        Ok(root) => root,
        Err(e) => {
            batch.error(e);
            batch.progress.lock().discovery_complete = true;
            return;
        }
    };
    let mut seen = HashSet::new();
    for source in &request.sources {
        if batch.cancelled() {
            break;
        }
        if !seen.insert(source.clone()) {
            continue;
        }
        let result = async {
            let entry = request
                .fs
                .stat(source)
                .await?
                .ok_or_else(|| FsError::NotFound {
                    path: source.clone(),
                })?;
            discover_one(
                &batch,
                &send,
                source.clone(),
                path::file_name(source).to_owned(),
                entry,
                root.clone(),
            )
            .await
        }
        .await;
        if let Err(e) = result {
            batch.error(e);
        }
    }
    batch.progress.lock().discovery_complete = true;
}

async fn discover_one(
    batch: &Batch,
    send: &mpsc::Sender<Job>,
    remote: String,
    name: String,
    entry: DirEntry,
    directory: LocalDirectory,
) -> Result<(), FsError> {
    let mut current = Some((remote, name, entry, directory));
    let mut stack: Vec<(String, LocalDirectory, std::vec::IntoIter<DirEntry>)> = Vec::new();
    loop {
        if batch.cancelled() {
            return Ok(());
        }
        if let Some((remote, name, entry, directory)) = current.take() {
            if !valid_name(&name) {
                batch.error(format!("{remote}: unsafe remote filename"));
            } else if entry.is_symlink || entry.kind == EntryKind::Other {
                skipped(batch, format!("{remote}: skipped link or special file"));
            } else if entry.kind == EntryKind::File {
                {
                    let mut p = batch.progress.lock();
                    p.discovered_files += 1;
                    p.total_bytes = p.total_bytes.saturating_add(entry.size.unwrap_or(0));
                }
                send.send(Job::Download(FileJob {
                    remote,
                    directory,
                    name,
                    expected_size: entry.size,
                }))
                .await
                .map_err(|_| FsError::Other("download discovery cancelled".into()))?;
            } else if stack.len() >= MAX_DEPTH {
                batch.error(format!("{remote}: directory depth exceeds {MAX_DEPTH}"));
            } else {
                match directory.child(&name) {
                    Ok(child) => match batch.request.fs().read_dir(&remote).await {
                        Ok(entries) => stack.push((remote, child, entries.into_iter())),
                        Err(e) => batch.error(e),
                    },
                    Err(e) => batch.error(e),
                }
            }
        }
        loop {
            let Some((parent, directory, entries)) = stack.last_mut() else {
                return Ok(());
            };
            if let Some(entry) = entries.next() {
                if !valid_name(&entry.name) {
                    batch.error(format!("{parent}: unsafe remote filename {:?}", entry.name));
                    continue;
                }
                current = Some((
                    path::join(parent, &entry.name),
                    entry.name.clone(),
                    entry,
                    directory.clone(),
                ));
                break;
            }
            stack.pop();
        }
    }
}

struct Reservation {
    path: PathBuf,
    active: Arc<parking_lot::Mutex<HashSet<PathBuf>>>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.active.lock().remove(&self.path);
    }
}
async fn reserve(batch: &Batch, job: &FileJob) -> Result<Option<(String, Reservation)>, FsError> {
    let mut number = 0;
    loop {
        if batch.cancelled() {
            return Err(FsError::Other("download cancelled".into()));
        }
        let name = if number == 0 {
            job.name.clone()
        } else {
            renamed(&job.name, number)
        };
        let destination = job.directory.path().join(&name);
        let exists = job.directory.exists(&name)?;
        if exists && batch.request.collisions() == CollisionPolicy::Skip {
            return Ok(None);
        }
        if (!exists || batch.request.collisions() == CollisionPolicy::Replace)
            && batch.local_destinations.lock().insert(destination.clone())
        {
            return Ok(Some((
                name,
                Reservation {
                    path: destination,
                    active: batch.local_destinations.clone(),
                },
            )));
        }
        match batch.request.collisions() {
            CollisionPolicy::Skip => return Ok(None),
            CollisionPolicy::Rename => {
                number += 1;
                if number > 10_000 {
                    return Err(FsError::Other(
                        "too many destination name collisions".into(),
                    ));
                }
            }
            CollisionPolicy::Replace => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
}
fn renamed(name: &str, number: u32) -> String {
    let (stem, extension) = name
        .rsplit_once('.')
        .filter(|(s, _)| !s.is_empty())
        .map_or((name, String::new()), |(s, e)| (s, format!(".{e}")));
    format!("{stem} ({number}){extension}")
}
pub(super) async fn download_file(batch: &Batch, job: FileJob) -> Result<bool, FsError> {
    let Some((mut name, mut reservation)) = reserve(batch, &job).await? else {
        return Ok(false);
    };
    let mut temporary = job.directory.stage()?;
    let mut received = 0_u64;
    let mut output = tokio::fs::File::from_std(
        temporary
            .file()
            .try_clone()
            .map_err(|e| local_error(job.directory.path(), e))?,
    );
    let mut reader = batch.request.fs().download(&job.remote).await?;
    loop {
        if batch.cancelled() {
            return Err(FsError::Other("download cancelled".into()));
        }
        let bytes = reader.read(CHUNK_SIZE).await?;
        if bytes.len() > CHUNK_SIZE {
            return Err(FsError::Other(
                "remote reader exceeded requested chunk size".into(),
            ));
        }
        if bytes.is_empty() {
            if job.expected_size.is_some_and(|size| size != received) {
                return Err(FsError::Other(
                    "remote file size changed during download".into(),
                ));
            }
            break;
        }
        received = received.saturating_add(bytes.len() as u64);
        if job.expected_size.is_some_and(|size| received > size) {
            return Err(FsError::Other("remote file grew during download".into()));
        }
        output
            .write_all(&bytes)
            .await
            .map_err(|e| local_error(job.directory.path(), e))?;
        let mut p = batch.progress.lock();
        p.sent_bytes = p.sent_bytes.saturating_add(bytes.len() as u64);
    }
    reader.close().await?;
    output
        .sync_all()
        .await
        .map_err(|e| local_error(job.directory.path(), e))?;
    drop(output);
    if batch.cancelled() {
        return Err(FsError::Other("download cancelled".into()));
    }
    // A new external collision after streaming cannot clobber an existing file.
    // Rename reuses the already synced private temporary without rereading data.
    for _ in 0..10_000 {
        if batch.cancelled() {
            return Err(FsError::Other("download cancelled".into()));
        }
        match temporary.publish(
            &name,
            batch.request.collisions() == CollisionPolicy::Replace,
        ) {
            Ok(()) => return Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                match batch.request.collisions() {
                    CollisionPolicy::Skip => return Ok(false),
                    CollisionPolicy::Rename => {
                        let Some((next, next_reservation)) = reserve(batch, &job).await? else {
                            return Ok(false);
                        };
                        name = next;
                        reservation = next_reservation;
                    }
                    CollisionPolicy::Replace => {
                        return Err(local_error(&job.directory.path().join(&name), e));
                    }
                }
            }
            Err(e) => return Err(local_error(&job.directory.path().join(&name), e)),
        }
    }
    drop(reservation);
    Err(FsError::Other(
        "too many concurrent destination collisions".into(),
    ))
}

#[cfg(unix)]
pub(super) fn open_local_source(path: &std::path::Path) -> Result<std::fs::File, FsError> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| local_error(path, e))?
            .join(path)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| FsError::Other("source has no parent".into()))?;
    let name = absolute
        .file_name()
        .ok_or_else(|| FsError::Other("source has no filename".into()))?;
    LocalDirectory::open(parent)?
        .read_file(name)
        .map_err(|e| local_error(path, e))
}
