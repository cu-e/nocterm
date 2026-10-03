//! AI settings use the same optimistic draft and Apply boundary as the other pages.
use super::SettingsView;
use gpui_kit::{
    App, Context, Entity, Window,
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
use nocterm_ai::registry::AgentRegistry;
use nocterm_settings::{AgentServerSettings, AiSettings, ApprovalPolicy};
use std::collections::BTreeMap;

pub(super) struct AgentRow {
    id: String,
    enabled: bool,
    inputs: Vec<Entity<InputState>>,
}
pub(super) struct AiForm {
    inputs: Vec<Entity<InputState>>,
    agents: Vec<AgentRow>,
}
impl AiForm {
    pub(super) fn new(
        settings: &AiSettings,
        window: &mut Window,
        cx: &mut Context<SettingsView>,
    ) -> Self {
        let inputs = [
            settings.default_agent.clone().unwrap_or_default(),
            settings.working_directory.clone().unwrap_or_default(),
            String::new(),
        ]
        .into_iter()
        .map(|value| cx.new(|cx| InputState::new(window, cx).default_value(value)))
        .collect();
        let mut ids: Vec<String> = vec!["claude".into(), "codex".into(), "hermes".into()];
        ids.extend(
            settings
                .agents
                .keys()
                .filter(|id| !matches!(id.as_str(), "claude" | "codex" | "hermes"))
                .cloned(),
        );
        let agents = ids
            .into_iter()
            .map(|id| {
                Self::row(
                    id.clone(),
                    settings.agents.get(&id).cloned().unwrap_or_default(),
                    window,
                    cx,
                )
            })
            .collect();
        Self { inputs, agents }
    }
    fn row(
        id: String,
        setting: AgentServerSettings,
        window: &mut Window,
        cx: &mut Context<SettingsView>,
    ) -> AgentRow {
        let values = [
            setting.name.unwrap_or_default(),
            setting.command.unwrap_or_default(),
            setting
                .args
                .as_ref()
                .map(|v| serde_json::to_string(v).expect("args"))
                .unwrap_or_default(),
            serde_json::to_string(&setting.env).expect("env"),
            serde_json::to_string(&setting.inherit_env).expect("inherit env"),
        ];
        AgentRow {
            id,
            enabled: setting.enabled,
            inputs: values
                .into_iter()
                .map(|value| cx.new(|cx| InputState::new(window, cx).default_value(value)))
                .collect(),
        }
    }
    pub(super) fn apply(&self, settings: &mut AiSettings, cx: &App) -> Result<(), String> {
        let optional = |text: String| Some(text.trim().to_owned()).filter(|v| !v.is_empty());
        settings.default_agent = optional(self.inputs[0].read(cx).value().to_string());
        settings.working_directory = optional(self.inputs[1].read(cx).value().to_string());
        if settings
            .working_directory
            .as_ref()
            .is_some_and(|path| !std::path::Path::new(path).is_absolute() || path.contains('\0'))
        {
            return Err("Agent working directory must be an absolute path.".into());
        }
        let mut agents = BTreeMap::new();
        for row in &self.agents {
            let text = |index: usize| row.inputs[index].read(cx).value().to_string();
            let args = text(2);
            let env = text(3);
            let inherit = text(4);
            let setting = AgentServerSettings {
                enabled: row.enabled,
                name: optional(text(0)),
                command: optional(text(1)),
                args: if args.trim().is_empty() {
                    None
                } else {
                    Some(serde_json::from_str(&args).map_err(|_| {
                        format!("{} arguments: enter a JSON array of strings.", row.id)
                    })?)
                },
                env: serde_json::from_str(&env).map_err(|_| {
                    format!("{} environment: enter a JSON object of strings.", row.id)
                })?,
                inherit_env: serde_json::from_str(&inherit).map_err(|_| {
                    format!(
                        "{} inherited environment: enter a JSON array of names.",
                        row.id
                    )
                })?,
            };
            validate_agent(&row.id, &setting)?;
            if setting != AgentServerSettings::default()
                || !matches!(row.id.as_str(), "claude" | "codex" | "hermes")
            {
                agents.insert(row.id.clone(), setting);
            }
        }
        settings.agents = agents;
        if let Some(id) = &settings.default_agent
            && AgentRegistry::new(settings).get(id).is_none()
            && settings.enabled
        {
            return Err("Default agent must name an enabled, configured agent.".into());
        }
        Ok(())
    }
}
fn validate_agent(id: &str, setting: &AgentServerSettings) -> Result<(), String> {
    if !nocterm_ai::registry::valid_id(id) {
        return Err(
            "Agent ids use letters, numbers, underscore or dash, up to 64 characters.".into(),
        );
    }
    if setting.enabled && setting.command.is_none() && !matches!(id, "claude" | "codex" | "hermes")
    {
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
impl SettingsView {
    pub(super) fn render_ai(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let disabled = self.saving.is_some();
        let builtin = AgentRegistry::new(&AiSettings::default());
        let mut form=v_flex().gap_4().child(div().text_lg().font_semibold().child("AI agents"))
   .child(Button::new("ai-enabled").label(if self.draft.ai.enabled{"AI enabled"}else{"AI disabled"}).small().ghost().selected(self.draft.ai.enabled).disabled(disabled).on_click(cx.listener(|this,_,_,cx|{this.draft.ai.enabled= !this.draft.ai.enabled;cx.notify();})))
   .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Turning AI off closes chats, stops agent processes and removes access to attached terminals. Nothing starts until you create a chat."))
   .child(div().text_sm().child("Local agents run as your user without a sandbox and can read your files. Nocterm excludes SSH and vault credentials from shared context. Output filtering cannot recognize every secret."))
   .child(ai_field(&self.ai.inputs[0],"Default agent","Empty asks when creating a chat. Use claude, codex, hermes, or a custom id.",disabled,cx))
   .child(ai_field(&self.ai.inputs[1],"Working directory","Empty uses Nocterm's private agent workspace. A custom path must be absolute.",disabled,cx))
   .child(h_flex().gap_2()
    .child(Button::new("ai-read-approval").small().ghost().selected(self.draft.ai.approval.terminal_read==ApprovalPolicy::Ask).disabled(disabled).label("Ask before reading terminals").on_click(cx.listener(|this,_,_,cx|{this.draft.ai.approval.terminal_read=flip(this.draft.ai.approval.terminal_read);cx.notify();})))
    .child(Button::new("ai-write-approval").small().ghost().selected(self.draft.ai.approval.terminal_write==ApprovalPolicy::Ask).disabled(disabled).label("Ask before typing in terminals").on_click(cx.listener(|this,_,_,cx|{this.draft.ai.approval.terminal_write=flip(this.draft.ai.approval.terminal_write);cx.notify();}))))
   .child(Button::new("ai-redact").small().ghost().selected(self.draft.ai.approval.redact_secrets).disabled(disabled).label("Filter private keys and common tokens in terminal output").on_click(cx.listener(|this,_,_,cx|{this.draft.ai.approval.redact_secrets= !this.draft.ai.approval.redact_secrets;cx.notify();})));
        for (index, row) in self.ai.agents.iter().enumerate() {
            let id = row.id.clone();
            let custom = builtin.get(&id).is_none();
            let command = builtin
                .get(&id)
                .map(|v| format!("Default: {} {}", v.command, v.args.join(" ")))
                .unwrap_or_else(|| "Executable required for a custom agent.".into());
            let mut section=v_flex().gap_2().pt_4().child(h_flex().gap_2().child(div().font_semibold().child(id.clone())).child(Button::new(("ai-agent-enabled",index)).small().ghost().selected(row.enabled).disabled(disabled).label("Enabled").on_click(cx.listener(move |this,_,_,cx|{this.ai.agents[index].enabled= !this.ai.agents[index].enabled;cx.notify();}))))
    .child(ai_field(&row.inputs[0],"Display name","Empty uses the built-in name or agent id.",disabled,cx))
    .child(ai_field(&row.inputs[1],"Executable",command,disabled,cx))
    .child(ai_field(&row.inputs[2],"Arguments","JSON array of strings. Empty keeps the built-in arguments; [] removes them.",disabled,cx))
    .child(ai_field(&row.inputs[3],"Environment overrides","JSON object of strings. Settings are plain text; do not put API keys or other secrets here.",disabled,cx))
    .child(ai_field(&row.inputs[4],"Inherited environment","JSON array of names. Use this for existing API key variables needed to authenticate the agent.",disabled,cx));
            if custom {
                section = section.child(
                    Button::new(("ai-agent-remove", index))
                        .small()
                        .ghost()
                        .disabled(disabled)
                        .label("Remove agent")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ai.agents.remove(index);
                            cx.notify();
                        })),
                );
            }
            form = form.child(section);
        }
        form.child(h_flex().gap_2().child(Input::new(&self.ai.inputs[2]).small().disabled(disabled)).child(Button::new("ai-add-agent").small().label("Add custom agent").disabled(disabled).on_click(cx.listener(|this,_,window,cx|{let id=this.ai.inputs[2].read(cx).value().trim().to_owned();if !nocterm_ai::registry::valid_id(&id)||this.ai.agents.iter().any(|row|row.id==id){this.message=Some(("Enter a unique agent id using letters, numbers, underscore or dash.".into(),true));}else{this.ai.agents.push(AiForm::row(id,AgentServerSettings::default(),window,cx));this.ai.inputs[2].update(cx,|input,cx|input.set_value("",window,cx));this.message=None;}cx.notify();})))).into_any_element()
    }
}
fn ai_field(
    input: &Entity<InputState>,
    label: impl Into<gpui_kit::SharedString>,
    hint: impl Into<gpui_kit::SharedString>,
    disabled: bool,
    cx: &App,
) -> gpui_kit::AnyElement {
    v_flex()
        .gap_1()
        .child(label.into())
        .child(Input::new(input).small().disabled(disabled))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(hint.into()),
        )
        .into_any_element()
}
fn flip(policy: ApprovalPolicy) -> ApprovalPolicy {
    match policy {
        ApprovalPolicy::Allow => ApprovalPolicy::Ask,
        ApprovalPolicy::Ask => ApprovalPolicy::Allow,
    }
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
    #[gpui_kit::test]
    fn ai_page_keeps_a_draft_validates_json_and_applies_master_switch(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{AppContext as _, test::TestWindowExt as _};
        use nocterm_ui::{ActiveSettings as _, SettingsStore};
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::in_memory(nocterm_settings::Settings::default()),
                cx,
            );
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| SettingsView::new(window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.select_page(4, window, cx));
            window.render_frame(cx);
            window.click("ai-enabled", cx);
            assert!(
                cx.settings().ai.enabled,
                "toggle remains a draft until Apply"
            );
            view.update(cx, |view, cx| {
                view.ai.agents[0].inputs[2]
                    .update(cx, |input, cx| input.set_value("bad JSON", window, cx));
                view.apply(window, cx);
                assert!(view.message.as_ref().unwrap().1);
                assert!(cx.settings().ai.enabled);
                view.ai.agents[0].inputs[2].update(cx, |input, cx| input.set_value("", window, cx));
                view.apply(window, cx);
                assert!(!view.message.as_ref().unwrap().1);
                assert!(!cx.settings().ai.enabled);
                assert!(
                    cx.settings().ai.agents.is_empty(),
                    "defaults need no agent overrides"
                );
                view.reset(window, cx);
                assert!(view.draft.ai.enabled);
                assert!(!cx.settings().ai.enabled);
            });
        })
        .unwrap();
    }
}
