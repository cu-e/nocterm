/// A protocol record this model cannot represent.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("Unsupported protocol record: {0}")]
    Unsupported(&'static str),
}
