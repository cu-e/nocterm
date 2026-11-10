//! Cooperative remote serialization; no expiry or automatic orphan stealing.
use super::{DIRECTORY_PERMISSIONS, HostAccessError, host};
use nocterm_core::error::SharedCause;
use nocterm_session::{
    FsError, RemoteFs,
    fs::{UploadMode, path, read_bounded},
};

/// One confirmed publication owns the fixed lock until checked removal.
/// Cancellation deliberately leaves an orphan: without a retained executor,
/// dropping an async cleanup cannot prove that removal finished before a retry.
pub(super) struct EditLock {
    path: String,
    token: String,
}
impl EditLock {
    pub(super) async fn acquire(
        fs: &dyn RemoteFs,
        directory: &str,
    ) -> Result<Self, HostAccessError> {
        let capabilities = fs.capabilities();
        if !capabilities.rename || !capabilities.remove {
            return Err(host(FsError::Unsupported(
                "cooperative authorized_keys lock requires no-clobber publication and removal"
                    .into(),
            )));
        }
        if fs.stat(directory).await.map_err(host)?.is_none() {
            fs.create_dir(directory).await.map_err(host)?;
            let _ = fs.set_permissions(directory, DIRECTORY_PERMISSIONS).await;
        }
        let path = path::join(directory, ".nocterm-authorized_keys.lock");
        let token = uuid::Uuid::new_v4().to_string();
        // Create stages privately and publishes without replacing an existing lock.
        let mut upload = fs
            .upload(&path, UploadMode::Create)
            .await
            .map_err(|error| acquisition_error(error, &path))?;
        if let Err(error) = upload.write(token.as_bytes().to_vec()).await {
            let _ = upload.cancel().await;
            return Err(host(error));
        }
        upload
            .finish()
            .await
            .map_err(|error| acquisition_error(error, &path))?;
        Ok(Self { path, token })
    }

    pub(super) async fn release(self, fs: &dyn RemoteFs) -> Result<(), HostAccessError> {
        let mut reader = fs.download(&self.path).await.map_err(host)?;
        let bytes = read_bounded(reader.as_mut(), self.token.len(), self.token.len())
            .await
            .map_err(|error| HostAccessError::Host(SharedCause::new(error)))?;
        reader.close().await.map_err(host)?;
        if bytes != self.token.as_bytes() {
            return Err(host(FsError::Other(format!(
                "authorized_keys lock ownership changed at {}; refusing removal",
                self.path
            ))));
        }
        // Cooperating editors never remove or replace an existing lock. A manual
        // writer can still race this check; recovery requires stopping all edits.
        fs.remove_file(&self.path).await.map_err(host)
    }
}

fn acquisition_error(error: FsError, path: &str) -> HostAccessError {
    match error {
        FsError::AlreadyExists { .. } => HostAccessError::EditLocked {
            path: path.to_owned(),
        },
        error => host(error),
    }
}
