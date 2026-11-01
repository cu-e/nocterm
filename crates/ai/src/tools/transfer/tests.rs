use super::*;
use crate::TerminalCall;
use nocterm_session::{HostAddress, Target};
use nocterm_transfers::{
    TransferCause, TransferId, TransferIssue, TransferOperation, TransferPath,
};

fn transfer() -> TransferFiles {
    TransferFiles {
        source_terminal_id: "t1".into(),
        sources: vec!["/srv/a".into(), "/srv/dir b".into()],
        destination_terminal_id: "t2".into(),
        destination: "/home/alice/in".into(),
        overwrite: false,
        yield_ms: Some(1000),
    }
}

fn rejection(change: impl FnOnce(&mut TransferFiles)) -> TerminalCallError {
    let mut call = transfer();
    change(&mut call);
    TerminalCall::TransferFiles(call).validate().unwrap_err()
}

fn progress(errors: Vec<TransferIssue>) -> Progress {
    Progress {
        id: TransferId::from_bytes([0xab; 16]).unwrap(),
        host: HostAddress::direct(Target {
            host: "example.test".into(),
            port: 22,
            user: "alice".into(),
        }),
        destination: "/srv/in".into(),
        direction: TransferDirection::Download,
        state: TransferState::Running,
        discovered_files: 12,
        completed_files: 10,
        prepared_directories: 2,
        skipped_files: 1,
        failed_files: 1,
        total_bytes: 1_048_576,
        sent_bytes: 524_288,
        discovery_complete: true,
        errors,
    }
}

fn issue(path: &str, cause: TransferCause) -> TransferIssue {
    TransferIssue::new(
        Some(TransferPath::Remote(path.into())),
        TransferOperation::Write,
        cause,
    )
}

#[test]
fn a_well_formed_transfer_validates_and_writes_on_both_terminals() {
    let call = TerminalCall::TransferFiles(transfer());
    assert!(call.validate().is_ok());
    assert!(call.writes());
    assert_eq!(call.capability(), crate::ToolCapability::Transfer);
    assert_eq!(call.terminal_id(), Some("t2"));
    assert_eq!(call.source_terminal_id(), Some("t1"));
    assert_eq!(call.terminals().collect::<Vec<_>>(), ["t2", "t1"]);
}

#[test]
fn terminals_lists_a_shared_source_and_destination_once() {
    let mut same = transfer();
    same.source_terminal_id = "t2".into();
    let call = TerminalCall::TransferFiles(same);
    assert_eq!(call.terminals().collect::<Vec<_>>(), ["t2"]);
}

#[test]
fn read_and_cancel_have_no_terminal_and_differ_in_writing() {
    let read = TerminalCall::ReadTransfer(ReadTransfer {
        transfer_id: "x".into(),
        yield_ms: None,
    });
    let cancel = TerminalCall::CancelTransfer(CancelTransfer {
        transfer_id: "x".into(),
    });
    for call in [&read, &cancel] {
        assert_eq!(call.terminals().count(), 0);
        assert!(call.target().is_none());
    }
    assert!(!read.writes());
    assert_eq!(read.capability(), crate::ToolCapability::Read);
    assert!(cancel.writes());
    assert_eq!(cancel.capability(), crate::ToolCapability::Transfer);
}

#[test]
fn transfer_terminal_ids_must_be_one_to_128_bytes() {
    for id in ["", &"t".repeat(129)] {
        assert_eq!(
            rejection(|call| call.source_terminal_id = id.into()),
            TerminalCallError::InvalidTarget
        );
        assert_eq!(
            rejection(|call| call.destination_terminal_id = id.into()),
            TerminalCallError::InvalidTarget
        );
    }
}

