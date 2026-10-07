// Nocterm regression tests, licensed under the upstream Apache-2.0 license.
use super::*;

/// Read the copy destination before Present can rotate or discard its pixels.
/// Temporarily select the destination for the existing staging readback helper.
fn read_swap_chain(renderer: &mut DirectXRenderer) -> image::RgbaImage {
    let resources = renderer.resources.as_mut().unwrap();
    let destination = resources.swap_chain_target.as_ref().unwrap().clone();
    let source = resources.render_target.replace(destination).unwrap();
    let result = renderer.read_target_to_image();
    renderer.resources.as_mut().unwrap().render_target = Some(source);
    result.unwrap()
}

fn assert_copy_before_present(
    renderer: &mut DirectXRenderer,
    scene: &Scene,
    expected_damage: fn(SceneDamage) -> bool,
) {
    let appearance = WindowBackgroundAppearance::Opaque;
    let revision = renderer.atlas.content_revision();
    let snapshot = SceneSnapshot::capture(scene, [renderer.width, renderer.height]);
    let damage = renderer
        .retained
        .damage(&snapshot, appearance, revision, true);
    assert!(expected_damage(damage), "unexpected transition: {damage:?}");
    renderer.render_damage(scene, appearance, damage).unwrap();
    let incremental = renderer.read_target_to_image().unwrap();
    let metadata = std::mem::take(&mut renderer.retained);
    let full = renderer.render_to_image(scene, appearance).unwrap();
    renderer.retained = metadata;
    assert_eq!(incremental.as_raw(), full.as_raw());

    let resources = renderer.resources.as_ref().unwrap();
    assert!(resources.retained_enabled);
    let source = resources.render_target.as_ref().unwrap();
    let destination = resources.swap_chain_target.as_ref().unwrap();
    assert_ne!(source.as_raw(), destination.as_raw());
    let devices = renderer.devices.as_ref().unwrap();
    let mut destination_view = None;
    unsafe {
        devices
            .device
            .CreateRenderTargetView(destination, None, Some(&mut destination_view))
            .unwrap();
        devices.device_context.OMSetRenderTargets(None, None);
        devices
            .device_context
            .ClearRenderTargetView(destination_view.as_ref().unwrap(), &[1.0, 0.0, 1.0, 1.0]);
    }
    assert_ne!(
        read_swap_chain(renderer).as_raw(),
        full.as_raw(),
        "sentinel must differ before copying"
    );
    renderer.copy_retained_to_swap_chain().unwrap();
    assert_eq!(
        read_swap_chain(renderer).as_raw(),
        full.as_raw(),
        "CopyResource must update the actual swap-chain destination"
    );
    assert_eq!(
        renderer.read_target_to_image().unwrap().as_raw(),
        full.as_raw(),
        "copying must preserve the retained source"
    );
    renderer.present().unwrap();
    renderer.retained.commit(snapshot, appearance, revision);
}

#[::core::prelude::v1::test]
fn retained_copy_updates_swap_chain_before_present_for_all_damage_modes_and_resize() {
    let devices = DirectXDevices::new().unwrap();
    let (mut renderer, _window) = renderer(&devices);
    assert!(
        renderer
            .resources
            .as_ref()
            .unwrap()
            .clear_view_context
            .is_some()
    );
    for (width, height) in [(128, 96), (192, 160), (96, 80), (128, 96)] {
        renderer
            .resize(size(DevicePixels(width), DevicePixels(height)))
            .unwrap();
        assert_copy_before_present(&mut renderer, &scene(Some(40.0), true), |damage| {
            matches!(damage, SceneDamage::Full)
        });
        assert_copy_before_present(&mut renderer, &scene(Some(44.0), true), |damage| {
            matches!(damage, SceneDamage::Partial(_))
        });
        assert_copy_before_present(&mut renderer, &scene(None, true), |damage| {
            matches!(damage, SceneDamage::Partial(_))
        });
        assert_copy_before_present(&mut renderer, &scene(None, true), |damage| {
            matches!(damage, SceneDamage::Unchanged)
        });
    }
}

fn mono_pattern(renderer: &DirectXRenderer, key: AtlasKey, coverage: u8) -> AtlasTile {
    let bytes = vec![coverage; 64];
    renderer
        .atlas
        .get_or_insert_with(key, &mut || {
            Ok(Some((
                size(DevicePixels(8), DevicePixels(8)),
                Cow::Borrowed(&bytes),
            )))
        })
        .unwrap()
        .unwrap()
}

#[::core::prelude::v1::test]
fn reused_atlas_tile_with_changed_bytes_redraws_the_same_scene() {
    let devices = DirectXDevices::new().unwrap();
    let (mut renderer, _window) = renderer(&devices);
    let key = glyph_key(false, false, 77);
    let original_tile = mono_pattern(&renderer, key.clone(), 255);
    let mut scene = scene(None, false);
    scene.insert_primitive(MonochromeSprite {
        order: 0,
        pad: 0,
        bounds: rect(40.0, 40.0, 8.0, 8.0),
        content_mask: mask(),
        color: hsla(0.0, 0.0, 1.0, 1.0),
        tile: original_tile,
        transformation: TransformationMatrix::unit(),
    });
    scene.finish();
    assert_pixels(&mut renderer, &scene, WindowBackgroundAppearance::Opaque);
    let original_pixels = renderer.read_target_to_image().unwrap();
    let original_snapshot = SceneSnapshot::capture(&scene, [128, 96]);
    let revision = renderer.atlas.content_revision();
    renderer.atlas.remove(&key);
    let replacement = mono_pattern(&renderer, key, 0);
    assert_eq!(
        replacement, original_tile,
        "sole atlas texture must reuse identical tile identity and coordinates"
    );
    assert_ne!(renderer.atlas.content_revision(), revision);
    let snapshot = SceneSnapshot::capture(&scene, [128, 96]);
    assert_eq!(
        snapshot.damage_since(&original_snapshot),
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
    assert_ne!(
        updated.as_raw(),
        original_pixels.as_raw(),
        "unchanged scene signature must not hide changed atlas contents"
    );
    let full = renderer
        .render_to_image(&scene, WindowBackgroundAppearance::Opaque)
        .unwrap();
    assert_eq!(updated.as_raw(), full.as_raw());
}
