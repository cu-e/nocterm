// SPDX-License-Identifier: Apache-2.0
// Nocterm regression coverage for GPUI's retained scene damage.
use super::*;
use crate::{DevicePixels, TileId, bounds, size};

pub(super) fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    bounds(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(width), ScaledPixels(height)),
    )
}

pub(super) fn mask() -> ContentMask<ScaledPixels> {
    ContentMask {
        bounds: rect(0.0, 0.0, 1024.0, 1024.0),
    }
}

pub(super) fn quad(x: f32, y: f32, hue: f32) -> Quad {
    Quad {
        bounds: rect(x, y, 10.0, 10.0),
        content_mask: mask(),
        background: Hsla {
            h: hue,
            s: 0.5,
            l: 0.5,
            a: 0.5,
        }
        .into(),
        ..Default::default()
    }
}

fn snapshot(scene: &mut Scene) -> SceneSnapshot {
    scene.finish();
    SceneSnapshot::capture(scene, [256, 256])
}

fn changed_rect(current: &SceneSnapshot, old: &SceneSnapshot) -> SceneDamageRect {
    match current.damage_since(old) {
        SceneDamage::Partial(rect) => rect,
        other => panic!("expected partial damage, got {other:?}"),
    }
}

pub(super) fn path(x: f32, order: u32) -> Path<ScaledPixels> {
    let mut p = Path::new(point(Pixels::from(x), Pixels::from(20.0))).scale(1.0);
    p.order = order;
    p.bounds = rect(x, 20.0, 10.0, 10.0);
    p.content_mask = mask();
    p.vertices.push(PathVertex {
        xy_position: point(ScaledPixels(x), ScaledPixels(20.0)),
        st_position: point(0.0, 1.0),
        content_mask: mask(),
    });
    p
}

#[test]
fn identical_scene_and_order_renumbering_are_unchanged() {
    let mut scene = Scene::default();
    scene.quads = vec![quad(20.0, 20.0, 0.1), quad(140.0, 20.0, 0.2)];
    scene.quads[1].order = 1;
    let old = snapshot(&mut scene);
    scene.quads[0].order = 100;
    scene.quads[1].order = 101;
    assert_eq!(
        snapshot(&mut scene).damage_since(&old),
        SceneDamage::Unchanged
    );
}

#[test]
fn insertion_and_deletion_only_damage_the_affected_tile() {
    let mut scene = Scene::default();
    scene.quads = vec![quad(20.0, 20.0, 0.1), quad(140.0, 140.0, 0.2)];
    let old = snapshot(&mut scene);
    scene.quads.insert(0, quad(75.0, 75.0, 0.3));
    let inserted = snapshot(&mut scene);
    let expected = SceneDamageRect {
        left: 64,
        top: 64,
        right: 128,
        bottom: 128,
    };
    assert_eq!(changed_rect(&inserted, &old), expected);
    scene.quads.remove(0);
    assert_eq!(changed_rect(&snapshot(&mut scene), &inserted), expected);
}

#[test]
fn moving_geometry_covers_old_and_new_pixels_and_clamps_viewport() {
    let mut scene = Scene::default();
    scene.quads.push(quad(20.0, 20.0, 0.1));
    let old = snapshot(&mut scene);
    scene.quads[0].bounds = rect(250.0, 250.0, 20.0, 20.0);
    assert_eq!(
        changed_rect(&snapshot(&mut scene), &old),
        SceneDamageRect {
            left: 0,
            top: 0,
            right: 256,
            bottom: 256
        }
    );
    let old = snapshot(&mut scene);
    scene.quads.clear();
    assert_eq!(
        changed_rect(&snapshot(&mut scene), &old),
        SceneDamageRect {
            left: 192,
            top: 192,
            right: 256,
            bottom: 256
        }
    );
}

#[test]
fn underline_appearance_and_clip_changes_damage_visible_pixels() {
    let mut scene = Scene::default();
    scene.underlines.push(Underline {
        order: 0,
        pad: 0,
        bounds: rect(75.0, 75.0, 30.0, 5.0),
        content_mask: mask(),
        color: Hsla::default(),
        thickness: ScaledPixels(1.0),
        wavy: false.into(),
    });
    let old = snapshot(&mut scene);
    scene.underlines[0].wavy = true.into();
    assert_eq!(
        changed_rect(&snapshot(&mut scene), &old),
        SceneDamageRect {
            left: 64,
            top: 64,
            right: 128,
            bottom: 128
        }
    );
    let old = snapshot(&mut scene);
    scene.underlines[0].content_mask.bounds = rect(0.0, 0.0, 10.0, 10.0);
    assert_eq!(
        changed_rect(&snapshot(&mut scene), &old),
        SceneDamageRect {
            left: 64,
            top: 64,
            right: 128,
            bottom: 128
        }
    );
}

