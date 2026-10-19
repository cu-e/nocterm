//! Local selection policy; placement remains in native topology.
use super::super::*;
use nocterm_ui::ActiveDesign as _;

#[derive(Default)]
pub(super) struct LocalDockState {
    selected: Option<EntityId>,
    height: Option<Pixels>,
    header_origin: Rc<Cell<Option<EntityId>>>,
}
impl LocalDockState {
    pub(super) fn selected(&self) -> Option<EntityId> {
        self.selected
    }
    pub(super) fn select(&mut self, id: Option<EntityId>) {
        self.selected = id;
    }
    pub(super) fn remember_height(&mut self, height: Option<Pixels>) {
        self.height = height;
    }
    pub(super) fn height(&self, window: &Window, cx: &App) -> Pixels {
        self.height.unwrap_or_else(|| {
            rems(cx.design().layout.local_terminal_height).to_pixels(window.rem_size())
        })
    }
    pub(super) fn header_origin(&self) -> Rc<Cell<Option<EntityId>>> {
        self.header_origin.clone()
    }
}
