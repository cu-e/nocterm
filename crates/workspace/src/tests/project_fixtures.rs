//! Original project surface fixtures over shared ordinary session Items.
use super::*;
use nocterm_project_model::ProjectId;
pub(super) type ProjectTab = crate::test_support::ProjectSurfaceProbe;
pub(super) fn add_project(
    workspace: &Entity<Workspace>,
    id: u8,
    window: &mut Window,
    cx: &mut App,
) -> (Entity<ProjectTab>, crate::ProjectSurface) {
    let factory = workspace.read(cx).project_surface_factory();
    let tab =
        cx.new(|cx| ProjectTab::new(factory, ProjectId::from_bytes([id; 16]), window, cx).unwrap());
    let surface = tab.read(cx).surface();
    workspace.update(cx, |workspace, cx| {
        workspace
            .add_project_workspace(tab.clone(), surface.clone(), window, cx)
            .unwrap()
    });
    (tab, surface)
}