#[test]
fn reordering_overlap_changes_pixels_but_disjoint_reordering_does_not() {
    let mut scene = Scene::default();
    scene.quads = vec![quad(20.0, 20.0, 0.1), quad(25.0, 25.0, 0.2)];
    let old = snapshot(&mut scene);
    scene.quads.swap(0, 1);
    assert_eq!(
        changed_rect(&snapshot(&mut scene), &old),
        SceneDamageRect {
            left: 0,
            top: 0,
            right: 64,
            bottom: 64
        }
    );
    scene.quads[1].bounds = rect(140.0, 140.0, 10.0, 10.0);
    let old = snapshot(&mut scene);
    scene.quads.swap(0, 1);
    assert_eq!(
        snapshot(&mut scene).damage_since(&old),
        SceneDamage::Unchanged
    );
}

#[test]
fn clips_fractional_edges_and_alpha_changes_are_covered() {
    let mut scene = Scene::default();
    let mut p = quad(63.8, 63.8, 0.1);
    p.content_mask.bounds = rect(63.8, 63.8, 0.4, 0.4);
    scene.quads.push(p);
    let old = snapshot(&mut scene);
    scene.quads[0].background = Hsla {
        a: 0.2,
        ..Hsla::default()
    }
    .into();
    assert_eq!(
        changed_rect(&snapshot(&mut scene), &old),
        SceneDamageRect {
            left: 0,
            top: 0,
            right: 128,
            bottom: 128
        }
    );
    scene.quads[0].bounds = rect(500.0, 500.0, 10.0, 10.0);
    let old = snapshot(&mut scene);
    scene.quads[0].background = Hsla {
        a: 0.8,
        ..Hsla::default()
    }
    .into();
    assert_eq!(
        snapshot(&mut scene).damage_since(&old),
        SceneDamage::Unchanged
    );
}

#[test]
fn shadows_cover_three_blur_radii_and_insets_use_element_bounds() {
    let shadow = Shadow {
        order: 0,
        blur_radius: ScaledPixels(10.0),
        bounds: rect(90.0, 90.0, 10.0, 10.0),
        corner_radii: Default::default(),
        content_mask: mask(),
        color: Hsla::default(),
        element_bounds: rect(180.0, 180.0, 10.0, 10.0),
        element_corner_radii: Default::default(),
        inset: 0,
        pad: 0,
    };
    let mut scene = Scene::default();
    scene.shadows.push(shadow);
    let old = snapshot(&mut scene);
    scene.shadows[0].color.a = 0.5;
    assert_eq!(
        changed_rect(&snapshot(&mut scene), &old),
        SceneDamageRect {
            left: 0,
            top: 0,
            right: 192,
            bottom: 192
        }
    );
    scene.shadows[0].inset = 1;
    let old = snapshot(&mut scene);
    scene.shadows[0].color.a = 0.8;
    assert_eq!(
        changed_rect(&snapshot(&mut scene), &old),
        SceneDamageRect {
            left: 128,
            top: 128,
            right: 192,
            bottom: 192
        }
    );
}

#[test]
fn paths_track_vertices_ids_and_copy_mode() {
    let mut scene = Scene::default();
    scene.paths = vec![path(20.0, 5), path(140.0, 5)];
    let old = snapshot(&mut scene);
    scene.paths[0].id = PathId(200);
    scene.paths[1].id = PathId(201);
    scene.paths.iter_mut().for_each(|p| p.order = 100);
    assert_eq!(
        snapshot(&mut scene).damage_since(&old),
        SceneDamage::Unchanged
    );
    scene.paths[1].order = 101;
    assert_ne!(
        snapshot(&mut scene).damage_since(&old),
        SceneDamage::Unchanged
    );
    let old = snapshot(&mut scene);
    scene.paths[0].vertices[0].st_position.y = 0.5;
    assert_ne!(
        snapshot(&mut scene).damage_since(&old),
        SceneDamage::Unchanged
    );
}

#[test]
fn path_batch_filter_preserves_mixed_order_copy_gaps() {
    let mut scene = Scene::default();
    scene.paths = vec![path(20.0, 1), path(140.0, 1)];
    let gap = SceneDamageRect {
        left: 70,
        top: 20,
        right: 80,
        bottom: 30,
    };
    let batch = PrimitiveBatch::Paths(0..2);
    assert!(!gap.intersects_batch(&scene, &batch));
    scene.paths[1].order = 2;
    assert!(gap.intersects_batch(&scene, &batch));
}

