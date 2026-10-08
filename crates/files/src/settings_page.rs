//! Settings → Explorer: what folder statistics count, and which programs
//! open files on this computer and on servers, by file type.

mod text;

use std::{collections::BTreeMap, time::Duration};

use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, SharedString,
    Subscription, Task, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        switch::Switch,
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_settings::{ExplorerSettings, INDEXING_ENTRIES_RANGE, OpenRule, Opener, Settings};
use nocterm_ui::{ActiveSettings as _, IconName, SettingsStore, edit_settings, form};
use nocterm_workspace::SettingsPage;

/// How long typing must pause before a field saves.
const DEBOUNCE: Duration = Duration::from_millis(600);
/// Editors offered with one click for files on servers.
const REMOTE_EDITORS: [&str; 5] = ["nano", "vim", "nvim", "micro", "mcedit"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    Local,
    Remote,
}

/// Which text a field edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Excluded,
    MaxLocal,
    MaxRemote,
    Program(Side),
    Args(Side),
    Extensions(usize),
    RuleProgram(usize, Side),
    RuleArgs(usize, Side),
}

impl Key {
    fn is_rule(self) -> bool {
        matches!(
            self,
            Key::Extensions(_) | Key::RuleProgram(..) | Key::RuleArgs(..)
        )
    }

    fn placeholder(self) -> &'static str {
        match self {
            Key::Excluded => ".cache, node_modules, target",
            Key::MaxLocal | Key::MaxRemote => "",
            Key::Program(Side::Local) => "System default",
            Key::Program(Side::Remote) => "nano",
            Key::RuleProgram(..) => "Default",
            Key::Args(_) | Key::RuleArgs(..) => "Arguments; {file} is the file",
            Key::Extensions(_) => "md, txt, Dockerfile",
        }
    }

    fn opener<'a>(self, explorer: &'a ExplorerSettings) -> Option<&'a Opener> {
        let open = &explorer.open;
        let pick = |rule: &'a OpenRule, side| match side {
            Side::Local => &rule.local,
            Side::Remote => &rule.remote,
        };
        match self {
            Key::Program(Side::Local) | Key::Args(Side::Local) => Some(&open.local),
            Key::Program(Side::Remote) | Key::Args(Side::Remote) => Some(&open.remote),
            Key::RuleProgram(ix, side) | Key::RuleArgs(ix, side) => {
                open.rules.get(ix).map(|rule| pick(rule, side))
            }
            _ => None,
        }
    }

    fn opener_mut(self, explorer: &mut ExplorerSettings) -> Option<&mut Opener> {
        let open = &mut explorer.open;
        match self {
            Key::Program(Side::Local) | Key::Args(Side::Local) => Some(&mut open.local),
            Key::Program(Side::Remote) | Key::Args(Side::Remote) => Some(&mut open.remote),
            Key::RuleProgram(ix, side) | Key::RuleArgs(ix, side) => {
                open.rules.get_mut(ix).map(|rule| match side {
                    Side::Local => &mut rule.local,
                    Side::Remote => &mut rule.remote,
                })
            }
            _ => None,
        }
    }

    /// The text a field shows for the saved settings.
    fn read(self, explorer: &ExplorerSettings) -> String {
        let indexing = &explorer.indexing;
        match self {
            Key::Excluded => indexing.excluded.join(", "),
            Key::MaxLocal => indexing.max_local_entries.to_string(),
            Key::MaxRemote => indexing.max_remote_entries.to_string(),
            Key::Extensions(ix) => explorer
                .open
                .rules
                .get(ix)
                .map(|rule| rule.extensions.join(", "))
                .unwrap_or_default(),
            Key::Program(_) | Key::RuleProgram(..) => self
                .opener(explorer)
                .map(|opener| opener.program.clone())
                .unwrap_or_default(),
            Key::Args(_) | Key::RuleArgs(..) => self
                .opener(explorer)
                .map(|opener| text::join_args(&opener.args))
                .unwrap_or_default(),
        }
    }

    /// Applies typed text, or explains why it cannot be saved.
    fn write(self, explorer: &mut ExplorerSettings, value: &str) -> Result<(), String> {
        let number = |value: &str| {
            let (start, end) = (
                *INDEXING_ENTRIES_RANGE.start(),
                *INDEXING_ENTRIES_RANGE.end(),
            );
            value
                .trim()
                .replace(['_', ' '], "")
                .parse::<u32>()
                .ok()
                .filter(|value| INDEXING_ENTRIES_RANGE.contains(value))
                .ok_or_else(|| format!("Enter a whole number from {start} to {end}."))
        };
        match self {
            Key::Excluded => explorer.indexing.excluded = text::split_list(value),
            Key::MaxLocal => explorer.indexing.max_local_entries = number(value)?,
            Key::MaxRemote => explorer.indexing.max_remote_entries = number(value)?,
            Key::Extensions(ix) => {
                let rule = explorer
                    .open
                    .rules
                    .get_mut(ix)
                    .ok_or("This file type was removed.")?;
                rule.extensions = text::split_list(value)
                    .into_iter()
                    .map(|extension| extension.trim_start_matches('.').to_owned())
                    .collect();
            }
            Key::Program(_) | Key::RuleProgram(..) => {
                self.opener_mut(explorer)
                    .ok_or("This file type was removed.")?
                    .program = value.trim().to_owned();
            }
            Key::Args(_) | Key::RuleArgs(..) => {
                self.opener_mut(explorer)
                    .ok_or("This file type was removed.")?
                    .args = text::split_args(value);
            }
        }
        Ok(())
    }
}

