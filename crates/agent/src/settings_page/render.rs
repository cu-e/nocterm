//! The AI agents page: the master switch, permissions, isolation and the
//! agents themselves.
use super::{
    AgentSettingsPage,
    agents::{edit_agent, save_checked},
    controls::{text_row, toggle},
    field::{FieldError, optional},
};
use crate::agent_icon::agent_icon;
use gpui_kit::{
    AnyElement, Context, SharedString, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Input,
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_ai::{
    AgentCatalog, AgentServerSettings, AiSettings, ApprovalPolicy, SandboxMode,
    agents::AgentProfile, sandbox::Availability, validate_agent,
};
use nocterm_ui::{IconName, form};
pub(super) fn render(
    view: &mut AgentSettingsPage,
    cx: &mut Context<AgentSettingsPage>,
) -> AnyElement {
    let ai = view.installed.settings().setting::<AiSettings>(cx).clone();
    let off = !ai.enabled;
    let mut page = v_flex()
        .w_full()
        .child(form::page_header(
            "AI agents",
            Some("Chat with coding agents about your terminals. Agents run on this computer as your user.".into()),
            cx,
        ))
        .child(general_section(view, &ai, off, cx))
        .child(permissions_section(&ai, off, cx))
        .child(terminal_access_section(&ai, off, cx))
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
fn general_section(
    view: &AgentSettingsPage,
    ai: &AiSettings,
    off: bool,
    cx: &mut Context<AgentSettingsPage>,
) -> AnyElement {
    form::section(
        "General",
        [
            form::row(
                "Enable AI agents",
                "Off hides the agent panel and stops every running agent.",
                toggle(
                    "ai-enabled",
                    ai.enabled,
                    false,
                    |s: &mut AiSettings, on| s.enabled = on,
                    cx,
                ),
                cx,
            ),
            form::row(
                "Default agent",
                "The agent a new chat starts with.",
                default_agent(view, ai, off, cx),
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
    )
}
fn permissions_section(
    ai: &AiSettings,
    off: bool,
    cx: &mut Context<AgentSettingsPage>,
) -> AnyElement {
    form::section(
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
    )
}
fn terminal_access_section(
    ai: &AiSettings,
    off: bool,
    cx: &mut Context<AgentSettingsPage>,
) -> AnyElement {
    form::section(
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
    )
}

fn policy(ask: bool) -> ApprovalPolicy {
    if ask {
        ApprovalPolicy::Ask
    } else {
        ApprovalPolicy::Allow
    }
}

fn default_agent(
    view: &AgentSettingsPage,
    ai: &AiSettings,
    disabled: bool,
    cx: &mut Context<AgentSettingsPage>,
) -> AnyElement {
    let agents = view.installed.agents().available(cx);
    let current = ai
        .default_agent
        .clone()
        .filter(|id| agents.get(id).is_some());
    let options = std::iter::once((None, SharedString::from("Ask"), None))
        .chain(agents.iter().map(|agent| {
            (
                Some(agent.id().to_owned()),
                agent.name().to_owned().into(),
                Some(agent_icon(agent.profile().copied())),
            )
        }))
        .collect::<Vec<_>>();
    h_flex()
        .gap_1()
        .flex_wrap()
        .justify_end()
        .children(
            options
                .into_iter()
                .enumerate()
                .map(|(index, (id, name, icon))| {
                    Button::new(("ai-default-agent", index))
                        .small()
                        .ghost()
                        .when_some(icon, Button::icon)
                        .label(name)
                        .disabled(disabled)
                        .selected(current == id)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.save(move |s: &mut AiSettings| s.default_agent = id, cx)
                        }))
                }),
        )
        .into_any_element()
}

fn isolation(
    view: &AgentSettingsPage,
    ai: &AiSettings,
    off: bool,
    cx: &mut Context<AgentSettingsPage>,
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

fn agent(
    view: &AgentSettingsPage,
    id: &str,
    ai: &AiSettings,
    off: bool,
    cx: &mut Context<AgentSettingsPage>,
) -> AnyElement {
    let mut block = v_flex()
        .w_full()
        .child(agent_heading(view, id, ai, off, cx));
    if view.ai.expanded.contains(id) {
        block = block.child(agent_details(view, id, cx));
    }
    block.into_any_element()
}
fn agent_label(catalog: &AgentCatalog, id: &str, ai: &AiSettings) -> (String, String, bool) {
    let setting = ai.agents.get(id).cloned().unwrap_or_default();
    let default = catalog.get(id);
    let name = setting
        .name
        .clone()
        .or_else(|| default.map(|profile| profile.name().to_owned()))
        .unwrap_or_else(|| id.to_owned());
    let description = match default.map(AgentProfile::launch_defaults) {
        Some(launch) => format!("{} {}", launch.command(), launch.args().join(" ")),
        None => setting.command.clone().unwrap_or_default(),
    };
    (name, description, setting.enabled)
}
fn agent_heading(
    view: &AgentSettingsPage,
    id: &str,
    ai: &AiSettings,
    off: bool,
    cx: &mut Context<AgentSettingsPage>,
) -> AnyElement {
    let (name, description, enabled) = agent_label(view.installed.agents().catalog(), id, ai);
    let expanded = view.ai.expanded.contains(id);
    let toggle_id = id.to_owned();
    let enable_id = id.to_owned();
    let header = h_flex()
        .w_full()
        .py_3()
        .gap_3()
        .child(
            h_flex()
                .id(SharedString::from(format!("ai-agent-{id}")))
                .test_support()
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
                .child(agent_icon(view.installed.agents().profile(id, cx)).small())
                .child(div().text_sm().font_medium().child(name))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(description),
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
                let catalog = this.installed.agents().catalog().clone();
                save_checked(
                    this,
                    move |s| {
                        edit_agent(&catalog, s, &id, |a| {
                            a.enabled = checked;
                            Ok(())
                        })
                    },
                    cx,
                );
            })),
        );
    header.into_any_element()
}
fn agent_details(
    view: &AgentSettingsPage,
    id: &str,
    cx: &mut Context<AgentSettingsPage>,
) -> AnyElement {
    let default = view.installed.agents().catalog().get(id);
    let key = |name: &str| format!("agent.{id}.{name}");
    let command_hint = match default {
        Some(profile) => format!(
            "Empty runs `{}`. Use an absolute path if the desktop cannot find it.",
            profile.launch_defaults().command()
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
    details.into_any_element()
}

fn add_agent(
    view: &AgentSettingsPage,
    off: bool,
    cx: &mut Context<AgentSettingsPage>,
) -> AnyElement {
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
                        .child(Input::new(&id).id("ai-new-id").small().disabled(off)),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&command)
                            .id("ai-new-command")
                            .small()
                            .disabled(off),
                    ),
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
            row.child(
                div()
                    .id("ai-add-error")
                    .test_support()
                    .child(form::error_text(error, cx)),
            )
        })
        .into_any_element()
}

fn add(view: &mut AgentSettingsPage, window: &mut Window, cx: &mut Context<AgentSettingsPage>) {
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
        Err(FieldError::DuplicateAgent(id.clone()))
    } else {
        validate_agent(view.installed.agents().catalog(), &id, &agent).map_err(FieldError::from)
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
        Err(error) => view.ai.add_error = Some(error.to_string().into()),
    }
    cx.notify();
}
