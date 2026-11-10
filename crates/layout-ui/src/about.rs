//! What the tooltip of a chip of the bar preview says, made when it shows: the chip's name,
//! whether the panel opens with the layout and, for an item of a user plugin, what the host
//! knows of the plugin, in the host's own words.

use std::sync::Arc;

use gpui_kit::{AnyView, App, AppContext as _, SharedString, Window, component::tooltip::Tooltip};
use nocterm_plugin_model::{ContributionId, PluginSet, PluginTier};

use crate::user_facts::SharedFacts;

/// The tooltip of the chip of `id`, named `name`.
pub(crate) fn tooltip(
    catalog: &Arc<PluginSet>,
    facts: Option<&SharedFacts>,
    id: &ContributionId,
    name: SharedString,
    open: bool,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let (catalog, facts, id) = (catalog.clone(), facts.cloned(), id.clone());
    move |_, cx| {
        let text = text(&catalog, facts.as_ref(), &id, &name, open, cx);
        cx.new(|_| Tooltip::new(text)).into()
    }
}

fn text(
    catalog: &PluginSet,
    facts: Option<&SharedFacts>,
    id: &ContributionId,
    name: &str,
    open: bool,
    cx: &App,
) -> SharedString {
    let mut lines = vec![name.to_owned()];
    if open {
        lines.push("Opens with the layout".to_owned());
    }
    let Some(plugin) = catalog
        .plugin_of(id)
        .filter(|plugin| plugin.tier() == PluginTier::User)
    else {
        return lines.join("\n").into();
    };
    lines.push(format!("Plugin \u{b7} {} ({})", plugin.name(), plugin.id()));
    if let Some(facts) = facts.and_then(|facts| facts.facts(plugin.id(), cx)) {
        lines.push(format!("Publisher: {} (unverified)", facts.publisher));
        lines.push(format!("Fingerprint: {}", facts.fingerprint));
        lines.push(if facts.capabilities.is_empty() {
            "Allowed: nothing".to_owned()
        } else {
            format!("Allowed: {}", facts.capabilities.join("; "))
        });
    }
    lines.join("\n").into()
}
