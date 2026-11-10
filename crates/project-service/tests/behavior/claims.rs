use nocterm_project_model::Catalog;
use nocterm_project_service::{ClaimOutcome, ProjectsError};
use nocterm_store::{ShutdownCoordinator, StoreError, test_support::gated_writer};

use crate::support::*;

#[gpui::test]
fn focus_requests_name_the_exact_retained_claim_and_refuse_another_window(
    cx: &mut gpui::TestAppContext,
) {
    let handle = memory(Catalog::new(vec![definition(1)]).unwrap(), cx);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = seen.clone();
    let _subscription =
        cx.update(|cx| handle.observe_focus(cx, move |claim, _| observed.borrow_mut().push(claim)));
    let original = cx.update(|cx| {
        let ClaimOutcome::Acquired(claim) = handle.claim(project(1), 1u64.into(), cx).unwrap()
        else {
            panic!("no claim")
        };
        assert!(matches!(
            handle.request_focus(project(1), 2u64.into(), cx),
            Err(ProjectsError::StaleClaim(_))
        ));
        handle.request_focus(project(1), 1u64.into(), cx).unwrap();
        claim
    });
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [original]);
    cx.update(|cx| {
        handle.release_claim(original, cx).unwrap();
        assert!(handle.request_focus(project(1), 1u64.into(), cx).is_err());
    });
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [original]);
}

#[gpui::test]
fn a_focus_request_captured_before_reopening_cannot_activate_the_replacement_claim(
    cx: &mut gpui::TestAppContext,
) {
    let handle = memory(Catalog::new(vec![definition(1)]).unwrap(), cx);
    let original = cx.update(
        |cx| match handle.claim(project(1), 1u64.into(), cx).unwrap() {
            ClaimOutcome::Acquired(claim) => claim,
            _ => panic!("no claim"),
        },
    );
    let focused = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = focused.clone();
    let model = handle.clone();
    let _subscription = cx.update(|cx| {
        handle.observe_focus(cx, move |claim, cx| {
            let model = model.clone();
            let observed = observed.clone();
            cx.defer(move |cx| {
                if claim == original && model.check_claim(original, cx).is_ok() {
                    observed.set(true);
                }
            });
        })
    });
    cx.update(|cx| {
        handle.request_focus(project(1), 1u64.into(), cx).unwrap();
        handle.release_claim(original, cx).unwrap();
        assert!(matches!(
            handle.claim(project(1), 1u64.into(), cx).unwrap(),
            ClaimOutcome::Acquired(_)
        ));
    });
    cx.run_until_parked();
    assert!(!focused.get());
}

#[gpui::test]
fn one_window_owns_a_project_and_repeated_opens_retain_the_original_claim(
    cx: &mut gpui::TestAppContext,
) {
    let handle = memory(Catalog::new(vec![definition(1)]).unwrap(), cx);
    cx.update(|cx| {
        let ClaimOutcome::Acquired(original) = handle.claim(project(1), 1u64.into(), cx).unwrap()
        else {
            panic!("first owner not acquired")
        };
        assert_eq!(
            handle.claim(project(1), 1u64.into(), cx).unwrap(),
            ClaimOutcome::Retained(original)
        );
        assert_eq!(
            handle.claim(project(1), 2u64.into(), cx).unwrap(),
            ClaimOutcome::FocusOwner {
                project: project(1),
                window: 1u64.into()
            }
        );
    });
}

