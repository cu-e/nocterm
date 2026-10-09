//! The AI agents page: the master switch, permissions, isolation and the
//! agents themselves.
use std::collections::BTreeSet;

use gpui_kit::{
    AnyElement, Context, Entity, SharedString, Window,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_ai::{
    registry::AgentRegistry,
    sandbox::{Availability, current_availability},
};
use nocterm_settings::{AgentServerSettings, AiSettings, ApprovalPolicy, SandboxMode};
use nocterm_ui::{IconName, SettingsExt as _, form};

use crate::{
    SettingsView,
    field::{environment, json, optional, string_list},
    pages::{text_row, toggle},
};

const BUILTIN: [&str; 3] = ["claude", "codex", "hermes"];

#[derive(Default)]
pub(crate) struct AiPage {
    /// Agents whose details are shown.
    expanded: BTreeSet<String>,
    /// Agents with fields, in display order.
    agents: Vec<String>,
    new_id: Option<Entity<InputState>>,
    new_command: Option<Entity<InputState>>,
    add_error: Option<SharedString>,
    /// Why the last switch could not be saved.
    error: Option<SharedString>,
    sandbox: Option<Availability>,
}

fn agent_ids(settings: &AiSettings) -> Vec<String> {
    let mut ids: Vec<String> = BUILTIN.map(str::to_owned).to_vec();
    ids.extend(
        settings
            .agents
            .keys()
            .filter(|id| !BUILTIN.contains(&id.as_str()))
            .cloned(),
    );
    ids
}

pub(crate) fn add_fields(
    view: &mut SettingsView,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) {
    view.add_field(
        "ai.working_directory",
        |s: &AiSettings| s.working_directory.clone().unwrap_or_default(),
        |s: &mut AiSettings, text| {
            let path = optional(text);
            if path.as_deref().is_some_and(|path| {
                !std::path::Path::new(path).is_absolute() || path.contains('\0')
            }) {
                return Err(
                    "Enter an absolute path, or leave empty for the private workspace.".into(),
                );
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
    view.ai.sandbox = Some(current_availability());
    sync(view, window, cx);
}

/// Adds fields for agents that appeared and drops those of removed ones.
pub(crate) fn sync(view: &mut SettingsView, window: &mut Window, cx: &mut Context<SettingsView>) {
    let ids = agent_ids(cx.setting::<AiSettings>());
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
    view: &mut SettingsView,
    id: String,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) {
    type Get = fn(&AgentServerSettings) -> String;
    type Set = fn(&mut AgentServerSettings, &str) -> Result<(), String>;
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
                a.inherit_env = serde_json::from_str(text)
                    .map_err(|_| "Inherited variables: enter a JSON array of names.".to_owned())?;
                Ok(())
            },
        ),
    ];
    for (name, get, set) in fields {
        let read_id = id.clone();
        let write_id = id.clone();
        view.add_field(
            format!("agent.{id}.{name}"),
            move |s: &AiSettings| get(&s.agents.get(&read_id).cloned().unwrap_or_default()),
            move |s, text| edit_agent(s, &write_id, |agent| set(agent, text)),
            window,
            cx,
        );
    }
}

/// Changes agent `id`, keeping only what differs from the built-in defaults.
fn edit_agent(
    settings: &mut AiSettings,
    id: &str,
    edit: impl FnOnce(&mut AgentServerSettings) -> Result<(), String>,
) -> Result<(), String> {
    let mut agent = settings.agents.get(id).cloned().unwrap_or_default();
    edit(&mut agent)?;
    validate_agent(id, &agent)?;
    if BUILTIN.contains(&id) && agent == AgentServerSettings::default() {
        settings.agents.remove(id);
    } else {
        settings.agents.insert(id.to_owned(), agent);
    }
    Ok(())
}

pub(crate) fn validate_agent(id: &str, setting: &AgentServerSettings) -> Result<(), String> {
    if !nocterm_ai::registry::valid_id(id) {
        return Err(
            "Agent ids use letters, numbers, underscore or dash, up to 64 characters.".into(),
        );
    }
    if setting.enabled && setting.command.is_none() && !BUILTIN.contains(&id) {
        return Err(format!("{id}: enter an executable."));
    }
    if setting
        .env
        .keys()
        .any(|key| nocterm_ai::env::credential_override(key))
    {
        return Err(format!(
            "{id}: inherit credential variables from your environment instead of storing them in settings."
        ));
    }
    if setting
        .env
        .keys()
        .chain(setting.inherit_env.iter())
        .any(|key| !nocterm_ai::env::valid_name(key) || nocterm_ai::env::blocked(key))
    {
        return Err(format!(
            "{id}: environment names must be identifiers; SSH, Nocterm and process injection variables are forbidden."
        ));
    }
    if setting
        .command
        .iter()
        .chain(setting.name.iter())
        .chain(setting.args.iter().flatten())
        .chain(setting.env.values())
        .any(|v| v.contains('\0'))
    {
        return Err(format!("{id}: values cannot contain NUL."));
    }
    Ok(())
}

/// Saves `edit` unless it would make the AI settings invalid.
fn save_checked(
    view: &mut SettingsView,
    edit: impl Fn(&mut AiSettings) -> Result<(), String> + 'static,
    cx: &mut Context<SettingsView>,
) {
    let mut probe = cx.setting::<AiSettings>().clone();
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
            view.ai.error = Some(error.into());
            cx.notify();
        }
    }
}

