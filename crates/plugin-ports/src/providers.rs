//! The record of one window's providers and the dispatch from a capability id to one of them.

use std::rc::Rc;

use gpui::App;
use nocterm_plugin_api::{
    Capability, Effect, HostError, LocalId, Observed,
    capability::{
        clock::Now,
        files::ServeFiles,
        monitor::UsageNow,
        notifications::{Own, PostNotice},
        panels::{SetPanelBadge, SetPanelOpen},
        terminals::{OpenTerminal, ShowTerminalBar, WriteTerminal},
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
    open_terminal: Sink<OpenTerminal>,
    write_terminal: Sink<WriteTerminal>,
    serve_files: Sink<ServeFiles>,
    terminal_bar: Sink<ShowTerminalBar>,
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

    pub fn with_open_terminal(mut self, provider: Rc<impl Apply<OpenTerminal> + 'static>) -> Self {
        self.open_terminal = Some(provider);
        self
    }

    pub fn with_write_terminal(
        mut self,
        provider: Rc<impl Apply<WriteTerminal> + 'static>,
    ) -> Self {
        self.write_terminal = Some(provider);
        self
    }

    /// The answers of the plugin to file requests of its terminals.
    pub fn with_serve_files(mut self, provider: Rc<impl Apply<ServeFiles> + 'static>) -> Self {
        self.serve_files = Some(provider);
        self
    }

    /// The bars of the plugin's terminal tabs.
    pub fn with_terminal_bar(
        mut self,
        provider: Rc<impl Apply<ShowTerminalBar> + 'static>,
    ) -> Self {
        self.terminal_bar = Some(provider);
        self
    }

    pub fn with_surface(mut self, provider: Rc<impl Surface + 'static>) -> Self {
        self.surface = Some(provider);
        self
    }

    /// Whether this record has a provider for the capability `id`. The storage, request and
    /// timer effects are not provider effects (the host handles them), so they are never
    /// provided here. An unknown id is not provided.
    pub fn provides(&self, id: &str) -> bool {
        match id {
            Now::ID => self.clock.is_some(),
            Kind::ID => self.workspace.is_some(),
            UsageNow::ID => self.usage.is_some(),
            CountsNow::ID => self.counts.is_some(),
            Own::ID => self.own_notices.is_some(),
            PostNotice::ID => self.post.is_some(),
            SetPanelOpen::ID => self.open.is_some(),
            SetPanelBadge::ID => self.badge.is_some(),
            OpenTerminal::ID => self.open_terminal.is_some(),
            WriteTerminal::ID => self.write_terminal.is_some(),
            ServeFiles::ID => self.serve_files.is_some(),
            ShowTerminalBar::ID => self.terminal_bar.is_some(),
            _ => false,
        }
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

    /// Performs an effect that a provider performs. The storage, request and timer effects are
    /// not provider effects: the host handles them itself, so they are `denied` here.
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
            Effect::TerminalsOpen(request) => deliver(&self.open_terminal, plugin, request, cx),
            Effect::TerminalsWrite(request) => deliver(&self.write_terminal, plugin, request, cx),
            Effect::TerminalsFiles(request) => deliver(&self.serve_files, plugin, request, cx),
            Effect::TerminalsBar(request) => deliver(&self.terminal_bar, plugin, request, cx),
            Effect::StorageSet(_)
            | Effect::StorageRemove(_)
            | Effect::NetRequest(_)
            | Effect::TimersSet(_) => Err(HostError::Denied),
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