#[test]
fn fallback_on_transform_surface_nonfinite_resize_and_budget() {
    let mut scene = Scene::default();
    let old = snapshot(&mut scene);
    scene.monochrome_sprites.push(MonochromeSprite {
        order: 0,
        pad: 0,
        bounds: rect(20.0, 20.0, 10.0, 10.0),
        content_mask: mask(),
        color: Hsla::default(),
        transformation: TransformationMatrix::unit().rotate(Radians(0.1)),
        tile: AtlasTile {
            texture_id: AtlasTextureId {
                index: 0,
                kind: crate::AtlasTextureKind::Monochrome,
            },
            tile_id: TileId(0),
            padding: 0,
            bounds: bounds(
                point(DevicePixels(0), DevicePixels(0)),
                size(DevicePixels(10), DevicePixels(10)),
            ),
        },
    });
    assert_eq!(snapshot(&mut scene).damage_since(&old), SceneDamage::Full);
    scene.monochrome_sprites.clear();
    scene.quads.push(quad(f32::NAN, 20.0, 0.1));
    assert_eq!(snapshot(&mut scene).damage_since(&old), SceneDamage::Full);
    scene.quads.clear();
    assert_eq!(
        SceneSnapshot::capture(&scene, [128, 128]).damage_since(&old),
        SceneDamage::Full
    );
    assert_eq!(
        SceneSnapshot::capture(&scene, [u32::MAX, u32::MAX]).damage_since(&old),
        SceneDamage::Full
    );
    #[cfg(not(target_os = "macos"))]
    scene.surfaces.push(PaintSurface {
        order: 0,
        bounds: rect(20.0, 20.0, 10.0, 10.0),
        content_mask: mask(),
    });
    #[cfg(not(target_os = "macos"))]
    assert_eq!(snapshot(&mut scene).damage_since(&old), SceneDamage::Full);
}

#[test]
fn excessive_primitive_storage_falls_back_before_copying_records() {
    let mut scene = Scene::default();
    let old = snapshot(&mut scene);
    scene.quads = vec![quad(20.0, 20.0, 0.1); 50_001];
    let excessive = snapshot(&mut scene);
    assert!(!excessive.supports_partial_updates());
    assert_eq!(excessive.damage_since(&old), SceneDamage::Full);
}

#[test]
fn capture_support_is_distinct_from_supported_full_damage() {
    let mut scene = Scene::default();
    let previous = snapshot(&mut scene);
    assert!(previous.supports_partial_updates());
    let resized = SceneSnapshot::capture(&scene, [128, 128]);
    assert!(resized.supports_partial_updates());
    assert_eq!(resized.damage_since(&previous), SceneDamage::Full);
    scene.quads.push(quad(f32::NAN, 20.0, 0.1));
    assert!(!snapshot(&mut scene).supports_partial_updates());
    let empty = Scene::default();
    assert!(!SceneSnapshot::capture(&empty, [0, 128]).supports_partial_updates());
    assert!(!SceneSnapshot::capture(&empty, [u32::MAX, u32::MAX]).supports_partial_updates());
}

#[test]
fn empty_clipped_and_zero_area_geometry_do_not_dirty_visible_tiles() {
    let mut scene = Scene::default();
    let empty = snapshot(&mut scene);
    assert_eq!(
        snapshot(&mut scene).damage_since(&empty),
        SceneDamage::Unchanged
    );
    for geometry in [
        rect(-30.0, -30.0, 10.0, 10.0),
        rect(260.0, 30.0, 10.0, 10.0),
        rect(30.0, 30.0, 0.0, 10.0),
        rect(30.0, 30.0, 10.0, 0.0),
    ] {
        let mut p = quad(0.0, 0.0, 0.1);
        p.bounds = geometry;
        scene.quads = vec![p];
        let hidden = snapshot(&mut scene);
        assert_eq!(hidden.damage_since(&empty), SceneDamage::Unchanged);
        assert_eq!(empty.damage_since(&hidden), SceneDamage::Unchanged);
    }
}

#[test]
fn viewport_edge_guard_and_half_open_batch_intersections_are_conservative() {
    let mut scene = Scene::default();
    let empty = SceneSnapshot::capture(&scene, [65, 67]);
    scene.quads.push(quad(64.5, 66.5, 0.1));
    scene.finish();
    assert_eq!(
        changed_rect(&SceneSnapshot::capture(&scene, [65, 67]), &empty),
        SceneDamageRect {
            left: 0,
            top: 64,
            right: 65,
            bottom: 67
        }
    );
    scene.quads[0].bounds = rect(65.0, 30.0, 10.0, 10.0);
    let damage = SceneDamageRect {
        left: 0,
        top: 0,
        right: 64,
        bottom: 64,
    };
    assert!(!damage.intersects_batch(&scene, &PrimitiveBatch::Quads(0..1)));
    scene.quads[0].bounds.origin.x = ScaledPixels(64.99);
    assert!(damage.intersects_batch(&scene, &PrimitiveBatch::Quads(0..1)));
}

