//! AppearanceSettings, Terminal, Local shell and SSH pages.
use super::{
    PageState,
    controls::{choices, short_row, text_row, toggle},
};
use gpui_kit::{AnyElement, Context, Window};
use nocterm_session::{LocalShellSettings, ShellSettings, SshSettings};
use nocterm_settings::SettingsSection;
use nocterm_terminal_model::{
    ClipboardWritePolicy, CursorShape, FONT_SIZE_RANGE, LINE_HEIGHT_RANGE, SCROLLBACK_RANGE,
    TerminalSettings,
};
use nocterm_ui::{
    form,
    settings_input::{
        SettingsInputError as FieldError, environment, json, number, optional, optional_number,
        string_list,
    },
};
pub(super) fn terminal_fields<P: PageState>(
    view: &mut P,
    window: &mut Window,
    cx: &mut Context<P>,
) {
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
}
pub(super) fn shell_fields<S: SettingsSection, P: PageState>(
    view: &mut P,
    prefix: &'static str,
    shell: fn(&mut S) -> &mut ShellSettings,
    read: fn(&S) -> &ShellSettings,
    window: &mut Window,
    cx: &mut Context<P>,
) {
    view.add_field(
        format!("{prefix}.program"),
        move |s| read(s).program.clone().unwrap_or_default(),
        move |s, text| {
            let program = optional(text);
            if program.as_deref().is_some_and(|p| p.contains('\0')) {
                return Err(FieldError::ExecutableContainsNul);
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
                return Err(FieldError::DirectoryContainsNul);
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
pub(super) fn terminal<P: PageState>(view: &P, cx: &mut Context<P>) -> Vec<AnyElement> {
    let terminal = view.settings().setting::<TerminalSettings>(cx).clone();
    vec![
        terminal_text(view, &terminal, cx),
        terminal_cursor(view, &terminal, cx),
        terminal_behavior(view, &terminal, cx),
        terminal_gutter(view, &terminal, cx),
    ]
}
pub(super) fn terminal_text<P: PageState>(
    view: &P,
    _terminal: &TerminalSettings,
    cx: &mut Context<P>,
) -> AnyElement {
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
    )
}
pub(super) fn terminal_cursor<P: PageState>(
    _view: &P,
    terminal: &TerminalSettings,
    cx: &mut Context<P>,
) -> AnyElement {
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
    )
}
pub(super) fn terminal_behavior<P: PageState>(
    view: &P,
    terminal: &TerminalSettings,
    cx: &mut Context<P>,
) -> AnyElement {
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
    )
}
pub(super) fn terminal_gutter<P: PageState>(
    _view: &P,
    terminal: &TerminalSettings,
    cx: &mut Context<P>,
) -> AnyElement {
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
    )
}
pub(super) fn local_shell<P: PageState>(view: &P, cx: &mut Context<P>) -> Vec<AnyElement> {
    let integration = view
        .settings()
        .setting::<LocalShellSettings>(cx)
        .integration;
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
pub(super) fn remote_shell<P: PageState>(view: &P, cx: &mut Context<P>) -> AnyElement {
    let integration = view
        .settings()
        .setting::<SshSettings>(cx)
        .launch
        .integration;
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
    )
}
