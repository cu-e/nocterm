//! Effects between a plugin's answer and the application: queued, re-checked when they are
//! applied, and reported back as outcomes.

use std::{
    collections::{BTreeMap, VecDeque},
    time::Instant,
};

use gpui::{App, WindowId};
use nocterm_plugin_api::{
    ContentHash, Effect, EffectOutcome, EffectRequest, Event, HostError, LocalId, capability,
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::PluginProviders;

use crate::{
    Call, PluginData, PluginDataChange,
    caps::MAX_OUTCOMES,
    deferred::{Deferred, Target},
    gesture::{Gestures, Input, TokenId},
    rate::RateLimits,
    registry::{Ready, Registry},
};

/// One effect, bound to who asked for it and where, as the host recorded it.
pub(crate) struct Queued {
    window: WindowId,
    plugin: PluginId,
    hash: ContentHash,
    local: LocalId,
    context: nocterm_plugin_api::ContributionContext,
    request: EffectRequest,
    token: Option<TokenId>,
    providers: PluginProviders,
}

impl Queued {
    pub(crate) fn new(
        call: &Call,
        hash: ContentHash,
        request: EffectRequest,
        token: Option<TokenId>,
        providers: &PluginProviders,
    ) -> Self {
        Self {
            window: call.window,
            plugin: call.plugin.clone(),
            hash,
            local: call.local.clone(),
            context: call.context,
            request,
            token,
            providers: providers.clone(),
        }
    }

    /// Who an answer that comes later goes to.
    fn target(&self) -> Target {
        let call = Call {
            window: self.window,
            plugin: self.plugin.clone(),
            local: self.local.clone(),
            context: self.context,
            in_focus: false,
            terminal: None,
        };
        Target::new(call, self.hash, self.providers.clone())
    }
}

/// The effect queue, the gesture tokens, the rate limits and the outcomes not yet reported.
#[derive(Default)]
pub(crate) struct Broker {
    queue: VecDeque<Queued>,
    rates: RateLimits,
    gestures: Gestures,
    outcomes: BTreeMap<PluginId, Vec<EffectOutcome>>,
}

impl Broker {
    pub(crate) fn mint(
        &mut self,
        plugin: &PluginId,
        window: WindowId,
        local: &LocalId,
        event: &Event,
        input: Input,
        now: Instant,
    ) -> Option<TokenId> {
        self.gestures.mint(plugin, window, local, event, input, now)
    }

    pub(crate) fn enqueue(&mut self, item: Queued) {
        self.queue.push_back(item);
    }

    /// What happened to the plugin's effects, which the next snapshot reports once.
    pub(crate) fn take_outcomes(&mut self, plugin: &PluginId) -> Vec<EffectOutcome> {
        self.outcomes.remove(plugin).unwrap_or_default()
    }

    pub(crate) fn outcomes(&self, plugin: &PluginId) -> Vec<EffectOutcome> {
        self.outcomes.get(plugin).cloned().unwrap_or_default()
    }

    /// Disable, update, uninstall and a fault: nothing queued for the plugin is applied.
    pub(crate) fn drop_plugin(&mut self, plugin: &PluginId) {
        self.queue.retain(|item| item.plugin != *plugin);
        self.gestures.forget(plugin);
        self.outcomes.remove(plugin);
    }

    /// The plugin is gone for good, so its rate history goes too.
    pub(crate) fn forget(&mut self, plugin: &PluginId) {
        self.drop_plugin(plugin);
        self.rates.forget(plugin);
    }

    pub(crate) fn release_window(&mut self, window: WindowId) {
        self.queue.retain(|item| item.window != window);
        self.gestures.release_window(window);
    }

    /// Checks every queued effect again and applies those the host performs itself. The
    /// effects a window's provider performs are returned approved, to be applied once the host's
    /// update has ended: a provider may read the host, open a panel whose view reads it, or
    /// update the workspace.
    pub(crate) fn drain(
        &mut self,
        registry: &mut Registry,
        deferred: &mut Deferred,
        data: &dyn PluginData,
        now: Instant,
        cx: &mut App,
    ) -> Vec<Checked> {
        let mut checked = Vec::new();
        while let Some(item) = self.queue.pop_front() {
            checked.push(
                match self.settle(&item, registry, deferred, data, now, cx) {
                    Ok(Settled::Provider) => Checked::Approved(Box::new(Approved {
                        plugin: item.plugin,
                        hash: item.hash,
                        key: item.request.key,
                        effect: item.request.effect,
                        providers: item.providers,
                    })),
                    Ok(Settled::Done) => Checked::Done(item.plugin, item.request.key, Ok(())),
                    Err(error) => Checked::Done(item.plugin, item.request.key, Err(error)),
                },
            );
        }
        checked
    }

    /// What happened to one effect, which the plugin's next snapshot reports.
    pub(crate) fn record(
        &mut self,
        plugin: &PluginId,
        key: Option<String>,
        result: Result<(), HostError>,
    ) {
        if let Err(error) = &result {
            tracing::debug!(plugin = %plugin.as_str(), %error, "plugin effect refused");
        }
        let outcomes = self.outcomes.entry(plugin.clone()).or_default();
        outcomes.push(EffectOutcome { key, result });
        let excess = outcomes.len().saturating_sub(MAX_OUTCOMES);
        outcomes.drain(..excess);
    }

    fn settle(
        &mut self,
        item: &Queued,
        registry: &mut Registry,
        deferred: &mut Deferred,
        data: &dyn PluginData,
        now: Instant,
        cx: &mut App,
    ) -> Result<Settled, HostError> {
        let effect = &item.request.effect;
        let entry = capability::capability(effect.capability()).ok_or(HostError::Unavailable)?;
        let ready = registry
            .running(&item.plugin, Some(&item.hash))
            .ok_or(HostError::Unavailable)?;
        ready.grant().check(&item.plugin, &item.hash, entry.id)?;
        // A tab whose files the plugin serves needs that grant as well.
        if matches!(effect, Effect::TerminalsOpen(open) if open.files) {
            let files =
                <nocterm_plugin_api::capability::files::ServeFiles as capability::Capability>::ID;
            ready.grant().check(&item.plugin, &item.hash, files)?;
        }
        // The contribution whose event queued the effect, and the one it names, must still be
        // the plugin's own and still be on this window's surface.
        let manifest = ready.package().manifest();
        let own = |local: &LocalId| manifest.has_contribution(local);
        if !own(&item.local) || !effect.local().is_none_or(own) {
            return Err(HostError::Unavailable);
        }
        item.providers.permit(&item.plugin, &item.local, cx)?;
        if let Some(target) = effect.local().filter(|target| **target != item.local) {
            item.providers.permit(&item.plugin, target, cx)?;
        }
        if entry.gesture
            && !self
                .gestures
                .spend(item.token, &item.plugin, item.window, &item.local, now)
        {
            return Err(HostError::Denied);
        }
        self.rates.admit(&item.plugin, entry, now)?;
        let key = item.request.key.clone().unwrap_or_default();
        match effect {
            Effect::StorageSet(_) | Effect::StorageRemove(_) => {
                store(ready, &item.plugin, effect, data, cx).map(|()| Settled::Done)
            }
            Effect::NetRequest(request) => {
                let built = crate::net::build(manifest, &ready.settings(), request);
                deferred
                    .admit_request(item.target(), key, built)
                    .map(|()| Settled::Done)
            }
            Effect::TimersSet(timer) => deferred
                .admit_timer(item.target(), key, timer.after_ms)
                .map(|()| Settled::Done),
            _ => Ok(Settled::Provider),
        }
    }
}

/// One queued effect after its check, in the order the plugin asked.
pub(crate) enum Checked {
    /// Applied by the host or refused: the plugin, the effect's key and the result.
    Done(PluginId, Option<String>, Result<(), HostError>),
    /// For a window's provider to apply.
    Approved(Box<Approved>),
}

/// How a checked effect is applied.
enum Settled {
    /// The host applied it.
    Done,
    /// A window's provider applies it once the host's update has ended.
    Provider,
}

/// An effect the host checked and a window's provider applies. It runs with no entity held, and
/// only while the package that asked for it still runs.
pub(crate) struct Approved {
    plugin: PluginId,
    hash: ContentHash,
    key: Option<String>,
    effect: Effect,
    providers: PluginProviders,
}

impl Approved {
    pub(crate) fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    pub(crate) fn hash(&self) -> &ContentHash {
        &self.hash
    }

    /// Applies the effect and gives its key and result, for [`Broker::record`].
    pub(crate) fn apply(self, cx: &mut App) -> (Option<String>, Result<(), HostError>) {
        let result = self.providers.perform(&self.plugin, self.effect, cx);
        (self.key, result)
    }
}

/// Applies a storage effect to the plugin's own document and queues the write.
fn store(
    ready: &mut Ready,
    plugin: &PluginId,
    effect: &Effect,
    data: &dyn PluginData,
    cx: &mut App,
) -> Result<(), HostError> {
    let current = ready.writable_data().map_err(|error| {
        tracing::debug!(plugin = %plugin.as_str(), %error, "plugin storage refused");
        HostError::Unavailable
    })?;
    let next = match effect {
        Effect::StorageSet(request) => current.with_set(request),
        Effect::StorageRemove(request) => current.with_removed(request),
        _ => return Err(HostError::Denied),
    }
    .map_err(|_| HostError::Denied)?;
    data.change(plugin.clone(), PluginDataChange::Replace(next.clone()), cx)
        .map_err(|error| {
            tracing::warn!(plugin = %plugin.as_str(), %error, "plugin storage was not saved");
            HostError::Unavailable
        })?;
    ready.set_data(next);
    Ok(())
}
