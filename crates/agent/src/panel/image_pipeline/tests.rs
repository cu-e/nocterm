//! Request methods reject old delivery even when a new chat reuses its key.
use super::*;
use nocterm_ai::testing::ScriptedImageCodec;
fn pipeline() -> ImagePipeline {
    let codec = Arc::new(ScriptedImageCodec::default());
    ImagePipeline::new(ImageDeps::new(codec.clone(), codec))
}
#[test]
fn cleared_generation_cannot_install_old_success_or_failure_over_a_reused_key() {
    let mut pipeline = pipeline();
    let key = (0, 0, false);
    let old = pipeline.request(key).unwrap();
    pipeline.clear();
    let current = pipeline.request(key).unwrap();
    let image = Arc::new(Image::from_bytes(gpui_kit::ImageFormat::Png, vec![1]));
    assert!(!pipeline.complete(old, Some(image.clone())));
    assert!(!pipeline.complete(old, None));
    assert!(matches!(pipeline.state(&key), Some(PreviewState::Loading)));
    assert!(pipeline.complete(current, Some(image)));
    assert!(matches!(pipeline.state(&key), Some(PreviewState::Ready(_))));
    assert!(pipeline.request(key).is_none());
    assert!(!pipeline.complete(current, None));
}
