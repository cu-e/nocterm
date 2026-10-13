//! Immutable optional ports; providers advertise only operations they implement.
use super::*;
use std::rc::Rc;
#[derive(Clone)]
pub struct TerminalCapabilities {
    state: Rc<dyn TerminalState>,
    text: Option<Rc<dyn TerminalTextSource>>,
    input: Option<Rc<dyn TerminalInput>>,
    live: Option<Rc<dyn LiveCommands>>,
    execution: Option<Rc<dyn TerminalExecution>>,
    prompts: Option<Rc<dyn SignInPrompts>>,
    reconnect: Option<Rc<dyn RemoteReconnect>>,
    paste: Option<Rc<dyn UserTerminalPaste>>,
}
impl TerminalCapabilities {
    pub fn new(state: Rc<dyn TerminalState>) -> Self {
        Self {
            state,
            text: None,
            input: None,
            live: None,
            execution: None,
            prompts: None,
            reconnect: None,
            paste: None,
        }
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
}
