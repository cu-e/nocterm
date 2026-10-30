//! Original runtime, policy and host capabilities shared by every agent window.
use crate::{AgentServices, ImageDeps, TerminalAuthOpener, runtime};
use gpui_kit::{App, Subscription};
use nocterm_agent_directory::AgentsHandle;
use nocterm_store::{SettingsHandle, ShutdownCoordinator, StoreError};
use nocterm_ui::{LayoutHandle, UiReady};
use std::{rc::Rc, sync::Arc};
mod source;
use crate::cloud_access::CloudAgentsAccess;
use source::SourceBinding;
#[derive(Clone)]
pub struct Installed {
    inner: Rc<Installation>,
    source: SourceBinding,
    cloud: Option<CloudAgentsAccess>,
}
struct Installation {
    agents: AgentsHandle,
    cloud_chats: nocterm_cloud_chat::CloudChatInstallation,
    settings: SettingsHandle,
    metadata: Arc<dyn nocterm_ai::metadata::ChatMetadataSource>,
    images: ImageDeps,
    layout: LayoutHandle,
    auth: Option<TerminalAuthOpener>,
    _settings_observation: Subscription,
}
impl Installed {
    pub fn runtime(&self) -> &runtime::RuntimeHandle {
        self.source.runtime()
    }
    pub fn for_environment(
        &self,
        access: nocterm_ai::local_environment::LocalEnvironmentAccess,
    ) -> Result<Self, nocterm_ai::storage::AgentStorageError> {
        Ok(Self {
            inner: self.inner.clone(),
            cloud: self.cloud.clone(),
            source: SourceBinding::fixed(self.runtime().for_environment(access)?),
        })
    }
    /// A hub window without an authenticated capture provider has no Local environment.
    pub fn unavailable_account(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            cloud: self.cloud.clone(),
            source: SourceBinding::fixed(self.runtime().unavailable_account()),
        }
    }
    pub fn with_account_source(
        &self,
        factory: Arc<dyn nocterm_hubs_model::source_identity::LocalAccountSourceFactory>,
    ) -> Self {
        Self {
            inner: self.inner.clone(),
            cloud: self.cloud.clone(),
            source: SourceBinding::account(self.runtime(), factory),
        }
    }
    /// The Cloud agents of the signed-in hubs, offered in the new-chat menu of every window
    /// made from this installation.
    pub fn with_cloud_agents(&self, access: CloudAgentsAccess) -> Self {
        Self {
            inner: self.inner.clone(),
            source: self.source.clone(),
            cloud: Some(access),
        }
    }
    /// A window of one hub offers only that hub's Cloud agents.
    pub fn for_hub_window(&self, hub: &nocterm_hubs_model::HubId) -> Self {
        Self {
            cloud: self.cloud.as_ref().map(|access| access.for_hub(hub)),
            ..self.clone()
        }
    }
    pub(crate) fn cloud_agents(&self) -> Option<&CloudAgentsAccess> {
        self.cloud.as_ref()
    }
    pub(crate) fn capture_runtime(
        &self,
    ) -> Result<runtime::RuntimeHandle, nocterm_ai::storage::AgentStorageError> {
        self.source.capture()
    }
    /// The agents available to start, for other surfaces such as settings.
    pub fn agents(&self) -> &AgentsHandle {
        &self.inner.agents
    }
    /// One shared Cloud owner across root windows and their captured environments.
    /// Native sources still retain and validate their original account membership.
    pub fn cloud_chats(&self) -> &nocterm_cloud_chat::CloudChatInstallation {
        &self.inner.cloud_chats
    }
    pub(crate) fn settings(&self) -> &SettingsHandle {
        &self.inner.settings
    }
    pub(crate) fn images(&self) -> &ImageDeps {
        &self.inner.images
    }
    pub(crate) fn layout(&self) -> &LayoutHandle {
        &self.inner.layout
    }
    pub(crate) fn auth(&self) -> Option<TerminalAuthOpener> {
        self.inner.auth.clone()
    }
    pub(crate) fn metadata(&self) -> Arc<dyn nocterm_ai::metadata::ChatMetadataSource> {
        self.inner.metadata.clone()
    }
}
pub(crate) fn install(
    services: AgentServices,
    images: ImageDeps,
    layout: LayoutHandle,
    ready: &UiReady,
    shutdown: &ShutdownCoordinator,
    cx: &mut App,
) -> (Installed, Result<(), StoreError>) {
    let settings = ready.settings().clone();
    let agents = AgentsHandle::new(services.catalog, settings.clone(), cx);
    let runtime = runtime::RuntimeHandle::new(
        runtime::RuntimeServices {
            settings: settings.clone(),
            agents: agents.clone(),
            connector: services.connector,
            bridge: services.bridge,
            command_jobs: services.command_jobs,
            storage: services.storage,
            workdir_resolver: services.workdir_resolver,
            workdir: services.workdir,
            terminal_auth: services.terminal_auth.is_some(),
            private_dirs: services.private_dirs,
            shared_dirs: services.shared_dirs,
        },
        cx,
    );
    let observation = runtime.observe_settings(&settings, cx);
    let cloud_chats =
        nocterm_cloud_chat::CloudChatInstallation::new(runtime.history_admission(cx), cx);
    let captured = runtime.clone();
    let registered = shutdown.register_capture(move |cx| captured.capture_shutdown(cx));
    (
        Installed {
            cloud: None,
            source: SourceBinding::fixed(runtime.clone()),
            inner: Rc::new(Installation {
                agents,
                cloud_chats,
                settings,
                metadata: services.metadata,
                images,
                layout,
                auth: services.terminal_auth,
                _settings_observation: observation,
            }),
        },
        registered,
    )
}
