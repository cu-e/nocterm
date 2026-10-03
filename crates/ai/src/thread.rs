use crate::acp;
#[derive(Clone, Debug)]
pub enum Entry {
    User(Vec<acp::ContentBlock>),
    Agent(String),
    Thought(String),
    Content(acp::ContentBlock),
    Tool(acp::ToolCall),
}
#[derive(Clone, Debug, Default)]
pub struct ThreadState {
    pub entries: Vec<Entry>,
    pub plan: Option<acp::Plan>,
    pub modes: Option<acp::SessionModeState>,
    pub current_mode: Option<acp::SessionModeId>,
    pub config_options: Vec<acp::SessionConfigOption>,
    pub usage: Option<acp::UsageUpdate>,
    pub commands: Vec<acp::AvailableCommand>,
    pub title: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadChange {
    Transcript,
    Controls,
    Metadata,
    Ignored,
}
impl ThreadState {
    pub fn push_user(&mut self, content: Vec<acp::ContentBlock>) {
        self.entries.push(Entry::User(content));
    }
    pub fn apply(&mut self, update: acp::SessionUpdate) -> ThreadChange {
        match update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => {
                self.chunk(chunk.content, false);
                ThreadChange::Transcript
            }
            acp::SessionUpdate::AgentThoughtChunk(chunk) => {
                self.chunk(chunk.content, true);
                ThreadChange::Transcript
            }
            acp::SessionUpdate::UserMessageChunk(chunk) => {
                if let Some(Entry::User(parts)) = self.entries.last_mut() {
                    parts.push(chunk.content);
                } else {
                    self.push_user(vec![chunk.content]);
                }
                ThreadChange::Transcript
            }
            acp::SessionUpdate::ToolCall(call) => {
                self.entries.push(Entry::Tool(call));
                ThreadChange::Transcript
            }
            acp::SessionUpdate::ToolCallUpdate(update) => {
                if let Some(Entry::Tool(call)) = self
                    .entries
                    .iter_mut()
                    .find(|e| matches!(e,Entry::Tool(t) if t.tool_call_id==update.tool_call_id))
                {
                    let f = update.fields;
                    if let Some(v) = f.kind {
                        call.kind = v;
                    }
                    if let Some(v) = f.status {
                        call.status = v;
                    }
                    if let Some(v) = f.title {
                        call.title = v;
                    }
                    if let Some(v) = f.name {
                        call.name = Some(v);
                    }
                    if let Some(v) = f.content {
                        call.content = v;
                    }
                    if let Some(v) = f.locations {
                        call.locations = v;
                    }
                    if let Some(v) = f.raw_input {
                        call.raw_input = Some(v);
                    }
                    if let Some(v) = f.raw_output {
                        call.raw_output = Some(v);
                    }
                }
                ThreadChange::Transcript
            }
            acp::SessionUpdate::Plan(plan) => {
                self.plan = Some(plan);
                ThreadChange::Transcript
            }
            acp::SessionUpdate::AvailableCommandsUpdate(update) => {
                self.commands = update.available_commands;
                ThreadChange::Controls
            }
            acp::SessionUpdate::CurrentModeUpdate(update) => {
                self.current_mode = Some(update.current_mode_id);
                ThreadChange::Controls
            }
            acp::SessionUpdate::ConfigOptionUpdate(update) => {
                self.config_options = update.config_options;
                ThreadChange::Controls
            }
            acp::SessionUpdate::SessionInfoUpdate(update) => {
                match update.title {
                    agent_client_protocol_schema::MaybeUndefined::Value(value) => {
                        self.title = Some(value)
                    }
                    agent_client_protocol_schema::MaybeUndefined::Null => self.title = None,
                    _ => {}
                }
                ThreadChange::Metadata
            }
            acp::SessionUpdate::UsageUpdate(update) => {
                self.usage = Some(update);
                ThreadChange::Controls
            }
            _ => ThreadChange::Ignored,
        }
    }
    fn chunk(&mut self, content: acp::ContentBlock, thought: bool) {
        if let acp::ContentBlock::Text(text) = content {
            match (self.entries.last_mut(), thought) {
                (Some(Entry::Agent(value)), false) | (Some(Entry::Thought(value)), true) => {
                    value.push_str(&text.text)
                }
                _ => self.entries.push(if thought {
                    Entry::Thought(text.text)
                } else {
                    Entry::Agent(text.text)
                }),
            }
        } else {
            self.entries.push(Entry::Content(content));
        }
    }
    pub fn config(
        &self,
        category: acp::SessionConfigOptionCategory,
    ) -> Vec<&acp::SessionConfigOption> {
        self.config_options
            .iter()
            .filter(|v| v.category.as_ref() == Some(&category))
            .collect()
    }
    pub fn config_overrides_modes(&self) -> bool {
        self.config_options
            .iter()
            .any(|v| v.category == Some(acp::SessionConfigOptionCategory::Mode))
    }
}
