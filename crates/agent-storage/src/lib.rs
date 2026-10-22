//! Bounded filesystem repositories for agent history and preferences.
//! Owns: private files, transcript/catalog discovery, sidecar stamps, draft merges
//! and native chat identity/time facts.
//! Does not own: conversation rules, views, scheduling or shutdown deadlines.
//! Ports used/provided: injected session LocalFs/LocalDirectory; AI
//! AgentStateRepository, ChatRepository and
//! ChatMetadataSource.
//! Settings section: none. Globals: none.
//! Invariants: (scope, ID) selects one held namespace; v2 bounded catalog headers
//! are authoritative before sidecars. General v1 retains full bounded scans. The
//! configured private root has trusted ancestry; generated descendants never use
//! absolute operations. Project storage requires held identity; portable General
//! retains the adapter's documented active-ancestor limitation. Cache errors never
//! revoke publication.
mod catalog;
mod chats;
mod input;
mod metadata;
mod namespace;
mod publication;
pub use metadata::NativeChatMetadata;
mod state;
pub use chats::FileChatRepository;
pub use state::FileAgentStateRepository;
