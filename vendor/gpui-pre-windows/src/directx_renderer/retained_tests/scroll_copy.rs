// Nocterm regression tests, licensed under the upstream Apache-2.0 license.
use super::*;

pub(super) fn tiles(renderer: &DirectXRenderer, subpixel: bool) -> Vec<AtlasTile> {
    (0..20)
        .map(|id| {
            let channels = if subpixel { 4 } else { 1 };
            let bytes: Vec<_> = (0..64 * channels)
                .map(|index| ((index * 53 + id as usize * 31) % 256) as u8)
                .collect();
            renderer
                .atlas
                .get_or_insert_with(glyph_key(subpixel, false, 500 + id), &mut || {
                    Ok(Some((
                        size(DevicePixels(8), DevicePixels(8)),
                        Cow::Borrowed(&bytes),
                    )))
                })
                .unwrap()
                .unwrap()
        })
        .collect()
}

fn scrolling_scene(
    renderer: &DirectXRenderer,
    tiles: &[AtlasTile],
    offset: usize,
    subpixel: bool,
    overlays: bool,
    clipped: bool,
    fractional: bool,
) -> Scene {
    let mut scene = Scene::default();
    let width = renderer.width as f32;
    let height = renderer.height as f32;
    scene.quads.push(quad(
        rect(0.0, 0.0, width, height),
        hsla(0.6, 0.4, 0.2, 1.0),
    ));
    scene.quads.push(Quad {
        order: 1,
        ..quad(rect(0.0, 0.0, 16.0, height), hsla(0.2, 0.5, 0.3, 1.0))
    });
    let content_mask = ContentMask {
        bounds: rect(16.0, if clipped { 6.0 } else { 0.0 }, width - 16.0, height),
    };
    for row in 0..renderer.height / 12 {
        for column in 0..8 {
            let id = (row as usize + offset) % tiles.len();
            let bounds = rect(
                20.0 + column as f32 * 10.0,
                row as f32 * 12.0 + 4.0 + if fractional { 0.25 } else { 0.0 },
                8.0,
                8.0,
            );
            let color = hsla(id as f32 * 0.04, 0.5, 0.7, 0.8);
            if subpixel {
                scene.subpixel_sprites.push(SubpixelSprite {
                    order: 2,
                    pad: 0,
                    bounds,
                    content_mask,
                    color,
                    tile: tiles[id],
                    transformation: TransformationMatrix::unit(),
                });
            } else {
                scene.monochrome_sprites.push(MonochromeSprite {
                    order: 2,
                    pad: 0,
                    bounds,
                    content_mask,
                    color,
                    tile: tiles[id],
                    transformation: TransformationMatrix::unit(),
                });
            }
        }
    }
    if overlays {
        // Stationary alpha selection and cursor must be repaired at both their
        // old translated source positions and their current destination positions.
        scene.quads.push(Quad {
            order: 3,
            ..quad(rect(18.0, 28.0, 86.0, 8.0), hsla(0.8, 0.5, 0.6, 0.4))
        });
        scene.quads.push(Quad {
            order: 4,
            ..quad(rect(24.0, 50.0, 4.0, 8.0), hsla(0.0, 0.0, 0.9, 0.7))
        });
    }
    scene.finish();
    scene
}

fn read_destination(renderer: &mut DirectXRenderer) -> image::RgbaImage {
    let resources = renderer.resources.as_mut().unwrap();
    let destination = resources.swap_chain_target.as_ref().unwrap().clone();
    let source = resources.render_target.replace(destination).unwrap();
    let pixels = renderer.read_target_to_image().unwrap();
    renderer.resources.as_mut().unwrap().render_target = Some(source);
    pixels
}

