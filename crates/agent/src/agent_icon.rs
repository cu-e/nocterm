//! The icon of an agent: its brand mark when nocterm knows the agent, a
//! generic bot otherwise.
use gpui_kit::{Styled as _, component::Icon};
use nocterm_ai::agents::AgentProfile;
use nocterm_ui::IconName;

pub(crate) fn agent_icon(profile: Option<AgentProfile>) -> Icon {
    let Some(mark) = profile.as_ref().and_then(AgentProfile::mark) else {
        return Icon::new(IconName::Bot);
    };
    let icon = Icon::empty().path(mark.asset());
    match mark.tint() {
        Some(rgb) => icon.text_color(gpui_kit::rgb(rgb)),
        None => icon,
    }
}
