use nocterm_session::{DirEntry, EntryKind, FsError, RemoteFs};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct Browser {
    generation: u64,
    path: Option<String>,
    /// The session's home directory, once a listing resolved it.
    home: Option<String>,
    entries: Vec<DirEntry>,
    loading: bool,
    error: Option<FsError>,
}

impl Browser {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }
    pub(crate) fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
    pub(crate) fn home(&self) -> Option<&str> {
        self.home.as_deref()
    }
    pub(crate) fn entries(&self) -> &[DirEntry] {
        &self.entries
    }
    pub(crate) fn loading(&self) -> bool {
        self.loading
    }
    pub(crate) fn error(&self) -> Option<&FsError> {
        self.error.as_ref()
    }
    pub(crate) fn remember_current_path_as_home(&mut self) {
        self.home = self.path.clone();
    }

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
