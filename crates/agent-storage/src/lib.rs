//! Bounded filesystem repositories for agent history and preferences.
//! Owns: private files, transcript/catalog discovery, sidecar stamps, draft merges
//! bounded provider account-log reads and native chat identity/time facts.
//! Does not own: conversation rules, views, scheduling or shutdown deadlines.
//! Ports used/provided: AI AgentStateRepository, ChatRepository, AccountUsageSource
//! and ChatMetadataSource.
//! Settings section: none. Globals: none.
//! Invariants: IDs are checked before paths; transcripts are authoritative; cache errors never revoke publication.
//! Feature `test-support` exposes actual historical decoders for app-owned fixtures.
//! Selected borrowed serialization is bounded before shared private-stage publication.
//! Legacy grammar supplies authentic borrowed extension allocation components;
//! its scalar admission policy and publication bytes remain separate and unchanged.
mod catalog;
mod chats;
mod format;
mod legacy;
#[cfg(feature = "test-support")]
pub use legacy::test_support;
mod loading;
mod metadata;
mod writing;
pub use metadata::NativeChatMetadata;
mod state;
mod usage;
pub use chats::FileChatRepository;
pub use state::FileAgentStateRepository;
pub use usage::CodexUsageReader;

#[cfg(test)]
mod history_fixtures;