#[test]
fn transfer_sources_and_destination_are_bounded() {
    let rejected = TerminalCallError::TransferRejected;
    assert_eq!(rejection(|call| call.sources.clear()), rejected);
    assert_eq!(
        rejection(|call| call.sources = vec!["/a".into(); MAX_TRANSFER_SOURCES + 1]),
        rejected
    );
    assert_eq!(
        rejection(|call| call.sources = vec![String::new()]),
        rejected
    );
    assert_eq!(
        rejection(|call| call.sources = vec!["/a\0".into()]),
        rejected
    );
    assert_eq!(
        rejection(|call| call.sources = vec!["/".repeat(MAX_TRANSFER_PATH_BYTES + 1)]),
        rejected
    );
    let long = "/".repeat(MAX_TRANSFER_PATH_BYTES);
    assert_eq!(
        rejection(|call| call.sources = vec![long.clone(); 17]),
        rejected
    );
    assert_eq!(rejection(|call| call.destination.clear()), rejected);
    assert_eq!(rejection(|call| call.destination = "/\0".into()), rejected);
    assert_eq!(rejection(|call| call.yield_ms = Some(10_001)), rejected);
    let mut edge = transfer();
    edge.sources = vec!["/a".into(); MAX_TRANSFER_SOURCES];
    edge.yield_ms = Some(10_000);
    assert!(TerminalCall::TransferFiles(edge).validate().is_ok());
}

#[test]
fn control_characters_cannot_reshape_a_transfer_path() {
    let rejected = TerminalCallError::TransferRejected;
    for text in [
        "/a\nInto: /tmp",
        "/a\rb",
        "/a\u{1b}[2J",
        "/a\tb",
        "/a\u{7f}",
    ] {
        assert_eq!(
            rejection(|call| call.sources = vec![text.into()]),
            rejected,
            "{text:?}"
        );
        assert_eq!(
            rejection(|call| call.destination = text.into()),
            rejected,
            "{text:?}"
        );
    }
}

#[test]
fn read_and_cancel_validation_name_their_own_rejection() {
    let read = |transfer_id: &str, yield_ms| {
        TerminalCall::ReadTransfer(ReadTransfer {
            transfer_id: transfer_id.into(),
            yield_ms,
        })
        .validate()
    };
    assert!(read("abc", Some(10_000)).is_ok());
    for result in [
        read("", None),
        read("a", Some(10_001)),
        read(&"a".repeat(129), None),
    ] {
        assert_eq!(result, Err(TerminalCallError::ReadTransferRejected));
    }
    let cancel = |transfer_id: &str| {
        TerminalCall::CancelTransfer(CancelTransfer {
            transfer_id: transfer_id.into(),
        })
        .validate()
    };
    assert!(cancel("abc").is_ok());
    assert_eq!(cancel(""), Err(TerminalCallError::InvalidTransferId));
    assert_eq!(
        cancel(&"a".repeat(129)),
        Err(TerminalCallError::InvalidTransferId)
    );
}

#[test]
fn report_describes_progress_for_the_agent() {
    let value = report(
        &progress(vec![issue("/srv/a/x", TransferCause::Cancelled)]),
        false,
    );
    assert_eq!(value["transfer_id"], "abababab-abab-abab-abab-abababababab");
    assert_eq!(value["state"], "running");
    assert_eq!(value["direction"], "download");
    assert_eq!(value["destination"], "/srv/in");
    assert_eq!(value["files"]["completed"], 10);
    assert_eq!(value["files"]["skipped"], 1);
    assert_eq!(value["bytes"]["transferred"], 524_288);
    assert_eq!(value["discovery_complete"], true);
    assert_eq!(value["failures"][0]["path"], "/srv/a/x");
    assert_eq!(value["failures"][0]["operation"], "write");
    assert_eq!(value["failures"][0]["error"], "Transfer cancelled.");
    assert_eq!(value["failures_truncated"], false);
}

#[test]
fn report_keeps_the_first_sixteen_failures_and_says_so() {
    let issues = (0..MAX_TRANSFER_FAILURES + 4)
        .map(|n| issue(&format!("/srv/{n}"), TransferCause::Cancelled))
        .collect();
    let value = report(&progress(issues), false);
    let failures = value["failures"].as_array().unwrap();
    assert_eq!(failures.len(), MAX_TRANSFER_FAILURES);
    assert_eq!(failures[0]["path"], "/srv/0");
    assert_eq!(value["failures_truncated"], true);
}

#[test]
fn report_redacts_paths_and_error_text_only_on_request() {
    let secret = "/srv/ghp_abcdefghijklmnopqrst/key";
    let mut input = progress(vec![issue(secret, TransferCause::Cancelled)]);
    input.destination = secret.into();
    let plain = report(&input, false);
    assert_eq!(plain["destination"], secret);
    assert_eq!(plain["failures"][0]["path"], secret);
    let redacted = report(&input, true).to_string();
    assert!(!redacted.contains("ghp_abcdefghijklmnopqrst"));
    assert!(redacted.contains("[REDACTED TOKEN]"));
}
