//! A single settings tab, using the same schema and store as the terminal.

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_settings::{
    AppearanceMode, CONNECT_TIMEOUT_RANGE, CursorShape, FONT_SIZE_RANGE, KEEPALIVE_RANGE,
    LINE_HEIGHT_RANGE, SCROLLBACK_RANGE, Settings,
};
use nocterm_ui::{
    ActiveDesign as _, ActiveSettings as _, IconName, SettingsStore, update_settings,
};
use nocterm_workspace::{Item, ItemEvent, OpenSettings, Workspace};

/// Installs the settings command. Reopening focuses the existing tab.
pub fn register(workspace: &mut Workspace) {
    workspace.register_action(|workspace, _: &OpenSettings, window, cx| {
        if let Some(item) = workspace.find_item::<SettingsView>() {
            workspace.activate_item_by_id(item.entity_id(), window, cx);
        } else {
            let item = cx.new(|cx| SettingsView::new(window, cx));
            workspace.add_item(item, window, cx);
        }
    });
}

#[derive(Default)]
struct Fields {
    font_family: String,
    font_size: String,
    line_height: String,
    scrollback: String,
    term: String,
    timeout: String,
    keepalive: String,
    local_program: String,
    local_args: String,
    local_cwd: String,
    local_env: String,
    remote_program: String,
    remote_args: String,
    remote_cwd: String,
    remote_env: String,
    auto_lock: String,
}

impl Fields {
    fn from_settings(settings: &Settings) -> Self {
        let terminal = &settings.terminal;
        Self {
            font_family: terminal.font_family.clone().unwrap_or_default(),
            font_size: terminal
                .font_size
                .map(|value| value.to_string())
                .unwrap_or_default(),
            line_height: terminal
                .line_height
                .map(|value| value.to_string())
                .unwrap_or_default(),
            scrollback: terminal.scrollback_lines.to_string(),
            term: terminal.term.clone(),
            timeout: settings.ssh.connect_timeout_secs.to_string(),
            keepalive: settings.ssh.keepalive_interval_secs.to_string(),
            local_program: settings.local.program.clone().unwrap_or_default(),
            local_args: serde_json::to_string(&settings.local.args)
                .expect("serializable arguments"),
            local_cwd: settings.local.cwd.clone().unwrap_or_default(),
            local_env: serde_json::to_string(&settings.local.env)
                .expect("serializable environment"),
            remote_program: settings.ssh.launch.program.clone().unwrap_or_default(),
            remote_args: serde_json::to_string(&settings.ssh.launch.args)
                .expect("serializable arguments"),
            remote_cwd: settings.ssh.launch.cwd.clone().unwrap_or_default(),
            remote_env: serde_json::to_string(&settings.ssh.launch.env)
                .expect("serializable environment"),
            auto_lock: settings.vault.auto_lock_minutes.to_string(),
        }
    }

