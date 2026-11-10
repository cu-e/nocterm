use futures::{
    FutureExt as _,
    future::{BoxFuture, pending, ready},
};
use nocterm_agent_tools::JobsFactory;
use nocterm_session::{
    ErrorTail, ExecError, ExecRequest, HostExec,
    command_jobs::{CommandJobs, CommandJobsFactory, CommandState, JobError},
    testing::FakeExec,
};
use std::sync::Arc;

fn start(
    jobs: &mut dyn CommandJobs,
    executor: Arc<dyn HostExec>,
    terminal: &str,
) -> (String, BoxFuture<'static, ()>) {
    jobs.start(
        terminal.into(),
        executor,
        ExecRequest::new("program"),
        pending().boxed(),
    )
    .unwrap()
    .into_run()
}

#[test]
fn four_active_commands_refuse_fifth_and_terminal_or_chat_foreign_ids() {
    let mut jobs = JobsFactory.create();
    assert!(!jobs.has_active());
    let executor = Arc::new(FakeExec::default());
    let mut runs = Vec::new();
    for _ in 0..4 {
        runs.push(start(&mut *jobs, executor.clone(), "original"));
    }
    assert!(
        jobs.has_active(),
        "unpolled starts must keep their owner active"
    );
    assert!(matches!(
        jobs.start(
            "original".into(),
            executor.clone(),
            ExecRequest::new("no"),
            pending().boxed()
        ),
        Err(JobError::Busy)
    ));
    for (id, terminal) in [
        (runs[0].0.as_str(), "foreign"),
        ("c99", "original"),
        ("bad", "original"),
    ] {
        assert!(matches!(
            jobs.snapshot(id, terminal),
            Err(JobError::Unknown)
        ));
    }
    assert!(matches!(
        JobsFactory.create().snapshot(&runs[0].0, "original"),
        Err(JobError::Unknown)
    ));
    assert!(executor.requests().is_empty());
}

#[test]
fn cancellation_or_elapsed_deadline_before_poll_never_executes() {
    futures::executor::block_on(async {
        for cancelled in [true, false] {
            let mut jobs = JobsFactory.create();
            let executor = Arc::new(FakeExec::default());
            let (id, run) = jobs
                .start(
                    "t".into(),
                    executor.clone(),
                    ExecRequest::new("no"),
                    ready(()).boxed(),
                )
                .unwrap()
                .into_run();
            if cancelled {
                jobs.cancel(&id, "t").unwrap();
            }
            run.await;
            assert!(executor.requests().is_empty());
            assert_eq!(
                jobs.snapshot(&id, "t").unwrap().state(),
                if cancelled {
                    CommandState::Cancelled
                } else {
                    CommandState::TimedOut
                }
            );
        }
    });
}

#[test]
fn output_keeps_prefix_while_draining_every_chunk_and_preserving_stderr_exit_and_arguments() {
    futures::executor::block_on(async {
        let mut jobs = JobsFactory.create();
        let executor = Arc::new(FakeExec::default());
        let sink = executor.queue_stream("program");
        let request = ExecRequest::new("program")
            .args(["a b", "$(literal)", "", "'quote'"])
            .stdin(b"first\nsecond\n".to_vec());
        let (id, run) = jobs
            .start(
                "t".into(),
                executor.clone(),
                request.clone(),
                pending().boxed(),
            )
            .unwrap()
            .into_run();
        let produce = async move {
            for _ in 0..40 {
                assert!(sink.send(vec![b'x'; 4096]).await);
            }
            let mut errors = ErrorTail::default();
            errors.push(&vec![b'e'; 8192]);
            errors.push(b"\nlast error");
            sink.finish(errors.exit(Some(7)));
        };
        futures::join!(run, produce);
        let snapshot = jobs.snapshot(&id, "t").unwrap();
        assert_eq!(snapshot.state(), CommandState::Exited);
        assert_eq!(snapshot.stdout(), vec![b'x'; 65536]);
        assert_eq!(snapshot.total_bytes(), 40 * 4096);
        assert_eq!(snapshot.exit().unwrap().status, Some(7));
        assert_eq!(snapshot.exit().unwrap().stderr.len(), 4096);
        assert!(snapshot.exit().unwrap().stderr.ends_with("last error"));
        assert_eq!(executor.requests(), vec![request]);
    });
}

#[test]
fn silent_exit_finishes_without_output() {
    futures::executor::block_on(async {
        let mut jobs = JobsFactory.create();
        let executor = Arc::new(FakeExec::new(vec![("program", Ok(("", 0, "")))]));
        let (id, run) = start(&mut *jobs, executor, "t");
        run.await;
        let snapshot = jobs.snapshot(&id, "t").unwrap();
        assert_eq!(snapshot.state(), CommandState::Exited);
        assert_eq!(snapshot.exit().unwrap().status, Some(0));
        assert!(snapshot.stdout().is_empty());
    });
}

#[test]
fn cancelling_running_command_drops_output_and_retains_unknown_exit() {
    futures::executor::block_on(async {
        let mut jobs = JobsFactory.create();
        let executor = Arc::new(FakeExec::default());
        let sink = executor.queue_stream("program");
        let (id, mut run) = start(&mut *jobs, executor.clone(), "t");
        assert!(run.as_mut().now_or_never().is_none());
        assert_eq!(executor.requests().len(), 1);
        assert!(sink.send(b"running".to_vec()).await);
        assert!(run.as_mut().now_or_never().is_none());
        jobs.cancel(&id, "t").unwrap();
        run.await;
        sink.closed().await;
        let snapshot = jobs.snapshot(&id, "t").unwrap();
        assert_eq!(snapshot.state(), CommandState::Cancelled);
        assert!(snapshot.exit().is_none());
        assert!(sink.is_closed());
    });
}