fn assert_scroll_pixels(renderer: &mut DirectXRenderer, scene: &Scene, expected_dy: i32) {
    let appearance = WindowBackgroundAppearance::Opaque;
    let revision = renderer.atlas.content_revision();
    let snapshot = SceneSnapshot::capture(scene, [renderer.width, renderer.height]);
    let damage = renderer
        .retained
        .damage(&snapshot, appearance, revision, true);
    let plan = renderer
        .retained
        .scroll(&snapshot, appearance, revision, true)
        .expect("transition must exercise verified scroll copying");
    assert_eq!(plan.dy, expected_dy);
    assert!(!plan.copied.is_empty());
    assert!(!plan.redraw.is_empty());

    let resources = renderer.resources.as_ref().unwrap();
    let devices = renderer.devices.as_ref().unwrap();
    let mut destination_view = None;
    unsafe {
        devices
            .device
            .CreateRenderTargetView(
                resources.swap_chain_target.as_ref().unwrap(),
                None,
                Some(&mut destination_view),
            )
            .unwrap();
        devices.device_context.OMSetRenderTargets(None, None);
        devices
            .device_context
            .ClearRenderTargetView(destination_view.as_ref().unwrap(), &[1.0, 0.0, 1.0, 1.0]);
    }
    let sentinel = read_destination(renderer);
    renderer
        .render_scroll(scene, appearance, &plan, damage)
        .unwrap();
    let incremental = renderer.read_target_to_image().unwrap();
    assert!(
        renderer
            .resources
            .as_ref()
            .unwrap()
            .scroll_scratch
            .is_some()
    );
    renderer.copy_retained_to_swap_chain().unwrap();
    let destination = read_destination(renderer);
    assert_ne!(destination.as_raw(), sentinel.as_raw());
    assert_eq!(
        destination.as_raw(),
        incremental.as_raw(),
        "scroll image must reach poisoned swap-chain destination before Present"
    );
    let metadata = std::mem::take(&mut renderer.retained);
    let full = renderer.render_to_image(scene, appearance).unwrap();
    renderer.retained = metadata;
    assert_eq!(
        incremental.as_raw(),
        full.as_raw(),
        "scroll-copy pixels must exactly equal full shader rasterization"
    );
    renderer.present().unwrap();
    renderer.retained.commit(snapshot, appearance, revision);
}

#[::core::prelude::v1::test]
fn retained_verified_scroll_up_down_clipping_cursor_selection_match_full_pixels() {
    let devices = DirectXDevices::new().unwrap();
    for subpixel in [false, true] {
        let (mut renderer, _window) = renderer(&devices);
        let tiles = tiles(&renderer, subpixel);
        for (width, height) in [(128, 96), (192, 160), (128, 96)] {
            renderer
                .resize(size(DevicePixels(width), DevicePixels(height)))
                .unwrap();
            assert!(
                renderer
                    .resources
                    .as_ref()
                    .unwrap()
                    .scroll_scratch
                    .is_none()
            );
            for (overlays, clipped) in [(false, false), (true, false), (false, true)] {
                let old = scrolling_scene(&renderer, &tiles, 0, subpixel, overlays, clipped, false);
                assert_pixels(&mut renderer, &old, WindowBackgroundAppearance::Opaque);
                let up = scrolling_scene(&renderer, &tiles, 1, subpixel, overlays, clipped, false);
                assert_scroll_pixels(&mut renderer, &up, -12);
                assert_scroll_pixels(&mut renderer, &old, 12);
            }
        }
    }
}

pub(super) fn dense_scene(
    renderer: &DirectXRenderer,
    tiles: &[AtlasTile],
    offset: usize,
    subpixel: bool,
) -> Scene {
    let mut scene = scrolling_scene(renderer, tiles, offset, subpixel, false, false, false);
    if subpixel {
        scene.subpixel_sprites = scene
            .subpixel_sprites
            .iter()
            .step_by(8)
            .flat_map(|first| {
                (0..80).map(move |column| {
                    let mut glyph = *first;
                    glyph.bounds.origin.x = ScaledPixels(20.0 + column as f32 * 9.0);
                    glyph
                })
            })
            .collect();
    } else {
        scene.monochrome_sprites = scene
            .monochrome_sprites
            .iter()
            .step_by(8)
            .flat_map(|first| {
                (0..80).map(move |column| {
                    let mut glyph = *first;
                    glyph.bounds.origin.x = ScaledPixels(20.0 + column as f32 * 9.0);
                    glyph
                })
            })
            .collect();
    }
    scene.finish();
    scene
}

