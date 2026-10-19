use crate::directory::DirectoryCapability;
use nocterm_session::local_fs::*;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

enum Step {
    Finished,
    Skip,
    Stop,
    Child(Arc<dyn LocalDirectory>),
}
struct Frame {
    directory: Arc<dyn LocalDirectory>,
    cursor: Box<dyn LocalCursor>,
    depth: usize,
}
impl Frame {
    fn new(directory: Arc<dyn LocalDirectory>, depth: usize) -> LocalResult<Self> {
        let cursor = directory.entries()?;
        Ok(Self {
            directory,
            cursor,
            depth,
        })
    }
    fn next(&mut self, count: &mut StatisticsAccumulator) -> LocalResult<Step> {
        let Some(name) = self.cursor.next()? else {
            return Ok(Step::Finished);
        };
        let path = self.directory.location().join(&name);
        let metadata = match self.directory.metadata(&name) {
            Ok(Some(metadata)) => metadata,
            Ok(None) => {
                return Ok(failed_entry(
                    count,
                    path.clone(),
                    LocalFsError::NotFound(path),
                ));
            }
            Err(error) => {
                return Ok(failed_entry(count, path, error));
            }
        };
        match count.observe(&metadata, self.depth + 1, self.depth == 0, &name) {
            ScanDecision::Enter => match self.directory.child(&name) {
                Ok(child) => Ok(Step::Child(child)),
                Err(error) => {
                    count.record_failure(path, error);
                    Ok(Step::Skip)
                }
            },
            ScanDecision::Skip => Ok(Step::Skip),
            ScanDecision::Stop => Ok(Step::Stop),
        }
    }
}
fn failed_entry(
    count: &mut StatisticsAccumulator,
    path: std::path::PathBuf,
    error: LocalFsError,
) -> Step {
    match count.observe_entry_failure(path, error) {
        ScanDecision::Stop => Step::Stop,
        ScanDecision::Enter | ScanDecision::Skip => Step::Skip,
    }
}
pub(crate) fn scan(
    path: &Path,
    policy: &ScanPolicy,
    cancel: &CancelFlag,
    progress: &dyn StatisticsObserver,
) -> LocalResult<Statistics> {
    let mut count = StatisticsAccumulator::new(policy.clone());
    if cancel.is_cancelled() {
        let result = count.cancelled();
        progress.update(result.clone());
        return Ok(result);
    }
    let root =
        match DirectoryCapability::strict(path).and_then(|root| Frame::new(Arc::new(root), 0)) {
            Ok(frame) => frame,
            Err(error) => {
                count.record_failure(path.to_owned(), error);
                let result = count.finish();
                progress.update(result.clone());
                return Ok(result);
            }
        };
    let mut stack = vec![root];
    let mut published = Instant::now();
    while let Some(frame) = stack.last_mut() {
        if cancel.is_cancelled() {
            let result = count.cancelled();
            progress.update(result.clone());
            return Ok(result);
        }
        let next = frame.next(&mut count);
        match next {
            Ok(Step::Finished) => {
                stack.pop();
            }
            Ok(Step::Child(child)) => {
                let depth = frame.depth + 1;
                let child_path = child.location();
                match Frame::new(child, depth) {
                    Ok(frame) => stack.push(frame),
                    Err(error) => count.record_failure(child_path, error),
                }
            }
            Ok(Step::Skip) => {}
            Ok(Step::Stop) => break,
            Err(error) => {
                count.record_failure(frame.directory.location(), error);
                stack.pop();
            }
        }
        if published.elapsed() >= Duration::from_millis(150) {
            progress.update(count.snapshot());
            published = Instant::now();
        }
    }
    let result = if cancel.is_cancelled() {
        count.cancelled()
    } else {
        count.finish()
    };
    progress.update(result.clone());
    Ok(result)
}
