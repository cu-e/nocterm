//! Pack-directory transaction. Atomic files cannot publish a directory tree.
use crate::archive::{check_destination, plain_directory};
use nocterm_themes::ThemeOperationError;
use std::{
    fs,
    path::{Path, PathBuf},
};
pub(crate) struct PackPublication {
    stage: tempfile::TempDir,
    backup: tempfile::TempDir,
    destination: PathBuf,
}
impl PackPublication {
    pub(crate) fn prepare(root: &Path, id: &str) -> Result<Self, ThemeOperationError> {
        plain_directory(root)?;
        let destination = root.join(id);
        check_destination(&destination)?;
        Ok(Self {
            stage: tempfile::Builder::new()
                .prefix(".install-")
                .tempdir_in(root)?,
            backup: tempfile::Builder::new()
                .prefix(".backup-")
                .tempdir_in(root)?,
            destination,
        })
    }
    pub(crate) fn stage(&self) -> &Path {
        self.stage.path()
    }
    pub(crate) fn publish(self) -> Result<(), ThemeOperationError> {
        self.publish_using(|from, to| fs::rename(from, to))
    }
    fn publish_using(
        self,
        mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
    ) -> Result<(), ThemeOperationError> {
        let old = self.backup.path().join("previous");
        let existed = check_destination(&self.destination)?;
        if existed {
            rename(&self.destination, &old).map_err(|e| {
                ThemeOperationError::filesystem(&self.destination, "back up previous pack", e)
            })?;
        }
        if let Err(publication) = rename(self.stage.path(), &self.destination) {
            if existed && let Err(rollback) = rename(&old, &self.destination) {
                let recovery = self.backup.keep().join("previous");
                return Err(ThemeOperationError::Rollback {
                    publication,
                    rollback,
                    recovery,
                });
            }
            return Err(ThemeOperationError::Publication {
                path: self.destination,
                source: publication,
            });
        }
        // The stage path is relinquished after commit; Drop must not remove a later owner.
        let _ = self.stage.keep();
        // Removal of the previous pack is best effort after committed publication.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_missing_stage_rolls_back_the_previous_directory() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("old");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("original"), b"retained").unwrap();
        let publication = PackPublication::prepare(root.path(), "old").unwrap();
        fs::remove_dir_all(publication.stage()).unwrap();
        assert!(publication.publish().is_err());
        assert_eq!(fs::read(destination.join("original")).unwrap(), b"retained");
    }
    #[test]
    fn a_changed_non_directory_destination_is_rejected_without_touching_it() {
        let root = tempfile::tempdir().unwrap();
        let publication = PackPublication::prepare(root.path(), "new").unwrap();
        fs::write(root.path().join("new"), b"other owner").unwrap();
        assert!(matches!(
            publication.publish(),
            Err(ThemeOperationError::NotPlain(_))
        ));
        assert_eq!(fs::read(root.path().join("new")).unwrap(), b"other owner");
    }
    #[test]
    fn committed_stage_relinquishes_its_temporary_path() {
        let root = tempfile::tempdir().unwrap();
        let publication = PackPublication::prepare(root.path(), "new").unwrap();
        let stage = publication.stage().to_owned();
        fs::write(stage.join("theme"), b"new").unwrap();
        publication.publish().unwrap();
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("later"), b"owner").unwrap();
        assert_eq!(fs::read(root.path().join("new/theme")).unwrap(), b"new");
        assert!(stage.join("later").is_file());
    }
    #[test]
    fn failed_rollback_preserves_the_actual_previous_pack_at_the_recovery_path() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("old")).unwrap();
        fs::write(root.path().join("old/original"), b"retained").unwrap();
        let publication = PackPublication::prepare(root.path(), "old").unwrap();
        fs::write(publication.stage().join("replacement"), b"new").unwrap();
        let mut renames = 0;
        let error = publication
            .publish_using(|from, to| {
                renames += 1;
                if renames == 1 {
                    fs::rename(from, to)
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "publication/rollback blocked",
                    ))
                }
            })
            .unwrap_err();
        let ThemeOperationError::Rollback {
            publication,
            rollback,
            recovery,
        } = error
        else {
            panic!("expected typed rollback recovery");
        };
        assert_eq!(publication.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(rollback.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read(recovery.join("original")).unwrap(), b"retained");
        assert!(!root.path().join("old").exists());
    }
}
