use nocterm_session::{DirEntry, EntryKind, FsError, RemoteFs};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct Browser {
    pub(crate) generation: u64,
    pub(crate) path: Option<String>,
    /// The session's home directory, once a listing resolved it.
    pub(crate) home: Option<String>,
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
        self.home = None;
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
            Ok((path, entries)) => {
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
    let mut entries = fs.read_dir(&directory).await?;
    if entries.len() > super::local::MAX_DIRECTORY_ENTRIES
        || entries.iter().map(|entry| entry.name.len()).sum::<usize>()
            > super::local::MAX_DIRECTORY_NAME_BYTES
    {
        return Err(FsError::Other(
            "Directory exceeds the Explorer listing limit; narrow the directory and refresh."
                .into(),
        ));
    }
    entries.retain(|entry| entry.name != "." && entry.name != "..");
    entries.sort_by_cached_key(|entry| {
        (
            entry.kind != EntryKind::Directory,
            entry.name.to_lowercase(),
            entry.name.clone(),
        )
    });
    Ok((directory, entries))
}
