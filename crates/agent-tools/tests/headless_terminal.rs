#[path = "headless_terminal/commands.rs"]
mod commands;
mod headless_support;
#[path = "headless_terminal/retained.rs"]
mod retained;
#[path = "headless_terminal/sessions.rs"]
mod sessions;
use headless_support::*;
use nocterm_ai::{RunCommand, SendInput, TerminalCall};
use nocterm_cloud_model::tool::{ToolEffectStatus, ToolExecutionError};
use nocterm_session::{Charset, Command, Event};

#[test]
fn text_uses_actual_vt_wrapping_hidden_cells_and_inclusive_cursor() {
    let mut host = Harness::new(Charset::Utf8);
    host.emit(b"first\r\npartial");
    let first = host.read(1, None);
    assert_eq!(first["text"], "first\npartial");
    host.emit(b"END\r\npass: \x1b[8msecret\x1b[0m!");
    let second = host.read(2, first["next_line"].as_u64());
    assert_eq!(second["text"], "partialEND\npass:       !");
    assert_eq!(second["first_line"], first["next_line"]);
    host.emit(b"\x1b[?1049h\x1b[Hmenu");
    let alt = host.read(3, Some(9999));
    assert_eq!(alt["text"], "menu");
    assert_eq!(alt["alt_screen"], true);
    host.emit(b"\x1b[?1049l");
    assert_eq!(host.read(4, None)["alt_screen"], false);
}

#[test]
fn fragmented_utf8_and_configured_legacy_charset_decode_and_encode() {
    let mut host = Harness::new(Charset::Utf8);
    host.emit([0xe2]);
    host.emit([0x82]);
    host.emit([0xac, 0xff]);
    assert_eq!(host.read(1, None)["text"], "€�");
    let mut host = Harness::new(Charset::Windows1251);
    host.emit([0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2]);
    assert_eq!(host.read(1, None)["text"], "Привет");
    host.call(
        2,
        TerminalCall::SendInput(SendInput {
            terminal_id: host.terminal.clone(),
            text: "Привет".into(),
            press_enter: true,
        }),
    )
    .unwrap();
    assert_eq!(
        host.pool.run_until(host.driver.next_command()),
        Some(Command::Input(vec![
            0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2, b'\r'
        ]))
    );
    assert!(
        host.call(
            3,
            TerminalCall::SendInput(SendInput {
                terminal_id: host.terminal.clone(),
                text: "中文".into(),
                press_enter: false
            })
        )
        .is_err()
    );
}

#[test]
fn clipboard_title_and_bell_osc_have_no_native_side_effects() {
    let mut host = Harness::new(Charset::Utf8);
    host.emit(b"\x1b]52;c;c2VjcmV0\x07\x1b]0;hostile title\x07\x07visible");
    assert_eq!(host.read(1, None)["text"], "visible");
    assert!(futures::FutureExt::now_or_never(host.driver.next_command()).is_none());
}

#[test]
fn command_requires_known_clean_prompt_and_output_idle_remains_unknown() {
    let mut host = Harness::new(Charset::Utf8);
    let command = |terminal: String| {
        TerminalCall::RunCommand(RunCommand {
            terminal_id: terminal,
            command: "echo hi".into(),
            timeout_ms: None,
            idle_ms: Some(100),
        })
    };
    assert!(host.call(1, command(host.terminal.clone())).is_err());
    host.emit(b"\x1b]133;A\x07$ ");
    host.call(
        2,
        TerminalCall::SendInput(SendInput {
            terminal_id: host.terminal.clone(),
            text: "unfinished".into(),
            press_enter: false,
        }),
    )
    .unwrap();
    assert!(host.call(3, command(host.terminal.clone())).is_err());
    host.emit(b"\x1b]133;A\x07");
    let request = host.request(4, command(host.terminal.clone()));
    let pending = host.start(request);
    host.emit(b"running...");
    host.tick(100);
    host.tick(100);
    let output = host.finish(pending).unwrap();
    assert_eq!(output.output().value()["completion"], "output_idle");
    assert_eq!(output.output().value()["completed"], false);
    assert_eq!(
        output.output().value()["exit_status"],
        serde_json::Value::Null
    );
    assert_eq!(output.effect_status(), &ToolEffectStatus::Unknown);
}

#[test]
fn prompt_return_is_observed_and_restore_markers_cannot_authorize_input() {
    let mut host = Harness::new(Charset::Utf8);
    host.emit(b"\x1b]133;A\x07");
    let call = TerminalCall::RunCommand(RunCommand {
        terminal_id: host.terminal.clone(),
        command: "true".into(),
        timeout_ms: None,
        idle_ms: None,
    });
    let pending = host.start(host.request(1, call));
    host.emit(b"\x1b]133;C\x07done\r\n\x1b]133;A\x07");
    host.tick(100);
    let output = host.finish(pending).unwrap();
    assert_eq!(output.output().value()["completed"], true);
    assert_eq!(output.effect_status(), &ToolEffectStatus::Observed);
    assert!(
        host.pool
            .run_until(host.driver.emit(Event::Restore(b"\x1b]133;A\x07".to_vec())))
    );
    host.pump();
    let call = TerminalCall::RunCommand(RunCommand {
        terminal_id: host.terminal.clone(),
        command: "false".into(),
        timeout_ms: None,
        idle_ms: None,
    });
    assert!(host.call(2, call).is_err());
}

#[test]
fn revoked_and_foreign_authority_are_refused_before_input_or_exec() {
    let mut host = Harness::new(Charset::Utf8);
    host.checks.refuse(ToolExecutionError::Revoked);
    assert!(matches!(
        host.call(
            1,
            TerminalCall::SendInput(SendInput {
                terminal_id: host.terminal.clone(),
                text: "danger".into(),
                press_enter: true
            })
        ),
        Err(ToolExecutionError::Revoked)
    ));
    assert!(matches!(
        host.call(2, host.exec("danger")),
        Err(ToolExecutionError::Revoked)
    ));
    assert!(host.executor.requests().is_empty());
    assert!(futures::FutureExt::now_or_never(host.driver.next_command()).is_none());
}
