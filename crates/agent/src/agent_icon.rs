//! The icon of an agent: its brand mark when nocterm knows the agent, a
//! generic bot otherwise.
use gpui_kit::component::Icon;
use nocterm_ai::agents::AgentProfile;

pub(crate) fn agent_icon(profile: Option<AgentProfile>) -> Icon {
    let mark = profile.as_ref().and_then(AgentProfile::mark);
    nocterm_ui::agent_icon(
        mark.map(|mark| mark.asset()),
        mark.and_then(|mark| mark.tint()),
    )
}
