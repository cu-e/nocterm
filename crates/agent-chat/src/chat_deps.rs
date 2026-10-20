//! What every chat needs from its host, without views.
//!
//! [`ChatDeps`] carries the runtime, agent directory, settings, chat
//! metadata, the window seat and the sign-in terminal. Views keep their own
//! collaborators (images, layout) in the panel host.
use crate::{runtime::RuntimeHandle, seat::Seat, sign_in::SignInTerminal};
use gpui::App;
use nocterm_agent_directory::AgentsHandle;
use nocterm_ai::{history::SavedChat, metadata::ChatMetadataSource};
use nocterm_store::SettingsHandle;
use std::{rc::Rc, sync::Arc};

#[derive(Clone)]
pub struct ChatDeps {
    inner: Rc<Deps>,
}
struct Deps {
    runtime: RuntimeHandle,
    agents: AgentsHandle,
    settings: SettingsHandle,
    metadata: Arc<dyn ChatMetadataSource>,
    seat: Seat,
    sign_in: Option<Rc<dyn SignInTerminal>>,
}
/// The shared collaborators [`ChatDeps`] is built from.
pub struct ChatServices {
    pub runtime: RuntimeHandle,
    pub agents: AgentsHandle,
    pub settings: SettingsHandle,
    pub metadata: Arc<dyn ChatMetadataSource>,
}
impl ChatDeps {
    pub fn new(
        services: ChatServices,
        seat: Seat,
        sign_in: Option<Rc<dyn SignInTerminal>>,
    ) -> Self {
        let ChatServices {
            runtime,
            agents,
            settings,
            metadata,
        } = services;
        Self {
            inner: Rc::new(Deps {
                runtime,
                agents,
                settings,
                metadata,
                seat,
                sign_in,
            }),
        }
    }
    pub fn runtime(&self) -> &RuntimeHandle {
        &self.inner.runtime
    }
    pub fn agents(&self) -> &AgentsHandle {
        &self.inner.agents
    }
    pub fn settings(&self) -> &SettingsHandle {
        &self.inner.settings
    }
    pub fn ai<'a>(&self, cx: &'a App) -> &'a nocterm_ai::AiSettings {
        self.settings().setting::<nocterm_ai::AiSettings>(cx)
    }
    pub fn enabled(&self, cx: &App) -> bool {
        self.ai(cx).enabled
    }
    /// What chats may do with the window.
    pub fn seat(&self) -> &Seat {
        &self.inner.seat
    }
    /// Runs sign-in terminals in this window, when the host can.
    pub fn sign_in(&self) -> Option<&Rc<dyn SignInTerminal>> {
        self.inner.sign_in.as_ref()
    }
    pub fn timestamp(&self) -> u64 {
        self.inner.metadata.timestamp()
    }
    pub fn new_chat(&self, agent: String) -> SavedChat {
        let identity = self.inner.metadata.new_identity();
        SavedChat::for_identity(agent, identity.id, identity.created)
    }
}