#[test]
fn retained_commands_evict_oldest_completed_result_and_allow_new_admission() {
    let mut jobs = JobsFactory.create();
    let executor = Arc::new(FakeExec::default());
    let mut ids = Vec::new();
    for _ in 0..17 {
        let (id, _run) = start(&mut *jobs, executor.clone(), "t");
        jobs.cancel(&id, "t").unwrap();
        ids.push(id);
    }
    assert!(matches!(
        jobs.snapshot(&ids[0], "t"),
        Err(JobError::Unknown)
    ));
    for id in &ids[1..] {
        assert_eq!(
            jobs.snapshot(id, "t").unwrap().state(),
            CommandState::Cancelled
        );
    }
    assert!(!jobs.has_active());
}

#[test]
fn completed_result_churn_never_evicts_an_older_running_command() {
    futures::executor::block_on(async {
        let mut jobs = JobsFactory.create();
        let executor = Arc::new(FakeExec::default());
        let sink = executor.queue_stream("program");
        let (running, mut run) = start(&mut *jobs, executor.clone(), "live");
        assert!(run.as_mut().now_or_never().is_none());
        let mut completed = Vec::new();
        for _ in 0..20 {
            let (id, _run) = start(&mut *jobs, executor.clone(), "short");
            jobs.cancel(&id, "short").unwrap();
            completed.push(id);
        }
        assert!(jobs.has_active());
        assert_eq!(
            jobs.snapshot(&running, "live").unwrap().state(),
            CommandState::Running
        );
        assert!(matches!(
            jobs.snapshot(&completed[0], "short"),
            Err(JobError::Unknown)
        ));
        assert!(!sink.is_closed());
        assert!(matches!(
            jobs.cancel(&running, "short"),
            Err(JobError::Unknown)
        ));
        assert!(jobs.snapshot(&running, "live").unwrap().active());
        jobs.cancel(&running, "live").unwrap();
        assert!(!jobs.has_active());
        run.await;
        sink.closed().await;
    });
}

#[test]
fn replacement_executor_revokes_only_its_original_terminal() {
    let mut jobs = JobsFactory.create();
    let original: Arc<dyn HostExec> = Arc::new(FakeExec::default());
    let replacement: Arc<dyn HostExec> = Arc::new(FakeExec::default());
    let (first, _run) = start(&mut *jobs, original.clone(), "first");
    let (other, _run) = start(&mut *jobs, original.clone(), "other");
    jobs.revoke_replaced(&[
        ("first".into(), Some(replacement)),
        ("other".into(), Some(original)),
    ]);
    assert_eq!(
        jobs.snapshot(&first, "first").unwrap().state(),
        CommandState::Cancelled
    );
    assert!(jobs.snapshot(&other, "other").unwrap().active());
    jobs.cancel_all();
    assert!(!jobs.snapshot(&other, "other").unwrap().active());
}

#[test]
fn startup_failure_preserves_the_typed_transport_error() {
    futures::executor::block_on(async {
        let mut jobs = JobsFactory.create();
        let executor = Arc::new(FakeExec::new(vec![(
            "program",
            Err(ExecError::Disconnected),
        )]));
        let (id, run) = start(&mut *jobs, executor, "t");
        run.await;
        let snapshot = jobs.snapshot(&id, "t").unwrap();
        assert_eq!(snapshot.state(), CommandState::Failed);
        assert!(matches!(snapshot.error(), Some(ExecError::Disconnected)));
        assert!(snapshot.exit().is_none());
    });
}

#[test]
fn absolute_deadline_covers_suspended_startup_and_silent_running_output() {
    futures::executor::block_on(async {
        for suspended in [true, false] {
            let mut jobs = JobsFactory.create();
            let executor = Arc::new(FakeExec::default());
            let sink = executor.queue_stream("program");
            if suspended {
                executor.suspend(nocterm_session::testing::Gate::pending());
            }
            let (expire, deadline) = async_channel::bounded::<()>(1);
            let deadline = async move {
                let _ = deadline.recv().await;
            }
            .boxed();
            let (id, mut run) = jobs
                .start(
                    "t".into(),
                    executor.clone(),
                    ExecRequest::new("program"),
                    deadline,
                )
                .unwrap()
                .into_run();
            assert!(run.as_mut().now_or_never().is_none());
            assert_eq!(executor.requests().len(), 1);
            assert_eq!(
                jobs.snapshot(&id, "t").unwrap().state(),
                if suspended {
                    CommandState::Starting
                } else {
                    CommandState::Running
                }
            );
            expire.close();
            run.await;
            assert_eq!(
                jobs.snapshot(&id, "t").unwrap().state(),
                CommandState::TimedOut
            );
            assert!(jobs.snapshot(&id, "t").unwrap().exit().is_none());
            sink.closed().await;
        }
    });
}

#[test]
fn dropping_the_registry_cancels_a_runner_even_when_its_read_waiter_is_gone() {
    futures::executor::block_on(async {
        let mut jobs = JobsFactory.create();
        let executor = Arc::new(FakeExec::default());
        let sink = executor.queue_stream("program");
        let (id, mut run) = start(&mut *jobs, executor, "t");
        assert!(run.as_mut().now_or_never().is_none());
        let observation = jobs.snapshot(&id, "t").unwrap();
        drop(observation);
        assert!(!sink.is_closed());
        drop(jobs);
        run.await;
        sink.closed().await;
        assert!(sink.is_closed());
    });
}
