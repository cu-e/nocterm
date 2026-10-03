//! Bounded regular-file reads for trust records and private authentication material.
use std::{
    fs::File,
    io::{self, Read as _},
    path::Path,
};
use zeroize::Zeroizing;

pub(crate) fn read(path: &Path, limit: usize) -> io::Result<Zeroizing<Vec<u8>>> {
    #[cfg(unix)]
    let file: File = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?
    .into();
    #[cfg(not(unix))]
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular file",
        ));
    }
    if metadata.len() > limit as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds the supported size limit",
        ));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds the supported size limit",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_limit_is_allowed_and_larger_or_special_files_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("key");
        std::fs::write(&path, b"1234").unwrap();
        assert_eq!(read(&path, 4).unwrap().as_slice(), b"1234");
        assert_eq!(
            read(&path, 3).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            read(directory.path(), 4096).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    #[cfg(unix)]
    #[test]
    fn fifo_is_rejected_without_waiting_for_a_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fifo");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &path,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            0,
        )
        .unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(read(&path, 4096).map(|_| ()));
        });
        assert!(
            receive
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap()
                .is_err()
        );
    }
}
