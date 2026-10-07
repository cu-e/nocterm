use super::*;
use nocterm_session::{ExecError, ExecFuture, ExecOutput};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Executor {
    calls: AtomicUsize,
    output: Mutex<Option<ExecOutput>>,
}
impl HostExec for Executor {
    fn exec(&self, _: ExecRequest) -> ExecFuture<ExecOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let output = self.output.lock().unwrap().take();
        async move { output.ok_or(ExecError::Unsupported) }.boxed()
    }
}
fn executor(output: Option<ExecOutput>) -> Arc<Executor> {
    Arc::new(Executor {
        calls: AtomicUsize::new(0),
        output: Mutex::new(output),
    })
}
#[test]
fn cancelled_or_expired_before_first_poll_never_starts_a_program() {
    futures::executor::block_on(async {
        for cancelled in [true, false] {
            let executor = executor(None);
            let mut registry = Jobs::default();
            let (id, state, cancel) = registry.insert("t1".into(), executor.clone()).unwrap();
            if cancelled {
                registry.get(&id, "t1").unwrap().cancel(State::Cancelled);
            }
            run(
                executor.clone(),
                ExecRequest::new("do-not-run"),
                state,
                cancel,
                futures::future::ready(()),
            )
            .await;
            assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
            assert_eq!(
                registry.get(&id, "t1").unwrap().snapshot().state,
                if cancelled {
                    State::Cancelled
                } else {
                    State::TimedOut
                }
            );
        }
    });
}
#[test]
fn output_is_drained_past_retention_and_preserves_actual_exit() {
    futures::executor::block_on(async {
        let (sink, output) = ExecOutput::channel();
        let executor = executor(Some(output));
        let mut registry = Jobs::default();
        let (id, state, cancel) = registry.insert("t1".into(), executor.clone()).unwrap();
        let produce = async move {
            for _ in 0..40 {
                assert!(sink.send(vec![b'x'; 4096]).await);
            }
            sink.finish(ExecExit {
                status: Some(7),
                stderr: "failure".into(),
            });
        };
        futures::join!(
            run(
                executor,
                ExecRequest::new("test"),
                state,
                cancel,
                futures::future::pending()
            ),
            produce
        );
        let state = registry.get(&id, "t1").unwrap().snapshot();
        assert_eq!(state.state, State::Exited);
        assert_eq!(state.stdout.len(), MAX_STDOUT);
        assert_eq!(state.total_bytes, 40 * 4096);
        assert_eq!(state.exit.unwrap().status, Some(7));
    });
}
#[test]
fn registry_limits_active_work_retains_finished_jobs_and_checks_ownership() {
    let executor = executor(None);
    let mut registry = Jobs::default();
    let first = registry.insert("t1".into(), executor.clone()).unwrap().0;
    for _ in 1..MAX_ACTIVE {
        registry.insert("t1".into(), executor.clone()).unwrap();
    }
    assert!(registry.insert("t1".into(), executor.clone()).is_err());
    assert!(registry.get(&first, "t2").is_err());
    assert!(Jobs::default().get(&first, "t1").is_err());
    registry.cancel_all();
    for _ in 0..MAX_RETAINED {
        let (id, _, _) = registry.insert("t1".into(), executor.clone()).unwrap();
        registry.get(&id, "t1").unwrap().cancel(State::Cancelled);
    }
    assert_eq!(registry.jobs.len(), MAX_RETAINED);
    assert!(registry.get(&first, "t1").is_err());
}
#[test]
fn silence_finishes_and_cancellation_drops_transport_output() {
    futures::executor::block_on(async {
        let (sink, output) = ExecOutput::channel();
        sink.finish(ExecExit {
            status: Some(0),
            stderr: String::new(),
        });
        drop(sink);
        let executor = executor(Some(output));
        let mut registry = Jobs::default();
        let (id, state, cancel) = registry.insert("t1".into(), executor.clone()).unwrap();
        run(
            executor,
            ExecRequest::new("true"),
            state,
            cancel,
            futures::future::pending(),
        )
        .await;
        assert_eq!(
            registry
                .get(&id, "t1")
                .unwrap()
                .snapshot()
                .exit
                .unwrap()
                .status,
            Some(0)
        );
    });
}

#[test]
fn cancelling_running_program_drops_output_and_keeps_exit_unknown() {
    futures::executor::block_on(async {
        let (sink, output) = ExecOutput::channel();
        let executor = executor(Some(output));
        let mut registry = Jobs::default();
        let (id, state, cancel) = registry.insert("t1".into(), executor.clone()).unwrap();
        let stop = async {
            assert!(sink.send(b"running".to_vec()).await);
            registry.get(&id, "t1").unwrap().cancel(State::Cancelled);
            sink.closed().await;
        };
        futures::join!(
            run(
                executor.clone(),
                ExecRequest::new("test"),
                state,
                cancel,
                futures::future::pending()
            ),
            stop
        );
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
        let state = registry.get(&id, "t1").unwrap().snapshot();
        assert_eq!(state.state, State::Cancelled);
        assert!(state.exit.is_none());
    });
}
