use crate::testing::{ExecResponse, FakeExec, Gate};
use crate::{ExecError, ExecExit, ExecRequest, HostExec};
use futures::{FutureExt as _, executor::block_on, join};

#[test]
fn response_order_errors_and_request_recording_match_the_execution_port() {
    let exec = FakeExec::default();
    exec.respond(
        "run",
        Ok(ExecResponse::chunks(
            vec![b"one".to_vec(), b"two".to_vec()],
            ExecExit {
                status: Some(7),
                stderr: "failed".into(),
            },
        )
        .unwrap()),
    );
    exec.respond("offline", Err(ExecError::Disconnected));
    block_on(async {
        let result = exec
            .exec(ExecRequest::new("run").arg("arg"))
            .await
            .unwrap()
            .collect(100)
            .await;
        assert_eq!(result.stdout, b"onetwo");
        assert_eq!(result.exit.status, Some(7));
        assert_eq!(result.exit.stderr, "failed");
        assert!(matches!(
            exec.exec(ExecRequest::new("offline")).await,
            Err(ExecError::Disconnected)
        ));
        assert!(matches!(
            exec.exec(ExecRequest::new("missing")).await,
            Err(ExecError::NotFound(_))
        ));
    });
    assert_eq!(exec.ran(), ["run arg", "offline", "missing"]);
}

#[test]
fn externally_driven_output_uses_backpressure_and_notices_cancellation() {
    let exec = FakeExec::default();
    let sink = exec.queue_stream("stream");
    block_on(async {
        let mut output = exec.exec(ExecRequest::new("stream")).await.unwrap();
        let ((), bytes) = join!(
            async {
                for i in 0..100 {
                    assert!(sink.send(vec![i]).await);
                }
                sink.finish(ExecExit {
                    status: Some(0),
                    stderr: String::new(),
                });
                drop(sink);
            },
            async {
                let mut bytes = Vec::new();
                while let Some(chunk) = output.next().await {
                    bytes.extend(chunk);
                }
                assert!(output.exit().await.success());
                bytes
            }
        );
        assert_eq!(bytes, (0..100).collect::<Vec<_>>());
    });
    let sink = exec.queue_stream("cancel");
    let output = block_on(exec.exec(ExecRequest::new("cancel"))).unwrap();
    drop(output);
    block_on(sink.closed());
    assert!(!block_on(sink.send(vec![1])));
}

#[test]
fn start_can_be_suspended_without_a_clock_and_canned_output_cannot_overfill_the_channel() {
    let exec = FakeExec::new(vec![("run", Ok(("done", 0, "")))]);
    let gate = Gate::pending();
    exec.suspend(gate.clone());
    let mut start = exec.exec(ExecRequest::new("run"));
    assert!(start.as_mut().now_or_never().is_none());
    assert_eq!(exec.ran(), ["run"]);
    gate.release();
    assert_eq!(
        block_on(async { start.await.unwrap().collect(100).await.text() }),
        "done"
    );
    assert!(matches!(
        ExecResponse::chunks(vec![vec![0]; 17], ExecExit::default()),
        Err(ExecError::Unsupported)
    ));
}

#[test]
fn external_program_requests_are_recorded_on_poll_and_cancellation_keeps_the_record() {
    use crate::{ProgramOpenRequest, ProgramOpener};
    let exec = FakeExec::default();
    let request =
        ProgramOpenRequest::new("editor".into(), vec!["literal $(value)".into()], None).unwrap();
    drop(exec.open(request.clone()));
    assert!(exec.opened_requests().is_empty());
    let gate = Gate::pending();
    exec.suspend_open(gate.clone());
    let mut launch = exec.open(request.clone());
    assert!(launch.as_mut().now_or_never().is_none());
    assert_eq!(exec.opened_requests(), [request]);
    drop(launch);
    gate.release();
    assert_eq!(exec.opened_requests().len(), 1);
    assert!(exec.requests().is_empty());
}

#[test]
fn external_launch_errors_keep_typed_native_sources_and_are_consumed_once() {
    use crate::{ProgramOpenError, ProgramOpenRequest, ProgramOpener};
    let exec = FakeExec::default();
    exec.queue_open(Err(ProgramOpenError::Start {
        program: "editor".into(),
        source: std::io::ErrorKind::PermissionDenied.into(),
    }))
    .unwrap();
    let request = ProgramOpenRequest::new("editor".into(), vec![], None).unwrap();
    assert!(
        matches!(block_on(exec.open(request.clone())), Err(ProgramOpenError::Start { source, .. }) if source.kind() == std::io::ErrorKind::PermissionDenied)
    );
    block_on(exec.open(request)).unwrap();
}
