//! Image preparation runs outside the UI and belongs to one composer revision.
use nocterm_ai::images::{ImageCollectionError, MAX_IMAGE_BYTES, PromptImage, PromptImageError};
use std::{io::Read as _, path::PathBuf};

/// Why attaching images to the composer failed.
#[derive(Debug, thiserror::Error)]
pub(super) enum ImageAttachError {
    #[error(transparent)]
    Read(#[from] std::io::Error),
    #[error(transparent)]
    Decode(#[from] base64::DecodeError),
    #[error(transparent)]
    Invalid(#[from] PromptImageError),
    #[error(transparent)]
    Collection(#[from] ImageCollectionError),
}

pub(super) fn read_images(
    paths: Vec<PathBuf>,
    inspector: &dyn nocterm_ai::images::ImageInspector,
) -> Result<Vec<PromptImage>, ImageAttachError> {
    paths
        .into_iter()
        .map(|path| {
            let mut file = std::fs::File::open(path)?;
            let mut bytes = Vec::new();
            std::io::Read::take(&mut file, (MAX_IMAGE_BYTES + 1) as u64).read_to_end(&mut bytes)?;
            Ok(PromptImage::validate(bytes, inspector)?)
        })
        .collect()
}
