//! Immutable optional ports; providers advertise only operations they implement.
use super::*;
use std::rc::Rc;
#[derive(Clone)]
pub struct TerminalCapabilities {
    state: Rc<dyn TerminalState>,
    fact_session: Option<Rc<dyn ConnectionFactSession>>,
    text: Option<Rc<dyn TerminalTextSource>>,
    input: Option<Rc<dyn TerminalInput>>,
    live: Option<Rc<dyn LiveCommands>>,
    execution: Option<Rc<dyn TerminalExecution>>,
    prompts: Option<Rc<dyn SignInPrompts>>,
    reconnect: Option<Rc<dyn RemoteReconnect>>,
    paste: Option<Rc<dyn UserTerminalPaste>>,
    project_parent: Option<Rc<dyn crate::ProjectParentCapture>>,
}
impl TerminalCapabilities {
    pub fn new(state: Rc<dyn TerminalState>) -> Self {
        Self {
            state,
            fact_session: None,
            text: None,
            input: None,
            live: None,
            execution: None,
            prompts: None,
            reconnect: None,
            paste: None,
            project_parent: None,
        }
    }
    pub fn with_fact_session(mut self, port: Rc<dyn ConnectionFactSession>) -> Self {
        self.fact_session = Some(port);
        self
    }
    pub fn fact_session(&self) -> Option<&Rc<dyn ConnectionFactSession>> {
        self.fact_session.as_ref()
    }
    pub fn state(&self) -> &Rc<dyn TerminalState> {
        &self.state
    }
    pub fn with_text(mut self, port: Rc<dyn TerminalTextSource>) -> Self {
        self.text = Some(port);
        self
    }
    pub fn text(&self) -> Option<&Rc<dyn TerminalTextSource>> {
        self.text.as_ref()
    }
    pub fn with_input(mut self, port: Rc<dyn TerminalInput>) -> Self {
        self.input = Some(port);
        self
    }
    pub fn input(&self) -> Option<&Rc<dyn TerminalInput>> {
        self.input.as_ref()
    }
    pub fn with_live(mut self, port: Rc<dyn LiveCommands>) -> Self {
        self.live = Some(port);
        self
    }
    pub fn live(&self) -> Option<&Rc<dyn LiveCommands>> {
        self.live.as_ref()
    }
    pub fn with_execution(mut self, port: Rc<dyn TerminalExecution>) -> Self {
        self.execution = Some(port);
        self
    }
    pub fn execution(&self) -> Option<&Rc<dyn TerminalExecution>> {
        self.execution.as_ref()
    }
    pub fn with_prompts(mut self, port: Rc<dyn SignInPrompts>) -> Self {
        self.prompts = Some(port);
        self
    }
    pub fn prompts(&self) -> Option<&Rc<dyn SignInPrompts>> {
        self.prompts.as_ref()
    }
    pub fn with_reconnect(mut self, port: Rc<dyn RemoteReconnect>) -> Self {
        self.reconnect = Some(port);
        self
    }
    pub fn reconnect(&self) -> Option<&Rc<dyn RemoteReconnect>> {
        self.reconnect.as_ref()
    }
    pub fn with_paste(mut self, port: Rc<dyn UserTerminalPaste>) -> Self {
        self.paste = Some(port);
        self
    }
    pub fn paste(&self) -> Option<&Rc<dyn UserTerminalPaste>> {
        self.paste.as_ref()
    }
    pub fn with_project_parent(mut self, port: Rc<dyn crate::ProjectParentCapture>) -> Self {
        self.project_parent = Some(port);
        self
    }
    pub fn project_parent(&self) -> Option<&Rc<dyn crate::ProjectParentCapture>> {
        self.project_parent.as_ref()
    }
}
