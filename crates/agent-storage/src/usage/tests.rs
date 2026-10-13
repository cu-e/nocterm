use super::*;
use futures::executor::block_on;
use nocterm_ai::usage::Window;

const CODEX_LINE: &str = r#"{"timestamp":"2026-10-03T17:42:09Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"limit_id":"codex","primary":{"used_percent":38.0,"window_minutes":10080,"resets_at":1791588105},"secondary":{"used_percent":7.5,"window_minutes":300,"resets_at":1791500000},"rate_limit_reached_type":null}}}"#;

#[test]
fn codex_limits_come_from_its_session_log() {
    let limits = codex_limits(CODEX_LINE).unwrap();
    assert_eq!(limits.len(), 2);
    assert_eq!(limits[0].window, Window::Weekly);
    assert_eq!(limits[0].used, Some(0.38));
    assert_eq!(limits[0].resets_at, Some(1_791_588_105));
    assert_eq!(limits[1].window, Window::FiveHour);
    assert!(codex_limits(r#"{"payload":{"type":"agent_message"}}"#).is_none());
    assert!(codex_limits(r#"{"payload":{"rate_limits":{"primary":null}}}"#).is_none());
}

#[test]
fn the_newest_codex_log_is_read() {
    let home = tempfile::tempdir().unwrap();
    let day = |path: &str| {
        let dir = home.path().join("sessions").join(path);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    };
    std::fs::write(
        day("2026/09/30").join("rollout-a.jsonl"),
        CODEX_LINE.replace("38.0", "99.0"),
    )
    .unwrap();
    let newest = day("2026/10/03");
    std::fs::write(newest.join("rollout-a.jsonl"), "{}\n").unwrap();
    std::fs::write(
        newest.join("rollout-b.jsonl"),
        format!("{CODEX_LINE}\n{{\"payload\":{{\"type\":\"agent_message\"}}}}\n"),
    )
    .unwrap();

    let limits = block_on(CodexUsageReader::new(Some(home.path().to_owned())).read(codex()))
        .unwrap()
        .unwrap();

    assert_eq!(limits[0].used, Some(0.38));
    assert!(
        block_on(CodexUsageReader::new(Some(home.path().join("missing"))).read(codex()))
            .unwrap()
            .is_none()
    );
}

fn codex() -> UsageAgent {
    UsageAgent {
        id: "custom".into(),
        command: "npx".into(),
        args: vec!["codex".into()],
    }
}

#[test]
fn unsupported_launch_does_not_access_the_configured_account_path() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let reader = CodexUsageReader::new(Some(file.path().to_owned()));
    let other = UsageAgent {
        id: "codex".into(),
        command: "claude".into(),
        args: Vec::new(),
    };
    assert!(block_on(reader.read(other)).unwrap().is_none());
    assert!(
        block_on(CodexUsageReader::new(None).read(codex()))
            .unwrap()
            .is_none()
    );
    let error = block_on(reader.read(codex())).unwrap_err();
    assert!(matches!(error, UsageReadError::Enumerate { .. }));
    use std::error::Error as _;
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<Arc<std::io::Error>>()
            .is_some()
    );
}

#[test]
fn malformed_and_partial_utf8_tail_lines_preserve_the_last_usable_report() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join("sessions/2026/10/03");
    std::fs::create_dir_all(&day).unwrap();
    let mut bytes = vec![b'x'; CODEX_TAIL_BYTES as usize + 32];
    bytes.extend_from_slice(b"\xff\xfe\n");
    bytes.extend_from_slice(CODEX_LINE.as_bytes());
    bytes.extend_from_slice(b"\n{\"rate_limits\": broken}\n{}\n");
    std::fs::write(day.join("rollout-a.jsonl"), bytes).unwrap();
    let limits = block_on(CodexUsageReader::new(Some(home.path().to_owned())).read(codex()))
        .unwrap()
        .unwrap();
    assert_eq!(limits[0].used, Some(0.38));
}

#[test]
fn records_outside_the_tail_are_not_read_and_corrupt_reports_return_no_limits() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join("sessions/2026/10/03");
    std::fs::create_dir_all(&day).unwrap();
    let log = day.join("rollout-a.jsonl");
    let bytes = format!("{CODEX_LINE}\n{}", "x".repeat(CODEX_TAIL_BYTES as usize));
    std::fs::write(&log, bytes).unwrap();
    let reader = CodexUsageReader::new(Some(home.path().to_owned()));
    assert!(block_on(reader.read(codex())).unwrap().is_none());
    std::fs::write(&log, "{\"rate_limits\": invalid}\n{}\n").unwrap();
    assert!(block_on(reader.read(codex())).unwrap().is_none());
}
