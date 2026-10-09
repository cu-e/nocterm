//! Directory listing within entry and byte budgets.
use super::*;
#[derive(Default)]
pub(super) struct ListingBudget {
    pub(super) entries: usize,
    pub(super) bytes: usize,
    pub(super) pages: usize,
}
impl ListingBudget {
    pub(super) fn page(&mut self) -> Result<(), FsError> {
        self.pages += 1;
        if self.pages > MAX_DIRECTORY_ENTRIES {
            return Err(FsError::Other(
                "remote directory listing exceeds the supported page limit".into(),
            ));
        }
        Ok(())
    }
    pub(super) fn entry(&mut self, name: &str) -> Result<(), FsError> {
        if name.is_empty() || name.contains(['/', '\0']) {
            return Err(FsError::Other(
                "remote directory contains an invalid entry name".into(),
            ));
        }
        self.entries += 1;
        self.bytes = self.bytes.saturating_add(name.len());
        if name.len() > MAX_ENTRY_NAME
            || self.entries > MAX_DIRECTORY_ENTRIES
            || self.bytes > MAX_DIRECTORY_BYTES
        {
            return Err(FsError::Other(
                "remote directory listing exceeds the supported size limit".into(),
            ));
        }
        Ok(())
    }
}
pub(super) async fn read_dir(sftp: &Sftp, dir: &str) -> Result<Vec<DirEntry>, FsError> {
    let handle = sftp
        .raw
        .opendir(dir)
        .await
        .map_err(|e| fs_error(e, dir))?
        .handle;
    let result = async {
        let mut entries = Vec::new();
        let mut budget = ListingBudget::default();
        loop {
            match sftp.raw.readdir(&handle).await {
                Ok(names) => {
                    budget.page()?;
                    for name in &names.files {
                        budget.entry(&name.filename)?;
                    }
                    // Following a listing link is optional metadata enrichment.
                    // Pipeline a small window rather than one network roundtrip
                    // per link; buffered retains the server's listing order.
                    let enriched = names
                        .files
                        .into_iter()
                        .filter(|name| name.filename != "." && name.filename != "..")
                        .map(|name| async move {
                            let mut item = entry(name.filename, &name.attrs);
                            if item.is_symlink
                                && let Ok(target) = sftp.raw.stat(path::join(dir, &item.name)).await
                            {
                                item.kind = entry_kind(target.attrs.file_type());
                                item.size = target.attrs.size;
                            }
                            item
                        });
                    let mut enriched = futures::stream::iter(enriched).buffered(8);
                    while let Some(item) = enriched.next().await {
                        entries.push(item);
                    }
                }
                Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => break,
                Err(e) => return Err(fs_error(e, dir)),
            }
        }
        Ok(entries)
    }
    .await;
    let closed = sftp.raw.close(handle).await.map_err(|e| fs_error(e, dir));
    closed?;
    result
}
