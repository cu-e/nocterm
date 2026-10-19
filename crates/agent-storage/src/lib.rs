//! Bounded filesystem repositories for agent history and preferences.
//! Owns: private files, transcript/catalog discovery, sidecar stamps, draft merges
//! bounded provider account-log reads and native chat identity/time facts.
//! Does not own: conversation rules, views, scheduling or shutdown deadlines.
//! Ports used/provided: AI AgentStateRepository, ChatRepository, AccountUsageSource
//! and ChatMetadataSource.
//! Settings section: none. Globals: none.
//! Invariants: IDs are checked before paths; transcripts are authoritative; cache errors never revoke publication.
mod catalog;
mod chats;
mod loading;
mod metadata;
pub use metadata::NativeChatMetadata;
mod state;
mod usage;
pub use chats::FileChatRepository;
pub use state::FileAgentStateRepository;
pub use usage::CodexUsageReader;