#[expect(clippy::too_many_lines, reason = "predates the limit")]
pub(crate) fn render(view: &mut SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let ai = cx.setting::<AiSettings>().clone();
    let off = !ai.enabled;
    let mut page = v_flex()
        .w_full()
        .child(form::page_header(
            "AI agents",
            Some("Chat with coding agents about your terminals. Agents run on this computer as your user.".into()),
            cx,
        ))
        .child(form::section(
            "General",
            [
                form::row(
                    "Enable AI agents",
                    "Off hides the agent panel and stops every running agent.",
                    toggle("ai-enabled", ai.enabled, false, |s: &mut AiSettings, on| s.enabled = on, cx),
                    cx,
                ),
                form::row(
                    "Default agent",
                    "The agent a new chat starts with.",
                    default_agent(&ai, off, cx),
                    cx,
                ),
                text_row(
                    view,
                    "ai.working_directory",
                    "Working directory",
                    "Where agents start. Empty uses a private folder in nocterm's state directory.",
                    cx,
                ),
            ],
            cx,
        ))
        .child(form::section(
            "Agent permissions",
            [form::row(
                "Ask before an agent uses its own tools",
                "Confirm the agent's requests to use its own files and tools. Off approves each request once; agents without an Allow once choice still ask. Terminal access is controlled separately below.",
                toggle(
                    "ai-agent-permissions",
                    ai.approval.agent_permissions == ApprovalPolicy::Ask,
                    off,
                    |s: &mut AiSettings, on| s.approval.agent_permissions = policy(on),
                    cx,
                ),
                cx,
            )],
            cx,
        ))
        .child(form::section(
            "Terminal access",
            [
                form::row(
                    "Ask before reading a terminal",
                    "Confirm each time an agent reads the output of an attached terminal, unless you allow that terminal for the chat.",
                    toggle(
                        "ai-read-approval",
                        ai.approval.terminal_read == ApprovalPolicy::Ask,
                        off,
                        |s: &mut AiSettings, on| s.approval.terminal_read = policy(on),
                        cx,
                    ),
                    cx,
                ),
                form::row(
                    "Ask before typing or running commands",
                    "Confirm each time an agent types into a terminal, runs a command or opens a server in the background.",
                    toggle(
                        "ai-write-approval",
                        ai.approval.terminal_write == ApprovalPolicy::Ask,
                        off,
                        |s: &mut AiSettings, on| s.approval.terminal_write = policy(on),
                        cx,
                    ),
                    cx,
                ),
                form::row(
                    "Hide secrets in terminal output",
                    "Replace private keys, API tokens, passwords and credentials in URLs before an agent sees them. Best effort: unusual secrets can slip through.",
                    toggle(
                        "ai-redact",
                        ai.approval.redact_secrets,
                        off,
                        |s: &mut AiSettings, on| s.approval.redact_secrets = on,
                        cx,
                    ),
                    cx,
                ),
            ],
            cx,
        ))
        .child(form::section("Isolation", [isolation(view, &ai, off, cx)], cx))
        .when_some(view.ai.error.clone(), |page, error| page.child(form::error_text(error, cx)));
    let mut agents: Vec<AnyElement> = Vec::new();
    for id in view.ai.agents.clone() {
        agents.push(agent(view, &id, &ai, off, cx));
    }
    agents.push(add_agent(view, off, cx));
    page = page.child(form::section("Agents", agents, cx));
    page.into_any_element()
}

fn policy(ask: bool) -> ApprovalPolicy {
    if ask {
        ApprovalPolicy::Ask
    } else {
        ApprovalPolicy::Allow
    }
}

