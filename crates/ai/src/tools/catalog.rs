//! Ordered tool definitions shared by MCP execution and transcript presentation.
use super::*;
use serde_json::{Value, json};

pub struct ToolSpec {
    pub name: &'static str,
    pub label: &'static str,
    /// Terminal tools are always listed; file tools only while a chat offers them.
    pub group: ToolGroup,
    pub description: &'static str,
    pub schema: fn() -> Value,
    /// Shape only: execution limits are checked separately by `validate`.
    pub parse: fn(Value) -> Option<TerminalCall>,
}

/// The tools an agent is given together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolGroup {
    /// The terminals, commands and transfers of a chat: always offered.
    Terminals,
    /// The files of attached terminals that serve files but run no programs.
    Files,
}

/// What a chat offers its agent now beyond the terminal tools. The bridge lists and admits only
/// these, and tells a connected agent when the offer changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolOffer {
    files: bool,
}

impl ToolOffer {
    /// The terminal tools and, when `files` is true, the file tools.
    pub fn new(files: bool) -> Self {
        Self { files }
    }

    pub fn offers(self, group: ToolGroup) -> bool {
        match group {
            ToolGroup::Terminals => true,
            ToolGroup::Files => self.files,
        }
    }

    /// The tools listed under this offer, in catalog order.
    pub fn tools(self) -> impl Iterator<Item = &'static ToolSpec> {
        TOOLS.iter().filter(move |spec| self.offers(spec.group))
    }
}

// The exhaustive name match makes each TerminalCall variant require a row here.
macro_rules! catalog {
    ($($variant:ident $(($arguments:ty))? => $name:literal, $label:literal, $group:ident, $description:literal;)*) => {
        enum Index { $($variant,)* }

        pub static TOOLS: &[ToolSpec] = &[$(ToolSpec {
            name: $name,
            label: $label,
            group: ToolGroup::$group,
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
            pub fn group(&self) -> ToolGroup {
                match self {
                    $(catalog!(@pattern $variant $($arguments)?) => TOOLS[Index::$variant as usize].group,)*
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
    ListTerminals => "list_terminals", "List terminals", Terminals,
        "List terminals attached to this chat, and attached servers that have no session yet (offline_servers). These are the only servers you can reach: nocterm holds their credentials, so never connect on your own with ssh. When the list is empty, tell the user to attach a terminal or server to the chat";
    OpenTerminal(OpenTerminal) => "open_terminal", "Connect", Terminals,
        "Connect to an attached offline server in the background, without disturbing the user; returns its terminal_id once connected. If it fails, tell the user in one short sentence why and stop";
    ReadTerminal(ReadTerminal) => "read_terminal", "Read terminal", Terminals,
        "Read bounded output of an attached terminal";
    SendInput(SendInput) => "send_input", "Send input", Terminals,
        "Send text to an attached terminal with user approval";
    RunCommand(RunCommand) => "run_command", "Run command", Terminals,
        "Type one command into a known empty live shell prompt. Requires shell integration; an output pause never proves completion. Prefer exec_command for a reliable exit status";
    ExecCommand(ExecCommand) => "exec_command", "Execute program", Terminals,
        "Run program with separate args and optional stdin on the attached terminal host over its current connection. Local execution passes args directly; SSH renders a POSIX command line, so arbitrary args require a POSIX-compatible remote command shell and are not guaranteed on Windows SSH. Fresh process: no inherited terminal cwd, environment, aliases or functions. Use an explicit shell for shell syntax. Returns actual exit status or an owned command_id for read_command/cancel_command. Output is bounded and continuously drained";
    ReadCommand(ReadCommand) => "read_command", "Read command", Terminals,
        "Read a bounded full snapshot of a command owned by this chat. Does not start or cancel a command; optional yield_ms waits briefly";
    CancelCommand(CancelCommand) => "cancel_command", "Cancel command", Terminals,
        "Cancel a command owned by this chat. SSH sends TERM and closes its channel; remote descendant termination is best effort";
    TransferFiles(TransferFiles) => "transfer_files", "Transfer files", Terminals,
        "Copy files and folders between the hosts of two attached terminals: this computer (a local terminal) to a server, a server to this computer, or a server to another server. nocterm relays the data over the connections it already holds, so servers need no credentials for each other and you must never use scp, rsync or ssh between them. Every path is absolute (no ~, no .. components). sources are files or folders on the source host; folders copy recursively, and each source is copied into destination under its own name. destination must be an existing folder; it is never created. Existing files are skipped and counted as skipped unless overwrite is true. Symbolic links and special files are skipped. At most 64 sources per call. Two local terminals are refused: use exec_command. Some environments only allow copies within one server. The copy runs in the background: after optional yield_ms (default 1000) it returns the current state and a transfer_id for read_transfer/cancel_transfer. Poll read_transfer until the state is completed, failed or cancelled. Whole files are published atomically; a cancelled or failed file leaves no partial file";
    ReadTransfer(ReadTransfer) => "read_transfer", "Read transfer", Terminals,
        "Read a snapshot of a transfer started by this chat: state, file and byte counts and up to 16 failures. Does not start or cancel a transfer; optional yield_ms waits briefly while it is active";
    CancelTransfer(CancelTransfer) => "cancel_transfer", "Cancel transfer", Terminals,
        "Cancel a transfer started by this chat. Files already published stay in place; files being copied are discarded";
    ListFiles(ListFiles) => "list_files", "List files", Files,
        "List a folder of an attached terminal whose list_terminals entry has file_tools: true. Such a terminal serves files but runs no programs (for example a console tab a plugin opened for a game server panel): exec_command and run_command cannot reach its files, so use the file tools for them. Paths are absolute from / with no ~, . or .. parts. Folders come first; at most 1000 entries. The file tools act only on terminals with file_tools: true, never on other terminals or this computer";
    ReadFile(ReadFile) => "read_file", "Read file", Files,
        "Read a UTF-8 text file of an attached terminal with file_tools: true, up to 1 MiB, a range of lines at a time (start_line from 1, max_lines up to 2000, at most 64 KiB per answer; next_start_line says where to continue). The content is exactly the file's text, except where it is shown as [REDACTED]: such text is hidden from you, so change other parts with edit_file and never write the marker back";
    WriteFile(WriteFile) => "write_file", "Write file", Files,
        "Write a whole UTF-8 text file (at most 512 KiB) on an attached terminal with file_tools: true, with user approval. An existing file is refused unless overwrite is true; prefer edit_file to change part of a file. The parent folder must exist";
    EditFile(EditFile) => "edit_file", "Edit file", Files,
        "Replace exact text in a file of an attached terminal with file_tools: true, with user approval. old_text must occur exactly once, copied exactly from read_file with its whitespace, unless replace_all is true. Send only the part that changes; the file is changed on its host";
    MakeDirectory(MakeDirectory) => "make_directory", "Create folder", Files,
        "Create a folder on an attached terminal with file_tools: true, with user approval. Its parent must exist; an existing folder is accepted";
    DeletePath(DeletePath) => "delete_path", "Delete", Files,
        "Delete a file or folder on an attached terminal with file_tools: true, with user approval. A folder is removed as the terminal's host removes folders, which may include its contents; it cannot be undone";
    MovePath(MovePath) => "move_path", "Move", Files,
        "Move or rename a file or folder within one attached terminal with file_tools: true, with user approval. Both paths are absolute";
}

pub fn get(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|spec| spec.name == name)
}

#[cfg(test)]
mod tests;
