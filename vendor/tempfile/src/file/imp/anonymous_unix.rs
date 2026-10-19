//! Original anonymous Unix engine with per-instance allocation owners.
use crate::anonymous::AnonymousOperations;
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    path::Path,
};

#[cfg(target_os = "linux")]
pub(crate) fn create<O: AnonymousOperations>(
    directory: &Path,
    operations: &mut O,
) -> Result<File, O::Error> {
    use rustix::{fs::OFlags, io::Errno};
    use std::os::unix::fs::OpenOptionsExt;
    let result = {
        let _native = operations.native_path(directory)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(OFlags::TMPFILE.bits() as i32)
            .open(directory)
    };
    match result {
        Ok(file) => Ok(file),
        Err(error) => match Errno::from_io_error(&error) {
            Some(Errno::OPNOTSUPP) | Some(Errno::ISDIR) | Some(Errno::NOENT) => {
                create_unix(directory, operations)
            }
            _ => Err(operations.native_error(error)),
        },
    }
}
#[cfg(not(target_os = "linux"))]
pub(crate) fn create<O: AnonymousOperations>(
    directory: &Path,
    operations: &mut O,
) -> Result<File, O::Error> {
    create_unix(directory, operations)
}
fn create_unix<O: AnonymousOperations>(
    directory: &Path,
    operations: &mut O,
) -> Result<File, O::Error> {
    crate::util::create_with_operations(
        operations,
        directory,
        OsStr::new(".tmp"),
        OsStr::new(""),
        crate::NUM_RAND_CHARS,
        create_unlinked,
    )
}
fn create_unlinked<O: AnonymousOperations>(
    path: O::Path,
    operations: &mut O,
) -> Result<File, O::Error> {
    let temporary;
    let mut path = path.as_ref();
    if !path.is_absolute() {
        let current = operations.current_dir()?;
        temporary = operations.join(current.as_ref(), path)?;
        path = temporary.as_ref();
    }
    // One real conversion lease covers serial open/remove CString backing. It
    // is paid before creation, so a second admission cannot strand a named file.
    let _native = operations.native_path(path)?;
    let file = super::create_named(path, &mut OpenOptions::new(), None)
        .map_err(|error| operations.native_error(error))?;
    // Same original ignore policy, not a new success/failure contract.
    let _ = fs::remove_file(path);
    Ok(file)
}
