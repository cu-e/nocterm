//! Neutral records the shared agent fixtures hand to the panel.
use nocterm_ai::model::auth::{PermissionChoice, PermissionEffect, ProviderPermission};
use nocterm_ai::model::controls::{ConfigOption, Modes};
use nocterm_ai::model::ids::SessionId;
use nocterm_ai::model::session::SessionOpened;
use nocterm_ai::model::tools::{ToolChanges, ToolPatch};

pub(super) fn opened(
    session: impl Into<SessionId>,
    config: Vec<ConfigOption>,
    modes: Option<Modes>,
) -> SessionOpened {
    let mut opened = SessionOpened::new(session);
    if !config.is_empty() {
        opened = opened.configuration(config);
    }
    match modes {
        Some(modes) => opened.modes(modes),
        None => opened,
    }
}

/// A provider permission request for one tool row with its offered choices.
pub(super) fn permission_request(
    session: impl Into<SessionId>,
    tool: &str,
    title: Option<&str>,
    choices: Vec<PermissionChoice>,
) -> ProviderPermission {
    let mut changes = ToolChanges::default();
    if let Some(title) = title {
        changes = changes.title(title);
    }
    ProviderPermission::new(session, ToolPatch::new(tool, changes), choices)
}

/// One allow-once choice, the common case.
pub(super) fn allow_once_choice(id: &str, label: &str) -> PermissionChoice {
    PermissionChoice::new(id, label, PermissionEffect::AllowOnce)
}
