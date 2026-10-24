//! The record of one window's providers and the dispatch from a capability id to one of them.

use std::rc::Rc;

use gpui::App;
use nocterm_plugin_api::{
    Capability, Effect, HostError, LocalId, Observed,
    capability::{
        clock::Now,
        monitor::UsageNow,
        notifications::{Own, PostNotice},
        panels::{SetPanelBadge, SetPanelOpen},
        transfers::CountsNow,
        workspace::Kind,
    },
};
use nocterm_plugin_model::PluginId;

use crate::{Apply, Observe, Surface};

type Source<C> = Option<Rc<dyn Observe<C>>>;
type Sink<C> = Option<Rc<dyn Apply<C>>>;

/// The providers of one window. A capability without a provider is `unavailable`.
#[derive(Clone, Default)]
pub struct PluginProviders {
    clock: Source<Now>,
    workspace: Source<Kind>,
    usage: Source<UsageNow>,
    counts: Source<CountsNow>,
    own_notices: Source<Own>,
    post: Sink<PostNotice>,
    open: Sink<SetPanelOpen>,
    badge: Sink<SetPanelBadge>,
    surface: Option<Rc<dyn Surface>>,
}

impl PluginProviders {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_clock(mut self, provider: Rc<impl Observe<Now> + 'static>) -> Self {
        self.clock = Some(provider);
        self
    }

    pub fn with_workspace(mut self, provider: Rc<impl Observe<Kind> + 'static>) -> Self {
        self.workspace = Some(provider);
        self
    }

    pub fn with_usage(mut self, provider: Rc<impl Observe<UsageNow> + 'static>) -> Self {
        self.usage = Some(provider);
        self
    }

    pub fn with_counts(mut self, provider: Rc<impl Observe<CountsNow> + 'static>) -> Self {
        self.counts = Some(provider);
        self
    }

    pub fn with_own_notices(mut self, provider: Rc<impl Observe<Own> + 'static>) -> Self {
        self.own_notices = Some(provider);
        self
    }

    pub fn with_post(mut self, provider: Rc<impl Apply<PostNotice> + 'static>) -> Self {
        self.post = Some(provider);
        self
    }

    pub fn with_open(mut self, provider: Rc<impl Apply<SetPanelOpen> + 'static>) -> Self {
        self.open = Some(provider);
        self
    }

    pub fn with_badge(mut self, provider: Rc<impl Apply<SetPanelBadge> + 'static>) -> Self {
        self.badge = Some(provider);
        self
    }

    pub fn with_surface(mut self, provider: Rc<impl Surface + 'static>) -> Self {
        self.surface = Some(provider);
        self
    }

    /// The observation `capability` for `plugin`, wrapped for a snapshot.
    ///
    /// A capability that is not an observation, or has no provider here, is `unavailable`.
    pub fn read(
        &self,
        capability: &str,
        plugin: &PluginId,
        cx: &App,
    ) -> Result<Observed, HostError> {
        match capability {
            Now::ID => sample(&self.clock, plugin, cx).map(Observed::Clock),
            Kind::ID => sample(&self.workspace, plugin, cx).map(Observed::Workspace),
            UsageNow::ID => sample(&self.usage, plugin, cx).map(Observed::Usage),
            CountsNow::ID => sample(&self.counts, plugin, cx).map(Observed::Counts),
            Own::ID => sample(&self.own_notices, plugin, cx).map(Observed::Notices),
            _ => Err(HostError::Unavailable),
        }
    }

    /// Performs an effect that a provider performs. The storage effects are not provider
    /// effects: the host handles them itself, so they are `denied` here.
    pub fn perform(
        &self,
        plugin: &PluginId,
        effect: Effect,
        cx: &mut App,
    ) -> Result<(), HostError> {
        match effect {
            Effect::NotificationsPost(request) => deliver(&self.post, plugin, request, cx),
            Effect::PanelsSetOpen(request) => deliver(&self.open, plugin, request, cx),
            Effect::PanelsSetBadge(request) => deliver(&self.badge, plugin, request, cx),
            Effect::StorageSet(_) | Effect::StorageRemove(_) => Err(HostError::Denied),
        }
    }

    /// Whether the plugin's contribution may be acted on in this window. Without a surface
    /// provider nothing is registered, so nothing may be.
    pub fn permit(&self, plugin: &PluginId, local: &LocalId, cx: &App) -> Result<(), HostError> {
        match &self.surface {
            Some(surface) => surface.permit(plugin, local, cx),
            None => Err(HostError::Unavailable),
        }
    }
}

fn sample<C: Capability>(
    source: &Source<C>,
    plugin: &PluginId,
    cx: &App,
) -> Result<C::Value, HostError> {
    match source {
        Some(provider) => provider.sample(plugin, cx),
        None => Err(HostError::Unavailable),
    }
}

fn deliver<C: Capability>(
    sink: &Sink<C>,
    plugin: &PluginId,
    request: C::Value,
    cx: &mut App,
) -> Result<(), HostError> {
    match sink {
        Some(provider) => provider.apply(plugin, request, cx),
        None => Err(HostError::Unavailable),
    }
}
