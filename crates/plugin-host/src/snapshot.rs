//! What a plugin is shown: the snapshot the host assembles for one call.

use std::time::Instant;

use gpui::App;
use nocterm_plugin_api::{
    ContributionContext, EffectOutcome, Family, HostError, Snapshot, capability::capability,
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::PluginProviders;

use crate::{cadence::Cadence, registry::Ready};

/// Who the snapshot is for, and the ports it is read through.
pub(crate) struct Subject<'a> {
    window: gpui::WindowId,
    plugin: &'a PluginId,
    context: ContributionContext,
    in_focus: bool,
    providers: &'a PluginProviders,
    terminal: Option<&'a str>,
}

impl<'a> Subject<'a> {
    pub(crate) fn new(
        window: gpui::WindowId,
        plugin: &'a PluginId,
        context: ContributionContext,
        in_focus: bool,
        providers: &'a PluginProviders,
    ) -> Self {
        Self {
            window,
            plugin,
            context,
            in_focus,
            providers,
            terminal: None,
        }
    }

    /// The subject of a terminal bar's call, which the snapshot names.
    pub(crate) fn for_terminal(self, terminal: Option<&'a str>) -> Self {
        Self { terminal, ..self }
    }
}

/// The snapshot for a plugin: its granted observations (sampled no faster than the cadence),
/// its resolved settings, its storage and the outcomes of its last effects.
pub(crate) fn assemble(
    ready: &Ready,
    subject: &Subject<'_>,
    cadence: &mut Cadence,
    outcomes: Vec<EffectOutcome>,
    now: Instant,
    cx: &App,
) -> Snapshot {
    let manifest = ready.package().manifest();
    let hash = ready.package().hash();
    let mut snapshot = Snapshot::new(subject.context);
    snapshot.terminal = subject.terminal.map(str::to_owned);
    let declared = manifest
        .required()
        .iter()
        .map(|entry| entry.id)
        .chain(manifest.optional().iter().map(|optional| optional.id()));
    for id in declared {
        let Some(entry) = capability(id) else {
            snapshot
                .observations
                .insert(id.to_owned(), Err(HostError::Unavailable));
            continue;
        };
        if entry.family != Family::Observe {
            continue;
        }
        let result = match ready.grant().check(subject.plugin, hash, entry.id) {
            Ok(()) => cadence.sample(
                (subject.window, subject.plugin, entry.id),
                subject.in_focus,
                now,
                || subject.providers.read(entry.id, subject.plugin, cx),
            ),
            Err(error) => Err(error),
        };
        snapshot.observations.insert(entry.id.to_owned(), result);
    }
    // A plugin names its secret settings for the host to send and never reads them.
    snapshot.settings = manifest
        .plugin()
        .settings()
        .without_secrets(&ready.settings());
    snapshot.storage = ready.data().storage.clone();
    snapshot.outcomes = outcomes;
    snapshot
}