    fn apply(&self, mut settings: Settings) -> Result<Settings, String> {
        fn number<T: std::str::FromStr + PartialOrd + std::fmt::Display>(
            text: &str,
            name: &str,
            range: std::ops::RangeInclusive<T>,
        ) -> Result<T, String> {
            let value = text
                .trim()
                .parse::<T>()
                .map_err(|_| format!("{name}: enter a number."))?;
            if !range.contains(&value) {
                return Err(format!(
                    "{name}: enter a value from {} to {}.",
                    range.start(),
                    range.end()
                ));
            }
            Ok(value)
        }
        settings.terminal.font_family =
            Some(self.font_family.trim().to_owned()).filter(|value| !value.is_empty());
        settings.terminal.font_size = if self.font_size.trim().is_empty() {
            None
        } else {
            Some(number(&self.font_size, "Font size", FONT_SIZE_RANGE)?)
        };
        settings.terminal.line_height = if self.line_height.trim().is_empty() {
            None
        } else {
            Some(number(&self.line_height, "Line height", LINE_HEIGHT_RANGE)?)
        };
        settings.terminal.scrollback_lines =
            number(&self.scrollback, "Scrollback", SCROLLBACK_RANGE)?;
        settings.terminal.term = self.term.trim().to_owned();
        if settings.terminal.term.is_empty() {
            return Err("Terminal type: enter a value.".into());
        }
        settings.ssh.connect_timeout_secs =
            number(&self.timeout, "Connection timeout", CONNECT_TIMEOUT_RANGE)?;
        settings.ssh.keepalive_interval_secs =
            number(&self.keepalive, "Keepalive interval", KEEPALIVE_RANGE)?;
        fn shell(
            program: &str,
            args: &str,
            cwd: &str,
            env: &str,
            integration: bool,
        ) -> Result<nocterm_settings::ShellSettings, String> {
            let shell = nocterm_settings::ShellSettings {
                program: Some(program.trim().into()).filter(|p: &String| !p.is_empty()),
                args: serde_json::from_str(args)
                    .map_err(|_| "Arguments: enter a JSON array of strings, e.g. [\"-l\"].")?,
                cwd: Some(cwd.trim().into()).filter(|p: &String| !p.is_empty()),
                env: serde_json::from_str(env)
                    .map_err(|_| "Environment: enter a JSON object of string values.")?,
                integration,
            };
            if shell.env.keys().any(|key| {
                key.is_empty()
                    || !key.bytes().enumerate().all(|(i, b)| {
                        b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                    })
            }) {
                return Err("Environment names must be shell identifiers.".into());
            }
            if shell
                .args
                .iter()
                .chain(shell.program.iter())
                .chain(shell.cwd.iter())
                .chain(shell.env.values())
                .any(|value| value.contains('\0'))
            {
                return Err("Shell values cannot contain NUL.".into());
            }
            Ok(shell)
        }
        settings.local = shell(
            &self.local_program,
            &self.local_args,
            &self.local_cwd,
            &self.local_env,
            settings.local.integration,
        )?;
        settings.ssh.launch = shell(
            &self.remote_program,
            &self.remote_args,
            &self.remote_cwd,
            &self.remote_env,
            settings.ssh.launch.integration,
        )?;
        settings.vault.auto_lock_minutes =
            number(&self.auto_lock, "Vault automatic lock", 1..=1440)?;
        Ok(settings)
    }
}

fn global_options(settings: &Settings) -> nocterm_settings::SessionOptions {
    nocterm_settings::SessionOptions {
        term: Some(settings.terminal.term.clone()),
        charset: Some(settings.terminal.charset),
        proxy: Some(settings.ssh.proxy.clone()),
        logging: Some(settings.logging.clone()),
    }
}

