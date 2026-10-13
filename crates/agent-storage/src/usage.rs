//! Bounded native reads of provider account limits; the runtime schedules them.
mod parser;
#[cfg(test)]
mod tests;

use futures::future::BoxFuture;
use nocterm_ai::usage::{AccountUsageSource, RateLimit, UsageAgent, UsageReadError};
use parser::codex_limits;
use std::{
    io::{Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

const CODEX_TAIL_BYTES: u64 = 256 * 1024;

/// Reads Codex's latest account report under a home supplied by composition.
pub struct CodexUsageReader {
    home: Option<PathBuf>,
}
impl CodexUsageReader {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self { home }
    }
}
impl AccountUsageSource for CodexUsageReader {
    fn read(
        &self,
        agent: UsageAgent,
    ) -> BoxFuture<'static, Result<Option<Vec<RateLimit>>, UsageReadError>> {
        let home = self.home.clone();
        Box::pin(async move {
            let codex = std::iter::once(&agent.command)
                .chain(&agent.args)
                .any(|part| part.contains("codex"));
            match home.filter(|_| codex) {
                Some(home) => read_codex_limits(&home),
                None => Ok(None),
            }
        })
    }
}
fn read_codex_limits(home: &Path) -> Result<Option<Vec<RateLimit>>, UsageReadError> {
    let mut dir = home.join("sessions");
    for _ in 0..3 {
        let Some(next) = newest(&dir, |path| path.is_dir())? else {
            return Ok(None);
        };
        dir = next;
    }
    let Some(log) = newest(&dir, |path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
    })?
    else {
        return Ok(None);
    };
    let mut file = match std::fs::File::open(&log) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(read_error(&log, error)),
    };
    let length = file
        .metadata()
        .map_err(|error| read_error(&log, error))?
        .len();
    file.seek(SeekFrom::Start(length.saturating_sub(CODEX_TAIL_BYTES)))
        .map_err(|source| UsageReadError::Seek {
            path: log.clone(),
            source: Arc::new(source),
        })?;
    let mut tail = Vec::new();
    file.take(CODEX_TAIL_BYTES)
        .read_to_end(&mut tail)
        .map_err(|error| read_error(&log, error))?;
    Ok(String::from_utf8_lossy(&tail)
        .lines()
        .rev()
        .find_map(codex_limits))
}
fn newest(dir: &Path, keep: impl Fn(&Path) -> bool) -> Result<Option<PathBuf>, UsageReadError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(UsageReadError::Enumerate {
                path: dir.to_owned(),
                source: Arc::new(source),
            });
        }
    };
    Ok(entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| keep(path))
        .max_by(|a, b| a.file_name().cmp(&b.file_name())))
}
fn read_error(path: &Path, source: std::io::Error) -> UsageReadError {
    UsageReadError::Read {
        path: path.to_owned(),
        source: Arc::new(source),
    }
}
