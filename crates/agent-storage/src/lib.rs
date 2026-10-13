//! Bounded filesystem repositories for agent history and preferences.
//! Owns: private files, transcript/catalog discovery, sidecar stamps, draft merges
//! and bounded provider account-log reads.
//! Does not own: conversation rules, views, scheduling or shutdown deadlines.
//! Ports used/provided: AI AgentStateRepository, ChatRepository and AccountUsageSource.
//! Settings section: none. Globals: none.
//! Invariants: IDs are checked before paths; transcripts are authoritative; cache errors never revoke publication.
mod catalog;
mod chats;
mod loading;
mod state;
mod usage;
pub use chats::FileChatRepository;
pub use state::FileAgentStateRepository;
pub use usage::CodexUsageReader;
