// Nocterm regression tests, licensed under the upstream Apache-2.0 license.
use super::super::headless::HeadlessRenderTarget;
use super::super::tests::gpu_test;
use super::*;
mod atlas;
mod fallback;
mod primitives;
mod reuse;
use gpui::{ContentMask, Hsla, PlatformAtlas, Quad, SceneSnapshot, bounds, hsla, point, size};

#[test]
fn retention_policy_requires_exact_opt_in_and_surface_capability() {
    for value in [
        None,
        Some(""),
        Some("0"),
        Some("false"),
        Some("true"),
        Some("01"),
        Some("1 "),
    ] {
        for capability in [false, true] {
            assert_eq!(
                surface_usage(value.map(std::ffi::OsStr::new), capability),
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
        }
    }
    assert_eq!(
        surface_usage(Some(std::ffi::OsStr::new("1")), false),
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    assert_eq!(
        surface_usage(Some(std::ffi::OsStr::new("1")), true),
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
    );
}

fn viewport(w: i32, h: i32) -> Size<DevicePixels> {
    size(DevicePixels(w), DevicePixels(h))
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    bounds(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(w), ScaledPixels(h)),
    )
}
fn quad(order: u32, r: Bounds<ScaledPixels>, color: Hsla) -> Quad {
    Quad {
        order,
        bounds: r,
        content_mask: ContentMask {
            bounds: rect(0., 0., 512., 512.),
        },
        background: color.into(),
        ..Default::default()
    }
}
fn scene(changed: bool) -> Scene {
    let mut scene = Scene::default();
    scene
        .quads
        .push(quad(0, rect(0., 0., 256., 192.), hsla(0.2, 0.3, 0.4, 1.)));
    scene.quads.push(quad(
        1,
        rect(75., 70., 9., 11.),
        if changed {
            hsla(0.8, 0.8, 0.6, 0.5)
        } else {
            hsla(0.1, 0.7, 0.5, 0.8)
        },
    ));
    scene
        .quads
        .push(quad(2, rect(73., 73., 14., 5.), hsla(0.3, 0.6, 0.6, 0.4)));
    scene.finish();
    scene
}
fn full(
    renderer: &mut WgpuHeadlessRenderer,
    scene: &Scene,
    dimensions: Size<DevicePixels>,
    premultiplied: bool,
    clear: wgpu::Color,
) -> Result<image::RgbaImage> {
    renderer.ensure_render_target(dimensions)?;
    let view = renderer.render_target.as_ref().unwrap().view.clone();
    renderer
        .core
        .render_frame(scene, &view, dimensions, premultiplied, clear)?;
    let image = renderer.read_image()?;
    renderer.check_gpu_errors()?;
    Ok(image)
}
fn retained(
    renderer: &mut WgpuHeadlessRenderer,
    scene: &Scene,
    dimensions: Size<DevicePixels>,
    premultiplied: bool,
    clear: wgpu::Color,
) -> Result<image::RgbaImage> {
    if !renderer
        .core
        .render_retained(scene, dimensions, premultiplied, clear)?
    {
        return full(renderer, scene, dimensions, premultiplied, clear);
    }
    let image = renderer.core.retained.image.as_ref().unwrap();
    let saved = renderer.render_target.take();
    renderer.render_target = Some(HeadlessRenderTarget {
        texture: image.texture.clone(),
        view: image.view.clone(),
    });
    let result = renderer.read_image();
    renderer.render_target = saved;
    renderer.check_gpu_errors()?;
    result
}
fn assert_matches(
    renderer: &mut WgpuHeadlessRenderer,
    scene: &Scene,
    dimensions: Size<DevicePixels>,
    premultiplied: bool,
    clear: wgpu::Color,
) -> Result<()> {
    let incremental = retained(renderer, scene, dimensions, premultiplied, clear)?;
    let oracle = full(renderer, scene, dimensions, premultiplied, clear)?;
    assert_eq!(incremental.as_raw(), oracle.as_raw());
    Ok(())
}

#[test]
fn partial_alpha_and_repeated_updates_match_original_full_pixels() -> Result<()> {
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    for premultiplied in [false, true] {
        renderer.core.resources.pipelines = WgpuRendererCore::create_pipelines(
            &renderer.core.resources.device,
            &renderer.core.resources.bind_group_layouts,
            renderer.core.target_format,
            if premultiplied {
                wgpu::CompositeAlphaMode::PreMultiplied
            } else {
                wgpu::CompositeAlphaMode::Opaque
            },
            renderer.core.rendering_params.path_sample_count,
            renderer.core.dual_source_blending,
            renderer.core.uses_webgl_instance_data,
        );
        renderer.core.retained.reset();
        for clear in [wgpu::Color::BLACK, wgpu::Color::TRANSPARENT] {
            for changed in [false, true, false, true] {
                assert_matches(
                    &mut renderer,
                    &scene(changed),
                    viewport(256, 192),
                    premultiplied,
                    clear,
                )?;
            }
            assert!(matches!(
                renderer.core.retained.last_damage,
                Some(SceneDamage::Partial(_))
            ));
            assert_matches(
                &mut renderer,
                &scene(true),
                viewport(256, 192),
                premultiplied,
                clear,
            )?;
            assert_eq!(
                renderer.core.retained.last_damage,
                Some(SceneDamage::Unchanged)
            );
        }
    }
    Ok(())
}

#[test]
fn unchanged_frame_copies_the_whole_retained_image_to_poisoned_destination() -> Result<()> {
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    let dimensions = viewport(256, 192);
    let scene = scene(false);
    let oracle = full(&mut renderer, &scene, dimensions, false, wgpu::Color::BLACK)?;
    renderer.ensure_render_target(dimensions)?;
    // A separate destination has COPY_DST: previous contents are deliberately unrelated.
    let texture = renderer
        .core
        .resources
        .device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("poisoned_present_destination"),
            size: wgpu::Extent3d {
                width: 256,
                height: 192,
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
    for _ in 0..3 {
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
                        load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                ..Default::default()
            });
        }
        renderer.core.resources.queue.submit([encoder.finish()]);
        renderer.core.render_retained_surface(
            &scene,
            &texture,
            dimensions,
            false,
            wgpu::Color::BLACK,
        )?;
        let saved = renderer.render_target.take();
        renderer.render_target = Some(HeadlessRenderTarget {
            texture: texture.clone(),
            view,
        });
        assert_eq!(renderer.read_image()?.as_raw(), oracle.as_raw());
        renderer.render_target = saved;
    }
    assert_eq!(
        renderer.core.retained.last_damage,
        Some(SceneDamage::Unchanged)
    );
    renderer.check_gpu_errors()?;
    Ok(())
}

