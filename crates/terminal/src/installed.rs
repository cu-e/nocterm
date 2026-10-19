//! Immutable capabilities retained by terminal models and every workspace window.
use crate::credentials::CredentialAssociationError;
use gpui_kit::{App, Task};
use nocterm_session::{CredentialId, CredentialStore, ShellLaunch, Transport};
use nocterm_workspace::SessionSpec;
use std::sync::Arc;

type LocalFactory = Arc<dyn Fn(ShellLaunch) -> Arc<dyn Transport> + Send + Sync>;
type Association = Arc<
    dyn Fn(&SessionSpec, CredentialId, &mut App) -> Task<Result<(), CredentialAssociationError>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct LocalSessions {
    factory: Option<LocalFactory>,
    default_program: String,
}
impl LocalSessions {
    pub fn new(factory: LocalFactory, default_program: String) -> Self {
        Self {
            factory: Some(factory),
            default_program,
        }
    }
    pub fn disabled(default_program: String) -> Self {
        Self {
            factory: None,
            default_program,
        }
    }
    pub fn is_available(&self) -> bool {
        self.factory.is_some()
    }
    pub fn default_program(&self) -> &str {
        &self.default_program
    }
    fn transport(&self, launch: ShellLaunch) -> Option<Arc<dyn Transport>> {
        self.factory.as_ref().map(|factory| factory(launch))
    }
}

#[derive(Clone)]
pub struct CredentialProvider {
    store: Arc<dyn CredentialStore>,
    association: Association,
}
impl CredentialProvider {
    pub fn new(
        store: Arc<dyn CredentialStore>,
        association: impl Fn(
            &SessionSpec,
            CredentialId,
            &mut App,
        ) -> Task<Result<(), CredentialAssociationError>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            store,
            association: Arc::new(association),
        }
    }
    pub(crate) fn store(&self) -> &Arc<dyn CredentialStore> {
        &self.store
    }
    pub(crate) fn association(&self) -> Association {
        self.association.clone()
    }
}

pub struct Deps {
    remote: Arc<dyn Transport>,
    local: LocalSessions,
    credentials: Option<CredentialProvider>,
    recording: Option<Arc<nocterm_store::RecordingRegistry>>,
}
impl Deps {
    pub fn new(remote: Arc<dyn Transport>, local: LocalSessions) -> Self {
        Self {
            remote,
            local,
            credentials: None,
            recording: None,
        }
    }
    pub fn with_credentials(mut self, credentials: CredentialProvider) -> Self {
        self.credentials = Some(credentials);
        self
    }
    pub fn with_recording(mut self, recording: Arc<nocterm_store::RecordingRegistry>) -> Self {
        self.recording = Some(recording);
        self
    }
    pub(crate) fn install(self, settings: nocterm_store::SettingsHandle) -> Installed {
        Installed(Arc::new(InstalledDeps {
            remote: self.remote,
            local: self.local,
            credentials: self.credentials,
            recording: self.recording,
            settings,
        }))
    }
}
struct InstalledDeps {
    remote: Arc<dyn Transport>,
    local: LocalSessions,
    credentials: Option<CredentialProvider>,
    recording: Option<Arc<nocterm_store::RecordingRegistry>>,
    settings: nocterm_store::SettingsHandle,
}
#[derive(Clone)]
pub struct Installed(Arc<InstalledDeps>);
impl Installed {
    pub(crate) fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.0.settings
    }
    pub(crate) fn remote(&self) -> Arc<dyn Transport> {
        self.0.remote.clone()
    }
    pub(crate) fn local_transport(&self, launch: ShellLaunch) -> Option<Arc<dyn Transport>> {
        self.0.local.transport(launch)
    }
    pub(crate) fn default_program(&self) -> &str {
        self.0.local.default_program()
    }
    pub(crate) fn credentials(&self) -> Option<&CredentialProvider> {
        self.0.credentials.as_ref()
    }
    pub(crate) fn recording(&self) -> Option<Arc<nocterm_store::RecordingRegistry>> {
        self.0.recording.clone()
    }
}

/// One isolated command request, prepared and consumed on the App thread.
pub struct LocalCommand {
    launch: ShellLaunch,
    title: String,
    transport: Arc<dyn Transport>,
    completion: Box<dyn FnOnce(nocterm_session::CloseReason, &mut App)>,
}
impl LocalCommand {
    pub fn new(
        launch: ShellLaunch,
        title: String,
        transport: Arc<dyn Transport>,
        completion: impl FnOnce(nocterm_session::CloseReason, &mut App) + 'static,
    ) -> Self {
        Self {
            launch,
            title,
            transport,
            completion: Box::new(completion),
        }
    }
    pub(crate) fn into_parts(
        self,
    ) -> (
        ShellLaunch,
        String,
        Arc<dyn Transport>,
        Box<dyn FnOnce(nocterm_session::CloseReason, &mut App)>,
    ) {
        (self.launch, self.title, self.transport, self.completion)
    }
}

#[cfg(test)]
mod tests;
