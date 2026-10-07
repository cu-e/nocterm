// SPDX-License-Identifier: Apache-2.0
// Exact opt-in reuse/memo regressions against the unchanged classic oracle.
use super::super::tests::{path, quad, rect};
use super::*;

fn reusable(scene: &Scene, viewport: [u32; 2]) -> ReusableSceneSnapshot {
    ReusableSceneSnapshot::capture(scene, viewport, None)
}
fn copy_scene(scene: &Scene) -> Scene {
    let mut next = Scene::default();
    next.quads = scene.quads.clone();
    next.paths = scene.paths.clone();
    next.finish();
    next
}
fn order_pair(bright_x: f32, reversed: bool) -> Scene {
    let mut a = quad(0., 0., 0.2);
    a.bounds = rect(0., 0., 128., 64.);
    let mut bright = quad(bright_x, 8., 0.8);
    bright.background.solid.a = 1.;
    a.order = u32::from(reversed);
    bright.order = u32::from(!reversed);
    let mut scene = Scene::default();
    scene.quads = vec![a, bright];
    scene.finish();
    scene
}

#[test]
fn memo_checks_different_old_record_after_true_and_after_false() {
    for (x, left, right) in [(100., 64, 128), (20., 0, 64)] {
        let old = order_pair(x, true);
        let current = order_pair(x, false);
        let old_reuse = reusable(&old, [128, 64]);
        let current_reuse = reusable(&current, [128, 64]);
        let expected = SceneDamage::Partial(SceneDamageRect {
            left,
            top: 0,
            right,
            bottom: 64,
        });
        let mut memo = SceneComparisonMemo::default();
        assert_eq!(current_reuse.damage_since(&old_reuse, &mut memo), expected);
        assert_eq!(
            SceneSnapshot::capture(&current, [128, 64])
                .damage_since(&SceneSnapshot::capture(&old, [128, 64])),
            expected
        );
    }
}

#[test]
fn memo_resets_for_multiple_previous_snapshots_and_recapture() {
    let a = order_pair(100., false);
    let b = order_pair(100., true);
    let mut current = reusable(&a, [128, 64]);
    let old_a = reusable(&a, [128, 64]);
    let old_b = reusable(&b, [128, 64]);
    let mut memo = SceneComparisonMemo::default();
    for old in [&old_a, &old_b, &old_a, &old_b] {
        assert_eq!(
            current.damage_since(old, &mut memo),
            current.snapshot.damage_since(&old.snapshot)
        );
    }
    current = ReusableSceneSnapshot::capture(&b, [128, 64], Some(current));
    for old in [&old_a, &old_b, &old_a] {
        assert_eq!(
            current.damage_since(old, &mut memo),
            current.snapshot.damage_since(&old.snapshot)
        );
    }
}

#[test]
fn reuse_matches_classic_for_add_remove_order_clips_paths_and_recovery() {
    let mut scenes = vec![Scene::default()];
    let mut scene = order_pair(100., false);
    scenes.push(copy_scene(&scene));
    scene.quads[0].content_mask.bounds = rect(8., 0., 52., 64.);
    scenes.push(copy_scene(&scene));
    scene.quads.pop();
    scenes.push(copy_scene(&scene));
    scene.paths = vec![path(20., 10), path(90., 11)];
    scene.finish();
    scenes.push(copy_scene(&scene));
    scene.paths[0].vertices[0].st_position.x = 0.75;
    scenes.push(copy_scene(&scene));
    scene.quads[0].background.solid.a = f32::NAN;
    scenes.push(scene);
    scenes.push(order_pair(20., true));
    let mut buffer = None;
    let mut prior = None;
    let mut memo = SceneComparisonMemo::default();
    for scene in scenes.iter().cycle().take(40) {
        let fresh = SceneSnapshot::capture(scene, [128, 64]);
        let next = ReusableSceneSnapshot::capture(scene, [128, 64], buffer.take());
        assert_eq!(
            next.supports_partial_updates(),
            fresh.supports_partial_updates()
        );
        assert!(next.snapshot.records == fresh.records);
        assert_eq!(next.snapshot.tiles, fresh.tiles);
        if let Some(old) = &prior {
            assert_eq!(
                next.damage_since(old, &mut memo),
                fresh.damage_since(&old.snapshot)
            );
        }
        buffer = prior.replace(next);
    }
}

