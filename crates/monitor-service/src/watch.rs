//! One generation-bound collection attempt and bounded retry cadence.
use crate::{MonitorWatch, WatchPlan, WatchStatus};
use gpui::{AsyncApp, WeakEntity};
use nocterm_monitor::{Platform, Watch, detect};
use nocterm_session::{ExecError, HostExec};
use std::{sync::Arc, time::Duration};

pub(super) async fn watch(
    this: WeakEntity<MonitorWatch>,
    exec: Arc<dyn HostExec>,
    container: bool,
    mut platform: Option<Platform>,
    plan: WatchPlan,
    generation: u64,
    cx: &mut AsyncApp,
) {
    loop {
        let outcome = run(
            &this,
            exec.as_ref(),
            container,
            &mut platform,
            plan,
            generation,
            cx,
        )
        .await;
        let retry = this.update(cx, |this, cx| {
            if this.generation != generation || this.plan.is_none() {
                return false;
            }
            let status = match outcome {
                Err(ExecError::Unsupported) => {
                    let name = match &platform {
                        Some(Platform::Unsupported(name)) => name.clone(),
                        Some(Platform::LinuxContainer) => "container cgroup accounting".into(),
                        _ => "unknown".into(),
                    };
                    this.set_status(WatchStatus::Unsupported(name), cx);
                    return false;
                }
                Err(error @ ExecError::Source(_)) => {
                    this.set_status(WatchStatus::Failed(error), cx);
                    return false;
                }
                Err(error) => WatchStatus::Failed(error),
                Ok(()) => {
                    WatchStatus::Failed(ExecError::Failed("The monitor script stopped.".into()))
                }
            };
            this.set_status(status, cx);
            true
        });
        if !matches!(retry, Ok(true)) {
            return;
        }
        cx.background_executor()
            .timer(Duration::from_secs(10))
            .await;
    }
}
async fn run(
    this: &WeakEntity<MonitorWatch>,
    exec: &dyn HostExec,
    container: bool,
    platform: &mut Option<Platform>,
    plan: WatchPlan,
    generation: u64,
    cx: &mut AsyncApp,
) -> Result<(), ExecError> {
    let detected = match platform.clone() {
        Some(platform) => platform,
        None => {
            let detected = detect(exec).await?;
            let detected = crate::scoped_platform(detected, container);
            *platform = Some(detected.clone());
            let accepted = this
                .update(cx, |this, cx| {
                    if this.generation != generation {
                        return false;
                    }
                    this.platform = Some(detected.clone());
                    this.set_status(WatchStatus::Starting, cx);
                    true
                })
                .map_err(|_| ExecError::Disconnected)?;
            if !accepted {
                return Err(ExecError::Disconnected);
            }
            detected
        }
    };
    let mut watch = Watch::start(exec, detected, plan.metrics, plan.interval).await?;
    while let Some(reading) = watch.next().await {
        if container
            && reading.container.as_ref().is_none_or(|counters| {
                counters.cpu_nanoseconds.is_none() && counters.memory_current.is_none()
            })
        {
            return Err(ExecError::Unsupported);
        }
        if !matches!(
            this.update(cx, |this, cx| this.accept(generation, reading, cx)),
            Ok(true)
        ) {
            return Ok(());
        }
    }
    Ok(())
}
