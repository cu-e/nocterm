//! Extract only validated, direct theme JSON files, then replace the pack.
use flate2::read::MultiGzDecoder;
use nocterm_themes::{
    ARCHIVE_LIMIT, ExtensionInfo, FILE_LIMIT, ThemeOperationError as ThemeError, parse_family,
    valid_id,
};
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path},
};

const DECOMPRESSED_LIMIT: u64 = 64 * 1024 * 1024;
fn check_id(id: &str) -> Result<(), ThemeError> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ThemeError::Identity)
    }
}
pub(crate) fn plain_directory(path: &Path) -> Result<(), ThemeError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(ThemeError::NotPlain(path.into())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path)
                .map_err(|e| ThemeError::filesystem(path, "create directory", e))?;
            Ok(())
        }
        Err(e) => Err(ThemeError::filesystem(path, "inspect directory", e)),
    }
}
/// Refuse to follow links at managed pack destinations.
pub(crate) fn check_destination(path: &Path) -> Result<bool, ThemeError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(ThemeError::NotPlain(path.into())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(ThemeError::filesystem(path, "inspect directory", e)),
    }
}
/// Installs validated originals. Every failure before publication leaves the old pack intact.
pub(crate) fn install_archive(
    dir: &Path,
    extension: &ExtensionInfo,
    bytes: &[u8],
) -> Result<(), ThemeError> {
    check_id(&extension.id)?;
    if bytes.len() > ARCHIVE_LIMIT {
        return Err(ThemeError::BodyLimit {
            limit: ARCHIVE_LIMIT,
        });
    }
    let publication = crate::publication::PackPublication::prepare(dir, &extension.id)?;
    let themes = publication.stage().join("themes");
    fs::create_dir(&themes).map_err(|e| ThemeError::filesystem(&themes, "create staging", e))?;
    extract(&themes, bytes).map_err(classify_expansion)?;
    let manifest = toml::to_string(extension).map_err(ThemeError::ManifestEncode)?;
    let manifest_path = publication.stage().join("extension.toml");
    nocterm_core::fs::write_private_atomic(&manifest_path, manifest.as_bytes())
        .map_err(|e| ThemeError::filesystem(manifest_path, "write manifest", e.into_io_error()))?;
    publication.publish()
}

fn extract(themes: &Path, bytes: &[u8]) -> Result<(), ThemeError> {
    let reader = Limited {
        inner: MultiGzDecoder::new(bytes),
        remaining: DECOMPRESSED_LIMIT,
    };
    let mut archive = tar::Archive::new(reader);
    let mut files = 0usize;
    for (index, entry) in archive.entries()?.enumerate() {
        if index >= 10_000 {
            return Err(ThemeError::ArchiveEntries);
        }
        let entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?;
        let Some(name) = theme_filename(&path).map(std::ffi::OsStr::to_os_string) else {
            continue;
        };
        if entry.size() > FILE_LIMIT as u64 {
            return Err(ThemeError::FileLimit);
        }
        files += 1;
        if files > 256 {
            return Err(ThemeError::ArchiveFiles);
        }
        let mut data = Vec::new();
        entry.take(FILE_LIMIT as u64 + 1).read_to_end(&mut data)?;
        parse_family(&data)?;
        let path = themes.join(name);
        nocterm_core::fs::write_private_atomic(&path, &data)
            .map_err(|e| ThemeError::filesystem(path, "write original", e.into_io_error()))?;
    }
    // Tar ends at its zero blocks. Still consume the entire gzip stream so
    // trailing data cannot hide a decompression bomb or a broken checksum.
    io::copy(&mut archive.into_inner(), &mut io::sink())?;
    if files == 0 {
        return Err(ThemeError::EmptyArchive);
    }
    Ok(())
}

fn theme_filename(path: &Path) -> Option<&std::ffi::OsStr> {
    let raw = path.to_str()?;
    let raw = raw.strip_prefix("./").unwrap_or(raw);
    let mut pieces = raw.split('/');
    if pieces.next()? != "themes" {
        return None;
    }
    let filename = pieces.next()?;
    if filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains(['\\', ':'])
        || pieces.next().is_some()
    {
        return None;
    }
    let mut components = path.components();
    let first = components.next()?;
    let first = if first == Component::CurDir {
        components.next()?
    } else {
        first
    };
    if first != Component::Normal("themes".as_ref()) {
        return None;
    }
    let Component::Normal(name) = components.next()? else {
        return None;
    };
    if components.next().is_some() || Path::new(name).extension()? != "json" {
        return None;
    }
    Some(name)
}
/// Remove only one validated extension directory; links are refused.
pub(crate) fn uninstall(dir: &Path, id: &str) -> Result<(), ThemeError> {
    check_id(id)?;
    match fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(ThemeError::NotPlain(dir.into()));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(ThemeError::filesystem(dir, "inspect installed root", e)),
    }
    let path = dir.join(id);
    if check_destination(&path)? {
        fs::remove_dir_all(&path).map_err(|e| ThemeError::filesystem(path, "remove pack", e))?;
    }
    Ok(())
}
struct Limited<R> {
    inner: R,
    remaining: u64,
}
impl<R: Read> Read for Limited<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut probe = [0];
            return if self.inner.read(&mut probe)? == 0 {
                Ok(0)
            } else {
                Err(io::Error::other(ExpansionLimit))
            };
        }
        let len = buffer.len().min(self.remaining as usize);
        let n = self.inner.read(&mut buffer[..len])?;
        self.remaining -= n as u64;
        Ok(n)
    }
}
#[cfg(test)]
mod tests;

#[derive(Debug)]
struct ExpansionLimit;
impl std::fmt::Display for ExpansionLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("archive exceeds 64 MiB decompressed")
    }
}
impl std::error::Error for ExpansionLimit {}
fn classify_expansion(error: ThemeError) -> ThemeError {
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(source) = cause {
        if source.is::<ExpansionLimit>() {
            return ThemeError::ArchiveDecompressed;
        }
        cause = source.source();
    }
    error
}
