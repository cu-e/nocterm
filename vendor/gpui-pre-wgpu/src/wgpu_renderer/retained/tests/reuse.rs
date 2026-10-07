// SPDX-License-Identifier: Apache-2.0
// Nocterm regression tests; native GPU fixtures use exclusive driver lifetimes.
// Successful publication and error/atlas-race storage ownership regressions.
use super::*;

fn storage_bytes(frame: &RetainedFrame) -> usize {
    frame
        .snapshot
        .as_ref()
        .map_or(0, ReusableSceneSnapshot::reserved_bytes)
        + frame
            .spare
            .as_ref()
            .map_or(0, ReusableSceneSnapshot::reserved_bytes)
        + frame.comparison.reserved_bytes()
}

#[test]
fn repeated_reused_frames_match_full_pixels_and_release_on_viewport_change() -> Result<()> {
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    for index in 0..40 {
        assert_matches(
            &mut renderer,
            &scene(index % 2 == 0),
            viewport(256, 192),
            false,
            wgpu::Color::BLACK,
        )?;
        assert!(
            storage_bytes(&renderer.core.retained)
                <= 2 * ReusableSceneSnapshot::reserved_byte_limit()
                    + SceneComparisonMemo::reserved_byte_limit()
        );
    }
    assert!(renderer.core.retained.spare.is_some());
    // Same tile count, different dimensions: stale capacities/pixels cannot survive.
    assert_matches(
        &mut renderer,
        &scene(false),
        viewport(192, 256),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    assert!(renderer.core.retained.spare.is_none());
    assert_eq!(renderer.core.retained.comparison.reserved_bytes(), 0);
    renderer.core.retained.invalidate();
    assert_eq!(storage_bytes(&renderer.core.retained), 0);
    Ok(())
}

#[test]
fn rendering_error_discards_both_snapshots_and_next_frame_repairs_fully() -> Result<()> {
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    for changed in [false, true, false] {
        assert_matches(
            &mut renderer,
            &scene(changed),
            viewport(256, 192),
            false,
            wgpu::Color::BLACK,
        )?;
    }
    assert!(renderer.core.retained.spare.is_some());
    let capacity = renderer.core.instance_data_capacity;
    let maximum = renderer.core.max_instance_data_size;
    renderer.core.instance_data_capacity = 16;
    renderer.core.max_instance_data_size = 16;
    let failed =
        renderer
            .core
            .render_retained(&scene(true), viewport(256, 192), false, wgpu::Color::BLACK);
    assert!(failed.is_err()); // Actual instance-encoding failure, no publication.
    assert_eq!(storage_bytes(&renderer.core.retained), 0);
    renderer.core.instance_data_capacity = capacity;
    renderer.core.max_instance_data_size = maximum;
    assert_matches(
        &mut renderer,
        &scene(true),
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    renderer.check_gpu_errors()?;
    Ok(())
}

#[test]
fn real_atlas_mutation_after_submission_rejects_commit_and_next_frame_repairs_fully() -> Result<()>
{
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    for changed in [false, true, false] {
        assert_matches(
            &mut renderer,
            &scene(changed),
            viewport(256, 192),
            false,
            wgpu::Color::BLACK,
        )?;
    }
    let before = renderer.core.atlas.content_revision();
    renderer.core.retained.mutate_atlas_before_commit = true;
    assert_matches(
        &mut renderer,
        &scene(true),
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_ne!(renderer.core.atlas.content_revision(), before);
    assert_eq!(storage_bytes(&renderer.core.retained), 0);
    assert_matches(
        &mut renderer,
        &scene(true),
        viewport(256, 192),
        false,
        wgpu::Color::BLACK,
    )?;
    assert_eq!(renderer.core.retained.last_damage, Some(SceneDamage::Full));
    renderer.check_gpu_errors()?;
    Ok(())
}
