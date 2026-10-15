use super::*;
use std::error::Error;

fn capture(cx: &mut TestAppContext) -> BoxFuture<'static, Result<(), nocterm_store::StoreError>> {
    cx.update(|cx| Runtime::global(cx).update(cx, |runtime, cx| runtime.capture_shutdown(cx)))
}

#[gpui_kit::test]
fn captured_cleanup_is_lazy_and_retains_the_original_agent_error(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.commands.shutdown_failure.store(true, Ordering::SeqCst);
    let work = capture(cx);
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    let error = cx.foreground_executor().block_test(work).unwrap_err();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
    let mut source: &(dyn Error + 'static) = &error;
    let original = loop {
        if let Some(agent) = source.downcast_ref::<AgentError>() {
            break agent;
        }
        source = source
            .source()
            .expect("capture lost the original AgentError");
    };
    assert!(matches!(original, AgentError::Io(message) if message == "Process tree still alive"));
    assert!(original.source().is_none());
}

#[gpui_kit::test]
fn capture_retries_failed_ordinary_cleanup_once(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    f.commands.shutdown_failure.store(true, Ordering::SeqCst);
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
    f.commands.shutdown_failure.store(false, Ordering::SeqCst);
    let work = capture(cx);
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
    cx.foreground_executor().block_test(work).unwrap();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 2);
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 2);
}

#[gpui_kit::test]
fn capture_bypasses_close_acknowledgement_and_late_close_joins_cleanup(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let (close, gate) = oneshot::channel();
    *f.commands.close_gate.lock().unwrap() = Some(gate);
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    let work = capture(cx);
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    cx.foreground_executor().block_test(work).unwrap();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
    close.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn capture_joins_successful_ordinary_cleanup(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
    let work = capture(cx);
    cx.foreground_executor().block_test(work).unwrap();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn capture_deduplicates_the_same_native_command_handle(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    first.update(cx, |thread, cx| {
        thread.rename(Some("Retained process owner".into()), cx)
    });
    new_chat(&f, cx);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    let work = capture(cx);
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    cx.foreground_executor().block_test(work).unwrap();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn capture_keeps_distinct_command_handles_with_shared_counters_independent(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    f.connector.use_distinct_process_handles();
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    first.update(cx, |thread, cx| {
        thread.rename(Some("Retained process owner".into()), cx)
    });
    new_chat(&f, cx);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    let work = capture(cx);
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 0);
    cx.foreground_executor().block_test(work).unwrap();
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 2);
}