/// Settings edits are a draft until Apply succeeds.
pub struct SettingsView {
    draft: Settings,
    session_options: Entity<nocterm_ui::SessionOptionsEditor>,
    inputs: Vec<Entity<InputState>>,
    message: Option<(SharedString, bool)>,
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = cx.settings().clone();
        let fields = Fields::from_settings(&draft);
        let values = [
            fields.font_family,
            fields.font_size,
            fields.line_height,
            fields.scrollback,
            fields.term,
            fields.timeout,
            fields.keepalive,
            fields.local_program,
            fields.local_args,
            fields.local_cwd,
            fields.local_env,
            fields.remote_program,
            fields.remote_args,
            fields.remote_cwd,
            fields.remote_env,
            fields.auto_lock,
        ];
        let inputs = values
            .into_iter()
            .map(|value| cx.new(|cx| InputState::new(window, cx).default_value(value)))
            .collect();
        let options = global_options(&draft);
        let session_options =
            cx.new(|cx| nocterm_ui::SessionOptionsEditor::new(options, false, window, cx));
        Self {
            session_options,
            draft,
            inputs,
            message: None,
        }
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let text = |index: usize| self.inputs[index].read(cx).value().to_string();
        let fields = Fields {
            font_family: text(0),
            font_size: text(1),
            line_height: text(2),
            scrollback: text(3),
            term: text(4),
            timeout: text(5),
            keepalive: text(6),
            local_program: text(7),
            local_args: text(8),
            local_cwd: text(9),
            local_env: text(10),
            remote_program: text(11),
            remote_args: text(12),
            remote_cwd: text(13),
            remote_env: text(14),
            auto_lock: text(15),
        };
        let result = fields.apply(self.draft.clone()).and_then(|mut settings| {
            let options = self.session_options.read(cx).options(cx)?;
            settings.terminal.term = options
                .term
                .unwrap_or_else(|| nocterm_settings::TerminalSettings::default().term);
            settings.terminal.charset = options.charset.unwrap_or_default();
            settings.ssh.proxy = options.proxy.unwrap_or_default();
            settings.logging = options.logging.unwrap_or_default();
            Ok(settings)
        });
        match result {
            Ok(settings) => match update_settings(cx, |current| *current = settings.clone()) {
                Ok(()) => {
                    self.draft = settings;
                    let persistent = cx.global::<SettingsStore>().is_persistent();
                    self.message = Some((
                        if persistent {
                            "Settings saved."
                        } else {
                            "Settings applied for this run. Saving is unavailable."
                        }
                        .into(),
                        false,
                    ));
                }
                Err(error) => {
                    self.message = Some((format!("Could not save settings: {error}").into(), true))
                }
            },
            Err(error) => self.message = Some((error.into(), true)),
        }
        cx.notify();
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft = Settings::default();
        let options = global_options(&self.draft);
        self.session_options
            .update(cx, |editor, cx| editor.reset(options, window, cx));
        let fields = Fields::from_settings(&self.draft);
        let values = [
            fields.font_family,
            fields.font_size,
            fields.line_height,
            fields.scrollback,
            fields.term,
            fields.timeout,
            fields.keepalive,
            fields.local_program,
            fields.local_args,
            fields.local_cwd,
            fields.local_env,
            fields.remote_program,
            fields.remote_args,
            fields.remote_cwd,
            fields.remote_env,
            fields.auto_lock,
        ];
        for (input, value) in self.inputs.iter().zip(values) {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.message = Some((
            "Defaults restored in the form. Apply to save.".into(),
            false,
        ));
        cx.notify();
    }

    fn field(
        &self,
        index: usize,
        label: &'static str,
        hint: &'static str,
        cx: &App,
    ) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(label)
            .child(Input::new(&self.inputs[index]).small())
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(hint),
            )
    }
}

