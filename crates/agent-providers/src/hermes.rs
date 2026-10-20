//! Nous Research Hermes Agent, which speaks ACP itself.
use nocterm_ai::agents::{AgentMark, AgentProfile, LaunchDefaults};

pub(crate) const PROFILE: AgentProfile = AgentProfile::new(
    "hermes",
    "Hermes",
    LaunchDefaults::new("hermes", &["acp"], &["HERMES_HOME"]),
)
.writes_home(&[".hermes"])
.with_mark(AgentMark::new(
    "icons/agents/hermes.svg",
    include_bytes!("../assets/hermes.svg"),
));
