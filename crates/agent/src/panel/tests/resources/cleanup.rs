use super::*;
use std::error::Error;

fn capture(
    f: &Fixture,
    cx: &mut TestAppContext,
) -> BoxFuture<'static, Result<(), nocterm_store::StoreError>> {
    cx.update(|cx| f.installation.installed.runtime().capture_shutdown(cx))
}

#[gpui_kit::test]
fn captured_cleanup_is_lazy_and_retains_the_original_agent_error(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.script.fail_shutdown();
    let work = capture(&f, cx);
    assert_eq!(f.script.shutdowns(), 0);
    let error = cx.foreground_executor().block_test(work).unwrap_err();
    assert_eq!(f.script.shutdowns(), 1);
    let mut source: &(dyn Error + 'static) = &error;
    let original = loop {
        if let Some(agent) = source.downcast_ref::<nocterm_ai::testing::ScriptedFailure>() {
            break agent;
        }
        source = source
            .source()
            .expect("capture lost the original agent cleanup error");
    };
    assert!(matches!(
        original,
        nocterm_ai::testing::ScriptedFailure::Cleanup
    ));
    assert!(original.source().is_none());
}

#[gpui_kit::test]
fn capture_retries_failed_ordinary_cleanup_once(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    f.script.fail_shutdown();
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    assert_eq!(f.script.shutdowns(), 1);
    f.script.repair_shutdown();
    let work = capture(&f, cx);
    assert_eq!(f.script.shutdowns(), 1);
    cx.foreground_executor().block_test(work).unwrap();
    assert_eq!(f.script.shutdowns(), 2);
    cx.run_until_parked();
    assert_eq!(f.script.shutdowns(), 2);
}

#[gpui_kit::test]
fn capture_bypasses_close_acknowledgement_and_late_close_joins_cleanup(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let close = f.script.gate_close();
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    assert_eq!(f.script.shutdowns(), 0);
    let work = capture(&f, cx);
    assert_eq!(f.script.shutdowns(), 0);
    cx.foreground_executor().block_test(work).unwrap();
    assert_eq!(f.script.shutdowns(), 1);
    close.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(f.script.shutdowns(), 1);
}

#[gpui_kit::test]
fn capture_joins_successful_ordinary_cleanup(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    assert_eq!(f.script.shutdowns(), 1);
    let work = capture(&f, cx);
    cx.foreground_executor().block_test(work).unwrap();
    assert_eq!(f.script.shutdowns(), 1);
}

#[gpui_kit::test]
fn capture_keeps_distinct_command_handles_with_shared_counters_independent(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    first.update(cx, |thread, cx| {
        thread.rename(Some("Retained process owner".into()), cx)
    });
    new_chat(&f, cx);
    assert_eq!(f.connector.connect_count(), 2);
    let work = capture(&f, cx);
    assert_eq!(f.script.shutdowns(), 0);
    cx.foreground_executor().block_test(work).unwrap();
    assert_eq!(f.script.shutdowns(), 2);
}
