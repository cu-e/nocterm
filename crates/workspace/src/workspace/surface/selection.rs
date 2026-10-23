//! Original source authority and expiring native selection receipts.
use super::*;
use gpui_kit::component::dock::NodeId;

pub(super) struct SurfaceSelection {
    source: ProjectSurfaceReceipt,
    generation: u64,
    selection_item: Option<EntityId>,
    presented: bool,
    closed: bool,
    presentation: (Option<NodeId>, bool),
}
impl SurfaceSelection {
    pub(super) fn new(source: ProjectSurfaceReceipt) -> Self {
        Self {
            source,
            generation: 0,
            selection_item: None,
            presented: source.context() == ContextId::General,
            closed: false,
            presentation: (None, false),
        }
    }
    pub(super) fn source(&self) -> ProjectSurfaceReceipt {
        self.source
    }
    pub(super) fn general(&self) -> bool {
        self.source.context() == ContextId::General
    }
    pub(super) fn closed(&self) -> bool {
        self.closed
    }
    pub(super) fn matches(&self, source: ProjectSurfaceReceipt) -> bool {
        !self.closed && self.source == source
    }
    pub(super) fn selection(&self, window_generation: u64) -> ContextSelectionReceipt {
        ContextSelectionReceipt::new(self.source, self.generation, window_generation)
    }
    pub(super) fn accepts(
        &self,
        target: ContextSelectionReceipt,
        window_generation: u64,
        presented: bool,
    ) -> bool {
        !self.closed
            && self.generation != u64::MAX
            && presented
            && self.selection(window_generation) == target
    }
    pub(super) fn validate(
        &self,
        source: ProjectSurfaceReceipt,
    ) -> Result<(), ContextSelectionError> {
        if source.window() != self.source.window() {
            return Err(ContextSelectionError::StaleWindow);
        }
        if !self.matches(source) {
            return Err(ContextSelectionError::StaleSurface);
        }
        Ok(())
    }
    pub(super) fn selected(
        &self,
        target: ContextSelectionReceipt,
        owner: &window::SurfaceWindow,
        cx: &App,
    ) -> bool {
        self.accepts(target, owner.generation(), owner.presented(self.source, cx))
    }
    pub(super) fn is_active(
        &self,
        source: ProjectSurfaceReceipt,
        owner: &window::SurfaceWindow,
        cx: &App,
    ) -> bool {
        self.matches(source) && owner.presented(source, cx)
    }
    pub(super) fn validate_source(
        &self,
        source: ProjectSurfaceReceipt,
        owner: &window::SurfaceWindow,
        window: &Window,
    ) -> Result<(), ContextSelectionError> {
        owner.check_window(window)?;
        self.validate(source)
    }
    pub(super) fn validate_general(
        &self,
        source: ProjectSurfaceReceipt,
        owner: &window::SurfaceWindow,
        window: &Window,
    ) -> Result<(), ContextSelectionError> {
        self.validate_source(source, owner, window)?;
        if !self.general() {
            return Err(ContextSelectionError::ProtectedContext);
        }
        Ok(())
    }
    pub(super) fn refresh(&mut self, tabs: &tabs::Tabs) -> bool {
        let selected = tabs
            .last_command()
            .or_else(|| tabs.active_item().map(ItemHandle::item_id));
        self.select(selected) && self.general()
    }
    pub(super) fn refresh_presentation(&mut self, presentation: (Option<NodeId>, bool)) -> bool {
        self.change_presentation(presentation) && self.general()
    }
    pub(super) fn terminal_request(
        &self,
        anchor: EntityId,
        owner: &window::SurfaceWindow,
    ) -> SurfaceEvent {
        let target = crate::LocalTerminalTarget::Beside(anchor);
        let selection = self.selection(owner.generation());
        if self.general() {
            SurfaceEvent::OpenLocal(selection, target)
        } else {
            SurfaceEvent::OpenTerminal(selection, Some(target))
        }
    }
    pub(super) fn set_presented(&mut self, presented: bool) -> bool {
        if self.closed || self.presented == presented {
            return false;
        }
        self.presented = presented;
        self.advance();
        true
    }
    pub(super) fn close(&mut self) -> bool {
        if self.closed {
            return false;
        }
        self.closed = true;
        self.presented = false;
        true
    }
    pub(super) fn select(&mut self, selected: Option<EntityId>) -> bool {
        if self.selection_item == selected {
            return false;
        }
        self.selection_item = selected;
        self.advance();
        true
    }
    pub(super) fn change_presentation(&mut self, presentation: (Option<NodeId>, bool)) -> bool {
        if self.presentation == presentation {
            return false;
        }
        self.presentation = presentation;
        if !self.general() {
            self.advance();
        }
        true
    }
    fn advance(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn selection_receipts_expire_after_hide_show_but_source_survives_until_close(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (window, workspace) = crate::workspace::tests::fixture(cx);
        cx.update_window(window, |_, _, cx| {
            let source = workspace.read(cx).general_surface_source(cx);
            let mut state = SurfaceSelection::new(source);
            let captured = state.selection(0);
            assert!(state.accepts(captured, 0, true));
            assert!(state.set_presented(false));
            assert!(state.set_presented(true));
            assert!(!state.accepts(captured, 0, true));
            assert!(state.matches(source));
            assert!(state.close());
            assert!(!state.close());
            assert!(!state.set_presented(true));
            assert!(!state.matches(source));
            assert!(state.validate(source).is_err());
        })
        .unwrap();
    }
}
