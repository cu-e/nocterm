// Nocterm modifications, licensed under the upstream Apache-2.0 license.
use super::*;
use std::borrow::Cow;
use windows::{Win32::UI::WindowsAndMessaging::*, core::w};

mod adjacent_atlas;
mod compact_copy;
mod copy_and_atlas;
mod scroll_copy;

struct TestWindow(HWND);
impl TestWindow {
    fn new() -> Self {
        Self(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Nocterm renderer test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                256,
                192,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        })
    }
}
impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0).unwrap() };
    }
}

fn renderer(devices: &DirectXDevices) -> (DirectXRenderer, TestWindow) {
    let window = TestWindow::new();
    let mut renderer = DirectXRenderer::new(window.0, devices, true).unwrap();
    renderer
        .resize(size(DevicePixels(128), DevicePixels(96)))
        .unwrap();
    (renderer, window)
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(width), ScaledPixels(height)),
    )
}
fn mask() -> ContentMask<ScaledPixels> {
    ContentMask {
        bounds: rect(0.0, 0.0, 256.0, 192.0),
    }
}
fn quad(bounds: Bounds<ScaledPixels>, color: Hsla) -> Quad {
    Quad {
        bounds,
        content_mask: mask(),
        background: color.into(),
        ..Default::default()
    }
}
fn scene(cursor: Option<f32>, alpha: bool) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(quad(rect(0.0, 0.0, 128.0, 96.0), hsla(0.6, 0.3, 0.2, 1.0)));
    scene.insert_primitive(quad(rect(0.0, 0.0, 20.0, 96.0), hsla(0.2, 0.5, 0.4, 1.0)));
    let mut path = Path::new(point(px(4.0), px(4.0)));
    path.line_to(point(px(15.0), px(5.0)));
    path.line_to(point(px(10.0), px(18.0)));
    path.line_to(point(px(4.0), px(4.0)));
    let mut path = path.scale(1.0);
    path.content_mask = mask();
    path.color = hsla(0.9, 0.7, 0.8, 0.6).into();
    scene.insert_primitive(path);
    if let Some(x) = cursor {
        scene.insert_primitive(quad(
            rect(x, 48.0, 3.0, 16.0),
            hsla(0.0, 0.0, 0.9, if alpha { 0.5 } else { 1.0 }),
        ));
    }
    scene.finish();
    scene
}

/// Compare live incremental pixels against the unchanged full-render oracle
/// on the same device and atlas. Keep successful live metadata for the next
/// transition; the oracle leaves identical current pixels in the target.
fn assert_pixels(
    renderer: &mut DirectXRenderer,
    scene: &Scene,
    appearance: WindowBackgroundAppearance,
) {
    renderer.draw(scene, appearance).unwrap();
    let incremental = renderer.read_target_to_image().unwrap();
    let metadata = std::mem::take(&mut renderer.retained);
    let full = renderer.render_to_image(scene, appearance).unwrap();
    renderer.retained = metadata;
    assert_eq!(
        incremental.as_raw(),
        full.as_raw(),
        "incremental/full pixel mismatch"
    );
}

#[::core::prelude::v1::test]
fn retained_cursor_move_erase_alpha_and_unchanged_match_full_pixels() {
    let devices = DirectXDevices::new().unwrap();
    let (mut renderer, _window) = renderer(&devices);
    assert!(
        renderer
            .resources
            .as_ref()
            .unwrap()
            .clear_view_context
            .is_some(),
        "test requires D3D11.1 ClearView support"
    );
    for (index, cursor) in [Some(44.0), Some(61.0), None, Some(44.5), Some(80.0)]
        .into_iter()
        .enumerate()
    {
        if index > 0 {
            let snapshot = SceneSnapshot::capture(&scene(cursor, true), [128, 96]);
            assert!(
                matches!(
                    renderer.retained.damage(
                        &snapshot,
                        WindowBackgroundAppearance::Opaque,
                        renderer.atlas.content_revision(),
                        true
                    ),
                    SceneDamage::Partial(_)
                ),
                "cursor transition must exercise actual partial rasterization"
            );
        }
        assert_pixels(
            &mut renderer,
            &scene(cursor, true),
            WindowBackgroundAppearance::Opaque,
        );
    }
    let scene = scene(Some(80.0), true);
    let snapshot = SceneSnapshot::capture(&scene, [128, 96]);
    assert!(matches!(
        renderer.retained.damage(
            &snapshot,
            WindowBackgroundAppearance::Opaque,
            renderer.atlas.content_revision(),
            true
        ),
        SceneDamage::Unchanged
    ));
    assert_pixels(&mut renderer, &scene, WindowBackgroundAppearance::Opaque);
}

