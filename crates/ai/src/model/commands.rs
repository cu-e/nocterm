/// A slash command the agent advertises.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableCommand {
    pub name: String,
    pub description: String,
    /// The hint for the single free-form argument, if the command takes one.
    pub hint: Option<String>,
}
impl AvailableCommand {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            hint: None,
        }
    }
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}