/// One line of text that saves itself when typing pauses.
struct Field {
    input: Entity<InputState>,
    error: Option<SharedString>,
    debounce: Option<Task<()>>,
    _subscription: Subscription,
}

pub struct ExplorerPage {
    focus: FocusHandle,
    fields: BTreeMap<Key, Field>,
    /// How many rules the rule fields were made for.
    rules: usize,
    _settings: Subscription,
}

fn explorer(cx: &App) -> ExplorerSettings {
    cx.settings().explorer.clone()
}

impl ExplorerPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.observe_global_in::<SettingsStore>(window, |page, window, cx| {
            page.sync(window, cx);
            cx.notify();
        });
        let mut page = Self {
            focus: cx.focus_handle(),
            fields: BTreeMap::new(),
            rules: 0,
            _settings: settings,
        };
        for key in [
            Key::Excluded,
            Key::MaxLocal,
            Key::MaxRemote,
            Key::Program(Side::Local),
            Key::Args(Side::Local),
            Key::Program(Side::Remote),
            Key::Args(Side::Remote),
        ] {
            page.add_field(key, window, cx);
        }
        page.add_rule_fields(window, cx);
        page
    }

    fn add_field(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) {
        let value = key.read(&explorer(cx));
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value)
                .placeholder(key.placeholder())
        });
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                move |page, _, event, window, cx| match event {
                    InputEvent::Change => {
                        let Some(field) = page.fields.get_mut(&key) else {
                            return;
                        };
                        field.debounce = Some(cx.spawn_in(window, async move |page, cx| {
                            cx.background_executor().timer(DEBOUNCE).await;
                            let _ = page.update(cx, |page, cx| page.commit(key, cx));
                        }));
                    }
                    InputEvent::Blur | InputEvent::PressEnter { .. } => page.commit(key, cx),
                    InputEvent::Focus => {}
                },
            );
        self.fields.insert(
            key,
            Field {
                input,
                error: None,
                debounce: None,
                _subscription: subscription,
            },
        );
    }

    fn add_rule_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fields.retain(|key, _| !key.is_rule());
        self.rules = explorer(cx).open.rules.len();
        for ix in 0..self.rules {
            for key in [
                Key::Extensions(ix),
                Key::RuleProgram(ix, Side::Local),
                Key::RuleArgs(ix, Side::Local),
                Key::RuleProgram(ix, Side::Remote),
                Key::RuleArgs(ix, Side::Remote),
            ] {
                self.add_field(key, window, cx);
            }
        }
    }

    /// Shows values saved elsewhere, except in fields being edited.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let explorer = explorer(cx);
        if explorer.open.rules.len() != self.rules {
            self.add_rule_fields(window, cx);
        }
        for (key, field) in &mut self.fields {
            let value = key.read(&explorer);
            let input = field.input.read(cx);
            let focused = input.focus_handle(cx).is_focused(window);
            if focused || field.debounce.is_some() || input.value() == value.as_str() {
                continue;
            }
            field.error = None;
            field
                .input
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }

    /// Saves a field's text if it is valid.
    fn commit(&mut self, key: Key, cx: &mut Context<Self>) {
        let Some(field) = self.fields.get_mut(&key) else {
            return;
        };
        field.debounce = None;
        let value = field.input.read(cx).value().to_string();
        let mut candidate = explorer(cx);
        match key.write(&mut candidate, &value) {
            Ok(()) => {
                field.error = None;
                if candidate != explorer(cx) {
                    edit_settings(cx, move |settings| {
                        let _ = key.write(&mut settings.explorer, &value);
                    })
                    .detach();
                }
            }
            Err(error) => field.error = Some(error.into()),
        }
        cx.notify();
    }

    fn input(&self, key: Key) -> AnyElement {
        match self.fields.get(&key) {
            Some(field) => Input::new(&field.input).small().into_any_element(),
            None => div().into_any_element(),
        }
    }

    fn error(&self, key: Key) -> Option<SharedString> {
        self.fields.get(&key).and_then(|field| field.error.clone())
    }

    /// A program field beside its arguments.
    fn command(&self, program: Key, args: Key) -> impl IntoElement {
        h_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .w(rems(10.))
                    .flex_shrink_0()
                    .child(self.input(program)),
            )
            .child(div().flex_1().min_w_0().child(self.input(args)))
    }

    fn rule(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let side = |label: &'static str, side| {
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label),
                )
                .child(self.command(Key::RuleProgram(ix, side), Key::RuleArgs(ix, side)))
        };
        v_flex()
            .w_full()
            .py_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.input(Key::Extensions(ix))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("explorer-rule-remove-{ix}")))
                            .small()
                            .ghost()
                            .icon(IconName::Trash)
                            .tooltip("Remove this file type")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                edit_settings(cx, move |settings| {
                                    if ix < settings.explorer.open.rules.len() {
                                        settings.explorer.open.rules.remove(ix);
                                    }
                                })
                                .detach();
                            })),
                    ),
            )
            .when_some(self.error(Key::Extensions(ix)), |rule, error| {
                rule.child(form::error_text(error, cx))
            })
            .child(side("On this computer", Side::Local))
            .child(side("On servers", Side::Remote))
            .into_any_element()
    }
}

