//! Native acquisition/publication of supplied snippet paths and UUID draft identities.
//!
//! Owns: core TOML load/save adaptation and native UUIDv4 generation.
//! Does not own: path discovery, validation/fallback policy, GUI, queues or views.
//! Ports provided: SnippetRepository, SnippetIds. Ports used: core persist load/save.
//! Settings section: none. Globals: none.
//! Invariants: exact existing core serializer/publication; original errors retained.
mod repository;
pub use repository::FileSnippetRepository;
pub struct NativeSnippetIds;
impl nocterm_snippets::SnippetIds for NativeSnippetIds {
    fn next(&self) -> uuid::Uuid {
        uuid::Uuid::new_v4()
    }
}
#[cfg(test)]
mod tests;
