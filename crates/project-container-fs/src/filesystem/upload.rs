use super::{codec, commands::FilesystemCommands, scripts};
use nocterm_session::FsError;
use nocterm_session::fs::UploadMode;
use std::sync::{Arc, atomic::AtomicBool};

pub(super) struct Upload {
    parent: String,
    parent_id: String,
    name: String,
    stage: String,
    stage_id: String,
    file_id: String,
    mode: UploadMode,
    offset: u64,
    failed: bool,
    sealed: bool,
    alive: Arc<AtomicBool>,
}
impl Upload {
    pub(super) async fn open(
        commands: &FilesystemCommands,
        path: String,
        mode: UploadMode,
        alive: Arc<AtomicBool>,
    ) -> Result<Self, FsError> {
        let parent = path
            .rsplit_once('/')
            .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
            .unwrap_or("/")
            .to_owned();
        let name = path.rsplit('/').next().unwrap_or_default().to_owned();
        let parent_id = codec::identity(
            &commands
                .run(scripts::PARENT, std::slice::from_ref(&path), None, 128)
                .await?,
        )?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| FsError::Other("cannot allocate private upload name".into()))?;
        let stage = format!(
            ".nocterm-{}.upload",
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let policy = if mode == UploadMode::Create {
            "create"
        } else {
            "replace"
        };
        let result = commands
            .run(
                scripts::UPLOAD_CREATE,
                &[
                    path.clone(),
                    parent_id.clone(),
                    stage.clone(),
                    policy.into(),
                ],
                None,
                256,
            )
            .await?;
        let ids: Vec<_> = result.split(|byte| *byte == 0).collect();
        if ids.len() != 3 {
            return Err(FsError::Other(
                "upload admission acknowledgement malformed; private stage retained".into(),
            ));
        }
        Ok(Self {
            parent,
            parent_id,
            name,
            stage,
            stage_id: codec::identity(ids[0])?,
            file_id: codec::identity(ids[1])?,
            mode,
            offset: 0,
            failed: false,
            sealed: false,
            alive,
        })
    }
    fn args(&self) -> Vec<String> {
        vec![
            self.parent.clone(),
            self.parent_id.clone(),
            self.stage.clone(),
            self.stage_id.clone(),
            self.file_id.clone(),
        ]
    }
    pub(super) fn alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(super) fn poison(&mut self) {
        self.failed = true;
    }
    pub(super) async fn write(
        &mut self,
        commands: &FilesystemCommands,
        chunks: Vec<Vec<u8>>,
    ) -> Result<(), FsError> {
        if self.sealed {
            return Err(FsError::Other("upload is sealed".into()));
        }
        if self.failed || chunks.len() > 8 || chunks.iter().any(|chunk| chunk.len() > 32768) {
            self.failed = true;
            return Err(FsError::Other(
                "upload is sealed, failed or exceeds chunk bounds".into(),
            ));
        }
        self.failed = true;
        for chunk in chunks {
            let end = self
                .offset
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| FsError::Other("upload size overflow".into()))?;
            let mut args = self.args();
            args.extend([self.offset.to_string(), end.to_string()]);
            let script = format!("{}\n{}", scripts::UPLOAD_CHECK, scripts::UPLOAD_WRITE);
            let ack = commands.run(&script, &args, Some(chunk), 128).await?;
            if codec::text(&ack)? != end.to_string() {
                return Err(FsError::Other(
                    "upload append acknowledgement malformed".into(),
                ));
            }
            self.offset = end;
        }
        self.failed = false;
        Ok(())
    }
    pub(super) async fn publish(
        &mut self,
        commands: &FilesystemCommands,
        name: Option<String>,
    ) -> Result<String, FsError> {
        if self.failed {
            return Err(FsError::Other(
                "upload acknowledgement was lost; publication refused".into(),
            ));
        }
        self.sealed = true;
        let name = name.unwrap_or_else(|| self.name.clone());
        if !nocterm_session::fs::valid_publication_name(&name) {
            return Err(FsError::Other("invalid publication name".into()));
        }
        let mut args = self.args();
        args.extend([
            self.offset.to_string(),
            name.clone(),
            if self.mode == UploadMode::Create {
                "create"
            } else {
                "replace"
            }
            .into(),
        ]);
        let script = format!("{}\n{}", scripts::UPLOAD_CHECK, scripts::UPLOAD_PUBLISH);
        self.failed = true;
        match commands.run(&script, &args, None, 128).await {
            Ok(_) => Ok(format!("{}/{}", self.parent.trim_end_matches('/'), name)),
            Err(error @ FsError::AlreadyExists { .. }) => {
                self.failed = false;
                Err(error)
            }
            Err(error) => Err(error),
        }
    }
    pub(super) async fn cleanup(&self, commands: &FilesystemCommands) -> Result<(), FsError> {
        commands
            .run(scripts::UPLOAD_CLEAN, &self.args(), None, 128)
            .await
            .map(|_| ())
    }
}
