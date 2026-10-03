use nocterm_session::{DirEntry, EntryKind, FsError, RemoteFs};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct Browser {
    pub(crate) generation: u64,
    pub(crate) path: Option<String>,
    pub(crate) entries: Vec<DirEntry>,
    pub(crate) loading: bool,
    pub(crate) error: Option<FsError>,
}

impl Browser {
    pub(crate) fn begin(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.loading = true;
        self.error = None;
        self.generation
    }

    pub(crate) fn clear(&mut self) {
        self.begin();
        self.path = None;
        self.entries.clear();
        self.loading = false;
    }

    pub(crate) fn finish(
        &mut self,
        generation: u64,
        result: Result<(String, Vec<DirEntry>), FsError>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        self.loading = false;
        match result {
            Ok((path, mut entries)) => {
                entries.retain(|entry| entry.name != "." && entry.name != "..");
                entries.sort_by(|left, right| {
                    (left.kind != EntryKind::Directory)
                        .cmp(&(right.kind != EntryKind::Directory))
                        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                        .then_with(|| left.name.cmp(&right.name))
                });
                self.path = Some(path);
                self.entries = entries;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        true
    }
}

pub(crate) async fn listing(
    fs: Arc<dyn RemoteFs>,
    directory: Option<String>,
) -> Result<(String, Vec<DirEntry>), FsError> {
    let directory = match directory {
        Some(directory) => directory,
        None => fs.home().await?,
    };
    let entries = fs.read_dir(&directory).await?;
    Ok((directory, entries))
}