#[::core::prelude::v1::test]
fn retained_shadow_wavy_underline_clip_and_paths_match_full_pixels() {
    let devices = DirectXDevices::new().unwrap();
    let (mut renderer, _window) = renderer(&devices);
    for offset in [0.0, 4.25, 16.0, 5.0] {
        let mut scene = scene(Some(45.0 + offset), true);
        scene.insert_primitive(Shadow {
            order: 0,
            blur_radius: ScaledPixels(5.0),
            bounds: rect(35.0 + offset, 20.0, 42.0, 30.0),
            corner_radii: Corners::all(ScaledPixels(4.0)),
            content_mask: ContentMask {
                bounds: rect(40.0, 18.0, 50.0, 44.0),
            },
            color: hsla(0.0, 0.0, 0.0, 0.4),
            element_bounds: rect(40.0 + offset, 25.0, 32.0, 20.0),
            element_corner_radii: Corners::all(ScaledPixels(4.0)),
            inset: 0,
            pad: 0,
        });
        scene.insert_primitive(Underline {
            order: 0,
            pad: 0,
            bounds: rect(30.0, 72.0, 64.0, 5.0),
            content_mask: mask(),
            color: hsla(0.8, 0.9, 0.6, 0.7),
            thickness: ScaledPixels(1.5 + offset / 16.0),
            wavy: true.into(),
        });
        let mut path = Path::new(point(px(42.0 + offset), px(24.0)));
        path.line_to(point(px(65.0), px(28.0)));
        path.line_to(point(px(48.0), px(43.0)));
        path.line_to(point(px(42.0 + offset), px(24.0)));
        let mut path = path.scale(1.0);
        path.content_mask = mask();
        path.color = hsla(0.1, 0.8, 0.6, 0.5).into();
        scene.insert_primitive(path);
        scene.finish();
        assert_pixels(
            &mut renderer,
            &scene,
            WindowBackgroundAppearance::Transparent,
        );
    }
}

fn glyph_key(subpixel: bool, emoji: bool, id: u32) -> AtlasKey {
    RenderGlyphParams {
        font_id: FontId(0),
        glyph_id: GlyphId(id),
        font_size: px(8.0),
        subpixel_variant: point(0, 0),
        scale_factor: 1.0,
        is_emoji: emoji,
        subpixel_rendering: subpixel,
        dilation: 0,
    }
    .into()
}
fn tile(renderer: &DirectXRenderer, key: AtlasKey, channels: usize) -> AtlasTile {
    let bytes = vec![150u8; 8 * 8 * channels];
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
fn retained_glyph_color_subpixel_and_atlas_reuse_match_full_pixels() {
    let devices = DirectXDevices::new().unwrap();
    let (mut renderer, _window) = renderer(&devices);
    let mono_key = glyph_key(false, false, 1);
    let mono = tile(&renderer, mono_key.clone(), 1);
    let subpixel = tile(&renderer, glyph_key(true, false, 2), 4);
    let colored = tile(&renderer, glyph_key(false, true, 3), 4);
    let revision = renderer.atlas.content_revision();
    let _extra = tile(&renderer, glyph_key(false, false, 4), 1);
    assert_ne!(
        revision,
        renderer.atlas.content_revision(),
        "successful uploads can alter neighboring linear-filtered samples"
    );
    let cached_revision = renderer.atlas.content_revision();
    let cached = tile(&renderer, mono_key.clone(), 1);
    assert_eq!(cached, mono);
    assert_eq!(
        cached_revision,
        renderer.atlas.content_revision(),
        "cache hits must not advance atlas revision"
    );
    for x in [32.0, 33.5, 65.0] {
        let mut scene = scene(None, false);
        scene.insert_primitive(MonochromeSprite {
            order: 0,
            pad: 0,
            bounds: rect(x, 36.0, 8.0, 8.0),
            content_mask: mask(),
            color: hsla(0.4, 0.8, 0.8, 0.7),
            tile: mono,
            transformation: TransformationMatrix::unit(),
        });
        scene.insert_primitive(SubpixelSprite {
            order: 0,
            pad: 0,
            bounds: rect(x, 52.0, 8.0, 8.0),
            content_mask: mask(),
            color: hsla(0.1, 0.8, 0.8, 1.0),
            tile: subpixel,
            transformation: TransformationMatrix::unit(),
        });
        scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false.into(),
            opacity: 0.6,
            bounds: rect(x, 68.0, 8.0, 8.0),
            content_mask: mask(),
            corner_radii: Corners::all(ScaledPixels(2.0)),
            tile: colored,
        });
        scene.finish();
        assert_pixels(&mut renderer, &scene, WindowBackgroundAppearance::Opaque);
    }
    renderer.atlas.remove(&mono_key);
    assert_ne!(revision, renderer.atlas.content_revision());
    let _replacement = tile(&renderer, glyph_key(false, false, 5), 1);
    assert_pixels(
        &mut renderer,
        &scene(Some(65.0), false),
        WindowBackgroundAppearance::Opaque,
    );
}