#[::core::prelude::v1::test]
fn retained_dense_multi_line_scroll_copy_matches_full_pixels() {
    let devices = DirectXDevices::new().unwrap();
    for subpixel in [false, true] {
        let (mut renderer, _window) = renderer(&devices);
        let tiles = tiles(&renderer, subpixel);
        for (width, height) in [(800, 320), (1080, 680), (1079, 679)] {
            renderer
                .resize(size(DevicePixels(width), DevicePixels(height)))
                .unwrap();
            let old = dense_scene(&renderer, &tiles, 0, subpixel);
            assert_pixels(&mut renderer, &old, WindowBackgroundAppearance::Opaque);
            let mut previous_offset = 0;
            for offset in [3, 6, 0, 6, 3] {
                let current = dense_scene(&renderer, &tiles, offset, subpixel);
                assert_scroll_pixels(
                    &mut renderer,
                    &current,
                    (previous_offset - offset as i32) * 12,
                );
                previous_offset = offset as i32;
            }
        }
    }
}

#[::core::prelude::v1::test]
fn retained_scroll_atlas_appearance_capability_fractional_and_scratch_fallbacks() {
    let devices = DirectXDevices::new().unwrap();
    let (mut renderer, _window) = renderer(&devices);
    let tiles = tiles(&renderer, false);
    let old = scrolling_scene(&renderer, &tiles, 0, false, false, false, false);
    let current = scrolling_scene(&renderer, &tiles, 1, false, false, false, false);
    assert_pixels(&mut renderer, &old, WindowBackgroundAppearance::Opaque);
    let snapshot = SceneSnapshot::capture(&current, [128, 96]);
    let revision = renderer.atlas.content_revision();
    assert!(
        renderer
            .retained
            .scroll(
                &snapshot,
                WindowBackgroundAppearance::Opaque,
                revision,
                false
            )
            .is_none()
    );
    assert!(
        renderer
            .retained
            .scroll(
                &snapshot,
                WindowBackgroundAppearance::Transparent,
                revision,
                true
            )
            .is_none()
    );
    // Atlas insertion can change linear-filter neighbors of existing tiles.
    let _ = tile(&renderer, glyph_key(false, false, 9000), 1);
    assert_ne!(renderer.atlas.content_revision(), revision);
    assert!(
        renderer
            .retained
            .scroll(
                &snapshot,
                WindowBackgroundAppearance::Opaque,
                renderer.atlas.content_revision(),
                true
            )
            .is_none()
    );
    assert_pixels(&mut renderer, &current, WindowBackgroundAppearance::Opaque);
    let fractional = scrolling_scene(&renderer, &tiles, 2, false, false, false, true);
    let snapshot = SceneSnapshot::capture(&fractional, [128, 96]);
    assert!(
        renderer
            .retained
            .scroll(
                &snapshot,
                WindowBackgroundAppearance::Opaque,
                renderer.atlas.content_revision(),
                true
            )
            .is_none()
    );
    assert_pixels(
        &mut renderer,
        &fractional,
        WindowBackgroundAppearance::Opaque,
    );

    assert_pixels(&mut renderer, &old, WindowBackgroundAppearance::Opaque);
    renderer.resources.as_mut().unwrap().scroll_scratch_failed = true;
    // The production draw path must replay normal damage after allocation failure.
    assert_pixels(&mut renderer, &current, WindowBackgroundAppearance::Opaque);
    assert!(
        renderer
            .resources
            .as_ref()
            .unwrap()
            .scroll_scratch
            .is_none()
    );
    renderer.retained.invalidate();
    assert!(
        renderer
            .retained
            .scroll(
                &SceneSnapshot::capture(&old, [128, 96]),
                WindowBackgroundAppearance::Opaque,
                renderer.atlas.content_revision(),
                true
            )
            .is_none()
    );
}