#[test]
fn moving_dense_content_bounds_actual_capacities_of_both_buffers_and_memo() {
    let viewport = [2048, 2048];
    let mut committed = None;
    let mut spare = None;
    let mut memo = SceneComparisonMemo::default();
    for frame in 0..320 {
        let mut scene = Scene::default();
        let x = (frame % 32) * 64;
        let y = ((frame / 32) % 32) * 64;
        for index in 0..384 {
            let mut p = quad(x as f32 + 8., y as f32 + 8., index as f32 / 400.);
            p.order = index;
            p.content_mask.bounds = rect(0., 0., 2048., 2048.);
            scene.quads.push(p);
        }
        scene.finish();
        let next = ReusableSceneSnapshot::capture(&scene, viewport, spare.take());
        assert!(next.supports_partial_updates());
        if let Some(old) = &committed {
            assert_eq!(
                next.damage_since(old, &mut memo),
                next.snapshot.damage_since(&old.snapshot)
            );
        }
        for snapshot in std::iter::once(&next).chain(committed.iter()) {
            let sum = snapshot
                .snapshot
                .tiles
                .iter()
                .map(Vec::capacity)
                .sum::<usize>();
            assert_eq!(sum, snapshot.index_capacity);
            assert!(sum <= MAX_TILE_REFERENCES);
            assert!(snapshot.snapshot.records.capacity() <= MAX_RECORDS);
            assert!(snapshot.snapshot.tiles.capacity() <= MAX_TILES);
            assert!(snapshot.reserved_bytes() <= ReusableSceneSnapshot::reserved_byte_limit());
        }
        assert!(memo.entries.capacity() <= MAX_RECORDS);
        let bytes = next.reserved_bytes()
            + committed
                .as_ref()
                .map_or(0, ReusableSceneSnapshot::reserved_bytes)
            + memo.reserved_bytes();
        assert!(
            bytes
                <= 2 * ReusableSceneSnapshot::reserved_byte_limit()
                    + SceneComparisonMemo::reserved_byte_limit()
        );
        spare = committed.replace(next);
    }
}

#[test]
fn historical_tile_capacity_exhaustion_releases_and_recovers() {
    let mut scene = Scene::default();
    scene.quads = vec![quad(4., 4., 0.2)];
    scene.finish();
    let mut buffer = reusable(&scene, [128, 64]);
    // Reserve nearly the entire capacity budget in a currently unused tile.
    // The next tile needs growth while this historical allocation persists.
    buffer.snapshot.tiles[0]
        .try_reserve_exact(MAX_TILE_REFERENCES - 4)
        .unwrap();
    buffer.index_capacity = buffer.snapshot.tiles.iter().map(Vec::capacity).sum();
    scene.quads = vec![quad(80., 4., 0.3); 8];
    scene.finish();
    let rejected = ReusableSceneSnapshot::capture(&scene, [128, 64], Some(buffer));
    assert!(!rejected.supports_partial_updates());
    assert_eq!(rejected.reserved_bytes(), 0);
    let recovered =
        ReusableSceneSnapshot::capture(&order_pair(20., false), [128, 64], Some(rejected));
    assert!(recovered.supports_partial_updates());
}

#[test]
fn equal_tile_counts_with_different_viewport_shapes_release_old_storage() {
    let old = reusable(&order_pair(20., false), [128, 64]);
    let next = ReusableSceneSnapshot::capture(&Scene::default(), [64, 128], Some(old));
    assert_eq!(next.viewport(), [64, 128]);
    assert!(next.supports_partial_updates());
    assert_eq!(next.snapshot.records.capacity(), 0);
    assert_eq!(next.index_capacity, 0);
}

#[test]
fn primitive_budget_and_nested_path_capacity_exhaustion_release_storage() {
    let mut budget = Scene::default();
    budget.quads = vec![quad(0., 0., 0.1); MAX_RECORDS + 1];
    let rejected = ReusableSceneSnapshot::capture(
        &budget,
        [128, 64],
        Some(reusable(&order_pair(20., false), [128, 64])),
    );
    assert!(!rejected.supports_partial_updates());
    assert_eq!(rejected.reserved_bytes(), 0);
    let mut source = Scene::default();
    source.paths = vec![path(10., 0)];
    let mut snapshot = reusable(&source, [128, 64]);
    assert!(snapshot.supports_partial_updates());
    // Stored nested capacities must be included in the actual reserved bytes.
    assert_eq!(snapshot.path_capacity, 1);
    assert_eq!(snapshot.vertex_capacity, 1);
    snapshot = ReusableSceneSnapshot::capture(&Scene::default(), [128, 64], Some(snapshot));
    assert_eq!(snapshot.path_capacity, 0);
    assert_eq!(snapshot.vertex_capacity, 0);
    assert!(snapshot.snapshot.records.is_empty());
}

#[test]
fn memo_capacity_failure_is_conservative_and_clears_old_entries() {
    let current = reusable(&order_pair(20., false), [128, 64]);
    let old = reusable(&order_pair(20., false), [128, 64]);
    let mut memo = SceneComparisonMemo::default();
    memo.entries.try_reserve_exact(MAX_RECORDS + 1).unwrap();
    assert_eq!(current.damage_since(&old, &mut memo), SceneDamage::Full);
    assert_eq!(memo.reserved_bytes(), 0);
    assert_eq!(
        current.damage_since(&old, &mut memo),
        SceneDamage::Unchanged
    );
}