impl EventEmitter<ItemEvent> for SettingsView {}
impl Focusable for SettingsView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.inputs[0].read(cx).focus_handle(cx)
    }
}
impl Item for SettingsView {
    fn tab_title(&self, _: &App) -> SharedString {
        "Settings".into()
    }
    fn tab_icon(&self, _: &App) -> IconName {
        IconName::Settings
    }
}
impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.draft.appearance.mode;
        let shape = self.draft.terminal.cursor_shape;
        let mut appearance = h_flex().gap_2();
        for (index, (label, value)) in [
            ("System", AppearanceMode::System),
            ("Light", AppearanceMode::Light),
            ("Dark", AppearanceMode::Dark),
        ]
        .into_iter()
        .enumerate()
        {
            appearance = appearance.child(
                Button::new(("appearance", index))
                    .small()
                    .ghost()
                    .label(label)
                    .selected(mode == value)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.appearance.mode = value;
                        cx.notify();
                    })),
            );
        }
        let mut cursor = h_flex().gap_2();
        for (index, (label, value)) in [
            ("Block", CursorShape::Block),
            ("Bar", CursorShape::Bar),
            ("Underline", CursorShape::Underline),
        ]
        .into_iter()
        .enumerate()
        {
            cursor = cursor.child(
                Button::new(("cursor", index))
                    .small()
                    .ghost()
                    .label(label)
                    .selected(shape == value)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.terminal.cursor_shape = value;
                        cx.notify();
                    })),
            );
        }
        let width = rems(cx.design().layout.settings_width);
        let form = v_flex()
            .p_6()
            .gap_4()
            .max_w(width)
            .child(div().text_lg().font_semibold().child("Settings"))
            .child(div().font_semibold().child("Appearance"))
            .child(appearance)
            .child(div().pt_4().font_semibold().child("Terminal"))
            .child(self.field(0, "Font family", "Leave empty to use the theme font.", cx))
            .child(self.field(
                1,
                "Font size",
                "6–72 pixels; empty uses the theme size.",
                cx,
            ))
            .child(self.field(
                2,
                "Line height",
                "1–3 times the font size; empty uses the theme value.",
                cx,
            ))
            .child("Cursor shape")
            .child(cursor)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("cursor-blink")
                            .small()
                            .ghost()
                            .label("Blink cursor")
                            .selected(self.draft.terminal.cursor_blink)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.terminal.cursor_blink =
                                    !this.draft.terminal.cursor_blink;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("copy-select")
                            .small()
                            .ghost()
                            .label("Copy on selection")
                            .selected(self.draft.terminal.copy_on_select)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.terminal.copy_on_select =
                                    !this.draft.terminal.copy_on_select;
                                cx.notify();
                            })),
                    ),
            )
            .child(self.field(3, "Scrollback lines", "0–1000000 lines.", cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("line-numbers")
                            .small()
                            .ghost()
                            .label("Line numbers")
                            .selected(self.draft.terminal.show_line_numbers)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.terminal.show_line_numbers =
                                    !this.draft.terminal.show_line_numbers;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("timestamps")
                            .small()
                            .ghost()
                            .label("Line timestamps (UTC)")
                            .selected(self.draft.terminal.show_timestamps)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.terminal.show_timestamps =
                                    !this.draft.terminal.show_timestamps;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().pt_4().font_semibold().child("Local shell"))
            .child(self.field(
                7,
                "Executable",
                "Empty uses the operating system shell. Applied on the next start.",
                cx,
            ))
            .child(self.field(
                8,
                "Arguments",
                "JSON string array, e.g. [\"-l\"]. Custom arguments disable automatic integration.",
                cx,
            ))
            .child(self.field(
                9,
                "Initial directory",
                "Empty uses your home directory.",
                cx,
            ))
            .child(self.field(
                10,
                "Environment",
                "JSON object, e.g. {\"LANG\":\"en_US.UTF-8\"}. Do not store secrets here.",
                cx,
            ))
            .child(
                Button::new("local-integration")
                    .small()
                    .ghost()
                    .label("Local shell integration")
                    .selected(self.draft.local.integration)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.draft.local.integration = !this.draft.local.integration;
                        cx.notify();
                    })),
            )
            .child(div().pt_4().font_semibold().child("Session defaults"))
            .child(self.session_options.clone())
            .child(div().pt_4().font_semibold().child("SSH"))
            .child(self.field(
                5,
                "Connection timeout",
                "1–600 seconds; applied to new connections and reconnects.",
                cx,
            ))
            .child(self.field(
                6,
                "Keepalive interval",
                "0–3600 seconds; zero disables probes. Applied to new connections and reconnects.",
                cx,
            ))
            .child(self.field(
                11,
                "Remote executable",
                "Empty preserves the server's default shell request. Profiles may override this.",
                cx,
            ))
            .child(self.field(
                12,
                "Remote arguments",
                "JSON string array; applied on reconnect.",
                cx,
            ))
            .child(self.field(
                13,
                "Remote initial directory",
                "Empty uses the server's home directory.",
                cx,
            ))
            .child(self.field(
                14,
                "Remote environment",
                "JSON object; SSH servers may restrict accepted variables.",
                cx,
            ))
            .child(
                Button::new("remote-integration")
                    .small()
                    .ghost()
                    .label("Remote shell integration")
                    .selected(self.draft.ssh.launch.integration)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.draft.ssh.launch.integration = !this.draft.ssh.launch.integration;
                        cx.notify();
                    })),
            )
            .child(div().pt_4().font_semibold().child("Credential vault"))
            .child(self.field(
                15,
                "Automatic lock",
                "1–1440 minutes since the last vault use.",
                cx,
            ));
        let footer = v_flex()
            .flex_shrink_0()
            .px_6()
            .py_4()
            .gap_2()
            .max_w(width)
            .when_some(self.message.clone(), |form, (message, error)| {
                form.child(
                    div()
                        .text_sm()
                        .text_color(if error {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(message),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("settings-reset")
                            .ghost()
                            .label("Restore defaults")
                            .on_click(cx.listener(Self::reset_click)),
                    )
                    .child(
                        Button::new("settings-apply")
                            .primary()
                            .label("Apply")
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    ),
            );
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(form),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(footer),
            )
    }
}
impl SettingsView {
    fn reset_click(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, WindowOptions, test::TestWindowExt as _};

