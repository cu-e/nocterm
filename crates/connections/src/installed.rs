//! Retained service handles, explicit policy and shared presentation/retry ownership.
use crate::{Connections, EditorInputs, PreparedConnections};
use gpui_kit::{App, Entity, Image, Subscription};
use nocterm_connections_model::{
    observation::CountryCode,
    profiles::{Profile, ProfileId},
};
use nocterm_connections_store::{FactsEvent, FactsPrepared, FactsService};
use nocterm_store::{SettingsHandle, ShutdownCoordinator, StoreError};
use nocterm_ui::UiReady;
use std::{rc::Rc, sync::Arc};
#[derive(Clone)]
pub struct Installed(Rc<Installation>);
struct Installation {
    connections: Entity<Connections>,
    facts: Entity<FactsService>,
    inputs: EditorInputs,
    settings: SettingsHandle,
    presentation: Rc<crate::facts_presentation::FactsPresentation>,
    sources: Rc<crate::probe_sources::ProbeSources>,
    _subscriptions: Vec<Subscription>,
}
impl Installed {
    pub(crate) fn from_owners(
        connections: Connections,
        facts: FactsPrepared,
        inputs: EditorInputs,
        settings: SettingsHandle,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<Self, StoreError> {
        let executor = cx.background_executor().clone();
        Self::from_owners_with_executor(
            connections,
            facts,
            inputs,
            settings,
            executor,
            shutdown,
            cx,
        )
    }
    pub(crate) fn from_owners_with_executor(
        connections: Connections,
        facts: FactsPrepared,
        inputs: EditorInputs,
        settings: SettingsHandle,
        executor: gpui_kit::BackgroundExecutor,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<Self, StoreError> {
        let connections = connections.into_entity(shutdown, cx)?;
        let carrier = connections.update(cx, |connections, cx| connections.publications(cx));
        let policy = Rc::new(crate::country_policy::SettingsCountryPolicy::new(
            settings.clone(),
        ));
        let (facts, mut subscriptions) = facts.install(carrier, policy, executor, shutdown, cx)?;
        let sources = Rc::new(crate::probe_sources::ProbeSources::default());
        let observed = facts.downgrade();
        let retries = sources.clone();
        let saved = connections.downgrade();
        subscriptions.push(settings.observe(cx, move |cx| {
            if let Some(facts) = observed.upgrade() {
                facts.update(cx, |facts, cx| facts.policy_changed(cx));
                if let Some(saved) = saved.upgrade() {
                    retries.scan(&saved, &facts, cx);
                }
            }
        }));
        let retries = sources.clone();
        let saved = connections.downgrade();
        subscriptions.push(cx.subscribe(&facts, move |facts, _: &FactsEvent, cx| {
            let (retries, saved, facts) = (retries.clone(), saved.clone(), facts.downgrade());
            cx.defer(move |cx| {
                if let (Some(saved), Some(facts)) = (saved.upgrade(), facts.upgrade()) {
                    retries.scan(&saved, &facts, cx);
                }
            });
        }));
        Ok(Self(Rc::new(Installation {
            connections,
            facts,
            inputs,
            settings,
            presentation: Rc::new(Default::default()),
            sources,
            _subscriptions: subscriptions,
        })))
    }
    pub fn connections(&self) -> &Entity<Connections> {
        &self.0.connections
    }
    pub fn facts(&self) -> &Entity<FactsService> {
        &self.0.facts
    }
    pub fn settings(&self) -> &SettingsHandle {
        &self.0.settings
    }
    pub(crate) fn inputs(&self) -> &EditorInputs {
        &self.0.inputs
    }
    pub(crate) fn sources(&self) -> &Rc<crate::probe_sources::ProbeSources> {
        &self.0.sources
    }
    pub(crate) fn detecting_country(&self, cx: &App) -> bool {
        self.0
            .settings
            .setting::<nocterm_ui::AppearanceSettings>(cx)
            .detect_server_country
    }
    pub(crate) fn os(&self, id: ProfileId, cx: &App) -> Option<&'static crate::os::Os> {
        self.facts().read(cx).os(id, cx).and_then(crate::os::find)
    }
    /// The system a profile's icon shows: the one chosen for it, else the detected one.
    pub(crate) fn shown_os(&self, profile: &Profile, cx: &App) -> Option<&'static crate::os::Os> {
        profile
            .icon
            .as_deref()
            .and_then(crate::os::find)
            .or_else(|| self.os(profile.id, cx))
    }
    pub(crate) fn shown_country(&self, profile: &Profile, cx: &App) -> Option<String> {
        profile.country.clone().or_else(|| {
            self.detecting_country(cx)
                .then(|| {
                    self.facts()
                        .read(cx)
                        .country(profile.id, cx)
                        .map(str::to_owned)
                })
                .flatten()
        })
    }
    pub fn flag(&self, code: &str, cx: &App) -> Option<Arc<Image>> {
        let code = CountryCode::new(code).ok()?;
        let state = self.facts().read(cx).flag(code).cloned();
        self.0.presentation.flag(code, state)
    }
    pub(crate) fn icon(&self, profile: &Profile, cx: &App) -> Option<Arc<Image>> {
        self.0.presentation.icon(profile, self.os(profile.id, cx))
    }
}
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
    ready
        .settings()
        .register::<nocterm_ui::TerminalSettings>(cx);
    let (connections, inputs, facts) = prepared.into_owners();
    Installed::from_owners(
        connections,
        facts,
        inputs,
        ready.settings().clone(),
        shutdown,
        cx,
    )
}
