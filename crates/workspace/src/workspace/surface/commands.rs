//! Initial opening issuance, folder metadata and inert monitoring identity.
use super::*;

#[derive(Default)]
pub(super) struct SurfaceState {
    initial_terminal: bool,
    root: Option<RootPath>,
    monitor_target: Option<ProjectTarget>,
}
impl SurfaceState {
    pub(super) fn monitor_target(&self) -> Option<ProjectTarget> {
        self.monitor_target.clone()
    }
    pub(super) fn set_monitor_target(&mut self, target: ProjectTarget) -> bool {
        if self.monitor_target.as_ref() == Some(&target) {
            return false;
        }
        self.monitor_target = Some(target);
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
        registered_project: bool,
        selection: ContextSelectionReceipt,
    ) -> Result<ContextSelectionReceipt, ContextSelectionError> {
        if !registered_project {
            return Err(ContextSelectionError::StaleSurface);
        }
        if self.initial_terminal {
            return Err(ContextSelectionError::InitialTerminalAlreadyRequested);
        }
        self.initial_terminal = true;
        Ok(selection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn only_a_registered_project_can_issue_its_initial_terminal_once(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (window, workspace) = crate::workspace::tests::fixture(cx);
        cx.update_window(window, |_, window, cx| {
            let general = workspace.read(cx).general_surface_source(cx);
            assert!(
                SurfaceState::default()
                    .initial_terminal(false, ContextSelectionReceipt::new(general, 0, 0))
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
            let mut state = SurfaceState::default();
            let selection = project.selection_receipt(cx);
            assert!(state.initial_terminal(false, selection).is_err());
            assert!(state.initial_terminal(true, selection).is_ok());
            assert!(matches!(
                state.initial_terminal(true, selection),
                Err(ContextSelectionError::InitialTerminalAlreadyRequested)
            ));
        })
        .unwrap();
    }
}
