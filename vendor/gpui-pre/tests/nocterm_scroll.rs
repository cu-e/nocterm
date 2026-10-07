// SPDX-License-Identifier: Apache-2.0
use ::gpui::*;

fn rect_area(rect: SceneDamageRect) -> u64 {
    (rect.right - rect.left) as u64 * (rect.bottom - rect.top) as u64
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    bounds(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(width), ScaledPixels(height)),
    )
}

fn scene(offset: u32, fractional: bool) -> Scene {
    let mut scene = Scene::default();
    let mask = ContentMask {
        bounds: rect(0.0, 0.0, 128.0, 96.0),
    };
    scene.quads.push(Quad {
        bounds: mask.bounds,
        content_mask: mask,
        background: hsla(0.6, 0.2, 0.2, 1.0).into(),
        ..Default::default()
    });
    for row in 0..8 {
        let y = row as f32 * 12.0 + 4.0 + if fractional { 0.25 } else { 0.0 };
        scene.monochrome_sprites.push(MonochromeSprite {
            order: 1,
            pad: 0,
            bounds: rect(20.0, y, 8.0, 8.0),
            content_mask: mask,
            color: hsla(0.2, 0.5, 0.8, 0.7),
            tile: AtlasTile {
                texture_id: AtlasTextureId {
                    index: 0,
                    kind: AtlasTextureKind::Monochrome,
                },
                tile_id: TileId(row + offset),
                padding: 0,
                bounds: bounds(
                    point(DevicePixels(0), DevicePixels(0)),
                    size(DevicePixels(8), DevicePixels(8)),
                ),
            },
            transformation: TransformationMatrix::unit(),
        });
    }
    scene.finish();
    scene
}

fn snapshot(scene: &Scene) -> SceneSnapshot {
    SceneSnapshot::capture(scene, [128, 96])
}

