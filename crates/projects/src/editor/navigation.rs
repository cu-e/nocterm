//! Relevant creation steps and validated reachability are one presentation owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum WizardStep {
    Connection,
    Project,
    Tools,
}
impl WizardStep {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Connection => "Connection",
            Self::Project => "Project",
            Self::Tools => "Tools",
        }
    }
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Connection => "project-step-connection",
            Self::Project => "project-step-project",
            Self::Tools => "project-step-tools",
        }
    }
}
pub(super) struct WizardNavigation {
    current: WizardStep,
    reached: WizardStep,
    tools: bool,
    focus: gpui_kit::FocusHandle,
}
impl WizardNavigation {
    pub(super) fn new(cx: &mut gpui_kit::App) -> Self {
        Self {
            current: WizardStep::Connection,
            reached: WizardStep::Connection,
            tools: false,
            focus: cx.focus_handle(),
        }
    }
    pub(super) fn focus(&self) -> gpui_kit::FocusHandle {
        self.focus.clone()
    }
    pub(super) fn current(&self) -> WizardStep {
        self.current
    }
    pub(super) fn steps(&self) -> &[WizardStep] {
        if self.tools {
            &[
                WizardStep::Connection,
                WizardStep::Project,
                WizardStep::Tools,
            ]
        } else {
            &[WizardStep::Connection, WizardStep::Project]
        }
    }
    pub(super) fn final_step(&self) -> bool {
        self.current == *self.steps().last().unwrap()
    }
    pub(super) fn can_visit(&self, step: WizardStep) -> bool {
        self.steps().contains(&step) && step <= self.reached
    }
    pub(super) fn visit(&mut self, step: WizardStep) {
        if self.can_visit(step) {
            self.current = step;
        }
    }
    pub(super) fn branch(&mut self, tools: bool) {
        if self.tools != tools {
            self.tools = tools;
            self.reached = self.current.min(WizardStep::Project);
            self.current = self.current.min(WizardStep::Project);
        }
    }
    pub(super) fn advance(&mut self) {
        self.current = match self.current {
            WizardStep::Connection => WizardStep::Project,
            WizardStep::Project if self.tools => WizardStep::Tools,
            step => step,
        };
        self.reached = self.reached.max(self.current);
    }
    pub(super) fn back(&mut self) {
        self.current = match self.current {
            WizardStep::Tools => WizardStep::Project,
            _ => WizardStep::Connection,
        };
    }
}
