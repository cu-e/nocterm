//! A synchronous startup preparation controlled by the existing shared suspension gate.
use super::Gate;
use crate::{LaunchValidationError, ShellLaunch, ShellLaunchPreparation};
#[derive(Clone)]
pub struct ScriptedLaunchPreparation {
    entered: Gate,
    released: Gate,
}
impl Default for ScriptedLaunchPreparation {
    fn default() -> Self {
        Self {
            entered: Gate::pending(),
            released: Gate::pending(),
        }
    }
}
impl ScriptedLaunchPreparation {
    pub async fn started(&self) {
        self.entered.wait().await;
    }
    pub fn release(&self) {
        self.released.release();
    }
}
impl ShellLaunchPreparation for ScriptedLaunchPreparation {
    fn prepare(&self, launch: ShellLaunch) -> Result<ShellLaunch, LaunchValidationError> {
        self.entered.release();
        futures::executor::block_on(self.released.wait());
        launch.validate()?;
        Ok(launch)
    }
}
