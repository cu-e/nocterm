//! The AI agents page: the master switch, permissions, isolation and the
//! agents themselves.
use super::{
    AgentSettingsPage, AiPage,
    field::{FieldError, environment, json, optional, string_list},
};
use gpui_kit::{AppContext as _, Context, Window, component::input::InputState};
use nocterm_ai::{
    AgentCatalog, AgentServerSettings, AiSettings, sandbox::SandboxProbe, validate_agent,
};
use std::collections::BTreeSet;
impl AiPage {
    pub(super) fn new(probe: std::sync::Arc<dyn SandboxProbe>) -> Self {
        Self {
            probe,
            expanded: BTreeSet::new(),
            agents: Vec::new(),
            new_id: None,
            new_command: None,
            add_error: None,
            error: None,
            sandbox: None,
        }
    }
}

fn agent_ids(catalog: &AgentCatalog, settings: &AiSettings) -> Vec<String> {
    let mut ids: Vec<String> = catalog
        .iter()
        .map(|profile| profile.id().to_owned())
        .collect();
    ids.extend(
        settings
            .agents
            .keys()
            .filter(|id| !catalog.is_builtin(id))
            .cloned(),
    );
    ids
}

pub(super) fn add_fields(
    view: &mut AgentSettingsPage,
    window: &mut Window,
    cx: &mut Context<AgentSettingsPage>,
) {
    view.add_field(
        "ai.working_directory",
        |s: &AiSettings| s.working_directory.clone().unwrap_or_default(),
        |s: &mut AiSettings, text| {
            let path = optional(text);
            if path.as_deref().is_some_and(|path| {
                !std::path::Path::new(path).is_absolute() || path.contains('\0')
            }) {
                return Err(FieldError::InvalidWorkingDirectory);
            }
            s.working_directory = path;
            Ok(())
        },
        window,
        cx,
    );
    view.ai.new_id = Some(cx.new(|cx| InputState::new(window, cx).placeholder("Id, e.g. gemini")));
    view.ai.new_command = Some(
        cx.new(|cx| InputState::new(window, cx).placeholder("Executable, e.g. /usr/bin/gemini")),
    );
    view.ai.sandbox = Some(view.ai.probe.availability());
    sync(view, window, cx);
}

/// Adds fields for agents that appeared and drops those of removed ones.
pub(super) fn sync(
    view: &mut AgentSettingsPage,
    window: &mut Window,
    cx: &mut Context<AgentSettingsPage>,
) {
    let ids = agent_ids(
        view.agents().catalog(),
        view.settings().setting::<AiSettings>(cx),
    );
    for id in view.ai.agents.clone() {
        if !ids.contains(&id) {
            view.remove_fields(&format!("agent.{id}."));
            view.ai.expanded.remove(&id);
        }
    }
    for id in &ids {
        if !view.ai.agents.contains(id) {
            agent_fields(view, id.clone(), window, cx);
        }
    }
    view.ai.agents = ids;
}

fn agent_fields(
    view: &mut AgentSettingsPage,
    id: String,
    window: &mut Window,
    cx: &mut Context<AgentSettingsPage>,
) {
    type Get = fn(&AgentServerSettings) -> String;
    type Set = fn(&mut AgentServerSettings, &str) -> Result<(), FieldError>;
    let fields: [(&str, Get, Set); 5] = [
        (
            "name",
            |a| a.name.clone().unwrap_or_default(),
            |a, text| {
                a.name = optional(text);
                Ok(())
            },
        ),
        (
            "command",
            |a| a.command.clone().unwrap_or_default(),
            |a, text| {
                a.command = optional(text);
                Ok(())
            },
        ),
        (
            "args",
            |a| a.args.as_ref().map(json).unwrap_or_default(),
            |a, text| {
                a.args = if text.trim().is_empty() {
                    None
                } else {
                    Some(string_list(text, "Arguments")?)
                };
                Ok(())
            },
        ),
        (
            "env",
            |a| json(&a.env),
            |a, text| {
                a.env = environment(text, "Environment")?;
                Ok(())
            },
        ),
        (
            "inherit",
            |a| json(&a.inherit_env),
            |a, text| {
                a.inherit_env =
                    serde_json::from_str(text).map_err(|_| FieldError::InheritedNotNames)?;
                Ok(())
            },
        ),
    ];
    for (name, get, set) in fields {
        let read_id = id.clone();
        let write_id = id.clone();
        let catalog = view.agents().catalog().clone();
        view.add_field(
            format!("agent.{id}.{name}"),
            move |s: &AiSettings| get(&s.agents.get(&read_id).cloned().unwrap_or_default()),
            move |s, text| edit_agent(&catalog, s, &write_id, |agent| set(agent, text)),
            window,
            cx,
        );
    }
}

/// Changes agent `id`, keeping only what differs from the built-in defaults.
pub(super) fn edit_agent(
    catalog: &AgentCatalog,
    settings: &mut AiSettings,
    id: &str,
    edit: impl FnOnce(&mut AgentServerSettings) -> Result<(), FieldError>,
) -> Result<(), FieldError> {
    let mut agent = settings.agents.get(id).cloned().unwrap_or_default();
    edit(&mut agent)?;
    validate_agent(catalog, id, &agent)?;
    if catalog.is_builtin(id) && agent == AgentServerSettings::default() {
        settings.agents.remove(id);
    } else {
        settings.agents.insert(id.to_owned(), agent);
    }
    Ok(())
}

/// Saves `edit` unless it would make the AI settings invalid.
pub(super) fn save_checked(
    view: &mut AgentSettingsPage,
    edit: impl Fn(&mut AiSettings) -> Result<(), FieldError> + 'static,
    cx: &mut Context<AgentSettingsPage>,
) {
    let mut probe = view.settings().setting::<AiSettings>(cx).clone();
    match edit(&mut probe) {
        Ok(()) => {
            view.ai.error = None;
            view.save(
                move |settings: &mut AiSettings| {
                    let _ = edit(settings);
                },
                cx,
            );
        }
        Err(error) => {
            view.ai.error = Some(error.to_string().into());
            cx.notify();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_a_builtin_back_to_its_defaults_removes_the_override() {
        let catalog = nocterm_agent_providers::catalog();
        let mut settings = AiSettings::default();
        edit_agent(&catalog, &mut settings, "claude", |a| {
            a.command = Some("/opt/claude".into());
            Ok(())
        })
        .unwrap();
        assert!(settings.agents.contains_key("claude"));
        edit_agent(&catalog, &mut settings, "claude", |a| {
            a.command = None;
            Ok(())
        })
        .unwrap();
        assert!(settings.agents.is_empty());
        assert!(
            edit_agent(&catalog, &mut settings, "mine", |_| Ok(())).is_err(),
            "a custom agent needs an executable"
        );
        assert!(settings.agents.is_empty());
    }
}
