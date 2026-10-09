use super::*;
use gpui_kit::TestAppContext;

#[gpui_kit::test]
fn queued_layout_snapshots_cannot_overwrite_the_latest_arrangement(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("layout.json");
    cx.update(|cx| {
        LayoutMemory::init(Some(file.clone()), cx);
        LayoutMemory::set("panel", px(100.), cx);
        LayoutMemory::set("panel", px(200.), cx);
        LayoutMemory::set_flag("right", true, cx);
        LayoutMemory::set("panel", px(300.), cx);
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
        LayoutMemory::init(Some(file.clone()), cx);
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
async fn blocked_disk_keeps_one_worker_and_coalesces_a_thousand_edits(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("layout.json");
    cx.update(|cx| LayoutMemory::init(Some(file.clone()), cx));
    let (gate, captures) = cx.update(|cx| {
        let memory = cx.global::<LayoutMemory>();
        (memory.gate.clone(), memory.captures.clone())
    });
    let active_disk = gate.normal().await.unwrap();
    cx.update(|cx| LayoutMemory::set("panel", px(100.), cx));
    cx.run_until_parked();
    assert_eq!(captures.load(Ordering::Relaxed), 1);
    for value in 101..=1100 {
        cx.update(|cx| LayoutMemory::set("panel", px(value as f32), cx));
    }
    cx.run_until_parked();
    assert_eq!(
        captures.load(Ordering::Relaxed),
        1,
        "edits must replace current state instead of queuing snapshot tasks"
    );
    drop(active_disk);
    cx.run_until_parked();
    assert_eq!(captures.load(Ordering::Relaxed), 2);
    let saved: Saved = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(saved.sizes["panel"], 1100.);
}

#[gpui_kit::test]
fn unchanged_layout_is_not_rewritten_at_quit(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("layout.json");
    std::fs::write(&file, "invalid untouched layout").unwrap();
    cx.update(|cx| LayoutMemory::init(Some(file.clone()), cx));
    cx.quit();
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "invalid untouched layout"
    );
}
