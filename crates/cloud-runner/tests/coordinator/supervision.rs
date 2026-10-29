//! Actual supervised lifetimes, reserved cleanup capacity and bounded honest drain.
use super::*;
use futures::{FutureExt, channel::oneshot};
use nocterm_cloud_model::tool::ToolExecutionError;
use nocterm_core::persist::ShutdownDeadline;
use std::time::Duration;

#[tokio::test]
async fn shutdown_reports_outstanding_native_work_until_its_actual_completion() {
    let fixture = Fixture::with_task_limits(SupervisorLimits::new(1, 1, 1, 1, 1).unwrap());
    let (release, receive) = oneshot::channel();
    fixture
        .storage
        .tool_tasks()
        .spawn(
            async move {
                receive.await.map_err(|_| ToolExecutionError::Revoked)?;
                Ok(())
            }
            .boxed(),
        )
        .unwrap();
    assert_eq!(fixture.storage.task_snapshot().outstanding, 1);
    assert!(matches!(
        fixture
            .actor
            .shutdown(ShutdownDeadline::new(Duration::from_millis(20)))
            .await
            .unwrap(),
        DrainReport::Deadline {
            outstanding_tasks: 1,
            ..
        }
    ));
    assert!(matches!(
        fixture
            .actor
            .accept(submit(1, ChatControlRevision::default()))
            .await,
        Err(CoordinatorError::Stopped)
    ));
    assert!(fixture.journal.head(stream()).await.unwrap().is_none());
    release.send(()).unwrap();
    assert_eq!(
        fixture
            .actor
            .shutdown(ShutdownDeadline::new(Duration::from_secs(1)))
            .await
            .unwrap(),
        DrainReport::Drained
    );
    assert_eq!(
        fixture.storage.task_snapshot().phase,
        SupervisorPhase::Closed
    );
    assert_eq!(
        fixture
            .actor
            .shutdown(ShutdownDeadline::new(Duration::from_secs(1)))
            .await
            .unwrap(),
        DrainReport::Drained
    );
}
#[tokio::test]
async fn retirement_keeps_reserved_capacity_when_the_native_lane_is_full() {
    let fixture = Fixture::with_task_limits(SupervisorLimits::new(1, 1, 1, 1, 1).unwrap());
    let (release, receive) = oneshot::channel();
    fixture
        .storage
        .tool_tasks()
        .spawn(
            async move {
                receive.await.map_err(|_| ToolExecutionError::Revoked)?;
                Ok(())
            }
            .boxed(),
        )
        .unwrap();
    assert!(matches!(
        fixture.storage.tool_tasks().spawn(async { Ok(()) }.boxed()),
        Err(ToolExecutionError::Capacity)
    ));
    fixture.engine.push(complete());
    let (entered, _resume) = fixture.engine.pause_after_reserve();
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let run = fixture.binding().await;
    fixture
        .actor
        .accept(inference(2, fixture.control().await, run.clone()))
        .await
        .unwrap();
    let actor = fixture.actor.clone();
    let worker = tokio::spawn(async move { actor.drive_one().await });
    entered.wait().await;
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.actor.status(&run).await.unwrap() != RunStatus::Interrupted {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.storage.task_snapshot().failed_native_tasks, 0);
    assert_eq!(fixture.engine.calls(), 0);
    release.send(()).unwrap();
    assert_eq!(
        fixture
            .actor
            .shutdown(ShutdownDeadline::new(Duration::from_secs(1)))
            .await
            .unwrap(),
        DrainReport::Drained
    );
}
#[tokio::test]
async fn unexpected_native_failure_is_observable_and_never_reported_as_a_clean_drain() {
    let fixture = Fixture::new();
    fixture
        .storage
        .tool_tasks()
        .spawn(async { Err(ToolExecutionError::UnsupportedCapability) }.boxed())
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.storage.task_snapshot().failed_native_tasks == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        fixture
            .actor
            .accept(submit(1, ChatControlRevision::default()))
            .await,
        Err(CoordinatorError::NativeTaskFailed)
    ));
    assert!(fixture.journal.head(stream()).await.unwrap().is_none());
    assert_eq!(
        fixture
            .actor
            .shutdown(ShutdownDeadline::new(Duration::from_secs(1)))
            .await
            .unwrap(),
        DrainReport::Failed {
            failed_native_tasks: 1
        }
    );
}
#[tokio::test]
async fn dropping_the_serving_future_closes_admission_even_without_a_normal_return() {
    let fixture = Fixture::new();
    let actor = fixture.actor.clone();
    let (_abort, cancellation) = futures::future::AbortHandle::new_pair();
    let serving = tokio::spawn(async move { actor.serve(cancellation).await });
    tokio::task::yield_now().await;
    serving.abort();
    assert!(serving.await.unwrap_err().is_cancelled());
    assert_eq!(
        fixture.storage.task_snapshot().phase,
        SupervisorPhase::Draining
    );
    assert!(matches!(
        fixture
            .actor
            .accept(submit(1, ChatControlRevision::default()))
            .await,
        Err(CoordinatorError::Stopped)
    ));
    assert_eq!(
        fixture
            .actor
            .shutdown(ShutdownDeadline::new(Duration::from_secs(1)))
            .await
            .unwrap(),
        DrainReport::Drained
    );
}
