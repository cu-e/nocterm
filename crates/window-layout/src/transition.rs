//! What a change of the `[layouts]` section means for a window that shows a layout.

use crate::{LayoutSettings, select};

/// How a window should treat a change of the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutTransition {
    /// The user chose another layout: apply its open panels.
    Switch,
    /// The selected layout was edited, created or lost: re-resolve and keep the open panels
    /// that are still placed.
    Edit,
    /// Nothing the window shows changed.
    None,
}

/// Compares two documents, with the legacy swap-sides flag as `select` reads it.
///
/// With `p` the layout `prev` selects and `n` the one `next` selects, a preset *exists* in a
/// document if it is a built-in or stored there:
/// - `Switch` iff `n != p`, `n` exists in `prev` and `p` exists in `next`;
/// - otherwise `Edit` iff the contents of `n` differ between the documents, `n` is new, or `p`
///   was deleted;
/// - otherwise `None`. A rename alone is `None`, because names are display-only.
///
/// This compares `[layouts]` only; a change to which plugins are enabled is not a layout change.
pub fn transition(
    prev: &LayoutSettings,
    next: &LayoutSettings,
    legacy_swapped: bool,
) -> LayoutTransition {
    let was = select(prev, legacy_swapped);
    let now = select(next, legacy_swapped);
    if now != was && prev.exists(&now) && next.exists(&was) {
        return LayoutTransition::Switch;
    }
    let is_new = !prev.exists(&now);
    let lost = !next.exists(&was);
    let changed = match (prev.preset(&now), next.preset(&now)) {
        (Some(before), Some(after)) => {
            before.bar != after.bar || before.tray != after.tray || before.open != after.open
        }
        _ => false,
    };
    if changed || is_new || lost {
        LayoutTransition::Edit
    } else {
        LayoutTransition::None
    }
}
