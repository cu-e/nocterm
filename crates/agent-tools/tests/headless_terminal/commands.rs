use crate::headless_support::*;
use futures::task::LocalSpawnExt;
use nocterm_ai::{CancelCommand, ReadCommand, TerminalCall};
use nocterm_cloud_model::tool::{
    BackgroundCommandOutcome, ScopedToolExecutor, ToolEffectStatus, ToolExecutionError,
};
use nocterm_session::{Charset, ExecExit, testing::ExecResponse};

#[test]
fn structured_program_keeps_literal_argv_and_durable_background_lifecycle() {
    let mut host = Harness::new(Charset::Utf8);
    host.executor.respond(
        "printf",
        Ok(ExecResponse::text("result", 7, "actual stderr")),
    );
    let mut call = host.exec("printf");
    let TerminalCall::ExecCommand(args) = &mut call else {
        unreachable!()
    };
    args.args = vec!["literal ; $(touch forbidden)".into(), "a b".into()];
    args.stdin = Some("literal stdin".into());
    let request = host.request(1, call);
    let pending = host.start(request.clone());
    let output = host.finish(pending).unwrap();
    assert!(matches!(
        output.effect_status(),
        ToolEffectStatus::Background(_)
    ));
    assert_eq!(host.executor.requests().len(), 1);
    assert_eq!(
        host.executor.requests()[0].args,
        ["literal ; $(touch forbidden)", "a b"]
    );
    assert_eq!(
        host.executor.requests()[0].stdin.as_deref(),
        Some(b"literal stdin".as_slice())
    );
    let durable = host.journal.observations();
    assert_eq!(durable.len(), 1);
    assert_eq!(
        durable[0].1,
        Some(BackgroundCommandOutcome::ObservedExit(7))
    );
    assert_eq!(durable[0].0.dispatch(), request.permit());
    let command = output.output().value()["command_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let read = host
        .call(
            2,
            TerminalCall::ReadCommand(ReadCommand {
                terminal_id: host.terminal.clone(),
                command_id: command,
                yield_ms: None,
            }),
        )
        .unwrap();
    assert_eq!(read.output().value()["stdout"], "result");
    assert_eq!(read.output().value()["stderr"], "actual stderr");
    assert_eq!(read.output().value()["exit_status"], 7);
    assert_eq!(read.effect_status(), &ToolEffectStatus::Observed);
    let again = host.start(request);
    assert!(host.finish(again).is_ok());
    assert_eq!(
        host.executor.requests().len(),
        1,
        "original dispatch is never executed twice"
    );
}

#[test]
fn journal_failure_and_revocation_before_worker_poll_prevent_native_execution() {
    let mut host = Harness::new(Charset::Utf8);
    host.journal.refuse(ToolExecutionError::OutcomeUnknown);
    assert!(host.call(1, host.exec("never")).is_err());
    assert!(host.executor.requests().is_empty());
    let mut host = Harness::new(Charset::Utf8);
    let request = host.request(1, host.exec("never"));
    let result = host.pool.run_until(host.tools.execute(request)).unwrap();
    assert!(matches!(
        result.effect_status(),
        ToolEffectStatus::Background(_)
    ));
    assert_eq!(
        host.journal.observations().len(),
        1,
        "durable linkage precedes first poll"
    );
    assert!(host.executor.requests().is_empty());
    host.checks.refuse(ToolExecutionError::Expired);
    host.pump();
    assert!(host.executor.requests().is_empty());
    assert_eq!(
        host.journal.observations()[0].1,
        Some(BackgroundCommandOutcome::Unknown)
    );
    assert!(matches!(
        host.task_errors.lock().unwrap().as_slice(),
        [ToolExecutionError::Expired]
    ));
}

#[test]
fn removed_original_session_cannot_start_a_queued_worker() {
    let mut host = Harness::new(Charset::Utf8);
    let request = host.request(1, host.exec("never"));
    host.pool.run_until(host.tools.execute(request)).unwrap();
    host.registry
        .lock()
        .unwrap()
        .remove(&host.terminal)
        .unwrap();
    host.pump();
    assert!(host.executor.requests().is_empty());
    assert_eq!(
        host.journal.observations()[0].1,
        Some(BackgroundCommandOutcome::Unknown)
    );
}

#[test]
fn cancellation_keeps_unknown_fence_and_foreign_terminal_cannot_cancel() {
    let mut host = Harness::new(Charset::Utf8);
    let sink = host.executor.queue_stream("long");
    let initial = host.call(1, host.exec("long")).unwrap();
    let command = initial.output().value()["command_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let other = host.call(
        2,
        TerminalCall::CancelCommand(CancelCommand {
            terminal_id: "other-terminal".into(),
            command_id: command.clone(),
        }),
    );
    assert!(matches!(other, Err(ToolExecutionError::StaleSession)));
    assert!(!sink.is_closed());
    let output = host
        .call(
            3,
            TerminalCall::CancelCommand(CancelCommand {
                terminal_id: host.terminal.clone(),
                command_id: command,
            }),
        )
        .unwrap();
    assert_eq!(output.output().value()["state"], "cancelled");
    assert_eq!(output.effect_status(), &ToolEffectStatus::Unknown);
    assert!(sink.is_closed());
    assert_eq!(
        host.journal.observations()[0].1,
        Some(BackgroundCommandOutcome::Unknown)
    );
}

#[test]
fn continuously_drains_beyond_prefix_and_observes_real_stderr_exit() {
    let mut host = Harness::new(Charset::Utf8);
    let sink = host.executor.queue_stream("stream");
    let initial = host.call(1, host.exec("stream")).unwrap();
    let command = initial.output().value()["command_id"]
        .as_str()
        .unwrap()
        .to_owned();
    host.pool
        .spawner()
        .spawn_local(async move {
            for _ in 0..40 {
                assert!(sink.send(vec![b'x'; 4096]).await);
            }
            sink.finish(ExecExit {
                status: Some(3),
                stderr: "bounded stderr".into(),
            });
        })
        .unwrap();
    host.pump();
    let read = host
        .call(
            2,
            TerminalCall::ReadCommand(ReadCommand {
                terminal_id: host.terminal.clone(),
                command_id: command,
                yield_ms: None,
            }),
        )
        .unwrap();
    let output = read.output().value();
    assert_eq!(output["stdout"].as_str().unwrap().len(), 64 * 1024);
    assert_eq!(output["total_bytes"], 40 * 4096);
    assert_eq!(output["truncated"], true);
    assert_eq!(output["exit_status"], 3);
    assert_eq!(output["stderr"], "bounded stderr");
}

#[test]
fn admission_limits_four_active_but_finished_churn_does_not_pin_registry() {
    let mut host = Harness::new(Charset::Utf8);
    let mut sinks = Vec::new();
    let mut commands = Vec::new();
    for id in 1..=4 {
        sinks.push(host.executor.queue_stream("long"));
        let output = host.call(id, host.exec("long")).unwrap();
        commands.push(
            output.output().value()["command_id"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert!(matches!(
        host.call(5, host.exec("fifth")),
        Err(ToolExecutionError::Capacity)
    ));
    for sink in &sinks {
        sink.finish(ExecExit {
            status: Some(0),
            stderr: String::new(),
        });
    }
    drop(sinks);
    host.pump();
    host.executor
        .respond("fast", Ok(ExecResponse::text("ok", 0, "")));
    for id in 6..=28 {
        host.call(id, host.exec("fast")).unwrap();
    }
    let oldest = host.call(
        29,
        TerminalCall::ReadCommand(ReadCommand {
            terminal_id: host.terminal.clone(),
            command_id: commands.remove(0),
            yield_ms: None,
        }),
    );
    assert!(
        oldest.is_err(),
        "the original bounded retained history evicts completed work"
    );
    assert_eq!(host.executor.requests().len(), 27);
    assert_eq!(host.journal.observations().len(), 27);
}

#[test]
fn deadline_cancels_observation_without_claiming_remote_exit() {
    let mut host = Harness::new(Charset::Utf8);
    let sink = host.executor.queue_stream("long");
    let mut call = host.exec("long");
    let TerminalCall::ExecCommand(args) = &mut call else {
        unreachable!()
    };
    args.timeout_ms = Some(10);
    let initial = host.call(1, call).unwrap();
    let command = initial.output().value()["command_id"]
        .as_str()
        .unwrap()
        .to_owned();
    host.tick(10);
    let output = host
        .call(
            2,
            TerminalCall::ReadCommand(ReadCommand {
                terminal_id: host.terminal.clone(),
                command_id: command,
                yield_ms: None,
            }),
        )
        .unwrap();
    assert_eq!(output.output().value()["state"], "timed_out");
    assert_eq!(
        output.output().value()["exit_status"],
        serde_json::Value::Null
    );
    assert_eq!(
        host.journal.observations()[0].1,
        Some(BackgroundCommandOutcome::Unknown)
    );
    assert!(sink.is_closed());
}

#[test]
fn dropping_a_read_waiter_never_cancels_its_background_command() {
    let mut host = Harness::new(Charset::Utf8);
    let sink = host.executor.queue_stream("long");
    let initial = host.call(1, host.exec("long")).unwrap();
    let command = initial.output().value()["command_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let read = host.request(
        2,
        TerminalCall::ReadCommand(ReadCommand {
            terminal_id: host.terminal.clone(),
            command_id: command.clone(),
            yield_ms: Some(1000),
        }),
    );
    assert!(futures::FutureExt::now_or_never(host.tools.execute(read)).is_none());
    assert!(!sink.is_closed());
    sink.finish(ExecExit {
        status: Some(0),
        stderr: String::new(),
    });
    drop(sink);
    host.pump();
    let result = host
        .call(
            3,
            TerminalCall::ReadCommand(ReadCommand {
                terminal_id: host.terminal.clone(),
                command_id: command,
                yield_ms: None,
            }),
        )
        .unwrap();
    assert_eq!(result.output().value()["exit_status"], 0);
    assert_eq!(
        host.journal.observations()[0].1,
        Some(BackgroundCommandOutcome::ObservedExit(0))
    );
}

#[test]
fn revoked_during_running_output_closes_stream_and_retains_unknown() {
    let mut host = Harness::new(Charset::Utf8);
    let sink = host.executor.queue_stream("long");
    host.call(1, host.exec("long")).unwrap();
    host.checks.refuse(ToolExecutionError::Revoked);
    host.tick(50);
    assert!(sink.is_closed());
    assert_eq!(
        host.journal.observations()[0].1,
        Some(BackgroundCommandOutcome::Unknown)
    );
    assert!(matches!(
        host.task_errors.lock().unwrap().as_slice(),
        [ToolExecutionError::Revoked]
    ));
}
