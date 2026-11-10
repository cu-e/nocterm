use super::*;
use crate::{
    CloseReason, Event, ExecOutput,
    testing::{Gate, ScriptedConnectionSource},
};
use futures::executor::block_on;
#[test]
fn revoked_pending_operation_never_resumes_its_effect() {
    let source = ScriptedConnectionSource::default();
    let gate = Gate::pending();
    let resumed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let marker = resumed.clone();
    let waiting = gate.clone();
    let mut future = source.source().guard(async move {
        waiting.wait().await;
        marker.store(true, std::sync::atomic::Ordering::Release);
        Ok::<_, SessionError>(())
    });
    assert!(future.as_mut().now_or_never().is_none());
    source.revoke();
    gate.release();
    assert!(matches!(
        block_on(future),
        Err(SessionError::Source(ConnectionSourceError::Revoked))
    ));
    assert!(!resumed.load(std::sync::atomic::Ordering::Acquire));
}
#[test]
fn revoked_session_rejects_input_and_suppresses_previously_buffered_output() {
    let source = ScriptedConnectionSource::default();
    let (session, driver) = crate::channel_with_source(None, source.source());
    block_on(driver.emit(Event::Output(b"A private output".to_vec())));
    source.revoke();
    assert!(!session.input(b"for B".to_vec()));
    assert!(matches!(
        block_on(session.next_event()),
        Some(Event::Closed(CloseReason::Failed(SessionError::Source(
            ConnectionSourceError::Revoked
        ))))
    ));
    assert!(block_on(session.next_event()).is_none());
    assert!(matches!(
        block_on(driver.next_command()),
        Some(crate::Command::Close)
    ));
}
#[test]
fn retained_exec_output_after_revocation_has_unknown_exit_and_no_buffered_data() {
    block_on(async {
        let source = ScriptedConnectionSource::default();
        let (sink, output) = ExecOutput::channel_with_source(source.source());
        sink.send(b"A private output".to_vec()).await;
        sink.finish(crate::ExecExit {
            status: Some(0),
            stderr: String::new(),
        });
        source.revoke();
        let collected = output.collect(100).await;
        assert!(collected.stdout.is_empty());
        assert_eq!(collected.exit.status, None);
        assert!(sink.is_closed());
    });
}
