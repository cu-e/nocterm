//! Immutable retained collaborators; installation observers live with the last handle.
use crate::{Connections, EditorInputs, PreparedConnections, ServerFacts};
use gpui_kit::{App, Entity, Subscription};
use nocterm_store::{SettingsHandle, ShutdownCoordinator, StoreError};
use nocterm_ui::UiReady;
use std::rc::Rc;
#[derive(Clone)]
pub struct Installed(Rc<Installation>);
struct Installation {
    connections: Entity<Connections>,
    facts: Entity<ServerFacts>,
    inputs: EditorInputs,
    settings: SettingsHandle,
    _subscriptions: Vec<Subscription>,
}
impl Installed {
    pub(crate) fn from_owners(
        connections: Connections,
        facts: ServerFacts,
        settings: SettingsHandle,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<Self, StoreError> {
        let connections = connections.into_entity(shutdown, cx)?;
        let inputs = connections.read(cx).editor_inputs().clone();
        let (facts, mut subscriptions) = facts.install(&connections, shutdown, cx)?;
        let observed = facts.downgrade();
        subscriptions.push(settings.observe(cx, move |cx| {
            let _ = observed.update(cx, |_, cx| cx.notify());
        }));
        Ok(Self(Rc::new(Installation {
            connections,
            facts,
            inputs,
            settings,
            _subscriptions: subscriptions,
        })))
    }
    /// The exact profile owner, also suitable for composition's credential callback.
    pub fn connections(&self) -> &Entity<Connections> {
        &self.0.connections
    }
    pub fn facts(&self) -> &Entity<ServerFacts> {
        &self.0.facts
    }
    pub fn settings(&self) -> &SettingsHandle {
        &self.0.settings
    }
    pub(crate) fn inputs(&self) -> &EditorInputs {
        &self.0.inputs
    }
    pub(crate) fn detecting_country(&self, cx: &App) -> bool {
        self.0
            .settings
            .setting::<nocterm_ui::AppearanceSettings>(cx)
            .detect_server_country
    }
}
/// Installs supplied owners without native acquisition or global connection lookup.
pub fn install(
    prepared: PreparedConnections,
    ready: &UiReady,
    shutdown: &ShutdownCoordinator,
    cx: &mut App,
) -> Result<Installed, StoreError> {
    crate::menu::init(cx);
    ready
        .settings()
        .register::<nocterm_ui::AppearanceSettings>(cx);
    let (connections, facts) = prepared.into_owners();
    Installed::from_owners(connections, facts, ready.settings().clone(), shutdown, cx)
}
