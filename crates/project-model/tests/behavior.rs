mod support;

#[path = "behavior/catalog.rs"]
mod catalog;
#[path = "behavior/catalog_state.rs"]
mod catalog_state;
#[path = "behavior/identity.rs"]
mod identity;
#[cfg(feature = "test-support")]
#[path = "behavior/repository.rs"]
mod repository;
