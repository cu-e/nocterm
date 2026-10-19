//! AppearanceSettings, Terminal, Local shell and SSH pages.
use super::{PageState, SshSettingsPage, controls::short_row};
use gpui_kit::{AnyElement, App, AppContext as _, Context, Entity, Window};
use nocterm_session::{
    CONNECT_TIMEOUT_RANGE, KEEPALIVE_RANGE, LoggingOptions, SessionOptions, SshSettings,
};
use nocterm_settings::SettingsDocument;
use nocterm_terminal_model::TerminalSettings;
use nocterm_ui::{SessionOptionsEditor, form, settings_input::number};
pub(super) fn new_session_options(
    settings: &nocterm_store::SettingsHandle,
    window: &mut Window,
    cx: &mut Context<SshSettingsPage>,
) -> Entity<SessionOptionsEditor> {
    let options = session_options(settings, cx);
    cx.new(|cx| SessionOptionsEditor::new(options, false, window, cx))
}
fn session_options(settings: &nocterm_store::SettingsHandle, cx: &App) -> SessionOptions {
    let terminal = settings.setting::<TerminalSettings>(cx);
    SessionOptions {
        term: Some(terminal.term.clone()),
        charset: Some(terminal.charset),
        proxy: Some(settings.setting::<SshSettings>(cx).proxy.clone()),
        logging: Some(settings.setting::<LoggingOptions>(cx).clone()),
    }
}
pub(super) fn apply_session_options(document: &mut SettingsDocument, options: SessionOptions) {
    document.update::<TerminalSettings>(|terminal| {
        terminal.term = options
            .term
            .unwrap_or_else(|| TerminalSettings::default().term);
        terminal.charset = options.charset.unwrap_or_default();
    });
    document.update::<SshSettings>(|ssh| ssh.proxy = options.proxy.unwrap_or_default());
    document.set(options.logging.unwrap_or_default());
}
pub(super) fn add_ssh_fields(
    view: &mut SshSettingsPage,
    window: &mut Window,
    cx: &mut Context<SshSettingsPage>,
) {
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
    super::render::shell_fields::<SshSettings, _>(
        view,
        "ssh.launch",
        |s| &mut s.launch,
        |s| &s.launch,
        window,
        cx,
    );
}
pub(super) fn ssh(view: &SshSettingsPage, cx: &mut Context<SshSettingsPage>) -> Vec<AnyElement> {
    vec![
        form::section(
            "Session defaults",
            [form::stacked_row(
                "Terminal type, encoding, proxy and output logs",
                "Used by every connection that does not set its own.",
                view.defaults.editor.clone(),
                view.defaults.error.clone(),
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
        super::render::remote_shell(view, cx),
    ]
}
