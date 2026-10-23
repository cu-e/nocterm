use std::sync::Arc;

use futures::{FutureExt as _, executor::block_on};
use nocterm_containers::Engine;
use nocterm_project_model::{HostLocator, ProjectId, RootPath};
use nocterm_project_providers::{
    ContainerLocation, ContainerOrigin, OwnedContainer, PreparedProject, ProjectSetupError,
    ProjectSetupRequest, SetupMode, WorkspaceImage, testing::FakeProjectPreparation,
};
use nocterm_project_service::WorkspaceSetup;
use nocterm_session::testing::{FakeExec, Gate};

fn fixture() -> (WorkspaceSetup, FakeProjectPreparation, OwnedContainer) {
    let project = ProjectId::from_bytes([1; 16]);
    let owned = OwnedContainer::new(
        ContainerLocation::new(
            HostLocator::Local,
            Engine::Docker,
            "a".repeat(64),
            RootPath::parse("/workspace").unwrap(),
            ContainerOrigin::Created,
        )
        .unwrap(),
        project,
        nocterm_project_providers::CreationToken::from_bytes([2; 16]),
    );
    let preparation = FakeProjectPreparation::default();
    preparation
        .respond(Ok(PreparedProject {
            target: owned.location().to_target().unwrap(),
            owned: Some(owned.clone()),
        }))
        .unwrap();
    let setup = WorkspaceSetup::new(
        Arc::new(preparation.clone()),
        Arc::new(FakeExec::default()),
        ProjectSetupRequest {
            project,
            host: HostLocator::Local,
            mode: SetupMode::Create {
                engine: Engine::Docker,
                image: WorkspaceImage::Ubuntu,
                languages: vec![],
                root: None,
            },
            repository: None,
        },
    );
    (setup, preparation, owned)
}

#[test]
fn failed_publication_reuses_the_prepared_target_without_creating_another_container() {
    let (setup, preparation, _) = fixture();
    let original = block_on(setup.prepare(Arc::new(|_| {}))).unwrap();
    setup.publishing();
    setup.publication_failed();
    assert_eq!(block_on(setup.prepare(Arc::new(|_| {}))).unwrap(), original);
    assert_eq!(preparation.requests().unwrap().len(), 1);
    setup.committed();
    block_on(setup.discard()).unwrap();
    assert!(preparation.discarded().unwrap().is_empty());
}

#[test]
fn dismissal_before_preparation_prevents_resource_creation() {
    let (setup, preparation, _) = fixture();
    block_on(setup.discard()).unwrap();
    assert!(matches!(
        block_on(setup.prepare(Arc::new(|_| {}))),
        Err(ProjectSetupError::Canceled)
    ));
    assert!(preparation.requests().unwrap().is_empty());
    assert!(preparation.discarded().unwrap().is_empty());
}

#[test]
fn dismissal_during_preparation_cleans_a_late_owned_resource() {
    let (setup, preparation, owned) = fixture();
    let gate = Gate::pending();
    let waiter = gate.clone();
    preparation
        .suspend(async move { waiter.wait().await }.boxed())
        .unwrap();
    let mut pending = Box::pin(setup.prepare(Arc::new(|_| {})));
    assert!(pending.as_mut().now_or_never().is_none());
    block_on(setup.discard()).unwrap();
    assert!(preparation.discarded().unwrap().is_empty());
    gate.release();
    assert!(matches!(
        block_on(pending),
        Err(ProjectSetupError::Canceled)
    ));
    assert_eq!(
        preparation.discarded().unwrap()[0].location(),
        owned.location()
    );
}

#[test]
fn a_second_submission_is_refused_while_the_original_preparation_is_pending() {
    let (setup, preparation, _) = fixture();
    let gate = Gate::pending();
    let waiter = gate.clone();
    preparation
        .suspend(async move { waiter.wait().await }.boxed())
        .unwrap();
    let mut pending = Box::pin(setup.prepare(Arc::new(|_| {})));
    assert!(pending.as_mut().now_or_never().is_none());
    assert!(matches!(
        block_on(setup.prepare(Arc::new(|_| {}))),
        Err(ProjectSetupError::InvalidInput(_))
    ));
    assert_eq!(preparation.requests().unwrap().len(), 1);
    gate.release();
    block_on(pending).unwrap();
    block_on(setup.discard()).unwrap();
}

#[test]
fn dismissal_defers_cleanup_until_a_pending_publication_reports_failure() {
    let (setup, preparation, _) = fixture();
    block_on(setup.prepare(Arc::new(|_| {}))).unwrap();
    setup.publishing();
    block_on(setup.discard()).unwrap();
    assert!(preparation.discarded().unwrap().is_empty());
    setup.publication_failed();
    block_on(setup.discard()).unwrap();
    assert_eq!(preparation.discarded().unwrap().len(), 1);
}

#[test]
fn cancellation_cannot_delete_a_resource_after_publication_commits() {
    let (setup, preparation, _) = fixture();
    block_on(setup.prepare(Arc::new(|_| {}))).unwrap();
    setup.publishing();
    block_on(setup.discard()).unwrap();
    setup.committed();
    block_on(setup.discard()).unwrap();
    assert!(preparation.discarded().unwrap().is_empty());
}
