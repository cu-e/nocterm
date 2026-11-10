//! Request methods reject old delivery even when a new chat reuses its key.
use super::*;
use nocterm_ai::testing::ScriptedImageCodec;
fn pipeline() -> ImagePipeline {
    let codec = Arc::new(ScriptedImageCodec::default());
    ImagePipeline::new(ImageDeps::new(codec.clone(), codec))
}
fn completed_preview() -> CompletedPreview {
    let bytes = include_bytes!("../tests/fixtures/prompt.png").to_vec();
    let bounds = nocterm_ai::images::PreviewBounds::new(640, 640).unwrap();
    let preview = nocterm_ai::images::EncodedPreview::new(bytes.into(), bounds).unwrap();
    CompletedPreview::from_encoded(preview)
}
#[test]
fn cleared_generation_cannot_install_old_success_or_failure_over_a_reused_key() {
    let mut pipeline = pipeline();
    let source = PreviewSource::Original(vec![7].into());
    let key = source.key();
    let old = pipeline.request(source.clone()).unwrap();
    pipeline.clear();
    let current = pipeline.request(source.clone()).unwrap();
    let image = completed_preview();
    assert!(!pipeline.complete(old, Some(image.clone())));
    assert!(!pipeline.complete(old, None));
    assert!(matches!(pipeline.state(&key), Some(PreviewState::Loading)));
    assert!(pipeline.complete(current, Some(image)));
    assert!(matches!(pipeline.state(&key), Some(PreviewState::Ready(_))));
    assert!(pipeline.request(source.clone()).is_none());
    assert!(!pipeline.complete(current, None));
}

#[test]
fn retained_source_prevents_aliasing_and_cloned_image_reuses_its_preview() {
    let mut pipeline = pipeline();
    let source = PreviewSource::Encoded("old image".to_owned().into());
    let key = source.key();
    let ticket = pipeline.request(source.clone()).unwrap();
    assert!(pipeline.complete(ticket, None));
    assert!(pipeline.request(source).is_none());
    let other = PreviewSource::Encoded("new image".to_owned().into());
    assert_ne!(key, other.key());
    assert!(pipeline.request(other).is_some());
}
#[test]
fn preview_admission_bounds_concurrent_decodes_and_reclaims_finished_sources() {
    let mut pipeline = pipeline();
    let first = PreviewSource::Original(vec![0; 5 * 1024 * 1024].into());
    let key = first.key();
    let ticket = pipeline.request(first.clone()).unwrap();
    assert!(pipeline.complete(ticket, None));
    for value in 1..=16 {
        let source = PreviewSource::Original(vec![value as u8; 5 * 1024 * 1024].into());
        if let Some(ticket) = pipeline.request(source) {
            assert!(pipeline.complete(ticket, None));
        }
    }
    assert!(pipeline.state(&key).is_none());
    pipeline.clear();
    for value in 0..MAX_LOADING {
        assert!(
            pipeline
                .request(PreviewSource::Original(vec![value as u8].into()))
                .is_some()
        );
    }
    assert!(
        pipeline
            .request(PreviewSource::Original(vec![99].into()))
            .is_none()
    );
}

#[test]
fn small_completed_previews_release_their_decode_reservation() {
    let mut pipeline = pipeline();
    let mut sources = Vec::new();
    for value in 0..16u8 {
        let source = PreviewSource::Original(vec![value; 1024].into());
        let ticket = pipeline.request(source.clone()).unwrap();
        let image = completed_preview();
        assert!(pipeline.complete(ticket, Some(image)));
        sources.push(source);
    }
    for source in sources {
        assert!(matches!(
            pipeline.state(&source.key()),
            Some(PreviewState::Ready(_))
        ));
    }
}
