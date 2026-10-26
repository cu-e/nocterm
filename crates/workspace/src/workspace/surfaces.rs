//! A window retains its permanent outer dock; it never replaces that topology.
use super::*;
use surface::{SurfaceEvent, TabSurface};

pub(super) struct WindowSurfaces {
    window: Rc<surface::window::SurfaceWindow>,
    outer: Entity<TabSurface>,
    _events: Subscription,
    source: Cell<ProjectSurfaceReceipt>,
}
impl WindowSurfaces {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let owner = ContextWindowReceipt::new(cx.entity_id(), window);
        let scope = surface::window::SurfaceWindow::new(owner);
        let outer = cx.new(|cx| {
            TabSurface::new(
                scope.clone(),
                nocterm_project_model::ContextId::General,
                None,
                window,
                cx,
            )
        });
        scope.register_outer(&outer);
        let events = cx.subscribe_in(
            &outer,
            window,
            |workspace, _, event: &SurfaceEvent, window, cx| {
                workspace.surface_event(event, window, cx);
            },
        );
        let source = outer.read(cx).source();
        Self {
            window: scope,
            outer,
            _events: events,
            source: Cell::new(source),
        }
    }
    pub(super) fn owner(&self) -> ContextWindowReceipt {
        self.window.owner()
    }
    pub(super) fn outer(&self) -> Entity<TabSurface> {
        self.outer.clone()
    }
    pub(super) fn frame(&self, cx: &App) -> surface::SurfaceFrame {
        self.outer.read(cx).frame(self.outer.clone().into(), cx)
    }
    pub(super) fn window(&self) -> Rc<surface::window::SurfaceWindow> {
        self.window.clone()
    }
    pub(super) fn observe_source(&self, source: ProjectSurfaceReceipt) -> bool {
        self.source.replace(source) != source
    }
}
impl Drop for WindowSurfaces {
    fn drop(&mut self) {
        self.window.retire();
    }
}
