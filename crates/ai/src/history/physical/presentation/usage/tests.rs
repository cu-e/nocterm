//! Real snapshot roles retain and merge usage facts without copying unchanged windows.
use crate::{
    history::{
        MAX_RESIDENT_BYTES,
        physical::{PresentationUpdate, SnapshotBuilder},
    },
    model::{
        json::deferred::DecodeLedger,
        native::{NativeList, NativeOwned},
        notice::UsageNotice,
        retention::{
            RetentionAuthority,
            document::{
                ConstructionRole, DocumentRetention, RetainedRole,
                tests::{snapshot, text},
            },
        },
    },
    usage::Window,
};

fn notice(ledger: &DecodeLedger, used: u64, windows: &[(&str, f64)]) -> NativeOwned<UsageNotice> {
    let cost = crate::model::native::usage::cost(0.5, text(ledger, "USD"));
    let context = crate::model::native::usage::context(used, 1000, NativeOwned::supplied(cost));
    let mut limits = NativeList::new(ledger, windows.len()).unwrap();
    for &(model, used) in windows {
        let window = crate::model::native::usage::weekly_model(text(ledger, model));
        limits
            .push(crate::model::native::usage::rate_limit(
                window,
                Some(used),
                Some(70),
                used >= 1.,
            ))
            .unwrap();
    }
    crate::model::native::notice::usage_notice(context, limits.finish()).unwrap()
}
fn install(role: &mut ConstructionRole, live: &mut RetainedRole, value: NativeOwned<UsageNotice>) {
    let mut builder = SnapshotBuilder::from_role(role, live).unwrap();
    builder
        .update_presentation(PresentationUpdate::Usage(value))
        .unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
}

fn used(role: &RetainedRole) -> u64 {
    let usage = role
        .snapshot()
        .unwrap()
        .presentation()
        .unwrap()
        .usage()
        .unwrap();
    usage.context().unwrap().used
}
fn limits(role: &RetainedRole) -> usize {
    let usage = role
        .snapshot()
        .unwrap()
        .presentation()
        .unwrap()
        .usage()
        .unwrap();
    usage.limits().len()
}

#[test]
fn usage_merges_last_report_per_window_and_shares_unchanged_physical_text() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut reader = roles.take_reader().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut active = roles.take_writer_active().unwrap();
    let account_baseline = scope.retained_and_pending_bytes().unwrap();
    live.replace(snapshot(&reader, "seed")).unwrap();
    let ledger = DecodeLedger::for_operation(&reader, MAX_RESIDENT_BYTES).unwrap();
    let ledger_baseline = ledger.live_requested_bytes();
    let model = "unchanged-model".repeat(4096);
    install(&mut reader, &mut live, notice(&ledger, 7, &[(&model, 0.1)]));
    active
        .replace(
            SnapshotBuilder::from_role(&mut reader, &live)
                .unwrap()
                .finish(None)
                .unwrap(),
        )
        .unwrap();
    let original = live
        .snapshot()
        .unwrap()
        .presentation()
        .unwrap()
        .usage()
        .unwrap();
    let pointer = {
        let window = original.limits().next().unwrap().window();
        let Window::WeeklyModel(original_window) = &*window else {
            panic!("model window");
        };
        original_window.as_ptr()
    };
    let retained_before = scope.retained_and_pending_bytes().unwrap();
    install(
        &mut reader,
        &mut live,
        notice(&ledger, 9, &[(&model, 0.2), ("a", 0.3), (&model, 0.9)]),
    );
    let current = live
        .snapshot()
        .unwrap()
        .presentation()
        .unwrap()
        .usage()
        .unwrap();
    assert_eq!(current.context().unwrap().used, 9);
    assert_eq!(current.limits().len(), 2);
    let limit = current.limits().last().unwrap();
    let window = limit.window();
    let Window::WeeklyModel(model) = &*window else {
        panic!("model window");
    };
    assert_eq!(model.as_ptr(), pointer);
    assert_eq!(limit.used(), Some(0.9));
    assert_eq!(limit.resets_at(), Some(70));
    assert!(!limit.reached());
    assert_eq!(used(&active), 7);
    assert!(scope.retained_and_pending_bytes().unwrap() >= retained_before);
    drop(window);
    active.clear();
    assert_eq!(limits(&live), 2);
    live.clear();
    reader.clear_source();
    assert_eq!(ledger.live_requested_bytes(), ledger_baseline);
    drop(ledger);
    assert_eq!(
        scope.retained_and_pending_bytes().unwrap(),
        account_baseline
    );
}
