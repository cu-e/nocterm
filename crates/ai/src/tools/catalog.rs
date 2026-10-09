//! Ordered tool definitions shared by MCP execution and transcript presentation.
use super::*;
use serde_json::{Value, json};

pub struct ToolSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
    /// Shape only: execution limits are checked separately by `validate`.
    pub parse: fn(Value) -> Option<TerminalCall>,
}

// The exhaustive name match makes each TerminalCall variant require a row here.
macro_rules! catalog {
    ($($variant:ident $(($arguments:ty))? => $name:literal, $label:literal, $description:literal;)*) => {
        enum Index { $($variant,)* }

        pub static TOOLS: &[ToolSpec] = &[$(ToolSpec {
            name: $name,
            label: $label,
            description: $description,
            schema: catalog!(@schema $($arguments)?),
            parse: catalog!(@parse $variant $($arguments)?),
        },)*];

        impl TerminalCall {
            pub fn name(&self) -> &'static str {
                match self {
                    $(catalog!(@pattern $variant $($arguments)?) => TOOLS[Index::$variant as usize].name,)*
                }
            }
        }
    };
    (@pattern $variant:ident) => { Self::$variant };
    (@pattern $variant:ident $arguments:ty) => { Self::$variant(_) };
    (@schema) => { || json!({"type":"object","properties":{},"additionalProperties":false}) };
    (@schema $arguments:ty) => { || json!(schemars::schema_for!($arguments)) };
    (@parse $variant:ident) => { |args| (args == json!({})).then_some(TerminalCall::$variant) };
    (@parse $variant:ident $arguments:ty) => {
        |args| serde_json::from_value::<$arguments>(args).ok().map(TerminalCall::$variant)
    };
}

catalog! {
    ListTerminals => "list_terminals", "List terminals",
        "List terminals attached to this chat, and attached servers that have no session yet (offline_servers). These are the only servers you can reach: nocterm holds their credentials, so never connect on your own with ssh. When the list is empty, tell the user to attach a terminal or server to the chat";
    OpenTerminal(OpenTerminal) => "open_terminal", "Connect",
        "Connect to an attached offline server in the background, without disturbing the user; returns its terminal_id once connected. If it fails, tell the user in one short sentence why and stop";
    ReadTerminal(ReadTerminal) => "read_terminal", "Read terminal",
        "Read bounded output of an attached terminal";
    SendInput(SendInput) => "send_input", "Send input",
        "Send text to an attached terminal with user approval";
    RunCommand(RunCommand) => "run_command", "Run command",
        "Type one command into a known empty live shell prompt. Requires shell integration; an output pause never proves completion. Prefer exec_command for a reliable exit status";
    ExecCommand(ExecCommand) => "exec_command", "Execute program",
        "Run program with separate args and optional stdin on the attached terminal host over its current connection. Local execution passes args directly; SSH renders a POSIX command line, so arbitrary args require a POSIX-compatible remote command shell and are not guaranteed on Windows SSH. Fresh process: no inherited terminal cwd, environment, aliases or functions. Use an explicit shell for shell syntax. Returns actual exit status or an owned command_id for read_command/cancel_command. Output is bounded and continuously drained";
    ReadCommand(ReadCommand) => "read_command", "Read command",
        "Read a bounded full snapshot of a command owned by this chat. Does not start or cancel a command; optional yield_ms waits briefly";
    CancelCommand(CancelCommand) => "cancel_command", "Cancel command",
        "Cancel a command owned by this chat. SSH sends TERM and closes its channel; remote descendant termination is best effort";
}

pub fn get(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|spec| spec.name == name)
}

#[cfg(test)]
mod tests;
