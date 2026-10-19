//! Explicit supplied image facts/previews for feature behavior; no native decoder.
use super::*;
use nocterm_ai::images::{EncodedPreview, ImageFacts, ImageFormat, PreviewBounds, PromptImage};
use nocterm_ai::testing::ScriptedImageCodec;
pub(super) fn image_bytes() -> Vec<u8> {
    include_bytes!("fixtures/prompt.png").to_vec()
}
pub(super) fn image_codec() -> Arc<ScriptedImageCodec> {
    let codec = Arc::new(ScriptedImageCodec::default());
    let bytes = image_bytes();
    codec.inspection(&bytes, Ok(ImageFacts::new(ImageFormat::Png, 2, 2).unwrap()));
    codec.preview(
        &bytes,
        Ok(
            EncodedPreview::new(bytes.clone().into(), PreviewBounds::new(640, 640).unwrap())
                .unwrap(),
        ),
    );
    codec
}
pub(super) fn image_deps() -> crate::ImageDeps {
    let codec = image_codec();
    crate::ImageDeps::new(codec.clone(), codec)
}
pub(super) fn prompt_image() -> PromptImage {
    PromptImage::validate(image_bytes(), image_codec().as_ref()).unwrap()
}
