//! Extract only validated, direct theme JSON files, then replace the pack.
use crate::{ARCHIVE_LIMIT, ExtensionInfo, FILE_LIMIT, ThemeError, parse_family};
use flate2::read::MultiGzDecoder;
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path},
};

const DECOMPRESSED_LIMIT: u64 = 64 * 1024 * 1024;
/// Registry identities are a single bounded lowercase path component.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && (id.as_bytes()[0].is_ascii_lowercase() || id.as_bytes()[0].is_ascii_digit())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}
fn check_id(id: &str) -> Result<(), ThemeError> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ThemeError::Invalid("invalid extension id".into()))
    }
}
fn plain_directory(path: &Path) -> Result<(), ThemeError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(ThemeError::Invalid(format!(
            "{} is not a plain directory",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}
/// Refuse to follow links at managed pack destinations.
fn check_destination(path: &Path) -> Result<bool, ThemeError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(ThemeError::Invalid(
            "extension destination is not a plain directory".into(),
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
/// Installs validated originals. Every failure before publication leaves the old pack intact.
pub fn install_archive(
    dir: &Path,
    extension: &ExtensionInfo,
    bytes: &[u8],
) -> Result<(), ThemeError> {
    check_id(&extension.id)?;
    if bytes.len() > ARCHIVE_LIMIT {
        return Err(ThemeError::Invalid("archive exceeds 16 MiB".into()));
    }
    plain_directory(dir)?;
    let destination = dir.join(&extension.id);
    check_destination(&destination)?;
    let stage = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(dir)?;
    let themes = stage.path().join("themes");
    fs::create_dir(&themes)?;
    let reader = Limited {
        inner: MultiGzDecoder::new(bytes),
        remaining: DECOMPRESSED_LIMIT,
    };
    let mut archive = tar::Archive::new(reader);
    let mut files = 0usize;
    for (index, entry) in archive.entries()?.enumerate() {
        if index >= 10_000 {
            return Err(ThemeError::Invalid("archive exceeds 10000 entries".into()));
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
            return Err(ThemeError::Invalid("theme file exceeds 8 MiB".into()));
        }
        files += 1;
        if files > 256 {
            return Err(ThemeError::Invalid(
                "archive exceeds 256 theme files".into(),
            ));
        }
        let mut data = Vec::new();
        entry.take(FILE_LIMIT as u64 + 1).read_to_end(&mut data)?;
        parse_family(&data)?;
        fs::write(themes.join(name), data)?;
    }
    // Tar ends at its zero blocks. Still consume the entire gzip stream so
    // trailing data cannot hide a decompression bomb or a broken checksum.
    io::copy(&mut archive.into_inner(), &mut io::sink())?;
    if files == 0 {
        return Err(ThemeError::Invalid(
            "archive contains no valid theme files".into(),
        ));
    }
    let manifest = toml::to_string(extension).map_err(|e| ThemeError::Invalid(e.to_string()))?;
    fs::write(stage.path().join("extension.toml"), manifest)?;
    let backup = tempfile::Builder::new()
        .prefix(".backup-")
        .tempdir_in(dir)?;
    let old = backup.path().join("previous");
    let existed = check_destination(&destination)?;
    if existed {
        fs::rename(&destination, &old)?;
    }
    if let Err(error) = fs::rename(stage.path(), &destination) {
        if existed && let Err(restore) = fs::rename(&old, &destination) {
            // Preserve the previous pack even when a concurrent filesystem
            // change prevents rollback; never let TempDir delete the only copy.
            let saved = backup.keep().join("previous");
            return Err(ThemeError::Invalid(format!(
                "could not publish extension: {error}; rollback failed: {restore}; previous pack preserved at {}",
                saved.display()
            )));
        }
        return Err(error.into());
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
    if filename.is_empty() || filename == "." || filename == ".." || pieces.next().is_some() {
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
pub fn uninstall(dir: &Path, id: &str) -> Result<(), ThemeError> {
    check_id(id)?;
    match fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(ThemeError::Invalid(
                "installed root is not a plain directory".into(),
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    }
    let path = dir.join(id);
    if check_destination(&path)? {
        fs::remove_dir_all(path)?;
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
                Err(io::Error::other("archive exceeds 64 MiB decompressed"))
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
