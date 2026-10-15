use super::*;
use gpui_kit::TestAppContext;

#[gpui_kit::test]
fn queued_layout_snapshots_cannot_overwrite_the_latest_arrangement(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("layout.json");
    cx.update(|cx| {
        LayoutMemory::init(Some(file.clone()), &ShutdownCoordinator::default(), cx);
        LayoutMemory::set("panel", px(100.), cx);
        LayoutMemory::set("panel", px(200.), cx);
        LayoutMemory::set_flag("right", true, cx);
        LayoutMemory::set("panel", px(300.), cx);
        assert_eq!(LayoutMemory::get("panel", cx), Some(px(300.)));
        assert!(LayoutMemory::flag("right", cx));
    });
    cx.run_until_parked();
    let saved: Saved = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved.sizes["panel"], 300.);
    assert!(saved.flags["right"]);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[gpui_kit::test]
fn quit_saves_captured_layout_without_application_updates(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("layout.json");
    cx.update(|cx| {
        LayoutMemory::init(Some(file.clone()), &ShutdownCoordinator::default(), cx);
        LayoutMemory::set("panel", px(100.), cx);
        LayoutMemory::set("panel", px(240.), cx);
        LayoutMemory::set_flag("right", true, cx);
    });
    cx.quit();
    let saved: Saved = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(saved.sizes["panel"], 240.);
    assert!(saved.flags["right"]);
}

#[gpui_kit::test]
fn unchanged_layout_is_not_rewritten_at_quit(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("layout.json");
    std::fs::write(&file, "invalid untouched layout").unwrap();
    cx.update(|cx| LayoutMemory::init(Some(file.clone()), &ShutdownCoordinator::default(), cx));
    cx.quit();
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "invalid untouched layout"
    );
}

#[gpui_kit::test]
fn layout_latest_arrangement_survives_more_edits_than_writer_capacity(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("layout.json");
    cx.update(|cx| {
        LayoutMemory::init(Some(file.clone()), &ShutdownCoordinator::default(), cx);
        for value in 1..=10000 {
            LayoutMemory::set("panel", px(value as f32), cx);
        }
        assert_eq!(LayoutMemory::get("panel", cx), Some(px(10000.)));
    });
    cx.quit();
    let saved: Saved = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(saved.sizes["panel"], 10000.);
}

#[gpui_kit::test]
fn explicit_layout_handles_and_legacy_split_share_the_registered_state(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("layout.json");
    cx.update(|cx| {
        let layout = LayoutMemory::init(Some(file.clone()), &ShutdownCoordinator::default(), cx);
        let other_window = layout.clone();
        layout.set("sidebar", px(210.4), cx).unwrap();
        assert_eq!(other_window.get("sidebar", cx), Some(px(210.)));
        assert_eq!(LayoutMemory::get("sidebar", cx), Some(px(210.)));
        LayoutMemory::set("history", px(175.), cx);
        assert_eq!(layout.get("history", cx), Some(px(175.)));
        other_window.set_flag("swapped", true, cx).unwrap();
        assert!(layout.flag("swapped", cx));
        assert!(LayoutMemory::flag("swapped", cx));
        layout.set("sidebar", px(f32::NAN), cx).unwrap();
        assert_eq!(layout.get("sidebar", cx), Some(px(210.)));
    });
    cx.quit();
    let saved: Saved = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(saved.sizes["sidebar"], 210.);
    assert_eq!(saved.sizes["history"], 175.);
    assert!(saved.flags["swapped"]);
}

#[gpui_kit::test]
fn failed_layout_save_retains_the_shared_visible_arrangement(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let blocked = directory.path().join("not-a-directory");
    std::fs::write(&blocked, b"unmodified backend").unwrap();
    let observed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (layout, _observation) = cx.update(|cx| {
        let layout = LayoutMemory::init(
            Some(blocked.join("layout.json")),
            &ShutdownCoordinator::default(),
            cx,
        );
        let changes = observed.clone();
        let observation = layout.observe_size("sidebar", cx, move |value, _| {
            changes.borrow_mut().push(value);
        });
        layout.set("sidebar", px(320.), cx).unwrap();
        layout.set_flag("swapped", true, cx).unwrap();
        (layout, observation)
    });
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(layout.get("sidebar", cx), Some(px(320.)));
        assert!(layout.flag("swapped", cx));
        assert_eq!(LayoutMemory::get("sidebar", cx), Some(px(320.)));
    });
    assert_eq!(*observed.borrow(), [Some(px(320.))]);
    assert_eq!(std::fs::read(blocked).unwrap(), b"unmodified backend");
}

#[gpui_kit::test]
fn size_observation_tracks_only_the_named_shared_scalar(cx: &mut TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    let observed = Rc::new(RefCell::new(Vec::new()));
    let (layout, _subscription) = cx.update(|cx| {
        let layout = LayoutHandle::in_memory(cx);
        let same = layout.clone();
        let observed = observed.clone();
        let subscription = layout.observe_size("history", cx, move |value, cx| {
            assert_eq!(same.get("history", cx), value);
            same.set_flag("callback", true, cx).unwrap();
            observed.borrow_mut().push(value);
        });
        (layout, subscription)
    });
    cx.run_until_parked();
    assert!(observed.borrow().is_empty());
    cx.update(|cx| {
        layout.set("other", px(999.), cx).unwrap();
        layout.set_flag("right", true, cx).unwrap();
        LayoutHandle::in_memory(cx)
            .set("history", px(321.), cx)
            .unwrap();
    });
    cx.run_until_parked();
    assert!(observed.borrow().is_empty());
    cx.update(|cx| layout.set("history", px(280.4), cx).unwrap());
    cx.run_until_parked();
    assert_eq!(*observed.borrow(), [Some(px(280.))]);
    cx.update(|cx| {
        layout.set("history", px(280.1), cx).unwrap();
        layout.set("history", px(f32::NAN), cx).unwrap();
        layout.set("history", px(f32::INFINITY), cx).unwrap();
        layout.set("history", px(0.), cx).unwrap();
    });
    cx.run_until_parked();
    assert_eq!(*observed.borrow(), [Some(px(280.))]);
}

#[gpui_kit::test]
fn dropping_size_observation_releases_only_that_observer(cx: &mut TestAppContext) {
    use std::{cell::Cell, rc::Rc};
    let calls = Rc::new(Cell::new(0));
    let (layout, subscription) = cx.update(|cx| {
        let layout = LayoutHandle::in_memory(cx);
        let calls = calls.clone();
        let subscription = layout.observe_size("history", cx, move |_, _| {
            calls.set(calls.get() + 1);
        });
        (layout, subscription)
    });
    cx.update(|cx| layout.set("history", px(240.), cx).unwrap());
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
    drop(subscription);
    cx.update(|cx| layout.set("history", px(320.), cx).unwrap());
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
    cx.read(|cx| assert_eq!(layout.get("history", cx), Some(px(320.))));
}
