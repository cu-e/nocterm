//! Bounded filesystem repositories for agent history and preferences.
//! Owns: private files, transcript/catalog discovery, sidecar stamps, draft merges
//! and native chat identity/time facts.
//! Does not own: conversation rules, views, scheduling or shutdown deadlines.
//! Ports used/provided: injected session LocalFs/LocalDirectory; AI
//! AgentStateRepository, ChatRepository and
//! ChatMetadataSource and bounded PreparedChatLoad using core allocation leases.
//! Settings section: none. Globals: none.
//! Invariants: (environment, context, ID) selects one held v3 namespace. Bounded
//! authoritative headers precede sidecars and full reads. Old v1/v2 history is
//! ThisComputer only and retains its bytes and paths. The configured private root
//! has trusted ancestry; generated descendants use held operations. Portable
//! identities support only old General history. Cache errors never revoke publication.
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
