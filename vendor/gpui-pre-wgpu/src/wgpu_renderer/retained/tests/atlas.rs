// Nocterm regression tests, licensed under the upstream Apache-2.0 license.
use super::*;
use gpui::{AtlasKey, Corners, ImageId, PolychromeSprite, RenderImageParams};
use std::borrow::Cow;

fn key(id: usize) -> AtlasKey {
    AtlasKey::Image(RenderImageParams {
        image_id: ImageId(id),
        frame_index: 0,
    })
}
pub(super) fn tile(
    renderer: &WgpuHeadlessRenderer,
    id: usize,
    bytes: &[u8],
) -> Result<gpui::AtlasTile> {
    Ok(renderer
        .core
        .atlas
        .get_or_insert_with(key(id), &mut || {
            Ok(Some((viewport(8, 8), Cow::Borrowed(bytes))))
        })?
        .unwrap())
}
fn image_scene(tile: gpui::AtlasTile) -> Scene {
    let mut scene = scene(false);
    scene.polychrome_sprites.push(PolychromeSprite {
        order: 3,
        pad: 0,
        grayscale: false.into(),
        opacity: 0.8,
        bounds: rect(78.0, 70.0, 16.0, 16.0),
        content_mask: ContentMask {
            bounds: rect(0., 0., 256., 192.),
        },
        corner_radii: Corners::all(ScaledPixels(0.)),
        tile,
    });
    scene.finish();
    scene
}

#[test]
fn atlas_uploads_removals_and_cache_hits_preserve_same_scene_pixels() -> Result<()> {
    let mut renderer = WgpuHeadlessRenderer::new()?;
    let first = tile(&renderer, 1, &[30; 256])?;
    let scene = image_scene(first);
    let original = retained(
        &mut renderer,
        &scene,
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    let old_revision = renderer.core.atlas.content_revision();
    // A real same-tile queued write changes pixels without changing any scene field.
    renderer
        .core
        .atlas
        .overwrite_tile_for_test(first, &[200; 256]);
    let changed = retained(
        &mut renderer,
        &scene,
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    assert_ne!(renderer.core.atlas.content_revision(), old_revision);
    assert_ne!(original.as_raw(), changed.as_raw());
    assert_eq!(
        changed.as_raw(),
        full(
            &mut renderer,
            &scene,
            viewport(256, 192),
            false,
            wgpu::Color::BLACK
        )?
        .as_raw()
    );
    let revision = renderer.core.atlas.content_revision();
    assert_eq!(tile(&renderer, 1, &[0; 256])?, first);
    assert_matches(
        &mut renderer,
        &scene,
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.atlas.content_revision(), revision);
    assert_eq!(
        renderer.core.retained.last_damage,
        Some(SceneDamage::Unchanged)
    );
    // An additive upload to the existing texture invalidates even unchanged scenes.
    let adjacent = tile(&renderer, 2, &[230; 256])?;
    assert_eq!(first.texture_id, adjacent.texture_id);
    assert_matches(
        &mut renderer,
        &scene,
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    renderer.core.atlas.remove(&key(1));
    assert_matches(
        &mut renderer,
        &scene,
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    renderer.core.atlas.clear();
    assert_matches(
        &mut renderer,
        &scene,
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    Ok(())
}
