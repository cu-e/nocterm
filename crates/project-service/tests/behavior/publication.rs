use nocterm_project_model::{
    Catalog, CatalogError, ProjectDefinition, ProjectStorageError, RootPath,
};
use nocterm_project_service::{ClaimOutcome, PreparedProjects, ProjectsError};
use nocterm_store::{ShutdownCoordinator, StoreError, test_support::gated_writer};

use crate::support::*;

#[gpui::test]
fn memory_publication_is_visible_before_the_returned_task_is_polled(cx: &mut gpui::TestAppContext) {
    let handle = memory(Catalog::default(), cx);
    let task = cx.update(|cx| handle.save(definition(1), None, cx));
    cx.update(|cx| assert_eq!(handle.catalog(cx).get(project(1)), Some(&definition(1))));
    task.detach();
}

#[gpui::test]
async fn creation_cannot_be_claimed_until_its_definition_is_durable(cx: &mut gpui::TestAppContext) {
    let (writer, release, _) = gated_writer().into_parts();
    let handle = install(
        PreparedProjects::loaded(Ok(None), true),
        writer,
        &ShutdownCoordinator::default(),
        cx,
    );
    let task = cx.update(|cx| handle.save(definition(1), None, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            handle.claim(project(1), 1u64.into(), cx),
            Err(ProjectsError::Catalog(CatalogError::Missing(_)))
        ))
    });
    release.send(Ok(())).unwrap();
    let binding = task.await.unwrap();
    cx.update(|cx| {
        handle.check_binding(binding, cx).unwrap();
        assert!(matches!(
            handle.claim(project(1), 1u64.into(), cx).unwrap(),
            ClaimOutcome::Acquired(_)
        ));
    });
}

#[gpui::test]
async fn failed_creation_keeps_the_project_absent_and_preserves_its_original_cause(
    cx: &mut gpui::TestAppContext,
) {
    use std::error::Error as _;
    let (writer, release, _) = gated_writer().into_parts();
    let handle = install(
        PreparedProjects::loaded(Ok(None), true),
        writer,
        &ShutdownCoordinator::default(),
        cx,
    );
    let task = cx.update(|cx| handle.save(definition(1), None, cx));
    cx.run_until_parked();
    release
        .send(Err(StoreError::backend(std::fmt::Error)))
        .unwrap();
    let error = task.await.unwrap_err();
    assert!(matches!(
        error,
        ProjectsError::Persistence(StoreError::Backend(_))
    ));
    assert!(
        error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::fmt::Error>()
            .is_some()
    );
    cx.update(|cx| {
        assert!(handle.catalog(cx).is_empty());
        assert!(matches!(
            handle.claim(project(1), 1u64.into(), cx),
            Err(ProjectsError::Catalog(CatalogError::Missing(_)))
        ));
    });
}

#[gpui::test]
async fn stale_expected_definition_is_checked_at_the_ordered_durable_base(
    cx: &mut gpui::TestAppContext,
) {
    let original = definition(1);
    let (writer, release, _) = gated_writer().into_parts();
    let handle = install(
        PreparedProjects::loaded(
            Ok(Some(Catalog::new(vec![original.clone()]).unwrap())),
            true,
        ),
        writer,
        &ShutdownCoordinator::default(),
        cx,
    );
    let mut draft = original.to_draft();
    draft.name = "First edit".into();
    let first = ProjectDefinition::try_from(draft).unwrap();
    let one = cx.update(|cx| handle.save(first.clone(), Some(original.clone()), cx));
    let two = cx.update(|cx| handle.save(original.clone(), Some(original), cx));
    release.send(Ok(())).unwrap();
    one.await.unwrap();
    assert!(matches!(
        two.await,
        Err(ProjectsError::Catalog(CatalogError::Conflict(_)))
    ));
    cx.update(|cx| assert_eq!(handle.catalog(cx).get(project(1)), Some(&first)));
}

#[gpui::test]
async fn a_failed_predecessor_does_not_change_the_base_for_a_later_checked_edit(
    cx: &mut gpui::TestAppContext,
) {
    let original = definition(1);
    let (writer, release, _) = gated_writer().into_parts();
    let handle = install(
        PreparedProjects::loaded(
            Ok(Some(Catalog::new(vec![original.clone()]).unwrap())),
            true,
        ),
        writer,
        &ShutdownCoordinator::default(),
        cx,
    );
    let mut first = original.to_draft();
    first.name = "Failed".into();
    let mut second = original.to_draft();
    second.name = "Accepted".into();
    let second = ProjectDefinition::try_from(second).unwrap();
    let one = cx.update(|cx| {
        handle.save(
            ProjectDefinition::try_from(first).unwrap(),
            Some(original.clone()),
            cx,
        )
    });
    let two = cx.update(|cx| handle.save(second.clone(), Some(original), cx));
    cx.run_until_parked();
    release
        .send(Err(StoreError::backend(std::fmt::Error)))
        .unwrap();
    assert!(one.await.is_err());
    two.await.unwrap();
    cx.update(|cx| assert_eq!(handle.catalog(cx).get(project(1)), Some(&second)));
}

