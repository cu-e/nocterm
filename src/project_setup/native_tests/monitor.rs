//! Own disposable cgroup accounting through the production scoped session executor.
use super::*;
use nocterm_monitor::{MetricSet, Platform, Reading, Sampler, Watch};

#[test]
#[ignore = "requires Docker and cached Ubuntu; checks cgroup metrics of an owned limited container"]
fn a_real_container_monitor_uses_its_own_memory_limit_and_cpu_counters() {
    container_monitor("docker.io/library/ubuntu:24.04");
}

#[test]
#[ignore = "requires Docker and cached Alpine; checks BusyBox cgroup collection"]
fn a_real_alpine_container_monitor_uses_its_own_resource_accounting() {
    container_monitor("docker.io/library/alpine:3.22");
}

fn container_monitor(image: &str) {
    let (runtime, exec, owned) = prepare_native(WorkspaceImage::Custom(image.into()), vec![]);
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let configured = block_on(async {
            exec.exec(ExecRequest::new("docker").args([
                "update",
                "--memory",
                "128m",
                "--memory-swap",
                "128m",
                "--cpus",
                "0.5",
                owned.location().container(),
            ]))
            .await
            .unwrap()
            .collect(8192)
            .await
        });
        assert!(configured.exit.success(), "{}", configured.exit.stderr);
        let (session, _driver) = container_session(exec.clone(), &owned);
        let scoped = session.exec().unwrap();
        block_on(observe_container(&*scoped));
    }));
    let removed = block_on(runtime.discard_owned(exec, owned));
    checked.unwrap();
    removed.unwrap();
}

async fn observe_container(exec: &dyn nocterm_session::HostExec) {
    let mut watch = Watch::start(
        exec,
        Platform::LinuxContainer,
        MetricSet::all(),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    let first = next_reading(&mut watch).await;
    let facts = first.container.as_ref().expect("own cgroup facts");
    assert_eq!(facts.memory_limit, Some(128 * 1024 * 1024));
    assert!(facts.memory_current.is_some_and(|bytes| bytes > 0));
    assert!(facts.cpu_nanoseconds.is_some());
    assert!(first.cpu.is_none() && first.memory.is_none());
    assert!(first.cores.is_none() && first.load.is_none() && first.uptime.is_none());
    let mut sampler = Sampler::new(10);
    sampler.apply(first);
    sampler.apply(next_reading(&mut watch).await);
    assert!(
        sampler
            .snapshot()
            .cpu
            .is_some_and(|load| load.is_finite() && load >= 0.)
    );
    assert_eq!(
        sampler.snapshot().container.as_ref().unwrap().memory_limit,
        Some(128 * 1024 * 1024)
    );
}

async fn next_reading(watch: &mut Watch) -> Reading {
    use futures::future::{Either, select};
    let reading = Box::pin(watch.next());
    let deadline = Box::pin(async_io::Timer::after(Duration::from_secs(10)));
    match select(reading, deadline).await {
        Either::Left((reading, _)) => {
            reading.expect("container collector stopped without a reading")
        }
        Either::Right(_) => panic!("container collector exceeded its deadline"),
    }
}