fn default_agent(ai: &AiSettings, disabled: bool, cx: &mut Context<SettingsView>) -> AnyElement {
    let registry = AgentRegistry::new(ai);
    let current = ai
        .default_agent
        .clone()
        .filter(|id| registry.get(id).is_some());
    let options = std::iter::once((None, SharedString::from("Ask")))
        .chain(
            registry
                .iter()
                .map(|launch| (Some(launch.id.clone()), launch.name.clone().into())),
        )
        .collect::<Vec<_>>();
    h_flex()
        .gap_1()
        .flex_wrap()
        .justify_end()
        .children(options.into_iter().enumerate().map(|(index, (id, name))| {
            Button::new(("ai-default-agent", index))
                .small()
                .ghost()
                .when_some(id.as_deref().map(nocterm_ui::agent_icon), Button::icon)
                .label(name)
                .disabled(disabled)
                .selected(current == id)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.save(move |s: &mut AiSettings| s.default_agent = id, cx)
                }))
        }))
        .into_any_element()
}

fn isolation(
    view: &SettingsView,
    ai: &AiSettings,
    off: bool,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let on = ai.sandbox == SandboxMode::Workspace;
    let (available, status) = match view.ai.sandbox.clone().unwrap_or(Availability::Unsupported) {
        Availability::Available(path) => (true, format!("Uses bubblewrap ({}).", path.display())),
        Availability::Missing => (
            false,
            "Install bubblewrap (the `bwrap` command) to use this.".to_owned(),
        ),
        Availability::Unsupported => (false, "Available on Linux only.".to_owned()),
    };
    form::row(
        "Isolate agents",
        format!(
            "Agents can change only their working directory and their own settings and caches. SSH and GPG keys, cloud credentials, password stores and nocterm's data are hidden. Turning this on or off restarts running agents. {status}"
        ),
        toggle(
            "ai-sandbox",
            on,
            off || (!available && !on),
            |s: &mut AiSettings, on| {
                s.sandbox = if on {
                    SandboxMode::Workspace
                } else {
                    SandboxMode::Off
                }
            },
            cx,
        ),
        cx,
    )
}

#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn agent(
    view: &SettingsView,
    id: &str,
    ai: &AiSettings,
    off: bool,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let setting = ai.agents.get(id).cloned().unwrap_or_default();
    let builtin = AgentRegistry::new(&AiSettings::default());
    let default = builtin.get(id);
    let name = setting
        .name
        .clone()
        .or_else(|| default.map(|launch| launch.name.clone()))
        .unwrap_or_else(|| id.to_owned());
    let expanded = view.ai.expanded.contains(id);
    let toggle_id = id.to_owned();
    let enable_id = id.to_owned();
    let enabled = setting.enabled;
    let header = h_flex()
        .w_full()
        .py_3()
        .gap_3()
        .child(
            h_flex()
                .id(SharedString::from(format!("ai-agent-{id}")))
                .flex_1()
                .min_w_0()
                .gap_2()
                .cursor_pointer()
                .child(
                    gpui_kit::component::Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .small()
                    .text_color(cx.theme().muted_foreground),
                )
                .child(nocterm_ui::agent_icon(id).small())
                .child(div().text_sm().font_medium().child(name))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(match default {
                            Some(launch) => format!("{} {}", launch.command, launch.args.join(" ")),
                            None => setting.command.clone().unwrap_or_default(),
                        }),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.ai.expanded.remove(&toggle_id) {
                        this.ai.expanded.insert(toggle_id.clone());
                    }
                    cx.notify();
                })),
        )
        .child(
            gpui_kit::component::switch::Switch::new(SharedString::from(format!(
                "ai-agent-enabled-{id}"
            )))
            .checked(enabled)
            .disabled(off)
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                let id = enable_id.clone();
                let checked = *checked;
                save_checked(
                    this,
                    move |s| {
                        edit_agent(s, &id, |a| {
                            a.enabled = checked;
                            Ok(())
                        })
                    },
                    cx,
                );
            })),
        );
    let mut block = v_flex().w_full().child(header);
    if expanded {
        let key = |name: &str| format!("agent.{id}.{name}");
        let command_hint = match default {
            Some(launch) => format!(
                "Empty runs `{}`. Use an absolute path if the desktop cannot find it.",
                launch.command
            ),
            None => "The program that speaks the Agent Client Protocol.".into(),
        };
        let mut details = v_flex()
            .w_full()
            .pl_6()
            .pb_2()
            .child(text_row(view, &key("name"), "Name", "Shown in the agent panel. Empty uses the built-in name.", cx))
            .child(text_row(view, &key("command"), "Executable", command_hint, cx))
            .child(text_row(view, &key("args"), "Arguments", "JSON array of strings, passed without a shell. Empty keeps the built-in arguments; [] passes none.", cx))
            .child(text_row(view, &key("env"), "Environment", "JSON object of extra variables. Stored in plain text: never put keys or passwords here.", cx))
            .child(text_row(view, &key("inherit"), "Inherited variables", "JSON array of names of your own variables the agent may see, such as an API key.", cx));
        if default.is_none() {
            let remove_id = id.to_owned();
            details = details.child(
                div().pt_2().child(
                    Button::new(SharedString::from(format!("ai-agent-remove-{id}")))
                        .small()
                        .ghost()
                        .icon(IconName::Trash)
                        .label("Remove agent")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = remove_id.clone();
                            this.save(
                                move |s: &mut AiSettings| {
                                    s.agents.remove(&id);
                                    if s.default_agent.as_deref() == Some(id.as_str()) {
                                        s.default_agent = None;
                                    }
                                },
                                cx,
                            );
                        })),
                ),
            );
        }
        block = block.child(details);
    }
    block.into_any_element()
}