fn switch(
    id: &'static str,
    checked: bool,
    edit: fn(&mut Settings, bool),
    cx: &mut Context<ExplorerPage>,
) -> Switch {
    Switch::new(id)
        .checked(checked)
        .on_click(cx.listener(move |_, checked: &bool, _, cx| {
            let checked = *checked;
            edit_settings(cx, move |settings| edit(settings, checked)).detach();
            cx.notify();
        }))
}

/// One-click choices for a program field.
fn presets(
    id: &'static str,
    current: &str,
    choices: &[(&'static str, &'static str)],
    side: Side,
    cx: &mut Context<ExplorerPage>,
) -> AnyElement {
    h_flex()
        .gap_1()
        .flex_wrap()
        .children(choices.iter().map(|&(label, program)| {
            Button::new(SharedString::from(format!("{id}-{label}")))
                .xsmall()
                .ghost()
                .label(label)
                .selected(current == program)
                .on_click(cx.listener(move |_, _, _, cx| {
                    edit_settings(cx, move |settings| {
                        let open = &mut settings.explorer.open;
                        let opener = match side {
                            Side::Local => &mut open.local,
                            Side::Remote => &mut open.remote,
                        };
                        if opener.program != program {
                            *opener = Opener::new(program, &[]);
                        }
                    })
                    .detach();
                }))
        }))
        .into_any_element()
}

