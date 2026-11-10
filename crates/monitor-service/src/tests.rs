//! Public captured-watch lifecycle and shared work, using the session execution fake.
use crate::{MonitorWatches, WatchPlan, WatchStatus};
use gpui::{AppContext as _, TestAppContext};
use nocterm_monitor::{MetricSet, MonitorMetric, Platform};
use nocterm_session::{ExecExit, HostExec, testing::FakeExec};
use std::{sync::Arc, time::Duration};

fn plan(metric: MonitorMetric, seconds: u64) -> WatchPlan {
    WatchPlan {
        metrics: [metric].into_iter().collect(),
        interval: Duration::from_secs(seconds),
    }
}

#[gpui::test]
fn consumers_share_one_captured_watch_and_the_last_one_stops_it(cx: &mut TestAppContext) {
    let exec = Arc::new(FakeExec::default());
    let sink = exec.queue_stream("sh");
    let host: Arc<dyn HostExec> = exec.clone();
    let registry = MonitorWatches::default();
    let (watch, again, first, second) = cx.update(|cx| {
        let first = cx.new(|_| ());
        let second = cx.new(|_| ());
        let watch = registry.watch(host.clone(), false, Some(Platform::Linux), cx);
        let again = registry.watch(host, false, Some(Platform::Linux), cx);
        watch.update(cx, |watch, cx| {
            watch.subscribe(first.entity_id(), plan(MonitorMetric::Memory, 30), cx)
        });
        again.update(cx, |watch, cx| {
            watch.subscribe(second.entity_id(), plan(MonitorMetric::Memory, 30), cx)
        });
        (watch, again, first, second)
    });
    cx.run_until_parked();
    assert_eq!(watch.entity_id(), again.entity_id());
    assert_eq!(exec.requests().len(), 1);
    assert!(!sink.is_closed());
    watch.update(cx, |watch, cx| watch.unsubscribe(first.entity_id(), cx));
    cx.run_until_parked();
    assert!(!sink.is_closed());
    again.update(cx, |watch, cx| watch.unsubscribe(second.entity_id(), cx));
    cx.run_until_parked();
    assert!(sink.is_closed());
    watch.read_with(cx, |watch, _| assert_eq!(watch.status(), &WatchStatus::Off));
}

#[gpui::test]
fn details_expand_one_collection_and_releasing_them_restores_the_remaining_plan(
    cx: &mut TestAppContext,
) {
    let exec = Arc::new(FakeExec::default());
    let slow = exec.queue_stream("sh");
    let registry = MonitorWatches::default();
    let (watch, summary, details) = cx.update(|cx| {
        let summary = cx.new(|_| ());
        let details = cx.new(|_| ());
        let watch = registry.watch(exec.clone(), false, Some(Platform::Linux), cx);
        watch.update(cx, |watch, cx| {
            watch.subscribe(summary.entity_id(), plan(MonitorMetric::Memory, 30), cx)
        });
        (watch, summary, details)
    });
    cx.run_until_parked();
    let fast = exec.queue_stream("sh");
    watch.update(cx, |watch, cx| {
        watch.subscribe(
            details.entity_id(),
            WatchPlan {
                metrics: MetricSet::all(),
                interval: Duration::from_secs(2),
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert!(slow.is_closed());
    assert!(!fast.is_closed());
    let request = exec.requests().last().unwrap().stdin.clone().unwrap();
    assert!(String::from_utf8(request).unwrap().contains("@@netdev"));
    let restored = exec.queue_stream("sh");
    watch.update(cx, |watch, cx| watch.unsubscribe(details.entity_id(), cx));
    cx.run_until_parked();
    assert!(fast.is_closed());
    assert!(!restored.is_closed());
    let request = exec.requests().last().unwrap().stdin.clone().unwrap();
    assert!(!String::from_utf8(request).unwrap().contains("@@netdev"));
    watch.update(cx, |watch, cx| watch.unsubscribe(summary.entity_id(), cx));
}

#[gpui::test]
fn different_executor_authorities_never_share_a_collection(cx: &mut TestAppContext) {
    let registry = MonitorWatches::default();
    let first = Arc::new(FakeExec::default());
    let second = Arc::new(FakeExec::default());
    let (a, b, c) = cx.update(|cx| {
        (
            registry.watch(first.clone(), false, None, cx),
            registry.watch(second, false, None, cx),
            registry.watch(first, true, Some(Platform::Linux), cx),
        )
    });
    assert_ne!(a.entity_id(), b.entity_id());
    assert_ne!(a.entity_id(), c.entity_id());
    c.read_with(cx, |watch, _| {
        assert_eq!(watch.platform(), Some(&Platform::LinuxContainer))
    });
}

#[gpui::test]
fn one_latest_reading_is_shared_as_data_without_a_second_sampler(cx: &mut TestAppContext) {
    let exec = Arc::new(FakeExec::default());
    let sink = exec.queue_stream("sh");
    let registry = MonitorWatches::default();
    let (watch, consumer) = cx.update(|cx| {
        let consumer = cx.new(|_| ());
        let watch = registry.watch(exec, false, Some(Platform::Linux), cx);
        watch.update(cx, |watch, cx| {
            watch.subscribe(consumer.entity_id(), plan(MonitorMetric::Memory, 30), cx)
        });
        (watch, consumer)
    });
    cx.run_until_parked();
    futures::executor::block_on(
        sink.send(b"@@meminfo\nMemTotal: 1000\nMemAvailable: 250\n@@end\n".to_vec()),
    );
    cx.run_until_parked();
    watch.read_with(cx, |watch, _| {
        assert_eq!(watch.status(), &WatchStatus::Running);
        assert_eq!(
            watch.reading().unwrap().1.memory,
            Some((1000 * 1024, 250 * 1024))
        );
    });
    watch.update(cx, |watch, cx| watch.unsubscribe(consumer.entity_id(), cx));
    sink.finish(ExecExit::default());
}
