// Nocterm regression tests, licensed under the upstream Apache-2.0 license.
use super::*;

#[::core::prelude::v1::test]
fn adjacent_atlas_upload_changes_scaled_same_scene_and_invalidates_retention() {
    let devices = DirectXDevices::new().unwrap();
    let (mut renderer, _window) = renderer(&devices);
    let original = tile(&renderer, glyph_key(false, false, 90), 1);
    renderer
        .atlas
        .initialize_texture_for_test(original, &[255; 64]);
    let mut scene = scene(None, false);
    scene.insert_primitive(MonochromeSprite {
        order: 0,
        pad: 0,
        bounds: rect(40.0, 40.0, 16.0, 16.0),
        content_mask: mask(),
        color: hsla(0.0, 0.0, 1.0, 1.0),
        tile: original,
        transformation: TransformationMatrix::unit(),
    });
    scene.finish();
    assert_pixels(&mut renderer, &scene, WindowBackgroundAppearance::Opaque);
    let original_pixels = renderer.read_target_to_image().unwrap();
    let snapshot = SceneSnapshot::capture(&scene, [128, 96]);
    let original_revision = renderer.atlas.content_revision();

    let bytes = [255; 64];
    let adjacent = renderer
        .atlas
        .get_or_insert_with(glyph_key(false, false, 91), &mut || {
            Ok(Some((
                size(DevicePixels(8), DevicePixels(8)),
                Cow::Borrowed(&bytes),
            )))
        })
        .unwrap()
        .unwrap();
    assert_eq!(original.texture_id, adjacent.texture_id);
    assert!(
        original.bounds.right() == adjacent.bounds.left()
            || original.bounds.bottom() == adjacent.bounds.top(),
        "fixture must upload immediately adjacent texels"
    );
    assert_ne!(original_revision, renderer.atlas.content_revision());
    assert_eq!(
        snapshot.damage_since(&SceneSnapshot::capture(&scene, [128, 96])),
        SceneDamage::Unchanged
    );
    assert!(matches!(
        renderer.retained.damage(
            &snapshot,
            WindowBackgroundAppearance::Opaque,
            renderer.atlas.content_revision(),
            true
        ),
        SceneDamage::Full
    ));

    renderer
        .draw(&scene, WindowBackgroundAppearance::Opaque)
        .unwrap();
    let updated = renderer.read_target_to_image().unwrap();
    let full = renderer
        .render_to_image(&scene, WindowBackgroundAppearance::Opaque)
        .unwrap();
    assert_ne!(
        original_pixels.as_raw(),
        full.as_raw(),
        "neighbor upload must actually change linear-filtered pixels of the existing scaled sprite"
    );
    assert_eq!(
        updated.as_raw(),
        full.as_raw(),
        "retention must redraw the unchanged scene after adjacent atlas upload"
    );
}