#[::core::prelude::v1::test]
fn retained_fractional_opaque_backdrop_interior_matches_full_pixels() {
    let devices = DirectXDevices::new().unwrap();
    for subpixel in [false, true] {
        let (mut renderer, _window) = renderer(&devices);
        renderer
            .resize(size(DevicePixels(1079), DevicePixels(679)))
            .unwrap();
        let tiles = tiles(&renderer, subpixel);
        let make = |renderer: &DirectXRenderer, offset| {
            let mut scene = dense_scene(renderer, &tiles, offset, subpixel);
            scene.quads[0].bounds = rect(0.25, 0.25, 1078.5, 678.5);
            scene.quads[0].content_mask.bounds = rect(1.25, 1.25, 1076.5, 676.5);
            scene.finish();
            scene
        };
        let old = make(&renderer, 0);
        assert_pixels(&mut renderer, &old, WindowBackgroundAppearance::Opaque);
        let current = make(&renderer, 3);
        let plan = renderer
            .retained
            .scroll(
                &SceneSnapshot::capture(&current, [1079, 679]),
                WindowBackgroundAppearance::Opaque,
                renderer.atlas.content_revision(),
                true,
            )
            .unwrap();
        assert!(plan.region.left >= 3 && plan.region.top >= 3);
        assert!(plan.region.right <= 1076 && plan.region.bottom <= 676);
        assert_scroll_pixels(&mut renderer, &current, -36);
        assert_scroll_pixels(&mut renderer, &old, 36);
    }
}

#[::core::prelude::v1::test]
fn retained_deepest_duplicate_backdrop_matches_full_pixels() {
    let devices = DirectXDevices::new().unwrap();
    for subpixel in [false, true] {
        let (mut renderer, _window) = renderer(&devices);
        renderer
            .resize(size(DevicePixels(1079), DevicePixels(679)))
            .unwrap();
        let tiles = tiles(&renderer, subpixel);
        let make = |renderer: &DirectXRenderer, offset| {
            let mut scene = dense_scene(renderer, &tiles, offset, subpixel);
            let backdrop = scene.quads[0];
            for order in 1..=6 {
                let mut layer = backdrop;
                layer.order = order;
                if order % 2 == 1 {
                    layer.corner_radii = Corners::all(ScaledPixels(4.0));
                    layer.background = hsla(0.8, 0.6, 0.7, 0.6).into();
                }
                scene.quads.push(layer);
            }
            for glyph in &mut scene.monochrome_sprites {
                glyph.order = 7;
            }
            for glyph in &mut scene.subpixel_sprites {
                glyph.order = 7;
            }
            scene.finish();
            scene
        };
        let old = make(&renderer, 0);
        assert_pixels(&mut renderer, &old, WindowBackgroundAppearance::Opaque);
        for (offset, dy) in [(3, -36), (6, -36), (0, 72)] {
            let current = make(&renderer, offset);
            assert_scroll_pixels(&mut renderer, &current, dy);
        }
    }
}

