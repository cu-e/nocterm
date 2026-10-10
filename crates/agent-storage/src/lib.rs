//! Bounded filesystem repositories for agent history and preferences.
//! Owns: private files, transcript/catalog discovery, sidecar stamps and draft merges.
//! Does not own: conversation rules, views, scheduling or shutdown deadlines.
//! Ports used/provided: AI AgentStateRepository and ChatRepository.
//! Settings section: none. Globals: none.
//! Invariants: IDs are checked before paths; transcripts are authoritative; cache errors never revoke publication.
mod catalog;
mod chats;
mod loading;
mod state;
pub use chats::FileChatRepository;
pub use state::FileAgentStateRepository;