fn add_agent(view: &SettingsView, off: bool, cx: &mut Context<SettingsView>) -> AnyElement {
    let (Some(id), Some(command)) = (view.ai.new_id.clone(), view.ai.new_command.clone()) else {
        return div().into_any_element();
    };
    v_flex()
        .w_full()
        .py_3()
        .gap_2()
        .child(div().text_sm().font_medium().child("Add an agent"))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .child(
                    div()
                        .w(gpui_kit::rems(10.))
                        .child(Input::new(&id).small().disabled(off)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&command).small().disabled(off)),
                )
                .child(
                    Button::new("ai-add-agent")
                        .small()
                        .label("Add")
                        .disabled(off)
                        .on_click(cx.listener(|this, _, window, cx| add(this, window, cx))),
                ),
        )
        .when_some(view.ai.add_error.clone(), |row, error| {
            row.child(form::error_text(error, cx))
        })
        .into_any_element()
}

fn add(view: &mut SettingsView, window: &mut Window, cx: &mut Context<SettingsView>) {
    let (Some(id_input), Some(command_input)) =
        (view.ai.new_id.clone(), view.ai.new_command.clone())
    else {
        return;
    };
    let id = id_input.read(cx).value().trim().to_owned();
    let command = optional(&command_input.read(cx).value());
    let agent = AgentServerSettings {
        command,
        ..AgentServerSettings::default()
    };
    let result = if view.ai.agents.contains(&id) {
        Err(format!("An agent named `{id}` already exists."))
    } else {
        validate_agent(&id, &agent)
    };
    match result {
        Ok(()) => {
            view.ai.add_error = None;
            view.ai.expanded.insert(id.clone());
            view.save(
                move |s: &mut AiSettings| {
                    s.agents.insert(id, agent);
                },
                cx,
            );
            id_input.update(cx, |input, cx| input.set_value("", window, cx));
            command_input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        Err(error) => view.ai.add_error = Some(error.into()),
    }
    cx.notify();
}

#[cfg(test)]
impl AiPage {
    pub(crate) fn new_id_input(&self) -> Entity<InputState> {
        self.new_id.clone().expect("created with the page")
    }
    pub(crate) fn new_command_input(&self) -> Entity<InputState> {
        self.new_command.clone().expect("created with the page")
    }
    pub(crate) fn has_add_error(&self) -> bool {
        self.add_error.is_some()
    }
}

#[cfg(test)]
pub(crate) fn add_for_test(
    view: &mut SettingsView,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) {
    add(view, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_custom_and_prohibited_environment() {
        let mut agent = AgentServerSettings::default();
        assert!(validate_agent("custom", &agent).is_err());
        agent.command = Some("/opt/acp".into());
        assert!(validate_agent("custom", &agent).is_ok());
        agent.env.insert("OPENAI_API_KEY".into(), "marker".into());
        assert!(validate_agent("custom", &agent).is_err());
        agent.env.clear();
        agent.inherit_env.push("SSH_AUTH_SOCK".into());
        assert!(validate_agent("custom", &agent).is_err());
        agent.inherit_env.clear();
        agent.args = Some(vec!["nul\0".into()]);
        assert!(validate_agent("custom", &agent).is_err());
    }

    #[test]
    fn editing_a_builtin_back_to_its_defaults_removes_the_override() {
        let mut settings = AiSettings::default();
        edit_agent(&mut settings, "claude", |a| {
            a.command = Some("/opt/claude".into());
            Ok(())
        })
        .unwrap();
        assert!(settings.agents.contains_key("claude"));
        edit_agent(&mut settings, "claude", |a| {
            a.command = None;
            Ok(())
        })
        .unwrap();
        assert!(settings.agents.is_empty());
        assert!(
            edit_agent(&mut settings, "mine", |_| Ok(())).is_err(),
            "a custom agent needs an executable"
        );
        assert!(settings.agents.is_empty());
    }
}
