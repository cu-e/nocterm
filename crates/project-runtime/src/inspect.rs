use crate::command::Commands;
use nocterm_containers::Engine;
use nocterm_project_providers::{ProjectSetupError, SetupPhase};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
pub(crate) struct Inspection {
    #[serde(rename = "Id")]
    pub(crate) id: String,
    #[serde(rename = "Config")]
    pub(crate) config: Config,
    #[serde(rename = "State")]
    pub(crate) state: State,
}
#[derive(Deserialize)]
pub(crate) struct Config {
    #[serde(rename = "WorkingDir", default)]
    pub(crate) workdir: String,
    #[serde(rename = "Labels", default)]
    labels: Option<BTreeMap<String, String>>,
}
#[derive(Deserialize)]
pub(crate) struct State {
    #[serde(rename = "Status")]
    pub(crate) status: String,
}
impl Inspection {
    pub(crate) fn owned_by(
        &self,
        project: nocterm_project_model::ProjectId,
        attempt: &nocterm_project_providers::CreationToken,
    ) -> bool {
        self.config
            .labels
            .as_ref()
            .and_then(|labels| labels.get("io.nocterm.project"))
            == Some(&project.to_string())
            && self
                .config
                .labels
                .as_ref()
                .and_then(|labels| labels.get("io.nocterm.attempt"))
                .is_some_and(|label| label == attempt.as_str())
    }
}
pub(crate) async fn inspect(
    commands: &Commands,
    engine: Engine,
    id: &str,
) -> Result<Inspection, ProjectSetupError> {
    let output = commands
        .run(
            SetupPhase::Inspecting,
            engine
                .command()
                .args(["inspect", "--type", "container", id]),
        )
        .await?;
    let mut entries: Vec<Inspection> = serde_json::from_str(&output)?;
    if entries.len() != 1 {
        return Err(ProjectSetupError::ContainerMissing);
    }
    let entry = entries.remove(0);
    if entry.id != id {
        return Err(ProjectSetupError::ContainerMissing);
    }
    Ok(entry)
}