#[gpui::test]
async fn rename_preserves_binding_but_restoring_equal_root_values_does_not_renew_old_bindings(
    cx: &mut gpui::TestAppContext,
) {
    let original = definition(1);
    let handle = memory(Catalog::new(vec![original.clone()]).unwrap(), cx);
    let binding = cx.update(|cx| handle.binding(project(1), cx).unwrap());
    let mut draft = original.to_draft();
    draft.name = "Renamed".into();
    let renamed = ProjectDefinition::try_from(draft).unwrap();
    cx.update(|cx| handle.save(renamed.clone(), Some(original.clone()), cx))
        .await
        .unwrap();
    cx.update(|cx| handle.check_binding(binding, cx).unwrap());
    let mut draft = renamed.to_draft();
    draft.root = RootPath::parse("/srv/other").unwrap();
    let other = ProjectDefinition::try_from(draft).unwrap();
    cx.update(|cx| handle.save(other.clone(), Some(renamed), cx))
        .await
        .unwrap();
    cx.update(|cx| handle.save(original, Some(other), cx))
        .await
        .unwrap();
    cx.update(|cx| {
        assert!(matches!(
            handle.check_binding(binding, cx),
            Err(ProjectsError::StaleBinding(_))
        ))
    });
}

#[gpui::test]
async fn malformed_loaded_catalog_remains_read_only_with_its_original_error(
    cx: &mut gpui::TestAppContext,
) {
    use std::path::PathBuf;
    let handle = install(
        PreparedProjects::loaded(
            Err(ProjectStorageError::read(
                PathBuf::from("projects.json"),
                std::fmt::Error,
            )),
            true,
        ),
        ready_writer(),
        &ShutdownCoordinator::default(),
        cx,
    );
    let error = cx
        .update(|cx| handle.save(definition(1), None, cx))
        .await
        .unwrap_err();
    assert!(matches!(error, ProjectsError::ReadOnly(_)));
    let mut source: &(dyn std::error::Error + 'static) = &error;
    while let Some(next) = source.source() {
        source = next;
        if source.downcast_ref::<std::fmt::Error>().is_some() {
            cx.update(|cx| assert!(handle.catalog(cx).is_empty()));
            return;
        }
    }
    panic!("original load cause missing");
}

#[gpui::test]
async fn each_queued_success_returns_its_original_binding_even_after_a_later_folder_cycle(
    cx: &mut gpui::TestAppContext,
) {
    let original = definition(1);
    let (writer, release, _) = gated_writer().into_parts();
    let handle = install(
        PreparedProjects::loaded(
            Ok(Some(Catalog::new(vec![original.clone()]).unwrap())),
            true,
        ),
        writer,
        &ShutdownCoordinator::default(),
        cx,
    );
    let mut draft = original.to_draft();
    draft.root = RootPath::parse("/srv/other").unwrap();
    let other = ProjectDefinition::try_from(draft).unwrap();
    let one = cx.update(|cx| handle.save(other.clone(), Some(original.clone()), cx));
    let two = cx.update(|cx| handle.save(original, Some(other), cx));
    release.send(Ok(())).unwrap();
    let first_binding = one.await.unwrap();
    let final_binding = two.await.unwrap();
    assert_ne!(first_binding, final_binding);
    cx.update(|cx| {
        assert!(matches!(
            handle.check_binding(first_binding, cx),
            Err(ProjectsError::StaleBinding(_))
        ));
        handle.check_binding(final_binding, cx).unwrap();
    });
}

#[gpui::test]
async fn editing_another_project_cannot_invalidate_an_unrelated_definition_binding(
    cx: &mut gpui::TestAppContext,
) {
    let first = definition(1);
    let second = definition(2);
    let handle = memory(Catalog::new(vec![first.clone(), second]).unwrap(), cx);
    let retained = cx.update(|cx| handle.binding(project(2), cx).unwrap());
    let mut changed = first.to_draft();
    changed.root = RootPath::parse("/srv/another-folder").unwrap();
    cx.update(|cx| {
        handle.save(
            ProjectDefinition::try_from(changed).unwrap(),
            Some(first),
            cx,
        )
    })
    .await
    .unwrap();
    cx.update(|cx| handle.check_binding(retained, cx).unwrap());
}

#[gpui::test]
fn publication_observation_is_driven_by_commit_even_when_the_caller_drops_its_task(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let handle = install(
        PreparedProjects::loaded(Ok(None), true),
        ready_writer(),
        &ShutdownCoordinator::default(),
        cx,
    );
    let changed = Arc::new(AtomicUsize::new(0));
    let observed = changed.clone();
    let subscription = cx.update(|cx| {
        handle.observe(cx, move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
        })
    });
    drop(cx.update(|cx| handle.save(definition(1), None, cx)));
    cx.run_until_parked();
    assert!(changed.load(Ordering::SeqCst) > 0);
    let before = changed.load(Ordering::SeqCst);
    cx.update(|cx| {
        handle.claim(project(1), 1u64.into(), cx).unwrap();
    });
    cx.run_until_parked();
    assert!(changed.load(Ordering::SeqCst) > before);
    drop(subscription);
}
