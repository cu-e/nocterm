//! Account refresh effects through public Runtime methods and the shared source fake.
use super::*;
use nocterm_ai::{
    testing::ScriptedUsageSource,
    usage::{RateLimit, UsageReadError, Window},
};

fn with_source(cx: &mut TestAppContext, source: Arc<ScriptedUsageSource>) -> Entity<Runtime> {
    let repository = Arc::new(repository());
    let storage = AgentStorage::prepared(repository.clone(), repository, Ok(Default::default()));
    runtime_with_usage_source(
        cx,
        Arc::new(ScriptedConnector::default()),
        storage,
        std::path::Path::new("/unused"),
        source,
        Arc::new(nocterm_ai::testing::ScriptedWorkdirResolver::accept_requests()),
    )
}
fn limit(used: f64) -> RateLimit {
    RateLimit {
        window: Window::Weekly,
        used: Some(used),
        resets_at: None,
        reached: false,
    }
}

#[gpui::test]
fn account_refresh_coalesces_and_retains_last_limits_on_missing_report_or_failure(
    cx: &mut TestAppContext,
) {
    let source = Arc::new(ScriptedUsageSource::default());
    let answer = source.pending();
    let runtime = with_source(cx, source.clone());
    runtime.update(cx, |runtime, cx| {
        runtime.refresh_limits("codex", cx);
        runtime.refresh_limits("codex", cx);
    });
    cx.run_until_parked();
    assert_eq!(source.requests().len(), 1);
    assert_eq!(source.requests()[0].id, "codex");
    answer.send(Ok(Some(vec![limit(0.38)]))).unwrap();
    cx.run_until_parked();
    runtime.read_with(cx, |runtime, _| {
        assert_eq!(
            runtime
                .limits("codex")
                .unwrap()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![limit(0.38)]
        )
    });
    source.push(Ok(None));
    runtime.update(cx, |runtime, cx| runtime.refresh_limits("codex", cx));
    cx.run_until_parked();
    source.push(Err(UsageReadError::read(
        "/unused/report",
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    )));
    runtime.update(cx, |runtime, cx| runtime.refresh_limits("codex", cx));
    cx.run_until_parked();
    runtime.read_with(cx, |runtime, _| {
        assert_eq!(
            runtime
                .limits("codex")
                .unwrap()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![limit(0.38)]
        )
    });
    assert_eq!(source.requests().len(), 3);
}

#[gpui::test]
fn stopped_runtime_rejects_late_usage_delivery_and_unknown_agent_has_no_request(
    cx: &mut TestAppContext,
) {
    let source = Arc::new(ScriptedUsageSource::default());
    let answer = source.pending();
    let runtime = with_source(cx, source.clone());
    runtime.update(cx, |runtime, cx| {
        runtime.refresh_limits("unknown-agent", cx)
    });
    cx.run_until_parked();
    assert!(source.requests().is_empty());
    runtime.update(cx, |runtime, cx| runtime.refresh_limits("codex", cx));
    cx.run_until_parked();
    assert_eq!(source.requests().len(), 1);
    let _drain = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    cx.run_until_parked();
    let _ = answer.send(Ok(Some(vec![limit(0.99)])));
    cx.run_until_parked();
    runtime.update(cx, |runtime, cx| {
        assert!(runtime.limits("codex").is_none());
        runtime.refresh_limits("codex", cx);
    });
    cx.run_until_parked();
    assert_eq!(source.requests().len(), 1);
}
