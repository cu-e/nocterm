// Nocterm regression tests, licensed under the upstream Apache-2.0 license.
use super::*;
use gpui::{FontId, GlyphId, MonochromeSprite, RenderGlyphParams, TransformationMatrix, px};
use std::borrow::Cow;

fn fallback_pixels(
    renderer: &mut WgpuHeadlessRenderer,
    scene: &Scene,
    dimensions: Size<DevicePixels>,
) -> Result<()> {
    let expected = full(renderer, scene, dimensions, false, wgpu::Color::BLACK)?;
    let texture = renderer
        .core
        .resources
        .device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("unsupported_scene_direct_destination"),
            size: wgpu::Extent3d {
                width: dimensions.width.0 as u32,
                height: dimensions.height.0 as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: renderer.core.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
    let view = texture.create_view(&Default::default());
    let mut encoder = renderer
        .core
        .resources
        .device
        .create_command_encoder(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::GREEN),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            ..Default::default()
        });
    }
    renderer.core.resources.queue.submit([encoder.finish()]);
    renderer.core.render_retained_surface(
        scene,
        &texture,
        dimensions,
        false,
        wgpu::Color::BLACK,
    )?;
    assert!(renderer.core.retained.snapshot.is_none());
    assert_eq!(renderer.core.retained.last_damage, None);
    let saved = renderer.render_target.take();
    renderer.render_target = Some(HeadlessRenderTarget { texture, view });
    let actual = renderer.read_image();
    renderer.render_target = saved;
    assert_eq!(actual?.as_raw(), expected.as_raw());
    renderer.check_gpu_errors()?;
    Ok(())
}

#[test]
fn transformed_scene_uses_direct_full_without_allocating_or_resizing_retained_image() -> Result<()>
{
    let mut renderer = WgpuHeadlessRenderer::new()?;
    let bytes = [200; 64];
    let tile = renderer
        .core
        .atlas
        .get_or_insert_with(
            RenderGlyphParams {
                font_id: FontId(0),
                glyph_id: GlyphId(77),
                font_size: px(8.),
                subpixel_variant: point(0, 0),
                scale_factor: 1.,
                is_emoji: false,
                subpixel_rendering: false,
                dilation: 0,
            }
            .into(),
            &mut || Ok(Some((viewport(8, 8), Cow::Borrowed(&bytes)))),
        )?
        .unwrap();
    let mut unsupported = scene(false);
    unsupported.monochrome_sprites.push(MonochromeSprite {
        order: 10,
        pad: 0,
        bounds: rect(70., 70., 16., 16.),
        content_mask: ContentMask {
            bounds: rect(0., 0., 256., 192.),
        },
        color: hsla(0.1, 0.8, 0.6, 1.),
        tile,
        transformation: TransformationMatrix {
            translation: [-25., 0.],
            ..TransformationMatrix::unit()
        },
    });
    unsupported.finish();
    assert!(!SceneSnapshot::capture(&unsupported, [256, 192]).supports_partial_updates());
    let glyph_pixels = full(
        &mut renderer,
        &unsupported,
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    let plain_pixels = full(
        &mut renderer,
        &scene(false),
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_ne!(glyph_pixels.as_raw(), plain_pixels.as_raw());
    fallback_pixels(&mut renderer, &unsupported, viewport(256, 192))?;
    assert!(
        renderer.core.retained.image.is_none(),
        "unsupported first frame must not allocate retention"
    );
    assert_matches(
        &mut renderer,
        &scene(false),
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    fallback_pixels(&mut renderer, &unsupported, viewport(128, 128))?;
    assert_eq!(
        renderer
            .core
            .retained
            .image
            .as_ref()
            .unwrap()
            .texture
            .width(),
        256,
        "unsupported resized frame must not replace retained resources"
    );
    assert_matches(
        &mut renderer,
        &scene(false),
        viewport(128, 128),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    Ok(())
}

#[test]
fn exhausted_record_budget_uses_original_direct_full_without_retained_allocation() -> Result<()> {
    let mut renderer = WgpuHeadlessRenderer::new()?;
    let mut scene = Scene::default();
    // Zero-area quads exercise the record cap without excessive fragment overdraw.
    scene.quads = vec![quad(0, rect(0., 0., 0., 0.), hsla(0., 0., 0., 0.)); 50_001];
    scene.finish();
    assert!(!SceneSnapshot::capture(&scene, [64, 64]).supports_partial_updates());
    fallback_pixels(&mut renderer, &scene, viewport(64, 64))?;
    assert!(renderer.core.retained.image.is_none());
    Ok(())
}

#[test]
fn ineligible_target_rejects_before_atlas_flush_and_snapshot_preparation() -> Result<()> {
    let mut renderer = WgpuHeadlessRenderer::new()?;
    let original_format = renderer.core.target_format;
    let original_limit = renderer.core.max_texture_size;
    let _tile = super::atlas::tile(&renderer, 991, &[180; 256])?;
    let revision = renderer.core.atlas.content_revision();
    let mut unsupported = Scene::default();
    unsupported.quads = vec![quad(0, rect(0., 0., 0., 0.), hsla(0., 0., 0., 0.)); 50_001];
    unsupported.finish();
    // Pending upload revision is a witness that the pre-capture preparation
    // has not run. All these conditions must take the shared cheap guard.
    for (disabled, format, dimensions, limit) in [
        (true, original_format, viewport(64, 64), original_limit),
        (
            false,
            wgpu::TextureFormat::Rgba16Float,
            viewport(64, 64),
            original_limit,
        ),
        (false, original_format, viewport(4097, 4096), 8192),
        (false, original_format, viewport(128, 128), 64),
        (false, original_format, viewport(0, 64), original_limit),
    ] {
        renderer.core.retained.disabled = disabled;
        renderer.core.target_format = format;
        renderer.core.max_texture_size = limit;
        assert!(!renderer.core.render_retained(
            &unsupported,
            dimensions,
            false,
            wgpu::Color::BLACK
        )?);
        assert_eq!(renderer.core.atlas.content_revision(), revision);
        assert!(renderer.core.retained.image.is_none());
        assert!(renderer.core.retained.snapshot.is_none());
    }
    renderer.core.target_format = original_format;
    renderer.core.max_texture_size = original_limit;
    renderer.core.retained.reset();
    renderer.core.atlas.before_frame();
    assert_ne!(
        renderer.core.atlas.content_revision(),
        revision,
        "upload witness must be real"
    );
    renderer.check_gpu_errors()?;
    Ok(())
}