#[::core::prelude::v1::test]
fn retained_masked_rounded_edge_columns_match_full_pixels() {
    let devices = DirectXDevices::new().unwrap();
    for subpixel in [false, true] {
        let (mut renderer, _window) = renderer(&devices);
        renderer
            .resize(size(DevicePixels(1064), DevicePixels(672)))
            .unwrap();
        let tiles = tiles(&renderer, subpixel);
        let make = |renderer: &DirectXRenderer, offset| {
            let mut scene = dense_scene(renderer, &tiles, offset, subpixel);
            let pane = rect(246.0, 161.0, 813.0, 442.0);
            scene.quads.clear();
            scene.quads.push(Quad {
                content_mask: ContentMask { bounds: pane },
                ..quad(pane, hsla(0.0, 0.0, 1.0, 1.0))
            });
            let position = |index: usize| {
                rect(
                    271.0 + (index % 80) as f32 * 8.0,
                    168.0 + (index / 80) as f32 * 17.0,
                    7.0,
                    13.0,
                )
            };
            scene.monochrome_sprites.clear();
            scene.subpixel_sprites.clear();
            for index in 0..25 * 80 {
                let id = (index / 80 + offset) % tiles.len();
                let color = hsla(id as f32 * 0.04, 0.5, 0.7, 0.8);
                if subpixel {
                    scene.subpixel_sprites.push(SubpixelSprite {
                        order: 1,
                        pad: 0,
                        bounds: position(index),
                        content_mask: ContentMask { bounds: pane },
                        color,
                        tile: tiles[id],
                        transformation: TransformationMatrix::unit(),
                    });
                } else {
                    scene.monochrome_sprites.push(MonochromeSprite {
                        order: 1,
                        pad: 0,
                        bounds: position(index),
                        content_mask: ContentMask { bounds: pane },
                        color,
                        tile: tiles[id],
                        transformation: TransformationMatrix::unit(),
                    });
                }
            }
            for (order, bounds, mask, radius, border) in [
                (
                    2,
                    rect(235.0, 139.0, 835.0, 507.0),
                    rect(246.0, 161.0, 10.0, 474.0),
                    20.0,
                    10.0,
                ),
                (
                    3,
                    rect(245.0, 149.0, 815.0, 487.0),
                    rect(246.0, 161.0, 10.0, 473.0),
                    10.0,
                    1.0,
                ),
                // Also exercise the mirrored right-edge conservative AA column.
                (
                    4,
                    rect(245.0, 149.0, 725.0, 487.0),
                    rect(959.0, 161.0, 10.0, 473.0),
                    10.0,
                    1.0,
                ),
            ] {
                scene.quads.push(Quad {
                    order,
                    bounds,
                    content_mask: ContentMask { bounds: mask },
                    background: hsla(0.0, 0.0, 0.93, 0.0).into(),
                    border_color: hsla(0.0, 0.0, 0.8, 1.0),
                    corner_radii: Corners::all(ScaledPixels(radius)),
                    border_widths: Edges::all(ScaledPixels(border)),
                    ..Default::default()
                });
            }
            scene.finish();
            scene
        };
        let old = make(&renderer, 0);
        assert_pixels(&mut renderer, &old, WindowBackgroundAppearance::Opaque);
        let current = make(&renderer, 6);
        assert_scroll_pixels(&mut renderer, &current, -102);
        assert_scroll_pixels(&mut renderer, &old, 102);
    }
}

