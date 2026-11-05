//! An inline image carried by a Cloud prompt. Consumers validate image data before admission;
//! the adapter maps this neutral vocabulary to the protocol's existing inline image parts.
use std::{fmt, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMime {
    Png,
    Jpeg,
    Gif,
    Webp,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PromptImage {
    pub mime: ImageMime,
    pub bytes: Arc<[u8]>,
}
impl fmt::Debug for PromptImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PromptImage")
            .field("mime", &self.mime)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}
