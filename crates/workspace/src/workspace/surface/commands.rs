//! Source identity, lifecycle and UI receipt transitions without entity access.
use super::*;
use gpui_kit::component::dock::NodeId;

pub(super) struct SurfaceState {
    source: ProjectSurfaceReceipt,
    generation: u64,
    selection_item: Option<EntityId>,
    presented: bool,
    closed: bool,
    presentation: (Option<NodeId>, bool),
    initial_terminal: bool,
    root: Option<RootPath>,
}
impl SurfaceState {
    pub(super) fn new(source: ProjectSurfaceReceipt) -> Self {
        Self {
            source,
            generation: 0,
            selection_item: None,
            presented: source.context() == ContextId::General,
            closed: false,
            presentation: (None, false),
            initial_terminal: false,
            root: None,
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
    pub(super) fn root(&self) -> Option<RootPath> {
        self.root.clone()
    }
    pub(super) fn set_root(&mut self, root: RootPath) -> bool {
        if self.root.as_ref() == Some(&root) {
            return false;
        }
        self.root = Some(root);
        true
    }
    pub(super) fn initial_terminal(
        &mut self,
        registered: bool,
        window_generation: u64,
    ) -> Result<ContextSelectionReceipt, ContextSelectionError> {
        self.validate(self.source)?;
        if self.general() || !registered {
            return Err(ContextSelectionError::StaleSurface);
        }
        if self.initial_terminal {
            return Err(ContextSelectionError::InitialTerminalAlreadyRequested);
        }
        self.initial_terminal = true;
        Ok(self.selection(window_generation))
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
            let mut state = SurfaceState::new(source);
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

    #[gpui_kit::test]
    fn only_a_registered_project_can_issue_its_initial_terminal_once(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (window, workspace) = crate::workspace::tests::fixture(cx);
        cx.update_window(window, |_, window, cx| {
            let general = workspace.read(cx).general_surface_source(cx);
            assert!(
                SurfaceState::new(general)
                    .initial_terminal(true, 0)
                    .is_err()
            );
            let outer = cx.new(|_| ());
            let project = workspace
                .read(cx)
                .project_surface_factory()
                .create(
                    nocterm_project_model::ProjectId::from_bytes([1; 16]),
                    outer.entity_id(),
                    window,
                    cx,
                )
                .unwrap();
            let mut state = SurfaceState::new(project.source(cx));
            assert!(state.initial_terminal(false, 0).is_err());
            assert!(state.initial_terminal(true, 0).is_ok());
            assert!(matches!(
                state.initial_terminal(true, 0),
                Err(ContextSelectionError::InitialTerminalAlreadyRequested)
            ));
        })
        .unwrap();
    }
}