impl Render for ExplorerPage {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let explorer = explorer(cx);
        let indexing = &explorer.indexing;
        let statistics = vec![
            form::row(
                "Count local folders",
                "Count the files and the size of the folder shown on this computer.",
                switch(
                    "explorer-index-local",
                    indexing.local,
                    |s, on| s.explorer.indexing.local = on,
                    cx,
                ),
                cx,
            ),
            form::row(
                "Count remote folders",
                "Count the files and the size of the folder shown on a server, over SFTP.",
                switch(
                    "explorer-index-remote",
                    indexing.remote,
                    |s, on| s.explorer.indexing.remote = on,
                    cx,
                ),
                cx,
            ),
            form::row(
                "Skip home folders",
                "Do not count a home folder itself: it is usually huge. Its subfolders are still counted.",
                switch(
                    "explorer-skip-home",
                    indexing.skip_home,
                    |s, on| s.explorer.indexing.skip_home = on,
                    cx,
                ),
                cx,
            ),
            form::row(
                "Skip file system roots",
                "Do not count / or a drive's root.",
                switch(
                    "explorer-skip-root",
                    indexing.skip_root,
                    |s, on| s.explorer.indexing.skip_root = on,
                    cx,
                ),
                cx,
            ),
            form::stacked_row(
                "Folders never entered",
                "Counted, but not looked into, wherever they are. Separate names with commas.",
                self.input(Key::Excluded),
                self.error(Key::Excluded),
                cx,
            ),
            form::row(
                "Local entry limit",
                "Stop counting a local folder after this many entries.",
                div().w(rems(8.)).child(self.input(Key::MaxLocal)),
                cx,
            ),
            form::row(
                "Remote entry limit",
                "Stop counting a remote folder after this many entries; each folder costs a round trip.",
                div().w(rems(8.)).child(self.input(Key::MaxRemote)),
                cx,
            ),
        ];
        let limit_errors = [Key::MaxLocal, Key::MaxRemote]
            .into_iter()
            .filter_map(|key| self.error(key))
            .map(|error| form::error_text(error, cx))
            .collect::<Vec<_>>();
        let remote_choices = REMOTE_EDITORS.map(|editor| (editor, editor));
        let open = vec![
            form::stacked_row(
                "On this computer",
                "Program and arguments for local files. Leave the program empty for the system's default application; a terminal editor needs a terminal, such as kitty -e nvim.",
                v_flex()
                    .gap_2()
                    .child(self.command(Key::Program(Side::Local), Key::Args(Side::Local)))
                    .child(presets(
                        "explorer-local-preset",
                        &explorer.open.local.program,
                        &[("System default", "")],
                        Side::Local,
                        cx,
                    )),
                self.error(Key::Args(Side::Local)),
                cx,
            ),
            form::stacked_row(
                "On servers",
                "The editor started in a new terminal tab on the server. {file} stands for the file; without it the file comes last.",
                v_flex()
                    .gap_2()
                    .child(self.command(Key::Program(Side::Remote), Key::Args(Side::Remote)))
                    .child(presets(
                        "explorer-remote-preset",
                        &explorer.open.remote.program,
                        &remote_choices,
                        Side::Remote,
                        cx,
                    )),
                self.error(Key::Args(Side::Remote)),
                cx,
            ),
        ];
        let mut types: Vec<AnyElement> = (0..self.rules).map(|ix| self.rule(ix, cx)).collect();
        if types.is_empty() {
            types.push(form::note(
                "Every file opens with the programs above. Add a file type to open some files differently.",
                cx,
            ));
        }
        types.push(
            h_flex()
                .py_2()
                .child(
                    Button::new("explorer-rule-add")
                        .small()
                        .icon(IconName::Plus)
                        .label("Add file type")
                        .on_click(cx.listener(|_, _, _, cx| {
                            edit_settings(cx, |settings| {
                                settings.explorer.open.rules.push(OpenRule::default());
                            })
                            .detach();
                        })),
                )
                .into_any_element(),
        );
        v_flex()
            .w_full()
            .track_focus(&self.focus)
            .child(form::page_header(
                "Explorer",
                Some("Folder sizes and the programs that open files.".into()),
                cx,
            ))
            .child(form::section("Folder statistics", statistics, cx))
            .children(limit_errors)
            .child(form::section("Open files", open, cx))
            .child(form::section("File types", types, cx))
    }
}

impl Focusable for ExplorerPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsPage for ExplorerPage {}

#[cfg(test)]
mod tests;