#[test]
fn overlap_order_across_primitive_kinds_changes_visible_pixels() {
    let mut scene = Scene::default();
    scene.quads.push(quad(20.0, 20.0, 0.1));
    scene.underlines.push(Underline {
        order: 1,
        pad: 0,
        bounds: rect(22.0, 22.0, 5.0, 3.0),
        content_mask: mask(),
        color: Hsla::default(),
        thickness: ScaledPixels(1.0),
        wavy: false.into(),
    });
    let old = snapshot(&mut scene);
    scene.quads[0].order = 2;
    assert_eq!(
        changed_rect(&snapshot(&mut scene), &old),
        SceneDamageRect {
            left: 0,
            top: 0,
            right: 64,
            bottom: 64
        }
    );
}

#[test]
fn invalid_bounds_masks_styles_and_vertices_fall_back_in_both_directions() {
    let mut empty = Scene::default();
    let valid = snapshot(&mut empty);
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut cases = Vec::new();
        let mut p = quad(20.0, 20.0, 0.1);
        p.bounds.size.width = ScaledPixels(invalid);
        cases.push(p);
        let mut p = quad(20.0, 20.0, 0.1);
        p.content_mask.bounds.origin.y = ScaledPixels(invalid);
        cases.push(p);
        let mut p = quad(20.0, 20.0, 0.1);
        p.border_widths.left = ScaledPixels(invalid);
        cases.push(p);
        let mut p = quad(20.0, 20.0, 0.1);
        p.corner_radii.top_left = ScaledPixels(invalid);
        cases.push(p);
        let mut p = quad(20.0, 20.0, 0.1);
        p.background = Hsla {
            a: invalid,
            ..Hsla::default()
        }
        .into();
        cases.push(p);
        for p in cases {
            let mut scene = Scene::default();
            scene.quads.push(p);
            let unsupported = snapshot(&mut scene);
            assert_eq!(unsupported.damage_since(&valid), SceneDamage::Full);
            assert_eq!(valid.damage_since(&unsupported), SceneDamage::Full);
        }
        let mut scene = Scene::default();
        scene.paths.push(path(20.0, 0));
        scene.paths[0].vertices[0].st_position.x = invalid;
        let unsupported = snapshot(&mut scene);
        assert_eq!(unsupported.damage_since(&valid), SceneDamage::Full);
        assert_eq!(valid.damage_since(&unsupported), SceneDamage::Full);
    }
    for malformed in [
        rect(20.0, 20.0, -1.0, 10.0),
        rect(f32::MAX, 20.0, f32::MAX, 10.0),
    ] {
        let mut scene = Scene::default();
        let mut p = quad(20.0, 20.0, 0.1);
        p.bounds = malformed;
        scene.quads.push(p);
        assert_eq!(snapshot(&mut scene).damage_since(&valid), SceneDamage::Full);
    }
}

#[test]
fn tile_reference_and_path_vertex_budgets_force_full_rendering() {
    let mut scene = Scene::default();
    let old = SceneSnapshot::capture(&scene, [1024, 1024]);
    let mut p = quad(0.0, 0.0, 0.1);
    p.bounds = rect(0.0, 0.0, 1024.0, 1024.0);
    scene.quads = vec![p; 1954];
    scene.finish();
    let unsupported = SceneSnapshot::capture(&scene, [1024, 1024]);
    assert_eq!(unsupported.damage_since(&old), SceneDamage::Full);
    assert_eq!(old.damage_since(&unsupported), SceneDamage::Full);
    scene.quads.clear();
    let mut p = path(20.0, 0);
    p.vertices.resize(500_001, p.vertices[0].clone());
    scene.paths.push(p);
    assert_eq!(
        SceneSnapshot::capture(&scene, [1024, 1024]).damage_since(&old),
        SceneDamage::Full
    );
}

#[test]
fn zero_size_and_tile_budget_boundary_have_safe_fallbacks() {
    let scene = Scene::default();
    let allowed = SceneSnapshot::capture(&scene, [16_384, 16_384]);
    assert_eq!(allowed.damage_since(&allowed), SceneDamage::Unchanged);
    for viewport in [[0, 256], [256, 0], [16_448, 16_384]] {
        let unsupported = SceneSnapshot::capture(&scene, viewport);
        assert_eq!(unsupported.damage_since(&unsupported), SceneDamage::Full);
        assert_eq!(unsupported.damage_since(&allowed), SceneDamage::Full);
    }
}
