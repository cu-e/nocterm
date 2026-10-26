//! The switch that allows user plugins at the next start, why a run holds them, and the notice
//! that a changed switch waits for a restart.

use gpui_kit::{
    AnyElement, Context, IntoElement, SharedString, TestSupportExt as _, component::switch::Switch,
    div, prelude::*,
};
use nocterm_ui::form;

use super::{section::UserPluginsSection, session::HeldBy};

/// The "Allow user plugins" row. It is absent in a developer session, which loads its one file
/// whatever the stored choice is.
pub(super) fn switch_row(
    section: &UserPluginsSection,
    cx: &mut Context<UserPluginsSection>,
) -> Option<AnyElement> {
    if section.dev_location(cx).is_some() {
        return None;
    }
    let allowed = Switch::new("user-plugins-allowed")
        .checked(section.allowed(cx))
        .on_click(cx.listener(|section, allowed: &bool, _, cx| {
            section.set_allowed(*allowed, cx);
        }));
    Some(form::row(
        "Allow user plugins",
        "Loads the plugins you installed when Nocterm starts. Off by default; a change applies at \
         the next start.",
        allowed,
        cx,
    ))
}

/// Why this run has no user plugins, and what a changed switch needs.
pub(super) fn explanation(
    section: &UserPluginsSection,
    cx: &mut Context<UserPluginsSection>,
) -> Vec<AnyElement> {
    let mut lines = Vec::new();
    if let Some(held) = section.held() {
        let text = match held {
            HeldBy::SafeMode => {
                "Safe mode: no user plugin is loaded in this run, whatever the switch says. \
                 Start Nocterm without --safe-mode to load them."
                    .to_owned()
            }
            HeldBy::NotAllowed => {
                "User plugins are off. Nothing you installed is loaded, and nothing can be \
                 installed, until you allow them and restart Nocterm."
                    .to_owned()
            }
            HeldBy::Unreadable { reason } => format!(
                "The [plugins] table of settings.toml does not read ({reason}), so no user \
                 plugin is loaded in this run. Fix the file and restart Nocterm."
            ),
        };
        lines.push(line("user-plugins-held", text, cx));
    }
    if section.dev_location(cx).is_none() && section.allowed(cx) != section.loaded() {
        let text = if section.allowed(cx) {
            "Restart Nocterm to load user plugins."
        } else {
            "Restart Nocterm to stop loading user plugins."
        };
        lines.push(line("user-plugins-restart", text, cx));
    }
    lines
}

fn line(
    id: &'static str,
    text: impl Into<SharedString>,
    cx: &Context<UserPluginsSection>,
) -> AnyElement {
    let text = text.into();
    div()
        .id(id)
        .aria_label(text.clone())
        .test_support()
        .child(form::note(text, cx))
        .into_any_element()
}