fn copied(plan: &SceneScrollPlan, x: u32, y: u32) -> bool {
    plan.copied
        .iter()
        .any(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

#[::core::prelude::v1::test]
fn upward_and_downward_overlap_copies_partition_region_exactly() {
    for (old_offset, current_offset, dy) in [(0, 1, -12), (1, 0, 12)] {
        let old = snapshot(&scene(old_offset, false));
        let current = snapshot(&scene(current_offset, false));
        let plan = current
            .scroll_since(&old)
            .expect("integer glyph scroll must reuse rows");
        assert_eq!(plan.dy, dy);
        assert!(
            plan.copied.iter().map(|r| rect_area(*r)).sum::<u64>()
                >= (plan.region.right - plan.region.left) as u64 * 80
        );
        for y in plan.region.top..plan.region.bottom {
            for x in plan.region.left..plan.region.right {
                let hits = plan
                    .copied
                    .iter()
                    .chain(&plan.redraw)
                    .filter(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                    .count();
                assert_eq!(hits, 1, "pixel ({x}, {y}) must be copied or redrawn once");
            }
        }
        for r in &plan.copied {
            assert!(r.top as i32 - dy >= 0);
            assert!(r.bottom as i32 - dy <= 96);
        }
    }
}

#[::core::prelude::v1::test]
fn stationary_cursor_and_alpha_selection_are_repaired_at_source_and_destination() {
    let overlay = Quad {
        order: 2,
        bounds: rect(18.0, 50.0, 24.0, 8.0),
        content_mask: ContentMask {
            bounds: rect(0.0, 0.0, 128.0, 96.0),
        },
        background: hsla(0.3, 0.8, 0.4, 0.5).into(),
        ..Default::default()
    };
    let mut old = scene(0, false);
    old.quads.push(overlay);
    old.finish();
    let mut current = scene(1, false);
    current.quads.push(overlay);
    current.finish();
    let plan = snapshot(&current).scroll_since(&snapshot(&old)).unwrap();
    for y in (38..46).chain(50..58) {
        assert!(
            !copied(&plan, 20, y),
            "stationary overlay row {y} needs replay"
        );
    }
    assert!(copied(&plan, 20, 25));
}

#[::core::prelude::v1::test]
fn clipping_and_unsupported_decoration_reject_only_their_rows() {
    let mut old = scene(0, false);
    let mut current = scene(1, false);
    for scene in [&mut old, &mut current] {
        scene.underlines.push(Underline {
            order: 2,
            pad: 0,
            bounds: rect(20.0, 50.0, 10.0, 2.0),
            content_mask: ContentMask {
                bounds: rect(0.0, 0.0, 128.0, 96.0),
            },
            color: hsla(0.0, 0.0, 1.0, 1.0),
            thickness: ScaledPixels(1.0),
            wavy: false.into(),
        });
        scene.monochrome_sprites[0].content_mask.bounds = rect(22.0, 0.0, 100.0, 96.0);
        scene.finish();
    }
    let plan = snapshot(&current).scroll_since(&snapshot(&old)).unwrap();
    assert!(!copied(&plan, 20, 6));
    assert!(!copied(&plan, 20, 50));
    assert!(copied(&plan, 20, 25));
}

#[::core::prelude::v1::test]
fn fractional_transformed_nonopaque_resize_and_unknown_coverage_fall_back() {
    let old = snapshot(&scene(0, false));
    assert!(
        snapshot(&scene(1, true))
            .scroll_since(&snapshot(&scene(0, true)))
            .is_none()
    );
    let mut current = scene(1, false);
    current.quads[0].background = hsla(0.6, 0.2, 0.2, 0.5).into();
    assert!(snapshot(&current).scroll_since(&old).is_none());
    let current = scene(1, false);
    assert!(
        SceneSnapshot::capture(&current, [128, 95])
            .scroll_since(&old)
            .is_none()
    );
    let mut current = current;
    for glyph in &mut current.monochrome_sprites {
        glyph.transformation.translation[1] = 1.0;
    }
    assert!(snapshot(&current).scroll_since(&old).is_none());
}

#[::core::prelude::v1::test]
fn exact_order_and_glyph_shader_inputs_are_required() {
    let old = snapshot(&scene(0, false));
    let mut current = scene(1, false);
    current.monochrome_sprites[2].color.a = 0.4;
    let plan = snapshot(&current).scroll_since(&old).unwrap();
    assert!(!copied(&plan, 20, 30));
    assert!(copied(&plan, 20, 18));
    current.monochrome_sprites[2].color.a = 0.7;
    current.monochrome_sprites[2].tile.padding = 1;
    let plan = snapshot(&current).scroll_since(&old).unwrap();
    assert!(!copied(&plan, 20, 30));
}

#[::core::prelude::v1::test]
fn repeated_glyphs_do_not_prove_changed_overlap_or_order() {
    let mut old = scene(0, false);
    let mut current = scene(1, false);
    for scene in [&mut old, &mut current] {
        // Repeated text offers several plausible offsets. The ordered proof
        // must still reject the rows where a second overlapping glyph differs.
        for sprite in &mut scene.monochrome_sprites {
            sprite.tile.tile_id = TileId(1);
        }
        let mut overlay = scene.monochrome_sprites[4];
        overlay.tile.tile_id = TileId(2);
        scene.monochrome_sprites.push(overlay);
    }
    // Make the ordinary scene damage cover the region despite repeated rows.
    current.monochrome_sprites[0].color.a = 0.4;
    current.monochrome_sprites[7].color.a = 0.4;
    old.finish();
    current.finish();
    let plan = snapshot(&current).scroll_since(&snapshot(&old)).unwrap();
    assert_eq!(plan.dy.abs(), 12);
    for y in 52..60 {
        assert!(!copied(&plan, 20, y), "overlap row {y} cannot be copied");
    }
}

#[::core::prelude::v1::test]
fn reordered_overlapping_glyphs_are_redrawn_even_when_their_translation_matches() {
    let mut old = scene(0, false);
    let mut current = scene(1, false);
    let mut overlay = old.monochrome_sprites[4];
    overlay.order = 2;
    overlay.tile.tile_id = TileId(99);
    overlay.color.a = 0.5;
    old.monochrome_sprites.push(overlay);
    overlay.order = 0;
    overlay.bounds.origin.y = ScaledPixels(overlay.bounds.origin.y.0 - 12.0);
    current.monochrome_sprites.push(overlay);
    old.finish();
    current.finish();
    let plan = snapshot(&current).scroll_since(&snapshot(&old)).unwrap();
    assert_eq!(plan.dy, -12);
    for y in 40..48 {
        assert!(!copied(&plan, 20, y), "reordered row {y} needs replay");
    }
    assert!(copied(&plan, 20, 25));
}

#[::core::prelude::v1::test]
fn backdrop_intersection_and_static_content_outside_it_are_preserved() {
    let mut old = scene(0, false);
    let mut current = scene(1, false);
    for scene in [&mut old, &mut current] {
        scene.quads[0].bounds = rect(16.0, 0.0, 96.0, 96.0);
        // This static UI is later than the backdrop but outside its region.
        scene.quads.push(Quad {
            order: 2,
            bounds: rect(0.0, 0.0, 12.0, 96.0),
            content_mask: ContentMask {
                bounds: rect(0.0, 0.0, 128.0, 96.0),
            },
            background: hsla(0.0, 0.0, 0.8, 0.5).into(),
            ..Default::default()
        });
        scene.finish();
    }
    let previous = snapshot(&old);
    let current = snapshot(&current);
    let plan = current.scroll_since(&previous).unwrap();
    assert_eq!(plan.region.left, 16);
    assert_eq!(plan.region.right, 64);
    for rect in &plan.copied {
        assert!(rect.left >= plan.region.left && rect.right <= plan.region.right);
    }
    let SceneDamage::Partial(damage) = current.damage_since(&previous) else {
        panic!("fixture must produce ordinary partial damage");
    };
    for y in damage.top..damage.bottom {
        for x in damage.left..damage.right {
            let hits = plan
                .copied
                .iter()
                .chain(&plan.redraw)
                .filter(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                .count();
            assert_eq!(hits, 1, "ordinary damage pixel ({x}, {y}) needs one repair");
        }
    }
    assert!(!copied(&plan, 5, 25));
    assert!(!copied(&plan, 80, 25));
    assert!(copied(&plan, 20, 25));
}

#[::core::prelude::v1::test]
fn changed_backdrop_and_scanline_work_budget_fall_back_safely() {
    let old = snapshot(&scene(0, false));
    let mut current = scene(1, false);
    current.quads[0].background = hsla(0.1, 0.2, 0.2, 1.0).into();
    assert!(snapshot(&current).scroll_since(&old).is_none());

    let mut old = scene(0, false);
    let mut current = scene(1, false);
    let mask = ContentMask {
        bounds: rect(0.0, 0.0, 128.0, 2000.0),
    };
    for (scene, alpha) in [(&mut old, 0.01), (&mut current, 0.02)] {
        scene.quads[0].bounds = mask.bounds;
        scene.quads[0].content_mask = mask;
        scene.quads.extend((0..1001).map(|_| Quad {
            order: 1,
            bounds: mask.bounds,
            content_mask: mask,
            background: hsla(0.1, 0.2, 0.2, alpha).into(),
            ..Default::default()
        }));
        for glyph in &mut scene.monochrome_sprites {
            glyph.order = 2;
        }
        scene.finish();
    }
    let old = SceneSnapshot::capture(&old, [128, 2000]);
    let current = SceneSnapshot::capture(&current, [128, 2000]);
    assert!(matches!(
        current.damage_since(&old),
        SceneDamage::Partial(_)
    ));
    assert!(current.scroll_since(&old).is_none());
}

#[::core::prelude::v1::test]
fn dense_rows_find_three_and_six_line_shifts_across_entire_previous_grid() {
    let make_scene = |offset: u32| {
        let mut scene = scene(offset, false);
        scene.quads[0].bounds = rect(0.0, 0.0, 800.0, 256.0);
        scene.quads[0].content_mask.bounds = scene.quads[0].bounds;
        let template = scene.monochrome_sprites[0];
        scene.monochrome_sprites.clear();
        for row in 0..20 {
            for column in 0..80 {
                let mut glyph = template;
                glyph.bounds = rect(
                    20.0 + column as f32 * 9.0,
                    row as f32 * 12.0 + 4.0,
                    8.0,
                    8.0,
                );
                glyph.content_mask.bounds = rect(0.0, 0.0, 800.0, 256.0);
                glyph.tile.tile_id = TileId(row + offset);
                scene.monochrome_sprites.push(glyph);
            }
        }
        scene.finish();
        SceneSnapshot::capture(&scene, [800, 256])
    };
    for (previous_offset, current_offset, dy) in [(0, 3, -36), (3, 0, 36), (0, 6, -72), (6, 0, 72)]
    {
        let previous = make_scene(previous_offset);
        let current = make_scene(current_offset);
        let plan = current
            .scroll_since(&previous)
            .expect("dense grid source search must include rows beyond first128 glyphs");
        assert_eq!(plan.dy, dy);
        assert!(
            plan.copied.iter().map(|r| rect_area(*r)).sum::<u64>() > rect_area(plan.region) / 2
        );
    }
}

#[::core::prelude::v1::test]
fn stationary_repeated_glyph_eligibility_scans_consume_the_work_budget() {
    let mut old = scene(0, false);
    let mut current = scene(1, false);
    let mut stationary = old.monochrome_sprites[3];
    stationary.tile.tile_id = TileId(999);
    stationary.bounds = rect(50.0, 40.0, 8.0, 8.0);
    // Each of these 1,000 anchors finds 1,000 matching old records, but no
    // nonzero shift. Eligibility checks alone exceed the 262,144-work limit.
    // Moving rows still offer a valid plan if those checks are not counted.
    for scene in [&mut old, &mut current] {
        scene.monochrome_sprites.extend([stationary; 1000]);
        scene.finish();
    }
    let old = snapshot(&old);
    let current = snapshot(&current);
    assert!(matches!(
        current.damage_since(&old),
        SceneDamage::Partial(_)
    ));
    assert!(
        current.scroll_since(&old).is_none(),
        "stationary-anchor eligibility searches must exhaust the bounded comparison budget"
    );
}

#[::core::prelude::v1::test]
fn fractional_opaque_backdrop_reuses_only_guarded_integer_interior() {
    let make = |offset| {
        let mut scene = scene(offset, false);
        scene.quads[0].bounds = rect(0.25, 0.25, 127.5, 95.5);
        scene.quads[0].content_mask.bounds = rect(1.25, 1.25, 125.5, 93.5);
        scene.finish();
        snapshot(&scene)
    };
    let previous = make(0);
    let current = make(1);
    let plan = current
        .scroll_since(&previous)
        .expect("fractional opaque backdrop has a safe interior");
    assert_eq!(plan.dy, -12);
    assert!(plan.region.left >= 3 && plan.region.top >= 3);
    assert!(plan.region.right <= 125 && plan.region.bottom <= 93);
    for rect in &plan.copied {
        assert!(rect.left >= plan.region.left && rect.right <= plan.region.right);
        assert!(rect.top >= plan.region.top && rect.bottom <= plan.region.bottom);
    }
    for (x, y) in [(1, 8), (20, 1), (20, 94)] {
        assert!(!copied(&plan, x, y));
        assert!(
            plan.redraw
                .iter()
                .any(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
        );
    }
}

#[::core::prelude::v1::test]
fn deepest_duplicate_backdrops_hide_unsupported_earlier_layers() {
    let make = |offset: u32| {
        let mut scene = scene(offset, false);
        scene.quads[0].bounds = rect(0.0, 0.0, 800.0, 256.0);
        scene.quads[0].content_mask.bounds = scene.quads[0].bounds;
        let backdrop = scene.quads[0];
        // Duplicate matching backgrounds must not pair the current final layer
        // with an old lower layer that exposes hidden unsupported primitives.
        for order in 1..=6 {
            let mut quad = backdrop;
            quad.order = order;
            if order % 2 == 1 {
                quad.corner_radii = Corners::all(ScaledPixels(4.0));
            }
            scene.quads.push(quad);
        }
        let mut template = scene.monochrome_sprites[0];
        template.order = 7;
        template.content_mask.bounds = backdrop.bounds;
        scene.monochrome_sprites.clear();
        for row in 0..20 {
            for column in 0..80 {
                let mut glyph = template;
                glyph.bounds = rect(
                    20.0 + column as f32 * 9.0,
                    row as f32 * 12.0 + 4.0,
                    8.0,
                    8.0,
                );
                glyph.tile.tile_id = TileId(row + offset);
                scene.monochrome_sprites.push(glyph);
            }
        }
        scene.finish();
        SceneSnapshot::capture(&scene, [800, 256])
    };
    for (old_offset, offset, dy) in [(0, 3, -36), (3, 0, 36)] {
        let plan = make(offset)
            .scroll_since(&make(old_offset))
            .expect("deepest identical opaque backdrops must hide all earlier layers");
        assert_eq!(plan.dy, dy);
        assert!(
            plan.copied.iter().map(|r| rect_area(*r)).sum::<u64>() > rect_area(plan.region) / 2
        );
    }
}

#[::core::prelude::v1::test]
fn actual_pane_masked_rounded_left_borders_preserve_scroll_interior() {
    let make = |offset: u32| {
        let mut scene = scene(offset, false);
        let pane = rect(246.0, 161.0, 813.0, 442.0);
        scene.quads[0].bounds = pane;
        scene.quads[0].content_mask.bounds = pane;
        let template = scene.monochrome_sprites[0];
        scene.monochrome_sprites.clear();
        for row in 0..25 {
            for column in 0..80 {
                let mut glyph = template;
                glyph.bounds = rect(
                    271.0 + column as f32 * 8.0,
                    168.0 + row as f32 * 17.0,
                    7.0,
                    13.0,
                );
                glyph.content_mask.bounds = pane;
                glyph.tile.tile_id = TileId(row + offset);
                scene.monochrome_sprites.push(glyph);
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
        SceneSnapshot::capture(&scene, [1064, 672])
    };
    for (old_offset, offset, dy) in [(0, 6, -102), (6, 0, 102)] {
        let old = make(old_offset);
        let current = make(offset);
        let SceneDamage::Partial(damage) = current.damage_since(&old) else {
            panic!("partial scene");
        };
        let plan = current
            .scroll_since(&old)
            .expect("conservative single border column must leave the wide pane interior reusable");
        assert_eq!(plan.dy, dy);
        assert_eq!(plan.region.left, 257);
        assert_eq!(plan.region.right, 960);
        for y in damage.top..damage.bottom {
            for x in damage.left..damage.right {
                let hits = plan
                    .copied
                    .iter()
                    .chain(&plan.redraw)
                    .filter(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                    .count();
                assert_eq!(
                    hits, 1,
                    "ordinary damage pixel({x},{y}) repaired exactly once"
                );
                if x == 256 {
                    assert!(!copied(&plan, x, y));
                }
            }
        }
    }
}

fn edge_fixture(offset: u32, width: u32) -> Scene {
    let mut scene = scene(offset, false);
    let viewport = rect(0.0, 0.0, width as f32, 96.0);
    scene.quads[0].bounds = viewport;
    scene.quads[0].content_mask.bounds = viewport;
    let template = scene.monochrome_sprites[0];
    scene.monochrome_sprites.clear();
    for row in 0..8 {
        for x in [20.0, 65.0, width as f32 - 8.0] {
            let mut glyph = template;
            glyph.bounds = rect(x, row as f32 * 12.0 + 4.0, 8.0, 8.0);
            glyph.content_mask.bounds = viewport;
            glyph.tile.tile_id = TileId(row + offset);
            scene.monochrome_sprites.push(glyph);
        }
    }
    scene
}

fn unsafe_vertical_strip(left: u32, right: u32, width: u32) -> Underline {
    assert!(right - left >= 2);
    Underline {
        order: 2,
        pad: 0,
        // The coverage guard adds one pixel on each side, yielding [left,right).
        bounds: rect(left as f32 + 1.0, 0.0, (right - left - 2) as f32, 96.0),
        content_mask: ContentMask {
            bounds: rect(0.0, 0.0, width as f32, 96.0),
        },
        color: hsla(0.0, 0.0, 1.0, 1.0),
        thickness: ScaledPixels(1.0),
        wavy: false.into(),
    }
}

fn edge_snapshot(mut scene: Scene, width: u32) -> SceneSnapshot {
    scene.finish();
    SceneSnapshot::capture(&scene, [width, 96])
}

#[::core::prelude::v1::test]
fn edge_crop_retains_at_least_half_of_an_odd_width() {
    for (right_start, expected) in [(96, None), (97, Some((32, 97)))] {
        let make = |offset| {
            let mut scene = edge_fixture(offset, 129);
            scene.underlines.push(unsafe_vertical_strip(0, 32, 129));
            scene
                .underlines
                .push(unsafe_vertical_strip(right_start, 129, 129));
            edge_snapshot(scene, 129)
        };
        let previous = make(0);
        let current = make(1);
        let SceneDamage::Partial(damage) = current.damage_since(&previous) else {
            panic!("fixture must produce ordinary partial damage");
        };
        assert_eq!(damage.right - damage.left, 129);
        let plan = current.scroll_since(&previous);
        match expected {
            None => assert!(plan.is_none(), "64 of 129 columns is below half"),
            Some((left, right)) => {
                let plan = plan.expect("65 of 129 columns is at least half");
                assert_eq!((plan.region.left, plan.region.right), (left, right));
                assert_eq!(plan.dy, -12);
            }
        }
    }
}

#[::core::prelude::v1::test]
fn edge_crop_combines_previous_left_and_current_right_coverage() {
    let mut old = edge_fixture(0, 128);
    old.underlines.push(unsafe_vertical_strip(0, 12, 128));
    let previous = edge_snapshot(old, 128);
    let mut current = edge_fixture(1, 128);
    current
        .underlines
        .push(unsafe_vertical_strip(116, 128, 128));
    let current = edge_snapshot(current, 128);
    let plan = current.scroll_since(&previous).unwrap();
    assert_eq!((plan.region.left, plan.region.right), (12, 116));
    assert_eq!(plan.dy, -12);
    let SceneDamage::Partial(damage) = current.damage_since(&previous) else {
        panic!("fixture must produce ordinary partial damage");
    };
    for y in damage.top..damage.bottom {
        for x in damage.left..damage.right {
            let hits = plan
                .copied
                .iter()
                .chain(&plan.redraw)
                .filter(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                .count();
            assert_eq!(hits, 1, "damage pixel ({x},{y}) must be repaired once");
            if !(12..116).contains(&x) {
                assert!(!copied(&plan, x, y), "both edge strips need redraw");
            }
        }
    }
}

#[::core::prelude::v1::test]
fn edge_crop_does_not_discard_an_unsafe_interior_column() {
    let make = |offset| {
        let mut scene = edge_fixture(offset, 128);
        scene.underlines.push(unsafe_vertical_strip(10, 18, 128));
        edge_snapshot(scene, 128)
    };
    let previous = make(0);
    let current = make(1);
    assert!(matches!(
        current.damage_since(&previous),
        SceneDamage::Partial(_)
    ));
    assert!(
        current.scroll_since(&previous).is_none(),
        "a detached interior obstacle must stay in the exact row proof"
    );
}

#[::core::prelude::v1::test]
fn edge_crop_interval_budget_is_shared_across_both_snapshots() {
    for (old_count, expected_plan) in [(256, true), (257, false)] {
        let make = |offset, count| {
            let mut scene = edge_fixture(offset, 128);
            scene
                .underlines
                .extend((0..count).map(|_| unsafe_vertical_strip(0, 12, 128)));
            edge_snapshot(scene, 128)
        };
        let previous = make(0, old_count);
        let current = make(1, 256);
        assert!(matches!(
            current.damage_since(&previous),
            SceneDamage::Partial(_)
        ));
        let plan = current.scroll_since(&previous);
        assert_eq!(
            plan.is_some(),
            expected_plan,
            "512 total intervals may crop; 513 must use the original region"
        );
        if let Some(plan) = plan {
            assert_eq!((plan.region.left, plan.region.right), (12, 128));
        }
    }
}

#[::core::prelude::v1::test]
fn unsupported_rows_keep_the_same_copy_partition_before_and_after_glyphs() {
    let make = |offset, decoration_order| {
        let mut scene = scene(offset, false);
        for glyph in &mut scene.monochrome_sprites {
            glyph.order = 2;
        }
        scene.underlines.push(Underline {
            order: decoration_order,
            pad: 0,
            bounds: rect(20.0, 50.0, 10.0, 2.0),
            content_mask: ContentMask {
                bounds: rect(0.0, 0.0, 128.0, 96.0),
            },
            color: hsla(0.0, 0.0, 1.0, 0.5),
            thickness: ScaledPixels(1.0),
            wavy: false.into(),
        });
        scene.finish();
        snapshot(&scene)
    };
    let before = make(1, 1).scroll_since(&make(0, 1)).unwrap();
    let after = make(1, 3).scroll_since(&make(0, 3)).unwrap();
    assert_eq!(before.dy, -12);
    assert_eq!(before.region, after.region);
    assert_eq!(before.copied, after.copied);
    assert_eq!(before.redraw, after.redraw);
    // Conservative AA coverage affects both the stationary row and the
    // translated old source row, regardless of when glyph references appear.
    for plan in [&before, &after] {
        for y in (37..41).chain(49..53) {
            assert!(!copied(plan, 20, y));
        }
        assert!(copied(plan, 20, 25));
    }
}
