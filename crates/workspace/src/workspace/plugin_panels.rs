//! What a user plugin may ask of the panels of its window, under the shell's rules.
//!
//! The window decides, not the plugin and not the host: a panel opens only when it is registered
//! in the window, its plugin is enabled and the user approved placing it; it never replaces a
//! panel that shows and never opens while a panel asks for the user. [`set_open`] is the one
//! transition that opens or closes a plugin's panel, so no other path reaches the rule's
//! subject without it. The rules about the panels themselves live in `PanelGroups`
//! (`open_for_plugin`); this module checks what the window knows about the contribution and
//! applies the transition.
//!
//! Only `ShellCapabilities` (the window's provider for plugins) and the user plugin
//! registrations of the application reach these functions, so `Workspace` itself gains no
//! method for plugins.

use gpui_kit::Context;
use nocterm_plugin_model::{ContributionId, USER_PREFIX};

use super::{Workspace, panel_groups::OpeningRefused};

/// Why the window refused a plugin's request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The contribution is not registered here, its plugin is disabled, the user did not
    /// approve placing it, or showing it would replace a panel that shows.
    Unavailable,
    /// A panel asks for the user right now.
    UserIsBeingAsked,
    /// The contribution does not belong to a user plugin; the window withdraws only those.
    NotUserContribution,
}

/// Whether the contribution is registered in this window, enabled and approved for placement.
pub(crate) fn permit(workspace: &Workspace, id: &ContributionId) -> Result<(), Refusal> {
    let usable = workspace.registrations.is_registered(id)
        && workspace.shell.is_enabled(id)
        && workspace.shell.set().is_placeable(id);
    usable.then_some(()).ok_or(Refusal::Unavailable)
}

/// Shows or closes the plugin's panel. Focus stays where it is, except that a panel that held it
/// gives it up when it closes.
pub(crate) fn set_open(
    workspace: &mut Workspace,
    id: &ContributionId,
    open: bool,
    window: &mut gpui_kit::Window,
    cx: &mut Context<Workspace>,
) -> Result<(), Refusal> {
    permit(workspace, id)?;
    let transition = if open {
        workspace
            .panels
            .open_for_plugin(id)
            .map_err(|refusal| match refusal {
                OpeningRefused::WouldReplace => Refusal::Unavailable,
                OpeningRefused::UserIsBeingAsked => Refusal::UserIsBeingAsked,
            })?
    } else {
        workspace.panels.close_for_plugin(id, window, cx)
    };
    if let Some(transition) = transition {
        workspace.apply_panel_transition(transition, window, cx);
    }
    Ok(())
}

/// Withdraws the registration of a contribution whose plugin was uninstalled or replaced. A
/// panel that shows closes without taking focus, and the contribution can register again.
/// Only a user contribution can be withdrawn: a built-in one stays registered for the life of the
/// window, so any other id is refused and nothing changes. A user contribution that is not
/// registered is a no-op.
pub(crate) fn withdraw(
    workspace: &mut Workspace,
    id: &ContributionId,
    window: &mut gpui_kit::Window,
    cx: &mut Context<Workspace>,
) -> Result<(), Refusal> {
    if !id.namespace().starts_with(USER_PREFIX) {
        return Err(Refusal::NotUserContribution);
    }
    if !workspace.registrations.is_registered(id) {
        return Ok(());
    }
    workspace.registrations.forget(id);
    workspace.chrome.status_bar_mut().remove(id);
    let (transition, panel) = workspace.panels.forget(id, window, cx);
    if let Some(transition) = transition {
        workspace.apply_panel_transition(transition, window, cx);
    }
    if let Some(panel) = panel {
        cx.defer(move |_| drop(panel));
    }
    workspace.sync_status_widgets(window, cx);
    cx.notify();
    Ok(())
}
