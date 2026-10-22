//! Explorer follows each workspace's ordinary session and initial folder.
use super::*;
use nocterm_project_model::RootPath;
#[gpui_kit::test]
fn folder_metadata_does_not_expose_another_workspaces_files(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (outer, a) = add_project(&workspace, 1, window, cx);
        a.set_root(RootPath::parse("/root").unwrap(), cx);
        let (_, b) = add_project(&workspace, 2, window, cx);
        b.set_root(RootPath::parse("/").unwrap(), cx);
        let source = workspace.read(cx).explorer_source(cx).unwrap();
        assert_eq!(source.project_id(), Some(b.project()));
        assert!(source.filesystem().is_none());
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(outer.entity_id(), window, cx);
        });
        assert_eq!(
            workspace.read(cx).explorer_source(cx).unwrap().project_id(),
            Some(a.project())
        );
    })
    .unwrap();
}