#[::core::prelude::v1::test]
fn retained_partial_glyph_runs_preserve_dense_clipped_transformed_and_overlap_pixels() {
    let devices = DirectXDevices::new().unwrap();
    for subpixel in [false, true] {
        let (mut renderer, _window) = renderer(&devices);
        renderer
            .resize(size(DevicePixels(1080), DevicePixels(680)))
            .unwrap();
        let tiles = tiles(&renderer, subpixel);
        let make = |changed: bool| {
            let mut scene = dense_scene(&renderer, &tiles, 0, subpixel);
            for (order, bounds, mask, translation) in [
                (
                    3,
                    rect(318.25, 92.5, 18.5, 13.25),
                    rect(320.0, 94.0, 15.0, 11.0),
                    0.0,
                ),
                (
                    4,
                    rect(280.0, 95.0, 10.0, 10.0),
                    rect(0.0, 0.0, 1080.0, 680.0),
                    40.0,
                ),
                // Unsupported transforms outside the damage are retained too.
                (
                    5,
                    rect(10.0, 400.0, 10.0, 10.0),
                    rect(0.0, 0.0, 1080.0, 680.0),
                    5.0,
                ),
            ] {
                let mut transformation = TransformationMatrix::unit();
                transformation.translation[0] = translation;
                let color = hsla(0.1, 0.7, 0.6, 0.5);
                if subpixel {
                    scene.subpixel_sprites.push(SubpixelSprite {
                        order,
                        pad: 0,
                        bounds,
                        content_mask: ContentMask { bounds: mask },
                        color,
                        tile: tiles[1],
                        transformation,
                    });
                } else {
                    scene.monochrome_sprites.push(MonochromeSprite {
                        order,
                        pad: 0,
                        bounds,
                        content_mask: ContentMask { bounds: mask },
                        color,
                        tile: tiles[1],
                        transformation,
                    });
                }
            }
            if changed {
                for glyph in &mut scene.monochrome_sprites {
                    if glyph.order == 3
                        || glyph.order == 4
                        || (glyph.bounds.left().0 >= 320.0
                            && glyph.bounds.right().0 <= 384.0
                            && glyph.bounds.top().0 >= 80.0
                            && glyph.bounds.bottom().0 <= 128.0)
                    {
                        glyph.color = hsla(0.8, 0.4, 0.7, 0.6);
                    }
                }
                for glyph in &mut scene.subpixel_sprites {
                    if glyph.order == 3
                        || glyph.order == 4
                        || (glyph.bounds.left().0 >= 320.0
                            && glyph.bounds.right().0 <= 384.0
                            && glyph.bounds.top().0 >= 80.0
                            && glyph.bounds.bottom().0 <= 128.0)
                    {
                        glyph.color = hsla(0.8, 0.4, 0.7, 0.6);
                    }
                }
            }
            scene.finish();
            scene
        };
        let old = make(false);
        let current = make(true);
        let damage = SceneDamageRect {
            left: 320,
            top: 80,
            right: 384,
            bottom: 128,
        };
        renderer
            .render_to_image(&old, WindowBackgroundAppearance::Opaque)
            .unwrap();
        let selected = renderer
            .prepare_damage_buffers(&current, &[damage])
            .unwrap()
            .unwrap();
        let mut drawn = 0;
        let total = if subpixel {
            current.subpixel_sprites.len()
        } else {
            current.monochrome_sprites.len()
        };
        for batch in current.batches() {
            let range = match batch {
                PrimitiveBatch::MonochromeSprites { range, .. } if !subpixel => range,
                PrimitiveBatch::SubpixelSprites { range, .. } if subpixel => range,
                _ => continue,
            };
            drawn += selected.map_range(range, subpixel, 0).len();
        }
        for order in [3, 4, 5] {
            let index = if subpixel {
                current
                    .subpixel_sprites
                    .iter()
                    .position(|glyph| glyph.order == order)
            } else {
                current
                    .monochrome_sprites
                    .iter()
                    .position(|glyph| glyph.order == order)
            }
            .unwrap();
            assert_eq!(
                selected.map_range(index..index + 1, subpixel, 0).len(),
                1,
                "clipped/transformed glyph order {order} must survive production union selection"
            );
        }
        assert!(
            drawn * 10 < total,
            "partial draw must submit less than10% of the dense glyphs"
        );
        renderer.pre_draw(&[1.0; 4], Some(damage)).unwrap();
        renderer
            .draw_batches(&current, Some(damage), Some((&selected, 0)))
            .unwrap();
        let partial = renderer.read_target_to_image().unwrap();
        let full = renderer
            .render_to_image(&current, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_eq!(
            partial.as_raw(),
            full.as_raw(),
            "culled partial glyph runs must exactly match full rendering"
        );
    }
}