    #[test]
    fn launch_fields_preserve_argument_boundaries_and_reject_invalid_environment() {
        let mut fields = Fields::from_settings(&Settings::default());
        fields.local_program = "/bin/bash".into();
        fields.local_args = r#"["-c","printf '%s' \"a b\""]"#.into();
        fields.local_env = r#"{"PATH":"/bin","LANG":"C"}"#.into();
        fields.local_cwd = "/tmp/space ' directory".into();
        let settings = fields.apply(Settings::default()).unwrap();
        assert_eq!(settings.local.args, vec!["-c", "printf '%s' \"a b\""]);
        assert_eq!(
            settings.local.cwd.as_deref(),
            Some("/tmp/space ' directory")
        );
        fields.local_env = r#"{"BAD;NAME":"value"}"#.into();
        assert!(
            fields
                .apply(Settings::default())
                .unwrap_err()
                .contains("identifiers")
        );
        fields.local_env = "{}".into();
        fields.auto_lock = "0".into();
        assert!(
            fields
                .apply(Settings::default())
                .unwrap_err()
                .contains("Vault")
        );
    }

    #[gpui_kit::test]
    fn apply_reports_save_failure_and_keeps_active_and_draft_values(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            // A directory cannot be replaced with a settings file.
            let file = nocterm_settings::SettingsFile::new(directory.path());
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::new(Settings::default(), file),
                cx,
            );
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| SettingsView::new(window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.inputs[1].update(cx, |input, cx| input.set_value("18", window, cx));
                view.apply(cx);
                let (message, error) = view.message.as_ref().unwrap();
                assert!(*error);
                assert!(message.starts_with("Could not save settings:"));
                assert_eq!(*cx.settings(), Settings::default());
                assert_eq!(view.draft, Settings::default());
                assert_eq!(
                    view.inputs[1].read(cx).value(),
                    "18",
                    "retain the edit for retry"
                );
            });
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn open_settings_reuses_one_tab_and_invalid_apply_keeps_active_values(cx: &mut TestAppContext) {
        let (handle, workspace) = cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::in_memory(Settings::default()),
                cx,
            );
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let workspace = cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    register(&mut workspace);
                    workspace
                });
                window.focus(&workspace.read(cx).focus_handle(cx), cx);
                workspace
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_action(Box::new(OpenSettings), cx);
        })
        .unwrap();
        cx.run_until_parked();
        let settings = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let settings = workspace.read(cx).find_item::<SettingsView>().unwrap();
                assert!(settings.read(cx).focus_handle(cx).is_focused(window));
                assert_eq!(workspace.read(cx).items().count(), 1);
                window.dispatch_action(Box::new(OpenSettings), cx);
                settings
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(workspace.read(cx).items().count(), 1);
            assert_eq!(
                workspace.read(cx).find_item::<SettingsView>().unwrap(),
                settings
            );
            settings.update(cx, |settings, cx| {
                settings.inputs[1].update(cx, |input, cx| input.set_value("100", window, cx));
                settings.apply(cx);
                assert!(settings.message.as_ref().unwrap().1);
                assert_eq!(*cx.settings(), Settings::default());
                settings.inputs[1].update(cx, |input, cx| input.set_value("18", window, cx));
                settings.apply(cx);
                assert!(!settings.message.as_ref().unwrap().1);
                assert_eq!(cx.settings().terminal.font_size, Some(18.));
                settings.reset(window, cx);
                assert_eq!(
                    cx.settings().terminal.font_size,
                    Some(18.),
                    "reset is a draft until Apply"
                );
            });
        })
        .unwrap();
    }

    #[test]
    fn accepts_each_numeric_boundary_and_rejects_values_beyond_it() {
        for (index, low, high, below, above) in [
            (0, "6", "72", "5.99", "72.01"),
            (1, "1", "3", "0.99", "3.01"),
            (2, "0", "1000000", "-1", "1000001"),
            (3, "1", "600", "0", "601"),
            (4, "0", "3600", "-1", "3601"),
        ] {
            for (text, valid) in [
                (low, true),
                (high, true),
                (below, false),
                (above, false),
                ("", false),
                ("abc", false),
            ] {
                let mut fields = Fields::from_settings(&Settings::default());
                let field = match index {
                    0 => &mut fields.font_size,
                    1 => &mut fields.line_height,
                    2 => &mut fields.scrollback,
                    3 => &mut fields.timeout,
                    _ => &mut fields.keepalive,
                };
                *field = format!(" {text} ");
                let optional = index < 2 && text.is_empty();
                assert_eq!(
                    fields.apply(Settings::default()).is_ok(),
                    valid || optional,
                    "field {index}: {text}"
                );
            }
        }
    }

    #[test]
    fn applies_trimmed_text_and_preserves_non_text_choices() {
        let mut settings = Settings::default();
        settings.appearance.mode = AppearanceMode::Dark;
        settings.terminal.cursor_shape = CursorShape::Bar;
        settings.terminal.copy_on_select = true;
        let mut fields = Fields::from_settings(&settings);
        fields.font_family = "  Test Font  ".into();
        fields.term = "  xterm-test  ".into();
        let applied = fields.apply(settings.clone()).unwrap();
        assert_eq!(applied.terminal.font_family.as_deref(), Some("Test Font"));
        assert_eq!(applied.terminal.term, "xterm-test");
        assert_eq!(applied.appearance, settings.appearance);
        assert_eq!(applied.terminal.cursor_shape, CursorShape::Bar);
        assert!(applied.terminal.copy_on_select);
    }

    #[test]
    fn validates_numbers_without_silently_clamping() {
        let mut fields = Fields::from_settings(&Settings::default());
        fields.font_size = "NaN".into();
        assert!(fields.apply(Settings::default()).is_err());
        fields.font_size = "73".into();
        assert!(fields.apply(Settings::default()).is_err());
        fields.font_size = "".into();
        fields.keepalive = "0".into();
        assert_eq!(
            fields
                .apply(Settings::default())
                .unwrap()
                .ssh
                .keepalive_interval_secs,
            0
        );
    }
    #[test]
    fn default_form_round_trips_and_empty_term_is_rejected() {
        let mut fields = Fields::from_settings(&Settings::default());
        assert_eq!(
            fields.apply(Settings::default()).unwrap(),
            Settings::default()
        );
        fields.term = "  ".into();
        assert!(fields.apply(Settings::default()).is_err());
    }
}
