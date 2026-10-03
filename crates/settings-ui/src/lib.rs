//! A single settings tab, using the same schema and store as the terminal.

mod ai_page;

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Task, Window,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        tab::{Tab, TabBar},
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
use nocterm_ui::{ActiveDesign as _, ActiveSettings as _, IconName, SettingsStore, save_settings};
use nocterm_workspace::{
    Item, ItemEvent, OpenSettings, SettingsPageHandle, SettingsPageSpec, Workspace,
};

const BUILTIN_PAGES: &[(&str, &str)] = &[
    ("appearance", "Appearance"),
    ("terminal", "Terminal"),
    ("local-shell", "Local shell"),
    ("ssh", "SSH"),
    ("ai", "AI"),
];

/// Installs Settings with its built-in pages.
pub fn register(workspace: &mut Workspace) {
    register_with_pages(workspace, Vec::new());
}
/// Installs Settings and feature-owned pages supplied by the application.
pub fn register_with_pages(workspace: &mut Workspace, pages: Vec<SettingsPageSpec>) {
    workspace.register_action(move |_, _: &OpenSettings, window, cx| {
        let workspace = cx.entity().downgrade();
        let pages = pages.clone();
        // Popup dismissal restores its old action context first. Opening the
        // Item afterwards lets its own field keep keyboard focus.
        window.defer(cx, move |window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                open_page(workspace, "", &pages, window, cx);
            });
        });
    });
}
/// Opens the singleton Settings Item and selects a page. Empty preserves its selection.
pub fn open_page(
    workspace: &mut Workspace,
    id: &str,
    pages: &[SettingsPageSpec],
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let item = workspace.find_item::<SettingsView>().unwrap_or_else(|| {
        let item = cx.new(|cx| SettingsView::with_pages(pages.to_vec(), window, cx));
        workspace.add_item(item.clone(), window, cx);
        item
    });
    if !id.is_empty() {
        item.update(cx, |view, cx| {
            if let Some(index) = view.page_index(id) {
                view.select_page(index, window, cx);
            }
        });
    }
    workspace.activate_item_by_id(item.entity_id(), window, cx);
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

    fn values(self) -> [String; 16] {
        [
            self.font_family,
            self.font_size,
            self.line_height,
            self.scrollback,
            self.term,
            self.timeout,
            self.keepalive,
            self.local_program,
            self.local_args,
            self.local_cwd,
            self.local_env,
            self.remote_program,
            self.remote_args,
            self.remote_cwd,
            self.remote_env,
            self.auto_lock,
        ]
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
    ai: ai_page::AiForm,
    selected_page: usize,
    pages: Vec<(SettingsPageSpec, Option<Box<dyn SettingsPageHandle>>)>,
    focus: FocusHandle,
    draft: Settings,
    base_revision: u64,
    saving: Option<Task<()>>,
    session_options: Entity<nocterm_ui::SessionOptionsEditor>,
    inputs: Vec<Entity<InputState>>,
    message: Option<(SharedString, bool)>,
}

impl SettingsView {
    #[cfg(test)]
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_pages(Vec::new(), window, cx)
    }
    fn with_pages(
        pages: Vec<SettingsPageSpec>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = cx.settings().clone();
        let ai = ai_page::AiForm::new(&draft.ai, window, cx);
        let fields = Fields::from_settings(&draft);
        let inputs = fields
            .values()
            .into_iter()
            .map(|value| cx.new(|cx| InputState::new(window, cx).default_value(value)))
            .collect();
        let options = global_options(&draft);
        let session_options =
            cx.new(|cx| nocterm_ui::SessionOptionsEditor::new(options, false, window, cx));
        Self {
            ai,
            selected_page: 0,
            pages: pages.into_iter().map(|spec| (spec, None)).collect(),
            focus: cx.focus_handle(),
            session_options,
            draft,
            base_revision: cx.global::<SettingsStore>().revision(),
            saving: None,
            inputs,
            message: None,
        }
    }

    pub fn selected_page_id(&self) -> &str {
        if self.selected_page < BUILTIN_PAGES.len() {
            BUILTIN_PAGES[self.selected_page].0
        } else {
            self.pages[self.selected_page - BUILTIN_PAGES.len()].0.id
        }
    }
    fn page_index(&self, id: &str) -> Option<usize> {
        BUILTIN_PAGES
            .iter()
            .position(|(key, _)| *key == id)
            .or_else(|| {
                self.pages
                    .iter()
                    .position(|(spec, _)| spec.id == id)
                    .map(|index| index + BUILTIN_PAGES.len())
            })
    }
    fn select_page(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= BUILTIN_PAGES.len() + self.pages.len() || index == self.selected_page {
            return;
        }
        if let Some((_, Some(page))) = self
            .selected_page
            .checked_sub(BUILTIN_PAGES.len())
            .and_then(|index| self.pages.get(index))
        {
            page.deactivate(window, cx);
        }
        self.selected_page = index;
        if let Some((spec, page)) = index
            .checked_sub(BUILTIN_PAGES.len())
            .and_then(|index| self.pages.get_mut(index))
            && page.is_none()
        {
            *page = Some(spec.create(window, cx));
        }
        window.focus(&self.focus_handle(cx), cx);
        cx.notify();
    }
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving.is_some() {
            return;
        }
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
            self.ai.apply(&mut settings.ai, cx)?;
            Ok(settings)
        });
        let settings = match result {
            Ok(settings) => settings,
            Err(error) => {
                self.message = Some((error.into(), true));
                cx.notify();
                return;
            }
        };
        let task = save_settings(cx, self.base_revision, settings.clone());
        // In-memory settings publish synchronously, preserving existing clients
        // which edit and immediately read their active settings.
        use futures::FutureExt as _;
        let mut task = Box::pin(task);
        if let Some(result) = task.as_mut().now_or_never() {
            self.saved(result, settings, window, cx);
        } else {
            self.message = Some(("Saving settings…".into(), false));
            self.saving = Some(cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.saving = None;
                    this.saved(result, settings, window, cx);
                });
            }));
            cx.notify();
        }
    }
    fn saved(
        &mut self,
        result: Result<u64, String>,
        settings: Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(revision) => {
                self.base_revision = revision;
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
                nocterm_ui::notice::success(
                    window,
                    cx,
                    "settings-save",
                    "Settings",
                    if persistent {
                        "Settings saved."
                    } else {
                        "Settings applied for this run."
                    },
                );
            }
            Err(error) => {
                let message = format!("Could not save settings: {error}");
                self.message = Some((message.clone().into(), true));
                nocterm_ui::notice::error(window, cx, "settings-save", "Settings", message);
            }
        }
        cx.notify();
    }
    fn replace_form(&mut self, settings: Settings, window: &mut Window, cx: &mut Context<Self>) {
        self.ai = ai_page::AiForm::new(&settings.ai, window, cx);
        self.draft = settings;
        let options = global_options(&self.draft);
        self.session_options
            .update(cx, |editor, cx| editor.reset(options, window, cx));
        for (input, value) in self
            .inputs
            .iter()
            .zip(Fields::from_settings(&self.draft).values())
        {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving.is_some() {
            return;
        }
        self.base_revision = cx.global::<SettingsStore>().revision();
        self.replace_form(cx.settings().clone(), window, cx);
        self.message = Some((
            "Saved settings loaded. Unsaved edits were discarded.".into(),
            false,
        ));
        cx.notify();
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving.is_some() {
            return;
        }
        // Restoring defaults changes the draft, not its optimistic base revision.
        self.replace_form(Settings::default(), window, cx);
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
            .child(
                Input::new(&self.inputs[index])
                    .small()
                    .disabled(self.saving.is_some()),
            )
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
        match self.selected_page {
            0 => self.focus.clone(),
            1 => self.inputs[0].read(cx).focus_handle(cx),
            2 => self.inputs[7].read(cx).focus_handle(cx),
            3 => self.inputs[5].read(cx).focus_handle(cx),
            4 => self.focus.clone(),
            index => self.pages[index - BUILTIN_PAGES.len()]
                .1
                .as_ref()
                .map(|page| page.focus_handle(cx))
                .unwrap_or_else(|| self.focus.clone()),
        }
    }
}
impl Item for SettingsView {
    fn tab_title(&self, _: &App) -> SharedString {
        "Settings".into()
    }
    fn tab_icon(&self, _: &App) -> IconName {
        IconName::Settings
    }
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (_, page) in &self.pages {
            if let Some(page) = page {
                page.close(window, cx);
            }
        }
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
                    .disabled(self.saving.is_some())
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
                    .disabled(self.saving.is_some())
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
            .when(self.selected_page == 0, |form| {
                form.child(div().font_semibold().child("Appearance"))
                    .child(appearance)
            })
            .when(self.selected_page == 1, |form| {
                form.child(div().pt_4().font_semibold().child("Terminal"))
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
                                    .disabled(self.saving.is_some())
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
                                    .disabled(self.saving.is_some())
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
                    .child(
                        Button::new("clipboard-write")
                            .small().ghost()
                            .label("Allow programs to write clipboard")
                            .disabled(self.saving.is_some())
                            .selected(self.draft.terminal.clipboard_write == nocterm_settings::ClipboardWritePolicy::FocusedTerminal)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.terminal.clipboard_write = if this.draft.terminal.clipboard_write == nocterm_settings::ClipboardWritePolicy::Deny {
                                    nocterm_settings::ClipboardWritePolicy::FocusedTerminal
                                } else { nocterm_settings::ClipboardWritePolicy::Deny };
                                cx.notify();
                            })),
                    )
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Off by default. When allowed, only the focused terminal can write your clipboard using OSC 52."))
                    .child(self.field(3, "Scrollback lines", "0–1000000 lines.", cx))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("line-numbers")
                                    .small()
                                    .disabled(self.saving.is_some())
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
                                    .disabled(self.saving.is_some())
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
            })
            .when(self.selected_page == 2, |form| {
                form
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
                    .disabled(self.saving.is_some())
                    .ghost()
                    .label("Local shell integration")
                    .selected(self.draft.local.integration)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.draft.local.integration = !this.draft.local.integration;
                        cx.notify();
                    })),
            )
            })
            .when(self.selected_page == 3, |form| {
                form
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
                    .disabled(self.saving.is_some())
                    .ghost()
                    .label("Remote shell integration")
                    .selected(self.draft.ssh.launch.integration)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.draft.ssh.launch.integration = !this.draft.ssh.launch.integration;
                        cx.notify();
                    })),
            )
            })
            .when(self.selected_page == 4, |form| form.child(self.render_ai(cx)))
            .when(self.selected_page_id() == "vault", |form| {
                form.child(self.field(
                    15,
                    "Automatic lock",
                    "1–1440 minutes since the last vault use.",
                    cx,
                ))
            });
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
                            .disabled(self.saving.is_some())
                            .ghost()
                            .label("Restore defaults")
                            .on_click(cx.listener(Self::reset_click)),
                    )
                    .child(
                        Button::new("settings-reload")
                            .ghost()
                            .label("Reload saved settings")
                            .disabled(self.saving.is_some())
                            .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                    )
                    .child(
                        Button::new("settings-apply")
                            .disabled(self.saving.is_some())
                            .primary()
                            .label("Apply")
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    ),
            );
        let tabs = TabBar::new("settings-pages")
            .menu(true)
            .selected_index(self.selected_page)
            .children(
                BUILTIN_PAGES
                    .iter()
                    .map(|(_, title)| Tab::new().label(*title)),
            )
            .children(
                self.pages
                    .iter()
                    .map(|(spec, _)| Tab::new().label(spec.title.clone())),
            )
            .on_click(
                cx.listener(|this, index: &usize, window, cx| this.select_page(*index, window, cx)),
            );
        let content = if let Some((_, Some(page))) = self
            .selected_page
            .checked_sub(BUILTIN_PAGES.len())
            .and_then(|index| self.pages.get(index))
        {
            v_flex()
                .flex_1()
                .min_h_0()
                .child(form)
                .child(div().flex_1().min_h_0().child(page.view()))
                .into_any_element()
        } else {
            div()
                .id(("settings-scroll", self.selected_page))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(form)
                .into_any_element()
        };
        v_flex()
            .size_full()
            .min_h_0()
            .track_focus(&self.focus)
            .child(
                div()
                    .flex_shrink_0()
                    .px_6()
                    .pt_4()
                    .child(div().text_lg().font_semibold().child("Settings"))
                    .child(tabs),
            )
            .child(content)
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

    use nocterm_workspace::SettingsPage;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    struct GuestPage {
        input: Entity<InputState>,
        deactivations: Rc<Cell<usize>>,
        closures: Rc<Cell<usize>>,
    }
    impl Focusable for GuestPage {
        fn focus_handle(&self, cx: &App) -> FocusHandle {
            self.input.read(cx).focus_handle(cx)
        }
    }
    impl Render for GuestPage {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().id("guest-page").child(Input::new(&self.input))
        }
    }
    impl SettingsPage for GuestPage {
        fn on_deactivate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            self.deactivations.set(self.deactivations.get() + 1);
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            self.closures.set(self.closures.get() + 1);
            self.on_deactivate(window, cx);
        }
    }
    #[gpui_kit::test]
    fn native_pages_preserve_drafts_and_lazy_guests_receive_hide_and_close(
        cx: &mut TestAppContext,
    ) {
        let creations = Rc::new(Cell::new(0));
        let deactivations = Rc::new(Cell::new(0));
        let closures = Rc::new(Cell::new(0));
        let guest = Rc::new(RefCell::new(None::<Entity<GuestPage>>));
        let descriptor = SettingsPageSpec::new("vault", "Vault", {
            let creations = creations.clone();
            let deactivations = deactivations.clone();
            let closures = closures.clone();
            let guest = guest.clone();
            move |window, cx| {
                creations.set(creations.get() + 1);
                let entity = cx.new(|cx| GuestPage {
                    input: cx.new(|cx| InputState::new(window, cx).masked(true)),
                    deactivations: deactivations.clone(),
                    closures: closures.clone(),
                });
                *guest.borrow_mut() = Some(entity.clone());
                entity
            }
        });
        let (handle, workspace) = cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::in_memory(Settings::default()),
                cx,
            );
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    register_with_pages(&mut workspace, vec![descriptor.clone()]);
                    workspace
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                open_page(workspace, "", std::slice::from_ref(&descriptor), window, cx);
            })
        })
        .unwrap();
        let settings = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(creations.get(), 0, "guest must be lazy");
                window.within("settings-pages").click(1usize, cx);
                let settings = workspace.read(cx).find_item::<SettingsView>().unwrap();
                assert_eq!(settings.read(cx).selected_page_id(), "terminal");
                settings.update(cx, |view, cx| {
                    view.inputs[1].update(cx, |input, cx| input.set_value("18", window, cx))
                });
                window.render_frame(cx);
                window.within("settings-pages").click(2usize, cx);
                assert_eq!(settings.read(cx).selected_page_id(), "local-shell");
                window.render_frame(cx);
                window.within("settings-pages").click(1usize, cx);
                assert_eq!(settings.read(cx).inputs[1].read(cx).value(), "18");
                window.render_frame(cx);
                window.click("settings-apply", cx);
                assert_eq!(cx.settings().terminal.font_size, Some(18.));
                settings
            })
            .unwrap();
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                open_page(
                    workspace,
                    "vault",
                    std::slice::from_ref(&descriptor),
                    window,
                    cx,
                )
            });
            assert_eq!(workspace.read(cx).items().count(), 1);
            assert_eq!(creations.get(), 1);
            window.render_frame(cx);
            window.input("guest master draft", cx);
            assert_eq!(
                guest
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .input
                    .read(cx)
                    .value(),
                "guest master draft"
            );
            settings.update(cx, |view, cx| {
                view.inputs[15].update(cx, |input, cx| input.set_value("30", window, cx))
            });
            window.click("settings-apply", cx);
            assert_eq!(cx.settings().vault.auto_lock_minutes, 30);
            assert_eq!(
                guest
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .input
                    .read(cx)
                    .value(),
                "guest master draft",
                "Apply saves settings, not the guest's independent security draft"
            );
            window.within("settings-pages").click(0usize, cx);
            assert_eq!(deactivations.get(), 1);
            assert!(
                guest
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .input
                    .read(cx)
                    .value()
                    .is_empty()
            );
            assert_eq!(settings.read(cx).selected_page_id(), "appearance");
            assert!(settings.read(cx).focus_handle(cx).is_focused(window));
            workspace.update(cx, |workspace, cx| {
                open_page(
                    workspace,
                    "vault",
                    std::slice::from_ref(&descriptor),
                    window,
                    cx,
                )
            });
            assert_eq!(creations.get(), 1, "reuse the guest entity");
            window.render_frame(cx);
            window.input("closing guest draft", cx);
            workspace.update(cx, |workspace, cx| workspace.close_item(0, window, cx));
            assert_eq!(closures.get(), 1);
            assert!(
                guest
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .input
                    .read(cx)
                    .value()
                    .is_empty()
            );
        })
        .unwrap();
    }

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
    async fn apply_reports_save_failure_and_keeps_active_and_draft_values(cx: &mut TestAppContext) {
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
                view.apply(window, cx);
                assert!(view.saving.is_some());
                assert_eq!(*cx.settings(), Settings::default());
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            view.update(cx, |view, cx| {
                assert!(view.saving.is_none());
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
    fn stale_settings_draft_retains_edits_until_reload_and_defaults_do_not_rebase(
        cx: &mut TestAppContext,
    ) {
        let (first_window, first, second_window, second) = cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::in_memory(Settings::default()),
                cx,
            );
            let (first_window, first) =
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| SettingsView::new(window, cx))
                })
                .unwrap();
            let (second_window, second) =
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| SettingsView::new(window, cx))
                })
                .unwrap();
            (first_window, first, second_window, second)
        });
        cx.update_window(first_window, |_, window, cx| {
            first.update(cx, |view, cx| {
                view.inputs[1].update(cx, |input, cx| input.set_value("18", window, cx));
                view.apply(window, cx);
                assert_eq!(view.base_revision, 1);
            })
        })
        .unwrap();
        cx.update_window(second_window, |_, window, cx| {
            second.update(cx, |view, cx| {
                view.inputs[1].update(cx, |input, cx| input.set_value("20", window, cx));
                view.apply(window, cx);
                assert!(
                    view.message
                        .as_ref()
                        .unwrap()
                        .0
                        .contains("Reload saved settings")
                );
                assert_eq!(view.inputs[1].read(cx).value(), "20");
                assert_eq!(cx.settings().terminal.font_size, Some(18.));
                view.reset(window, cx);
                assert_eq!(
                    view.base_revision, 0,
                    "defaults cannot authorize overwriting a newer draft"
                );
                view.apply(window, cx);
                assert!(view.message.as_ref().unwrap().1);
                view.reload(window, cx);
                assert_eq!(view.base_revision, 1);
                assert_eq!(view.inputs[1].read(cx).value(), "18");
                view.inputs[1].update(cx, |input, cx| input.set_value("22", window, cx));
                view.apply(window, cx);
                assert!(!view.message.as_ref().unwrap().1);
                assert_eq!(cx.settings().terminal.font_size, Some(22.));
            })
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
                settings.apply(window, cx);
                assert!(settings.message.as_ref().unwrap().1);
                assert_eq!(*cx.settings(), Settings::default());
                settings.inputs[1].update(cx, |input, cx| input.set_value("18", window, cx));
                settings.apply(window, cx);
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
