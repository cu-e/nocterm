use crate::model::notice::PlanStep;
#[derive(Clone, Debug, Default)]
pub(super) struct ThreadControls {
    plan: Option<Vec<PlanStep>>,
}
impl ThreadControls {
    pub(super) fn plan(&self) -> Option<&[PlanStep]> {
        self.plan.as_deref()
    }
    pub(super) fn replace_plan(&mut self, plan: Vec<PlanStep>) {
        self.plan = Some(plan);
    }
}
