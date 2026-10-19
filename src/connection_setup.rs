//! Application wiring for preloaded connection documents and native ports.
use nocterm_connections::{ConnectionWriters, EditorInputs, ObservationDeps, PreparedConnections};
use nocterm_connections_io::{FileConnectionRepository, NativeObservations, NativeValues};
use nocterm_connections_model::repository::ConnectionRepository;
use nocterm_core::Paths;
use nocterm_store::StoreError;
use std::sync::Arc;
/// Called on the existing startup background executor before bootstrap.
pub(crate) fn native(paths: &Paths) -> PreparedConnections {
    let repository: Arc<dyn ConnectionRepository> = Arc::new(FileConnectionRepository::new(
        paths.connections_file(),
        paths.recents_file(),
        paths.state_dir().join("servers.toml"),
    ));
    let documents = repository.load();
    let profiles = repository.clone();
    let recents = repository.clone();
    let facts = repository;
    let writers = ConnectionWriters::new(
        Arc::new(move |snapshot| {
            let repository = profiles.clone();
            Box::pin(async move {
                repository
                    .save_profiles(&snapshot)
                    .map_err(StoreError::backend)
            })
        }),
        Arc::new(move |snapshot| {
            let repository = recents.clone();
            Box::pin(async move {
                repository
                    .save_recents(&snapshot)
                    .map_err(StoreError::backend)
            })
        }),
        Arc::new(move |snapshot| {
            let repository = facts.clone();
            Box::pin(async move {
                repository
                    .save_facts(&snapshot)
                    .map_err(StoreError::backend)
            })
        }),
    );
    let defaults = NativeValues::editor_defaults();
    let values = Arc::new(NativeValues);
    let observations = NativeObservations::new(paths.state_dir().join("flags"));
    PreparedConnections::new(
        documents,
        writers,
        EditorInputs::new(defaults, values.clone()),
        values,
        ObservationDeps::new(
            observations.resolver(),
            observations.locator(),
            observations.flags(),
            true,
            Arc::new(nocterm_connections_io::NativeOsDetector),
        ),
    )
}
/// Inert test/no-persistence inputs; never constructs native adapters or reads environment.
#[cfg(test)]
pub(crate) fn memory() -> PreparedConnections {
    PreparedConnections::memory(nocterm_connections_model::draft::EditorDefaults {
        user: Some("tester".into()),
        home: None,
    })
}

#[cfg(test)]
mod tests;
