//! A captured permanent outer destination precedes ordinary native launch.
use super::*;

pub struct GeneralTerminalTarget {
    owner: Rc<surface::window::SurfaceWindow>,
    selection: ContextSelectionReceipt,
}
impl GeneralTerminalTarget {
    pub(super) fn new(
        owner: Rc<surface::window::SurfaceWindow>,
        selection: ContextSelectionReceipt,
        window: &Window,
        cx: &App,
    ) -> Result<Self, crate::ContextSelectionError> {
        let target = Self { owner, selection };
        target.validate(window, cx)?;
        Ok(target)
    }
    pub fn selection(&self) -> ContextSelectionReceipt {
        self.selection
    }
    pub fn validate(&self, window: &Window, cx: &App) -> Result<(), crate::ContextSelectionError> {
        self.owner.check_window(window)?;
        if self.selection.surface().context() != nocterm_project_model::ContextId::General {
            return Err(crate::ContextSelectionError::ProtectedContext);
        }
        self.owner.resolve(self.selection.surface(), cx)?;
        Ok(())
    }
}
