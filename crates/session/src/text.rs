//! Pure errors for sending text in a session's configured charset.
use crate::Charset;

/// Text cannot be represented without loss in the configured charset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Some characters cannot be sent in {}. Change the session charset or edit the text.", .0.label())]
pub struct TextEncodingError(Charset);

impl TextEncodingError {
    pub fn new(charset: Charset) -> Self {
        Self(charset)
    }

    pub fn charset(self) -> Charset {
        self.0
    }
}
