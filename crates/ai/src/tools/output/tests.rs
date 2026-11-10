use super::*;
use nocterm_session::{ExecError, ExecExit, command_jobs::CommandState};
use serde_json::json;

#[test]
fn command_wire_preserves_lossy_output_exit_counts_and_explicit_redaction_policy() {
    let snapshot = CommandSnapshot {
        state: CommandState::Exited,
        stdout: b"password=secret\xff".to_vec(),
        total_bytes: 100,
        exit: Some(ExecExit {
            status: Some(7),
            stderr: "token=hidden".into(),
        }),
        error: Some(ExecError::Failed("password=private".into())),
    };
    let literal = json!(CommandReport::new("c1", &snapshot, false));
    assert_eq!(
        literal,
        json!({
            "command_id": "c1", "state": "exited", "stdout": "password=secret�",
            "stderr": "token=hidden", "exit_status": 7, "total_bytes": 100,
            "truncated": true, "error": "could not start the program: password=[REDACTED]",
        })
    );
    let redacted = json!(CommandReport::new("c1", &snapshot, true));
    assert_eq!(redacted["stdout"], "password=[REDACTED]");
    assert_eq!(redacted["stderr"], "token=[REDACTED]");
    assert_eq!(redacted["total_bytes"], 100);
    assert_eq!(redacted["truncated"], true);
}

#[test]
fn terminal_wire_keeps_cursor_rules_and_observation_never_invents_an_exit() {
    let snapshot = || TerminalReport::new("界".into(), 5, 7, true, false);
    let expected = json!({
        "text": "界", "first_line": 5, "next_line": 7,
        "cursor_semantics": "inclusive snapshot; replace overlapping lines",
        "truncated": true, "alt_screen": false,
    });
    assert_eq!(json!(snapshot()), expected);
    for reason in ["prompt_returned", "output_idle", "timeout"] {
        let mut observation = expected.clone();
        observation["completion"] = json!(reason);
        observation["completed"] = json!(reason == "prompt_returned");
        observation["exit_status"] = json!(null);
        assert_eq!(json!(snapshot().observed(reason.into())), observation);
        observation["future_field"] = json!("kept by display fallback");
        let decoded: ObservationReport = serde_json::from_value(observation).unwrap();
        assert_eq!(decoded.terminal.next_line, 7);
        assert_eq!(decoded.completed, reason == "prompt_returned");
        assert!(decoded.exit_status.is_none());
    }
}

#[test]
fn unfinished_commands_report_unknown_exit_and_do_not_mark_exact_output_truncated() {
    assert_eq!(
        json!(CommandReport::new("c2", &CommandSnapshot::default(), false)),
        json!({
            "command_id": "c2", "state": "starting", "stdout": "", "stderr": "",
            "exit_status": null, "total_bytes": 0, "truncated": false, "error": null,
        })
    );
}