#[gpui::test]
async fn failed_removal_keeps_the_original_owning_window_claim(cx: &mut gpui::TestAppContext) {
    let original = definition(1);
    let (writer, release, _) = gated_writer().into_parts();
    let handle = install(
        nocterm_project_service::PreparedProjects::loaded(
            Ok(Some(Catalog::new(vec![original.clone()]).unwrap())),
            true,
        ),
        writer,
        &ShutdownCoordinator::default(),
        cx,
    );
    let old = cx.update(
        |cx| match handle.claim(project(1), 1u64.into(), cx).unwrap() {
            ClaimOutcome::Acquired(claim) => claim,
            _ => panic!("no owner"),
        },
    );
    let task = cx.update(|cx| handle.remove(original, cx));
    cx.run_until_parked();
    release
        .send(Err(StoreError::backend(std::fmt::Error)))
        .unwrap();
    assert!(task.await.is_err());
    cx.update(|cx| {
        assert_eq!(
            handle.claim(project(1), 1u64.into(), cx).unwrap(),
            ClaimOutcome::Retained(old)
        );
        assert_eq!(
            handle.claim(project(1), 2u64.into(), cx).unwrap(),
            ClaimOutcome::FocusOwner {
                project: project(1),
                window: 1u64.into()
            }
        );
    });
}

#[gpui::test]
fn independent_installations_cannot_borrow_each_others_definition_or_window_receipts(
    cx: &mut gpui::TestAppContext,
) {
    let catalog = Catalog::new(vec![definition(1)]).unwrap();
    let first = memory(catalog.clone(), cx);
    let second = memory(catalog, cx);
    cx.update(|cx| {
        let binding = first.binding(project(1), cx).unwrap();
        assert!(matches!(
            second.check_binding(binding, cx),
            Err(ProjectsError::StaleBinding(_))
        ));
        let ClaimOutcome::Acquired(first_claim) = first.claim(project(1), 1u64.into(), cx).unwrap()
        else {
            panic!("no first owner")
        };
        let ClaimOutcome::Acquired(second_claim) =
            second.claim(project(1), 1u64.into(), cx).unwrap()
        else {
            panic!("no second owner")
        };
        assert!(matches!(
            second.release_claim(first_claim, cx),
            Err(ProjectsError::StaleClaim(_))
        ));
        assert_eq!(
            second.claim(project(1), 1u64.into(), cx).unwrap(),
            ClaimOutcome::Retained(second_claim)
        );
    });
}

#[gpui::test]
fn a_stale_release_cannot_close_a_new_claim_in_the_same_window(cx: &mut gpui::TestAppContext) {
    let handle = memory(Catalog::new(vec![definition(1)]).unwrap(), cx);
    cx.update(|cx| {
        let ClaimOutcome::Acquired(old) = handle.claim(project(1), 1u64.into(), cx).unwrap() else {
            panic!("no first owner")
        };
        handle.release_claim(old, cx).unwrap();
        let ClaimOutcome::Acquired(new) = handle.claim(project(1), 1u64.into(), cx).unwrap() else {
            panic!("no new owner")
        };
        assert_ne!(old, new);
        assert!(matches!(
            handle.release_claim(old, cx),
            Err(ProjectsError::StaleClaim(_))
        ));
        assert_eq!(
            handle.claim(project(1), 1u64.into(), cx).unwrap(),
            ClaimOutcome::Retained(new)
        );
    });
}

#[gpui::test]
async fn durable_removal_releases_the_owner_but_same_name_reimport_never_retargets_old_claim(
    cx: &mut gpui::TestAppContext,
) {
    let original = definition(1);
    let handle = memory(Catalog::new(vec![original.clone()]).unwrap(), cx);
    let old = cx.update(
        |cx| match handle.claim(project(1), 1u64.into(), cx).unwrap() {
            ClaimOutcome::Acquired(claim) => claim,
            _ => panic!("no owner"),
        },
    );
    cx.update(|cx| handle.remove(original, cx)).await.unwrap();
    cx.update(|cx| handle.save(definition(2), None, cx))
        .await
        .unwrap();
    cx.update(|cx| {
        assert!(matches!(
            handle.release_claim(old, cx),
            Err(ProjectsError::StaleClaim(_))
        ));
        assert!(matches!(
            handle.claim(project(2), 2u64.into(), cx).unwrap(),
            ClaimOutcome::Acquired(_)
        ));
    });
}
