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
    PluginData, PluginDataChange,
    caps::MAX_OUTCOMES,
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
    request: EffectRequest,
    token: Option<TokenId>,
    providers: PluginProviders,
}

impl Queued {
    pub(crate) fn new(
        origin: (WindowId, &PluginId, ContentHash, &LocalId),
        request: EffectRequest,
        token: Option<TokenId>,
        providers: &PluginProviders,
    ) -> Self {
        let (window, plugin, hash, local) = origin;
        Self {
            window,
            plugin: plugin.clone(),
            hash,
            local: local.clone(),
            request,
            token,
            providers: providers.clone(),
        }
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

    /// Applies every queued effect, checking each again first.
    pub(crate) fn drain(
        &mut self,
        registry: &mut Registry,
        data: &dyn PluginData,
        now: Instant,
        cx: &mut App,
    ) {
        while let Some(item) = self.queue.pop_front() {
            let result = self.settle(&item, registry, data, now, cx);
            if let Err(error) = &result {
                tracing::debug!(plugin = %item.plugin.as_str(), %error, "plugin effect refused");
            }
            let outcomes = self.outcomes.entry(item.plugin.clone()).or_default();
            outcomes.push(EffectOutcome {
                key: item.request.key.clone(),
                result,
            });
            let excess = outcomes.len().saturating_sub(MAX_OUTCOMES);
            outcomes.drain(..excess);
        }
    }

    fn settle(
        &mut self,
        item: &Queued,
        registry: &mut Registry,
        data: &dyn PluginData,
        now: Instant,
        cx: &mut App,
    ) -> Result<(), HostError> {
        let effect = &item.request.effect;
        let entry = capability::capability(effect.capability()).ok_or(HostError::Unavailable)?;
        let ready = registry
            .running(&item.plugin, Some(&item.hash))
            .ok_or(HostError::Unavailable)?;
        ready.grant().check(&item.plugin, &item.hash, entry.id)?;
        // The contribution whose event queued the effect, and the one it names, must still be
        // the plugin's own and still be on this window's surface.
        let manifest = ready.package().manifest();
        let own = |local: &LocalId| manifest.view(local.as_str()).is_some();
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
        match effect {
            Effect::StorageSet(_) | Effect::StorageRemove(_) => {
                store(ready, &item.plugin, effect, data, cx)
            }
            _ => item.providers.perform(&item.plugin, effect.clone(), cx),
        }
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