#[::core::prelude::v1::test]
fn retained_resize_appearance_and_clearview_fallback_match_full_pixels() {
    let devices = DirectXDevices::new().unwrap();
    let (mut renderer, _window) = renderer(&devices);
    assert_pixels(
        &mut renderer,
        &scene(Some(40.0), false),
        WindowBackgroundAppearance::Opaque,
    );
    renderer
        .resize(size(DevicePixels(96), DevicePixels(80)))
        .unwrap();
    assert!(!renderer.retained.is_valid());
    assert_pixels(
        &mut renderer,
        &scene(Some(50.0), false),
        WindowBackgroundAppearance::Transparent,
    );
    renderer.resources.as_mut().unwrap().clear_view_context = None;
    assert_pixels(
        &mut renderer,
        &scene(Some(60.0), true),
        WindowBackgroundAppearance::Opaque,
    );
    assert_pixels(
        &mut renderer,
        &scene(Some(70.0), true),
        WindowBackgroundAppearance::Opaque,
    );
}

#[::core::prelude::v1::test]
fn retained_shared_context_restores_rasterizer_each_frame() {
    let devices = DirectXDevices::new().unwrap();
    let (mut first, _first_window) = renderer(&devices);
    let (mut second, _second_window) = renderer(&devices);
    for x in [40.0, 48.0, 64.0] {
        assert_pixels(
            &mut first,
            &scene(Some(x), false),
            WindowBackgroundAppearance::Opaque,
        );
        assert_pixels(
            &mut second,
            &scene(Some(90.0 - x / 2.0), true),
            WindowBackgroundAppearance::Transparent,
        );
    }
}

#[::core::prelude::v1::test]
fn retained_device_recovery_and_failed_draw_invalidate_metadata() {
    let devices = DirectXDevices::new().unwrap();
    let (mut renderer, _window) = renderer(&devices);
    let scene = scene(Some(40.0), false);
    assert_pixels(&mut renderer, &scene, WindowBackgroundAppearance::Opaque);
    let target = renderer
        .resources
        .as_mut()
        .unwrap()
        .swap_chain_target
        .take();
    assert!(
        renderer
            .draw(&scene, WindowBackgroundAppearance::Opaque)
            .is_err()
    );
    assert!(!renderer.retained.is_valid());
    renderer.resources.as_mut().unwrap().swap_chain_target = target;
    assert_pixels(&mut renderer, &scene, WindowBackgroundAppearance::Opaque);
    renderer.handle_device_lost(&devices).unwrap();
    assert!(!renderer.retained.is_valid());
    assert!(renderer.skip_draws);
    renderer
        .draw(&scene, WindowBackgroundAppearance::Opaque)
        .unwrap();
    assert!(!renderer.retained.is_valid());
    renderer.mark_drawable();
    assert_pixels(&mut renderer, &scene, WindowBackgroundAppearance::Opaque);
}

#[::core::prelude::v1::test]
fn retained_allocation_fallback_renders_full_without_caching() {
    let devices = DirectXDevices::new().unwrap();
    let (mut renderer, _window) = renderer(&devices);
    let resources = renderer.resources.as_mut().unwrap();
    let target = resources.swap_chain_target.as_ref().unwrap().clone();
    let mut view = None;
    unsafe {
        devices
            .device
            .CreateRenderTargetView(&target, None, Some(&mut view))
            .unwrap()
    };
    resources.render_target = Some(target);
    resources.render_target_view = view;
    resources.retained_enabled = false;
    for x in [40.0, 48.0, 64.0, 64.0] {
        let scene = scene(Some(x), true);
        let appearance = WindowBackgroundAppearance::Opaque;
        let resources = renderer.resources.as_ref().unwrap();
        assert_eq!(
            resources.render_target.as_ref().unwrap().as_raw(),
            resources.swap_chain_target.as_ref().unwrap().as_raw()
        );
        unsafe {
            // A sentinel detects pixels that full rendering failed to replace.
            devices.device_context.ClearRenderTargetView(
                resources.render_target_view.as_ref().unwrap(),
                &[1.0, 0.0, 1.0, 1.0],
            );
        }
        // The fallback target IS the flip-model back buffer. Compare its pixels
        // before Present transfers ownership; its contents after Present are
        // not a readback oracle. Persistent retained targets do not rotate.
        renderer
            .render_damage(&scene, appearance, SceneDamage::Full)
            .unwrap();
        let rendered = renderer.read_target_to_image().unwrap();
        let full = renderer.render_to_image(&scene, appearance).unwrap();
        let mismatched_bytes = rendered
            .as_raw()
            .iter()
            .zip(full.as_raw())
            .filter(|(actual, expected)| actual != expected)
            .count();
        assert_eq!(
            mismatched_bytes, 0,
            "fallback/full pixels differ before Present"
        );

        // Exercise the production draw/Present path, including an unchanged
        // scene. Previously valid metadata must never survive fallback draws.
        renderer.retained.commit(
            SceneSnapshot::capture(&scene, [128, 96]),
            appearance,
            renderer.atlas.content_revision(),
        );
        assert!(renderer.retained.is_valid());
        renderer.draw(&scene, appearance).unwrap();
        assert!(!renderer.retained.is_valid());
    }
}
