#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("Unsupported semantic record: {0}")]
    Unsupported(&'static str),
    #[error("Extension data exceeds the bounded document capacity")]
    ExtensionLimit,
    #[error("Reserved client metadata cannot be accepted as provider evidence")]
    ReservedMetadata,
}
