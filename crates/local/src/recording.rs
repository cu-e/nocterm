//! Exclusive pinned output files. The shared persistence service schedules operations.
use futures::{FutureExt, future::BoxFuture};
use nocterm_session::recording::{
    AppendBlock, AppendOutcome, OutputLog, OutputLogFactory, PhysicalPrefix, RecordingError,
    RecordingOptions, SealOutcome,
};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub struct LocalOutputLogFactory {
    directory: PathBuf,
}
impl LocalOutputLogFactory {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
}
impl OutputLogFactory for LocalOutputLogFactory {
    fn create(
        &self,
        options: RecordingOptions,
    ) -> BoxFuture<'static, Result<Arc<dyn OutputLog>, RecordingError>> {
        let directory = options.directory.unwrap_or_else(|| self.directory.clone());
        async move {
            let path = directory.join(format!("session-{}.log", uuid::Uuid::new_v4()));
            create_at(
                path,
                u64::from(options.max_file_mib.clamp(1, 1024)) * 1024 * 1024,
            )
            .map(|log| Arc::new(log) as Arc<dyn OutputLog>)
        }
        .boxed()
    }
}
fn create_at(path: PathBuf, limit: u64) -> Result<LocalOutputLog, RecordingError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| RecordingError::Create {
            path: path.clone(),
            source: Arc::new(source),
        })?;
    }
    let file =
        nocterm_core::fs::create_private(&path).map_err(|source| RecordingError::Create {
            path: path.clone(),
            source: Arc::new(source.into_io_error()),
        })?;
    Ok(LocalOutputLog {
        path: path.clone(),
        state: Arc::new(Mutex::new(AppendFile::new(file, path, limit))),
    })
}
struct LocalOutputLog {
    path: PathBuf,
    state: Arc<Mutex<AppendFile<File>>>,
}
impl OutputLog for LocalOutputLog {
    fn path(&self) -> &Path {
        &self.path
    }
    fn append(&self, block: AppendBlock) -> BoxFuture<'static, AppendOutcome> {
        let state = self.state.clone();
        async move { lock(&state).append(block) }.boxed()
    }
    fn seal(&self) -> BoxFuture<'static, SealOutcome> {
        let state = self.state.clone();
        async move { lock(&state).seal() }.boxed()
    }
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
trait DurableWrite: Write + Send {
    fn sync(&mut self) -> io::Result<()>;
}
impl DurableWrite for File {
    fn sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}
/// Owns one descriptor and bounded last-operation metadata, never reopens its name.
struct AppendFile<W> {
    writer: Option<W>,
    path: PathBuf,
    prefix: PhysicalPrefix,
    limit: u64,
    last: Option<(AppendBlock, AppendOutcome)>,
    sealed: Option<SealOutcome>,
    failure: Option<RecordingError>,
}
impl<W: DurableWrite> AppendFile<W> {
    fn new(writer: W, path: PathBuf, limit: u64) -> Self {
        Self {
            writer: Some(writer),
            path,
            prefix: PhysicalPrefix::default(),
            limit,
            last: None,
            sealed: None,
            failure: None,
        }
    }
    fn append(&mut self, block: AppendBlock) -> AppendOutcome {
        if self.prefix.closed() {
            return AppendOutcome::new(self.prefix.clone(), Some(RecordingError::Closing));
        }
        if self.last.as_ref().is_some_and(|(last, _)| last == &block) {
            return self.last.as_ref().unwrap().1.clone();
        }
        if let Some(error) = &self.failure {
            return AppendOutcome::new(self.prefix.clone(), Some(error.clone()));
        }
        let error = self.validate(&block).err();
        if let Some(error) = error {
            return self.reject(block, error);
        }
        let mut partial = 0;
        let mut error = None;
        if let Some(writer) = self.writer.as_mut() {
            while partial < block.bytes().len() {
                match writer.write(&block.bytes()[partial..]) {
                    Ok(0) => {
                        error = Some(io::ErrorKind::WriteZero.into());
                        break;
                    }
                    Ok(count) => partial += count,
                    Err(source) if source.kind() == io::ErrorKind::Interrupted => {}
                    Err(source) => {
                        error = Some(source);
                        break;
                    }
                }
            }
        }
        let written = self.prefix.written_bytes() + partial as u64;
        let complete = error.is_none();
        self.prefix = PhysicalPrefix::new(
            if complete {
                block.sequence()
            } else {
                self.prefix.complete_sequence()
            },
            written,
            if complete { 0 } else { partial as u64 },
            false,
            false,
        );
        let error = error.map(|source| RecordingError::Append {
            path: self.path.clone(),
            offset: written,
            source: Arc::new(source),
        });
        self.failure = error.clone();
        let outcome = AppendOutcome::new(self.prefix.clone(), error);
        self.last = Some((block, outcome.clone()));
        outcome
    }
    fn validate(&self, block: &AppendBlock) -> Result<(), RecordingError> {
        if self.prefix.complete_sequence().checked_add(1) != Some(block.sequence())
            || block.expected_offset() != self.prefix.written_bytes()
        {
            return Err(RecordingError::SequenceMismatch);
        }
        if self
            .prefix
            .written_bytes()
            .checked_add(block.bytes().len() as u64)
            .is_none_or(|bytes| bytes > self.limit)
        {
            return Err(RecordingError::Limit);
        }
        Ok(())
    }
    fn reject(&mut self, block: AppendBlock, error: RecordingError) -> AppendOutcome {
        self.failure = Some(error.clone());
        let outcome = AppendOutcome::new(self.prefix.clone(), Some(error));
        self.last = Some((block, outcome.clone()));
        outcome
    }
    fn seal(&mut self) -> SealOutcome {
        if let Some(outcome) = &self.sealed {
            return outcome.clone();
        }
        let mut synced = false;
        let mut error = None;
        if let Some(mut writer) = self.writer.take() {
            match writer.flush() {
                Err(source) => {
                    error = Some(RecordingError::Flush {
                        path: self.path.clone(),
                        source: Arc::new(source),
                    })
                }
                Ok(()) => match writer.sync() {
                    Err(source) => {
                        error = Some(RecordingError::Sync {
                            path: self.path.clone(),
                            source: Arc::new(source),
                        })
                    }
                    Ok(()) => synced = true,
                },
            }
            drop(writer);
        }
        self.prefix = PhysicalPrefix::new(
            self.prefix.complete_sequence(),
            self.prefix.written_bytes(),
            self.prefix.partial_bytes(),
            synced,
            true,
        );
        let outcome = AppendOutcome::new(self.prefix.clone(), self.failure.clone().or(error));
        self.sealed = Some(outcome.clone());
        outcome
    }
}
#[cfg(test)]
mod tests;
