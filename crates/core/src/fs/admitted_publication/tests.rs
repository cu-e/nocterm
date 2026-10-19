use super::*;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Credit {
    path: std::path::PathBuf,
    drops: Arc<AtomicUsize>,
    must_be_absent: bool,
}
impl Drop for Credit {
    fn drop(&mut self) {
        if self.must_be_absent {
            assert!(
                !self.path.exists(),
                "stage cleanup must precede credit refund"
            );
        }
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn prepared(
    destination: &Path,
    stage: &Path,
    drops: &Arc<AtomicUsize>,
) -> PreparedPrivateStage<Credit, Credit> {
    let destination = AdmittedPath::try_copy(
        destination,
        Credit {
            path: destination.to_owned(),
            drops: Arc::clone(drops),
            must_be_absent: false,
        },
    )
    .unwrap();
    let stage = AdmittedPath::try_copy(
        stage,
        Credit {
            path: stage.to_owned(),
            drops: Arc::clone(drops),
            must_be_absent: true,
        },
    )
    .unwrap();
    PreparedPrivateStage::new(destination, stage)
}

#[test]
fn admitted_stage_uses_private_mode_and_original_atomic_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("history");
    let stage = directory.path().join(".stage");
    std::fs::write(&destination, b"old").unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let publication = prepared(&destination, &stage, &drops)
        .stage_with(|writer| writer.write_all(b"new"))
        .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"old");
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&stage).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    publication.publish().unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn producer_failure_and_panic_clean_stage_before_refunding_credits() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("history");
    let stage = directory.path().join(".stage");
    std::fs::write(&destination, b"old").unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let failure = prepared(&destination, &stage, &drops).stage_with(|writer| {
        writer.write_all(b"partial")?;
        Err::<(), _>(io::Error::from(io::ErrorKind::InvalidData))
    });
    assert!(matches!(
        failure,
        Err(AdmittedStageError::Producer(_, _, _))
    ));
    drop(failure);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = prepared(&destination, &stage, &drops).stage_with::<io::Error>(|writer| {
                writer.write_all(b"partial")?;
                panic!("producer panic");
            });
        }))
        .is_err()
    );
    assert_eq!(drops.load(Ordering::SeqCst), 4);
    assert_eq!(std::fs::read(&destination).unwrap(), b"old");
}

#[test]
fn failed_publish_moves_original_destination_path_into_concrete_error() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("directory");
    let stage = directory.path().join(".stage");
    std::fs::create_dir(&destination).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let prepared = prepared(&destination, &stage, &drops);
    let pointer = prepared
        .destination
        .as_path()
        .as_os_str()
        .as_encoded_bytes()
        .as_ptr();
    let publication = prepared
        .stage_with(|writer| writer.write_all(b"new"))
        .unwrap();
    let error = publication.publish().unwrap_err();
    assert_eq!(
        error.path().as_os_str().as_encoded_bytes().as_ptr(),
        pointer
    );
    assert_eq!(error.path(), destination);
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<FileError>()
            .is_some()
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    drop(error);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn existing_stage_is_refused_without_touching_either_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("history");
    let stage = directory.path().join(".stage");
    std::fs::write(&destination, b"old").unwrap();
    std::fs::write(&stage, b"other stage").unwrap();
    let destination = AdmittedPath::try_copy(&destination, ()).unwrap();
    let stage_path = AdmittedPath::try_copy(&stage, ()).unwrap();
    let result = PreparedPrivateStage::new(destination, stage_path)
        .stage_with(|writer| writer.write_all(b"new"));
    let Err(AdmittedStageError::Native(error)) = result else {
        panic!("exclusive stage must refuse an existing file");
    };
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(error.path()).unwrap(), b"old");
    assert_eq!(std::fs::read(&stage).unwrap(), b"other stage");
}

#[cfg(unix)]
#[test]
fn admitted_publish_replaces_final_symlink_without_touching_its_target() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("history");
    let target = directory.path().join("target");
    let stage = directory.path().join(".stage");
    std::fs::write(&target, b"untouched").unwrap();
    std::os::unix::fs::symlink(&target, &destination).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    prepared(&destination, &stage, &drops)
        .stage_with(|writer| writer.write_all(b"new"))
        .unwrap()
        .publish()
        .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
    assert!(
        !std::fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}
