//! Explicit supplied image facts/previews for feature behavior; no native decoder.
use super::*;
use nocterm_ai::images::{EncodedPreview, ImageFacts, ImageFormat, PreviewBounds};
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
/// Inspects `bytes` with `codec` and adds the image to the chat's composer.
pub(super) fn attach_image(
    thread: &mut crate::thread::AgentThread,
    bytes: Vec<u8>,
    codec: Arc<ScriptedImageCodec>,
    cx: &mut gpui_kit::Context<crate::thread::AgentThread>,
) {
    let request = thread.prepare_image(&bytes).unwrap();
    let inspected = futures::executor::block_on(request.inspect(codec).expect("admitted work"));
    thread
        .accept_composer_image(inspected.expect("inspected image"), cx)
        .unwrap();
}
