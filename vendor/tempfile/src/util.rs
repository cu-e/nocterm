use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

mod creation;
pub(crate) use creation::create as create_with_operations;

pub fn create_helper<R>(
    base: &Path,
    prefix: &OsStr,
    suffix: &OsStr,
    random_len: usize,
    mut f: impl FnMut(PathBuf) -> io::Result<R>,
) -> io::Result<R> {
    create_with_operations(
        &mut crate::anonymous::Legacy,
        base,
        prefix,
        suffix,
        random_len,
        |path, _| f(path),
    )
}
