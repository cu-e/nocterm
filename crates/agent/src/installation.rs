//! Original runtime, policy and host capabilities shared by every agent window.
use crate::{AgentServices, ImageDeps, TerminalAuthOpener, runtime};
use gpui_kit::{App, Subscription};
use nocterm_store::{SettingsHandle, ShutdownCoordinator, StoreError};
use nocterm_ui::{LayoutHandle, UiReady};
use std::{rc::Rc, sync::Arc};
#[derive(Clone)]
pub struct Installed {
    inner: Rc<Installation>,
}
struct Installation {
    runtime: runtime::RuntimeHandle,
    settings: SettingsHandle,
    metadata: Arc<dyn nocterm_ai::metadata::ChatMetadataSource>,
    images: ImageDeps,
    layout: LayoutHandle,
    auth: Option<TerminalAuthOpener>,
    _settings_observation: Subscription,
}
impl Installed {
    pub fn runtime(&self) -> &runtime::RuntimeHandle {
        &self.inner.runtime
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
    pub(crate) fn timestamp(&self) -> u64 {
        self.inner.metadata.timestamp()
    }
    pub(crate) fn new_chat(&self, agent: String) -> nocterm_ai::history::SavedChat {
        let identity = self.inner.metadata.new_identity();
        nocterm_ai::history::SavedChat::for_identity(agent, identity.id, identity.created)
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
    let runtime = runtime::RuntimeHandle::new(
        runtime::RuntimeServices {
            settings: settings.clone(),
            connector: services.connector,
            bridge: services.bridge,
            storage: services.storage,
            usage_source: services.usage_source,
            workdir_resolver: services.workdir_resolver,
            workdir: services.workdir,
            terminal_auth: services.terminal_auth.is_some(),
            private_dirs: services.private_dirs,
            shared_dirs: services.shared_dirs,
        },
        cx,
    );
    let observation = runtime.observe_settings(&settings, cx);
    let captured = runtime.clone();
    let registered = shutdown.register_capture(move |cx| captured.capture_shutdown(cx));
    (
        Installed {
            inner: Rc::new(Installation {
                runtime,
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
