//! Explicit actions retain exact authority and publish only after acknowledgment.
use crate::support::*;
use futures::{FutureExt as _, future::BoxFuture};
use nocterm_containers::Engine;
use nocterm_project_model::*;
use nocterm_project_providers::{testing::FakeWorkspaceContainerActions, *};
use nocterm_project_service::{
    PreparedProjects, ProjectsError, ProjectsHandle, RemovalContainerAction, RemovalOperation,
    WorkspaceRemovalError,
};
use nocterm_session::testing::{FakeExec, Gate};
use nocterm_store::ShutdownCoordinator;
use std::sync::Arc;
fn container(value: u8, host: HostLocator, root: &str) -> ProjectDefinition {
    let mut draft = definition(value).to_draft();
    draft.target = ContainerLocation::new(
        host,
        Engine::Docker,
        "a".repeat(64),
        RootPath::parse(root).unwrap(),
        ContainerOrigin::Attached,
    )
    .unwrap()
    .to_target()
    .unwrap();
    draft.try_into().unwrap()
}
fn action(
    definition: &ProjectDefinition,
    fake: &FakeWorkspaceContainerActions,
    action: ContainerAction,
) -> RemovalContainerAction {
    let location = ContainerLocation::from_target(definition.target()).unwrap();
    let host = location.host().clone();
    RemovalContainerAction::new(
        Arc::new(fake.clone()),
        ContainerActionSource::new(location, &host, Arc::new(FakeExec::default())).unwrap(),
        action,
    )
}
fn begin(
    handle: &ProjectsHandle,
    definition: &ProjectDefinition,
    action: Option<RemovalContainerAction>,
    cx: &mut gpui::TestAppContext,
) -> Result<RemovalOperation, WorkspaceRemovalError> {
    cx.update(|cx| {
        handle.remove_workspace(
            definition.clone(),
            handle.binding(definition.id(), cx).unwrap(),
            action,
            cx,
        )
    })
}
fn suspended(gate: &Gate) -> BoxFuture<'static, ()> {
    let gate = gate.clone();
    async move { gate.wait().await }.boxed()
}
fn fixture(original: &ProjectDefinition, cx: &mut gpui::TestAppContext) -> ProjectsHandle {
    memory(Catalog::new(vec![original.clone()]).unwrap(), cx)
}
#[gpui::test]
async fn keep_never_invokes_container_actions_and_removes_only_metadata(
    cx: &mut gpui::TestAppContext,
) {
    let original = container(1, HostLocator::Local, "/home");
    let handle = fixture(&original, cx);
    begin(&handle, &original, None, cx)
        .unwrap()
        .wait()
        .await
        .unwrap();
    cx.update(|cx| assert!(handle.catalog(cx).get(original.id()).is_none()));
}
#[gpui::test]
async fn successful_explicit_actions_precede_checked_removal_and_keep_the_original_source(
    cx: &mut gpui::TestAppContext,
) {
    for choice in [ContainerAction::Stop, ContainerAction::Delete] {
        let original = container(1, HostLocator::Local, "/home");
        let handle = fixture(&original, cx);
        let fake = FakeWorkspaceContainerActions::default();
        let gate = Gate::pending();
        fake.suspend(suspended(&gate)).unwrap();
        fake.respond(Ok(ContainerActionOutcome { action: choice }))
            .unwrap();
        let request = action(&original, &fake, choice);
        let exec = request.source().executor();
        let op = begin(&handle, &original, Some(request), cx).unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(handle.catalog(cx).get(original.id()), Some(&original)));
        gate.release();
        op.wait().await.unwrap();
        assert_eq!(
            op.outcome(),
            Some(ContainerActionOutcome { action: choice })
        );
        assert!(Arc::ptr_eq(
            &fake.calls().unwrap()[0].source.executor(),
            &exec
        ));
        cx.update(|cx| assert!(handle.catalog(cx).get(original.id()).is_none()));
    }
}
#[gpui::test]
async fn failed_or_uncertain_action_keeps_the_original_definition_and_claim(
    cx: &mut gpui::TestAppContext,
) {
    let original = container(1, HostLocator::Local, "/home");
    let handle = fixture(&original, cx);
    let claim = cx.update(|cx| {
        match handle
            .claim(original.id(), gpui::WindowId::from(7), cx)
            .unwrap()
        {
            nocterm_project_service::ClaimOutcome::Acquired(claim) => claim,
            _ => panic!("fresh claim"),
        }
    });
    let fake = FakeWorkspaceContainerActions::default();
    fake.respond(Err(ContainerActionError::TimedOut)).unwrap();
    let op = begin(
        &handle,
        &original,
        Some(action(&original, &fake, ContainerAction::Delete)),
        cx,
    )
    .unwrap();
    assert!(matches!(
        op.wait().await,
        Err(WorkspaceRemovalError::Action(
            ContainerActionError::TimedOut
        ))
    ));
    cx.update(|cx| {
        assert_eq!(handle.catalog(cx).get(original.id()), Some(&original));
        handle.check_claim(claim, cx).unwrap();
    });
}
#[gpui::test]
async fn duplicate_resource_actions_serialize_across_directories_and_profile_aliases(
    cx: &mut gpui::TestAppContext,
) {
    let host = |profile: &str| HostLocator::Ssh {
        profile: profile.into(),
        target: nocterm_session::Target::new("dev", "server.example", 22),
        via: vec![],
    };
    let first = container(1, host("first"), "/one");
    let second = container(2, host("alias"), "/two");
    let handle = memory(
        Catalog::new(vec![first.clone(), second.clone()]).unwrap(),
        cx,
    );
    let fake = FakeWorkspaceContainerActions::default();
    let gate = Gate::pending();
    fake.suspend(suspended(&gate)).unwrap();
    fake.respond(Ok(ContainerActionOutcome {
        action: ContainerAction::Stop,
    }))
    .unwrap();
    let op = begin(
        &handle,
        &first,
        Some(action(&first, &fake, ContainerAction::Stop)),
        cx,
    )
    .unwrap();
    assert!(matches!(
        begin(
            &handle,
            &second,
            Some(action(&second, &fake, ContainerAction::Delete)),
            cx
        ),
        Err(WorkspaceRemovalError::Action(ContainerActionError::Busy))
    ));
    gate.release();
    op.wait().await.unwrap();
    assert_eq!(fake.calls().unwrap().len(), 1);
}
#[gpui::test]
async fn catalog_failure_preserves_known_effect_and_retry_never_repeats_delete(
    cx: &mut gpui::TestAppContext,
) {
    let original = container(1, HostLocator::Local, "/home");
    let (writer, release, _) = nocterm_store::test_support::gated_writer().into_parts();
    let handle = install(
        PreparedProjects::loaded(
            Ok(Some(Catalog::new(vec![original.clone()]).unwrap())),
            true,
        ),
        writer,
        &ShutdownCoordinator::default(),
        cx,
    );
    let fake = FakeWorkspaceContainerActions::default();
    fake.respond(Ok(ContainerActionOutcome {
        action: ContainerAction::Delete,
    }))
    .unwrap();
    let op = begin(
        &handle,
        &original,
        Some(action(&original, &fake, ContainerAction::Delete)),
        cx,
    )
    .unwrap();
    cx.run_until_parked();
    release
        .send(Err(nocterm_store::StoreError::backend(
            std::io::Error::other("disk full"),
        )))
        .unwrap();
    assert!(matches!(
        op.wait().await,
        Err(WorkspaceRemovalError::Catalog(_))
    ));
    assert_eq!(op.outcome().unwrap().action, ContainerAction::Delete);
    cx.update(|cx| {
        assert_eq!(handle.catalog(cx).get(original.id()), Some(&original));
        handle.retry_removal(&op, cx).unwrap();
    });
    op.wait().await.unwrap();
    assert_eq!(fake.calls().unwrap().len(), 1);
    cx.update(|cx| assert!(handle.catalog(cx).get(original.id()).is_none()));
}
#[gpui::test]
async fn stale_binding_refuses_before_io_and_changed_target_during_action_preserves_replacement(
    cx: &mut gpui::TestAppContext,
) {
    let original = container(1, HostLocator::Local, "/home");
    let handle = fixture(&original, cx);
    let fake = FakeWorkspaceContainerActions::default();
    let binding = cx.update(|cx| handle.binding(original.id(), cx)).unwrap();
    let replacement = container(1, HostLocator::Local, "/changed");
    cx.update(|cx| handle.save(replacement.clone(), Some(original.clone()), cx))
        .await
        .unwrap();
    assert!(matches!(
        cx.update(|cx| handle.remove_workspace(
            original.clone(),
            binding,
            Some(action(&original, &fake, ContainerAction::Delete)),
            cx
        )),
        Err(WorkspaceRemovalError::Catalog(ProjectsError::StaleBinding(
            _
        )))
    ));
    assert!(fake.calls().unwrap().is_empty());
    let gate = Gate::pending();
    fake.suspend(suspended(&gate)).unwrap();
    fake.respond(Ok(ContainerActionOutcome {
        action: ContainerAction::Stop,
    }))
    .unwrap();
    let op = begin(
        &handle,
        &replacement,
        Some(action(&replacement, &fake, ContainerAction::Stop)),
        cx,
    )
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| handle.save(original.clone(), Some(replacement), cx))
        .await
        .unwrap();
    gate.release();
    assert!(matches!(
        op.wait().await,
        Err(WorkspaceRemovalError::Catalog(ProjectsError::StaleBinding(
            _
        )))
    ));
    assert!(op.outcome().is_some());
    cx.update(|cx| assert_eq!(handle.catalog(cx).get(original.id()), Some(&original)));
}
#[gpui::test]
async fn dropping_receivers_does_not_cancel_an_admitted_action_or_publication(
    cx: &mut gpui::TestAppContext,
) {
    let original = container(1, HostLocator::Local, "/home");
    let handle = fixture(&original, cx);
    let fake = FakeWorkspaceContainerActions::default();
    let gate = Gate::pending();
    fake.suspend(suspended(&gate)).unwrap();
    fake.respond(Ok(ContainerActionOutcome {
        action: ContainerAction::Stop,
    }))
    .unwrap();
    let op = begin(
        &handle,
        &original,
        Some(action(&original, &fake, ContainerAction::Stop)),
        cx,
    )
    .unwrap();
    drop(op.wait());
    drop(op);
    cx.run_until_parked();
    gate.release();
    cx.run_until_parked();
    cx.update(|cx| assert!(handle.catalog(cx).get(original.id()).is_none()));
}
#[gpui::test]
async fn shutdown_freezes_admission_cancels_and_drains_without_late_publication(
    cx: &mut gpui::TestAppContext,
) {
    let original = container(1, HostLocator::Local, "/home");
    let shutdown = ShutdownCoordinator::default();
    let handle = install(
        PreparedProjects::loaded(
            Ok(Some(Catalog::new(vec![original.clone()]).unwrap())),
            false,
        ),
        ready_writer(),
        &shutdown,
        cx,
    );
    let fake = FakeWorkspaceContainerActions::default();
    let gate = Gate::pending();
    fake.suspend(suspended(&gate)).unwrap();
    fake.respond(Err(ContainerActionError::Canceled)).unwrap();
    let op = begin(
        &handle,
        &original,
        Some(action(&original, &fake, ContainerAction::Delete)),
        cx,
    )
    .unwrap();
    cx.run_until_parked();
    let work = cx.update(|cx| shutdown.capture_all(cx));
    assert!(fake.calls().unwrap()[0].cancel.is_cancelled());
    let drain = cx.background_executor.spawn(work);
    gate.release();
    assert!(drain.await.failures().is_empty());
    assert!(op.wait().await.is_err());
    cx.update(|cx| assert_eq!(handle.catalog(cx).get(original.id()), Some(&original)));
}
