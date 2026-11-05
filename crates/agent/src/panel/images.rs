//! Image preparation runs outside the UI and belongs to one composer revision.
use nocterm_ai::images::{
    ImageCollectionBudget, ImageCollectionError, MAX_IMAGE_BYTES, PromptImage, PromptImageError,
};
use std::{io::Read as _, path::PathBuf};

/// Why attaching images to the composer failed.
#[derive(Debug, thiserror::Error)]
pub(super) enum ImageAttachError {
    #[error(transparent)]
    Conversation(#[from] nocterm_agent_chat::conversation::ConversationError),
    #[error(transparent)]
    Read(#[from] std::io::Error),
    #[error(transparent)]
    Decode(#[from] base64::DecodeError),
    #[error(transparent)]
    Invalid(#[from] PromptImageError),
    #[error(transparent)]
    Collection(#[from] ImageCollectionError),
    #[error("Image processing is busy. Try again when the current images are ready.")]
    Busy,
}

pub(super) fn read_images(
    paths: Vec<PathBuf>,
    inspector: &dyn nocterm_ai::images::ImageInspector,
    mut budget: ImageCollectionBudget,
) -> Result<Vec<PromptImage>, ImageAttachError> {
    budget.check_count(paths.len())?;
    paths
        .into_iter()
        .map(|path| {
            let mut file = std::fs::File::open(path)?;
            let mut bytes = Vec::new();
            std::io::Read::take(&mut file, (MAX_IMAGE_BYTES + 1) as u64).read_to_end(&mut bytes)?;
            budget.admit(bytes.len())?;
            Ok(PromptImage::validate(bytes, inspector)?)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_ai::testing::ScriptedImageCodec;
    #[test]
    fn excessive_selection_is_refused_before_opening_any_path() {
        let codec = ScriptedImageCodec::default();
        let budget = ImageCollectionBudget::remaining(&[]).unwrap();
        let paths = vec![PathBuf::from("missing-image.png"); nocterm_ai::images::MAX_IMAGES + 1];
        assert!(matches!(
            read_images(paths, &codec, budget),
            Err(ImageAttachError::Collection(_))
        ));
        assert!(codec.requests().is_empty());
    }
    #[test]
    fn aggregate_bytes_are_refused_before_the_next_decode() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("image.png");
        std::fs::write(&path, b"uninspected bytes").unwrap();
        let codec = ScriptedImageCodec::default();
        let mut budget = ImageCollectionBudget::remaining(&[]).unwrap();
        budget
            .admit(nocterm_ai::images::MAX_TOTAL_IMAGE_BYTES)
            .unwrap();
        assert!(matches!(
            read_images(vec![path], &codec, budget),
            Err(ImageAttachError::Collection(_))
        ));
        assert!(codec.requests().is_empty());
    }
}
