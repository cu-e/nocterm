use std::sync::Arc;

use nocterm_project_model::{ProjectMetadataError, testing::SequenceProjectMetadata};
use nocterm_project_service::{PreparedProjects, ProjectsError, ProjectsHandle};
use nocterm_store::{ShutdownCoordinator, test_support::gated_writer};

use crate::support::*;

#[gpui::test]
async fn shutdown_freezes_new_claims_and_drains_the_already_admitted_catalog(
    cx: &mut gpui::TestAppContext,
) {
    let shutdown = ShutdownCoordinator::default();
    let (writer, release, writes) = gated_writer().into_parts();
    let handle = install(
        PreparedProjects::loaded(Ok(None), true),
        writer,
        &shutdown,
        cx,
    );
    let one = cx.update(|cx| handle.save(definition(1), None, cx));
    cx.run_until_parked();
    let two = cx.update(|cx| handle.save(definition(2), None, cx));
    let work = cx.update(|cx| shutdown.capture_all(cx));
    cx.update(|cx| {
        assert!(matches!(
            handle.claim(project(1), 1u64.into(), cx),
            Err(ProjectsError::Closing)
        ))
    });
    assert!(matches!(
        cx.update(|cx| handle.save(definition(3), None, cx)).await,
        Err(ProjectsError::Closing)
    ));
    release.send(Ok(())).unwrap();
    let report = work.await;
    assert!(!report.timed_out());
    assert!(report.failures().is_empty());
    one.await.unwrap();
    two.await.unwrap();
    let final_catalog = writes.lock().unwrap().last().unwrap().clone();
    assert!(final_catalog.get(project(1)).is_some());
    assert!(final_catalog.get(project(2)).is_some());
    assert!(final_catalog.get(project(3)).is_none());
}

#[gpui::test]
async fn a_closing_coordinator_refuses_installation_without_exposing_a_fallback(
    cx: &mut gpui::TestAppContext,
) {
    let shutdown = ShutdownCoordinator::default();
    let work = cx.update(|cx| shutdown.capture_all(cx));
    let result = cx.update(|cx| {
        ProjectsHandle::install(
            PreparedProjects::loaded(Ok(None), false),
            ready_writer(),
            Arc::new(SequenceProjectMetadata::new(17, 123)),
            &shutdown,
            cx,
        )
    });
    assert!(matches!(result, Err(ProjectsError::Closing)));
    assert!(work.await.failures().is_empty());
}

#[gpui::test]
fn identity_allocation_failure_is_typed_and_does_not_create_a_definition(
    cx: &mut gpui::TestAppContext,
) {
    let metadata = Arc::new(SequenceProjectMetadata::new(17, 123));
    metadata.fail_next(ProjectMetadataError::entropy(std::fmt::Error));
    let handle = cx.update(|cx| {
        ProjectsHandle::install(
            PreparedProjects::loaded(Ok(None), false),
            ready_writer(),
            metadata,
            &ShutdownCoordinator::default(),
            cx,
        )
        .unwrap()
    });
    cx.update(|cx| {
        assert!(matches!(
            handle.new_identity(cx),
            Err(ProjectsError::Metadata(ProjectMetadataError::Entropy(_)))
        ));
        assert!(handle.catalog(cx).is_empty());
        assert_eq!(
            handle.new_identity(cx).unwrap().id,
            nocterm_project_model::ProjectId::from_bytes(17u128.to_be_bytes())
        );
    });
}

#[gpui::test]
async fn dropping_external_handles_does_not_discard_admitted_publication(
    cx: &mut gpui::TestAppContext,
) {
    let shutdown = ShutdownCoordinator::default();
    let (writer, release, writes) = gated_writer().into_parts();
    let handle = install(
        PreparedProjects::loaded(Ok(None), true),
        writer,
        &shutdown,
        cx,
    );
    let task = cx.update(|cx| handle.save(definition(1), None, cx));
    cx.run_until_parked();
    drop(handle);
    release.send(Ok(())).unwrap();
    task.await.unwrap();
    assert_eq!(
        writes.lock().unwrap().last().unwrap().get(project(1)),
        Some(&definition(1))
    );
}

#[gpui::test]
async fn a_full_queue_refuses_new_work_without_discarding_admitted_definitions(
    cx: &mut gpui::TestAppContext,
) {
    let (writer, release, _) = gated_writer().into_parts();
    let handle = install(
        PreparedProjects::loaded(Ok(None), true),
        writer,
        &ShutdownCoordinator::default(),
        cx,
    );
    let first = cx.update(|cx| handle.save(definition(1), None, cx));
    cx.run_until_parked();
    let mut pending = Vec::new();
    for value in 2..=33 {
        pending.push(cx.update(|cx| handle.save(definition(value), None, cx)));
    }
    assert!(matches!(
        cx.update(|cx| handle.save(definition(34), None, cx)).await,
        Err(ProjectsError::Full)
    ));
    release.send(Ok(())).unwrap();
    first.await.unwrap();
    for task in pending {
        task.await.unwrap();
    }
    cx.update(|cx| {
        assert_eq!(handle.catalog(cx).len(), 33);
        assert!(handle.catalog(cx).get(project(34)).is_none());
    });
}

#[gpui::test]
async fn shutdown_refuses_new_workspace_preparation_without_creating_resources(
    cx: &mut gpui::TestAppContext,
) {
    use nocterm_project_providers::{
        ProjectSetupRequest, SetupMode, testing::FakeProjectPreparation,
    };
    let shutdown = ShutdownCoordinator::default();
    let handle = install(
        PreparedProjects::loaded(Ok(None), true),
        ready_writer(),
        &shutdown,
        cx,
    );
    let preparation = FakeProjectPreparation::default();
    let work = cx.update(|cx| shutdown.capture_all(cx));
    let result = cx.update(|cx| {
        handle.prepare_workspace(
            Arc::new(preparation.clone()),
            Arc::new(nocterm_session::testing::FakeExec::default()),
            ProjectSetupRequest {
                project: project(1),
                host: nocterm_project_model::HostLocator::Local,
                mode: SetupMode::Direct {
                    root: nocterm_project_model::RootPath::parse("/workspace").unwrap(),
                },
                repository: None,
            },
            cx,
        )
    });
    assert!(matches!(result, Err(ProjectsError::Closing)));
    assert!(preparation.requests().unwrap().is_empty());
    assert!(work.await.failures().is_empty());
}
