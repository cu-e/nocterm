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
