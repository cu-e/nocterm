//! Appearance, Terminal, Local shell and SSH pages.
use gpui_kit::{
    AnyElement, App, Context, Entity, SharedString, Window,
    component::{
        Disableable as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Input,
        switch::Switch,
        v_flex,
    },
    prelude::*,
    rems,
};
use nocterm_settings::{
    Appearance, CARD_GAP_RANGE, CARD_RADIUS_RANGE, CONNECT_TIMEOUT_RANGE, ClipboardWritePolicy,
    CursorShape, FONT_SIZE_RANGE, KEEPALIVE_RANGE, LINE_HEIGHT_RANGE, LocalShellSettings,
    LoggingOptions, SCROLLBACK_RANGE, SessionOptions, SettingsDocument, SettingsSection,
    ShellSettings, SshSettings, TerminalSettings,
};
use nocterm_ui::{SessionOptionsEditor, SettingsExt as _, form};

use crate::{
    Page, SettingsView,
    field::{environment, json, number, optional, optional_number, string_list},
};

pub(crate) fn new_session_options(
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Entity<SessionOptionsEditor> {
    let options = session_options(cx);
    cx.new(|cx| SessionOptionsEditor::new(options, false, window, cx))
}

fn session_options(cx: &App) -> SessionOptions {
    let terminal = cx.setting::<TerminalSettings>();
    SessionOptions {
        term: Some(terminal.term.clone()),
        charset: Some(terminal.charset),
        proxy: Some(cx.setting::<SshSettings>().proxy.clone()),
        logging: Some(cx.setting::<LoggingOptions>().clone()),
    }
}

pub(crate) fn apply_session_options(document: &mut SettingsDocument, options: SessionOptions) {
    document.update::<TerminalSettings>(|terminal| {
        terminal.term = options
            .term
            .unwrap_or_else(|| TerminalSettings::default().term);
        terminal.charset = options.charset.unwrap_or_default();
    });
    document.update::<SshSettings>(|ssh| ssh.proxy = options.proxy.unwrap_or_default());
    document.set(options.logging.unwrap_or_default());
}

#[expect(clippy::too_many_lines, reason = "predates the limit")]
pub(crate) fn add_fields(
    view: &mut SettingsView,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) {
    view.add_field(
        "appearance.card_gap",
        |s: &Appearance| s.card_gap.to_string(),
        |s: &mut Appearance, text| {
            s.card_gap = number(text, "Card gap", CARD_GAP_RANGE)?;
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        "appearance.card_radius",
        |s: &Appearance| s.card_radius.to_string(),
        |s: &mut Appearance, text| {
            s.card_radius = number(text, "Card radius", CARD_RADIUS_RANGE)?;
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        "terminal.font_family",
        |s: &TerminalSettings| s.font_family.clone().unwrap_or_default(),
        |s: &mut TerminalSettings, text| {
            s.font_family = optional(text);
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        "terminal.font_size",
        |s: &TerminalSettings| s.font_size.map(|v| v.to_string()).unwrap_or_default(),
        |s: &mut TerminalSettings, text| {
            s.font_size = optional_number(text, "Font size", FONT_SIZE_RANGE)?;
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        "terminal.line_height",
        |s: &TerminalSettings| s.line_height.map(|v| v.to_string()).unwrap_or_default(),
        |s: &mut TerminalSettings, text| {
            s.line_height = optional_number(text, "Line height", LINE_HEIGHT_RANGE)?;
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        "terminal.scrollback",
        |s: &TerminalSettings| s.scrollback_lines.to_string(),
        |s: &mut TerminalSettings, text| {
            s.scrollback_lines = number(text, "Scrollback", SCROLLBACK_RANGE)?;
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        "ssh.timeout",
        |s: &SshSettings| s.connect_timeout_secs.to_string(),
        |s: &mut SshSettings, text| {
            s.connect_timeout_secs = number(text, "Connection timeout", CONNECT_TIMEOUT_RANGE)?;
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        "ssh.keepalive",
        |s: &SshSettings| s.keepalive_interval_secs.to_string(),
        |s: &mut SshSettings, text| {
            s.keepalive_interval_secs = number(text, "Keepalive interval", KEEPALIVE_RANGE)?;
            Ok(())
        },
        window,
        cx,
    );
    shell_fields::<LocalShellSettings>(view, "local", |s| &mut s.0, |s| &s.0, window, cx);
    shell_fields::<SshSettings>(
        view,
        "ssh.launch",
        |s| &mut s.launch,
        |s| &s.launch,
        window,
        cx,
    );
}

/// Program, arguments, directory and environment of a shell.
fn shell_fields<S: SettingsSection>(
    view: &mut SettingsView,
    prefix: &'static str,
    shell: fn(&mut S) -> &mut ShellSettings,
    read: fn(&S) -> &ShellSettings,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) {
    view.add_field(
        format!("{prefix}.program"),
        move |s| read(s).program.clone().unwrap_or_default(),
        move |s, text| {
            let program = optional(text);
            if program.as_deref().is_some_and(|p| p.contains('\0')) {
                return Err("Executable cannot contain NUL.".into());
            }
            shell(s).program = program;
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        format!("{prefix}.args"),
        move |s| json(&read(s).args),
        move |s, text| {
            shell(s).args = string_list(text, "Arguments")?;
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        format!("{prefix}.cwd"),
        move |s| read(s).cwd.clone().unwrap_or_default(),
        move |s, text| {
            let cwd = optional(text);
            if cwd.as_deref().is_some_and(|p| p.contains('\0')) {
                return Err("Directory cannot contain NUL.".into());
            }
            shell(s).cwd = cwd;
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        format!("{prefix}.env"),
        move |s| json(&read(s).env),
        move |s, text| {
            shell(s).env = environment(text, "Environment")?;
            Ok(())
        },
        window,
        cx,
    );
}

pub(crate) fn render(
    view: &mut SettingsView,
    page: Page,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let sections = match page {
        Page::Appearance => crate::appearance::render(view, cx),
        Page::Terminal => terminal(view, cx),
        Page::LocalShell => local_shell(view, cx),
        Page::Ssh => ssh(view, cx),
        Page::Ai => unreachable!("the AI page renders itself"),
    };
    v_flex()
        .w_full()
        .child(form::page_header(page.title(), None, cx))
        .children(sections)
        .into_any_element()
}

/// A switch that saves `set(settings, checked)` when flipped.
pub(crate) fn toggle<S: SettingsSection>(
    id: impl Into<gpui_kit::ElementId>,
    checked: bool,
    disabled: bool,
    set: fn(&mut S, bool),
    cx: &mut Context<SettingsView>,
) -> Switch {
    Switch::new(id)
        .checked(checked)
        .disabled(disabled)
        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
            let checked = *checked;
            this.save(move |settings| set(settings, checked), cx);
        }))
}

/// Mutually exclusive choices shown side by side.
pub(crate) fn choices<S: SettingsSection, T: Copy + PartialEq + 'static>(
    id: &'static str,
    current: T,
    options: &[(&'static str, T)],
    set: fn(&mut S, T),
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    h_flex()
        .gap_1()
        .children(options.iter().enumerate().map(|(index, (label, value))| {
            let value = *value;
            Button::new((id, index))
                .small()
                .ghost()
                .label(*label)
                .selected(current == value)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.save(move |settings| set(settings, value), cx)
                }))
        }))
        .into_any_element()
}

/// A text input saved by its field, with the field's error under it.
pub(crate) fn text_row(
    view: &SettingsView,
    key: &str,
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    let field = view.field(key);
    form::stacked_row(
        label,
        description,
        Input::new(&field.input).small(),
        field.error.clone(),
        cx,
    )
}

/// A short text input on the right of its label.
pub(crate) fn short_row(
    view: &SettingsView,
    key: &str,
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    let field = view.field(key);
    let row = form::row(
        label,
        description,
        gpui_kit::div()
            .w(rems(10.))
            .child(Input::new(&field.input).small()),
        cx,
    );
    match field.error.clone() {
        Some(error) => v_flex()
            .child(row)
            .child(form::error_text(error, cx))
            .into_any_element(),
        None => row,
    }
}

#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn terminal(view: &SettingsView, cx: &mut Context<SettingsView>) -> Vec<AnyElement> {
    let terminal = cx.setting::<TerminalSettings>().clone();
    vec![
        form::section(
            "Text",
            [
                text_row(
                    view,
                    "terminal.font_family",
                    "Font family",
                    "Empty uses the theme's monospace font.",
                    cx,
                ),
                short_row(
                    view,
                    "terminal.font_size",
                    "Font size",
                    "6–72 pixels. Empty uses the theme size.",
                    cx,
                ),
                short_row(
                    view,
                    "terminal.line_height",
                    "Line height",
                    "1–3 times the font size. Empty uses the theme value.",
                    cx,
                ),
            ],
            cx,
        ),
        form::section(
            "Cursor",
            [
                form::row(
                    "Shape",
                    "",
                    choices(
                        "cursor",
                        terminal.cursor_shape,
                        &[
                            ("Block", CursorShape::Block),
                            ("Bar", CursorShape::Bar),
                            ("Underline", CursorShape::Underline),
                        ],
                        |s: &mut TerminalSettings, shape| s.cursor_shape = shape,
                        cx,
                    ),
                    cx,
                ),
                form::row(
                    "Blink",
                    "Blink the cursor while the terminal has focus.",
                    toggle(
                        "cursor-blink",
                        terminal.cursor_blink,
                        false,
                        |s: &mut TerminalSettings, on| s.cursor_blink = on,
                        cx,
                    ),
                    cx,
                ),
            ],
            cx,
        ),
        form::section(
            "Behavior",
            [
                short_row(
                    view,
                    "terminal.scrollback",
                    "Scrollback",
                    "Lines of history kept, 0–1000000.",
                    cx,
                ),
                form::row(
                    "Highlight terminal output",
                    "Color dates, addresses and important messages in plain output. Programs keep their own colors.",
                    toggle(
                        "semantic-highlighting",
                        terminal.semantic_highlighting,
                        false,
                        |s: &mut TerminalSettings, on| s.semantic_highlighting = on,
                        cx,
                    ),
                    cx,
                ),
                form::row(
                    "Copy on selection",
                    "Copy text to the clipboard as soon as it is selected.",
                    toggle(
                        "copy-select",
                        terminal.copy_on_select,
                        false,
                        |s: &mut TerminalSettings, on| s.copy_on_select = on,
                        cx,
                    ),
                    cx,
                ),
                form::row(
                    "Programs may write the clipboard",
                    "Allow OSC 52 clipboard writes while the terminal has focus. Reading the clipboard is never allowed.",
                    toggle(
                        "clipboard-write",
                        terminal.clipboard_write == ClipboardWritePolicy::FocusedTerminal,
                        false,
                        |s: &mut TerminalSettings, on| {
                            s.clipboard_write = if on {
                                ClipboardWritePolicy::FocusedTerminal
                            } else {
                                ClipboardWritePolicy::Deny
                            }
                        },
                        cx,
                    ),
                    cx,
                ),
            ],
            cx,
        ),
        form::section(
            "Gutter",
            [
                form::row(
                    "Line numbers",
                    "Number logical lines outside the terminal grid.",
                    toggle(
                        "line-numbers",
                        terminal.show_line_numbers,
                        false,
                        |s: &mut TerminalSettings, on| s.show_line_numbers = on,
                        cx,
                    ),
                    cx,
                ),
                form::row(
                    "Timestamps",
                    "Show when each line was first written, in UTC.",
                    toggle(
                        "timestamps",
                        terminal.show_timestamps,
                        false,
                        |s: &mut TerminalSettings, on| s.show_timestamps = on,
                        cx,
                    ),
                    cx,
                ),
            ],
            cx,
        ),
    ]
}

fn local_shell(view: &SettingsView, cx: &mut Context<SettingsView>) -> Vec<AnyElement> {
    let integration = cx.setting::<LocalShellSettings>().integration;
    vec![
        form::section(
            "Launch",
            [
                text_row(
                    view,
                    "local.program",
                    "Executable",
                    "Empty uses your login shell. Applies to new shells.",
                    cx,
                ),
                text_row(
                    view,
                    "local.args",
                    "Arguments",
                    "JSON array, e.g. [\"-l\"]. Custom arguments turn off shell integration.",
                    cx,
                ),
                text_row(
                    view,
                    "local.cwd",
                    "Start in",
                    "Empty uses your home folder.",
                    cx,
                ),
                text_row(
                    view,
                    "local.env",
                    "Environment",
                    "JSON object, e.g. {\"LANG\":\"en_US.UTF-8\"}. Do not store secrets here.",
                    cx,
                ),
            ],
            cx,
        ),
        form::section(
            "Integration",
            [form::row(
                "Shell integration",
                "Track the current folder and prompt in supported shells.",
                toggle(
                    "local-integration",
                    integration,
                    false,
                    |s: &mut LocalShellSettings, on| s.integration = on,
                    cx,
                ),
                cx,
            )],
            cx,
        ),
    ]
}

fn ssh(view: &SettingsView, cx: &mut Context<SettingsView>) -> Vec<AnyElement> {
    let integration = cx.setting::<SshSettings>().launch.integration;
    vec![
        form::section(
            "Session defaults",
            [form::stacked_row(
                "Terminal type, encoding, proxy and output logs",
                "Used by every connection that does not set its own.",
                view.session_options.clone(),
                view.session_options_error.clone(),
                cx,
            )],
            cx,
        ),
        form::section(
            "Connection",
            [
                short_row(
                    view,
                    "ssh.timeout",
                    "Connection timeout",
                    "Seconds, 1–600. Applies to new connections.",
                    cx,
                ),
                short_row(
                    view,
                    "ssh.keepalive",
                    "Keepalive interval",
                    "Seconds, 0–3600; 0 turns probes off.",
                    cx,
                ),
            ],
            cx,
        ),
        form::section(
            "Remote shell",
            [
                text_row(
                    view,
                    "ssh.launch.program",
                    "Executable",
                    "Empty asks the server for its default shell. Connections may override this.",
                    cx,
                ),
                text_row(
                    view,
                    "ssh.launch.args",
                    "Arguments",
                    "JSON array; applies on reconnect.",
                    cx,
                ),
                text_row(
                    view,
                    "ssh.launch.cwd",
                    "Start in",
                    "Empty uses the remote home folder.",
                    cx,
                ),
                text_row(
                    view,
                    "ssh.launch.env",
                    "Environment",
                    "JSON object; servers may refuse some variables.",
                    cx,
                ),
                form::row(
                    "Shell integration",
                    "Track the current folder and prompt on remote hosts.",
                    toggle(
                        "remote-integration",
                        integration,
                        false,
                        |s: &mut SshSettings, on| s.launch.integration = on,
                        cx,
                    ),
                    cx,
                ),
            ],
            cx,
        ),
    ]
}