#[test]
fn lifecycle_changes_invalidate_before_reuse_and_match_full_pixels() -> Result<()> {
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    let scene = scene(false);
    for dimensions in [viewport(256, 192), viewport(193, 137), viewport(256, 192)] {
        assert_matches(&mut renderer, &scene, dimensions, false, wgpu::Color::BLACK)?;
        assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
        assert_matches(&mut renderer, &scene, dimensions, false, wgpu::Color::BLACK)?;
        assert_eq!(
            renderer.core.retained.last_damage,
            Some(SceneDamage::Unchanged)
        );
    }
    renderer.core.is_bgr = true;
    assert_matches(
        &mut renderer,
        &scene,
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    assert_matches(
        &mut renderer,
        &scene,
        viewport(256, 192),
        true,
        wgpu::Color::TRANSPARENT,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    renderer.core.retained.invalidate();
    assert_matches(
        &mut renderer,
        &scene,
        viewport(256, 192),
        true,
        wgpu::Color::TRANSPARENT,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    Ok(())
}

#[test]
fn limits_and_invalid_allocation_fall_back_without_accepted_invalid_handles() -> Result<()> {
    assert!(bounded_size(viewport(4096, 4096), 8192));
    assert!(!bounded_size(viewport(4097, 4096), 8192));
    assert!(!bounded_size(viewport(0, 10), 8192));
    assert!(!bounded_size(viewport(10, -1), 8192));
    assert!(!bounded_size(viewport(8193, 1), 8192));
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    eprintln!("NOCTERM_NATIVE_ADAPTER: {:?}", renderer.core.adapter_info);
    assert!(
        allocation::create_image(
            &renderer.core.resources.device,
            viewport(0, 10),
            renderer.core.target_format
        )
        .is_err()
    );
    renderer.check_gpu_errors()?;
    assert_matches(
        &mut renderer,
        &scene(false),
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    renderer.core.retained.disable();
    assert!(!renderer.core.render_retained(
        &scene(true),
        viewport(256, 192),
        false,
        wgpu::Color::BLACK
    )?);
    assert!(renderer.core.retained.snapshot.is_none());
    let oracle = full(
        &mut renderer,
        &scene(true),
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(oracle.dimensions(), (256, 192));
    Ok(())
}

#[test]
fn overwrite_clear_shader_is_valid() {
    let module = wgpu::naga::front::wgsl::parse_str(include_str!("clear.wgsl")).unwrap();
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
fn surface_without_copy_destination_uses_original_full_renderer() -> Result<()> {
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    let dimensions = viewport(256, 192);
    assert_matches(
        &mut renderer,
        &scene(false),
        dimensions,
        false,
        wgpu::Color::BLACK,
    )?;
    renderer.ensure_render_target(dimensions)?;
    let target = renderer.render_target.as_ref().unwrap().texture.clone();
    assert!(
        renderer
            .core
            .render_retained_surface(
                &scene(true),
                &target,
                viewport(255, 192),
                false,
                wgpu::Color::BLACK
            )
            .is_err()
    );
    assert!(renderer.core.retained.snapshot.is_none());
    renderer.check_gpu_errors()?;
    assert!(!target.usage().contains(wgpu::TextureUsages::COPY_DST));
    renderer.core.render_retained_surface(
        &scene(true),
        &target,
        dimensions,
        false,
        wgpu::Color::BLACK,
    )?;
    assert!(renderer.core.retained.snapshot.is_none());
    let fallback = renderer.read_image()?;
    let oracle = full(
        &mut renderer,
        &scene(true),
        dimensions,
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(fallback.as_raw(), oracle.as_raw());
    Ok(())
}

#[test]
fn matching_rgba_bgra_and_srgb_formats_reallocate_then_repair_exact_pixels() -> Result<()> {
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        renderer.core.target_format = format;
        renderer.core.resources.pipelines = WgpuRendererCore::create_pipelines(
            &renderer.core.resources.device,
            &renderer.core.resources.bind_group_layouts,
            format,
            wgpu::CompositeAlphaMode::Opaque,
            renderer.core.rendering_params.path_sample_count,
            renderer.core.dual_source_blending,
            renderer.core.uses_webgl_instance_data,
        );
        renderer.core.resources.invalidate_intermediate_textures();
        renderer.render_target = None;
        let clear = wgpu::Color {
            r: 0.1,
            g: 0.2,
            b: 0.4,
            a: 0.7,
        };
        let mut first = scene(false);
        first.quads.remove(0);
        first.finish();
        assert_matches(&mut renderer, &first, viewport(256, 192), false, clear)?;
        assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
        let mut second = scene(true);
        second.quads.remove(0);
        second.finish();
        // Nontrivial clear stays exposed inside repaired tiles and beneath alpha
        // quads; an opaque background would hide clear conversion errors.
        assert_matches(&mut renderer, &second, viewport(256, 192), false, clear)?;
        assert!(matches!(
            renderer.core.retained.last_damage,
            Some(SceneDamage::Partial(_))
        ));
    }
    assert!(!supported_format(wgpu::TextureFormat::Rgba16Float));
    Ok(())
}

#[test]
fn whole_viewport_damage_uses_original_full_clear_then_local_damage_stays_partial() -> Result<()> {
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    assert_matches(
        &mut renderer,
        &scene(false),
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    let mut next = scene(false);
    next.quads[0].background = hsla(0.8, 0.5, 0.7, 1.).into();
    next.finish();
    assert_matches(
        &mut renderer,
        &next,
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    next.quads[1].background = hsla(0.7, 0.8, 0.6, 0.5).into();
    next.finish();
    assert_matches(
        &mut renderer,
        &next,
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert!(matches!(
        renderer.core.retained.last_damage,
        Some(SceneDamage::Partial(_))
    ));
    Ok(())
}
