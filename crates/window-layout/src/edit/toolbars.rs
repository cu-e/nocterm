//! The toolbar operations: showing a toolbar along an edge and putting it back where its hints
//! say. Hiding one is `RemoveToTray`, which also takes it off its edge.

use nocterm_plugin_model::{ContributionKind, ContributionRef, Edge};

use super::{EditError, Limit, Subject};
use crate::{LayoutPreset, MAX_ZONE_ITEMS};

/// Shows the toolbar at the end of `edge`, off any other edge and out of the tray.
pub(super) fn show(
    preset: &mut LayoutPreset,
    toolbar: ContributionRef<'_>,
    edge: Edge,
) -> Result<(), EditError> {
    let id = toolbar.id();
    if toolbar.kind() != ContributionKind::Toolbar {
        return Err(EditError::WrongKind(Subject::Contribution(id.clone())));
    }
    if preset.toolbars.edge_of(id) == Some(edge) {
        return Ok(());
    }
    if preset.toolbars.edge(edge).len() >= MAX_ZONE_ITEMS {
        return Err(EditError::Full(Limit::ZoneItems));
    }
    preset.toolbars.remove(id);
    preset.tray.retain(|held| held != id);
    preset.toolbars.edge_mut(edge).push(id.clone());
    Ok(())
}

/// Shows the toolbar along the edge its hints name, unless an edge shows it already.
pub(super) fn restore(
    preset: &mut LayoutPreset,
    toolbar: ContributionRef<'_>,
) -> Result<(), EditError> {
    if preset.toolbars.edge_of(toolbar.id()).is_some() {
        return Ok(());
    }
    show(preset, toolbar, toolbar.edge().unwrap_or(Edge::Top))
}
