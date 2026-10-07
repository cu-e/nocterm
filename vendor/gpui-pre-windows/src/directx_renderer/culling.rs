// Nocterm modifications, licensed under the upstream Apache-2.0 license.
//! Pack ordered glyphs for each repair strip into one upload, then map offsets.
use super::*;
use std::ops::Range;

mod selection;

pub(super) struct GlyphSelection {
    mono: selection::Selected<MonochromeSprite>,
    subpixel: selection::Selected<SubpixelSprite>,
}

impl GlyphSelection {
    pub(super) fn capture(scene: &Scene, damage: &[SceneDamageRect]) -> Option<Self> {
        let count = scene
            .monochrome_sprites
            .len()
            .checked_add(scene.subpixel_sprites.len())?;
        if !selection::within_budget(count, damage.len()) {
            return None;
        }
        let visible = |strip: usize, index: usize, subpixel: bool| {
            let texture_id = if subpixel {
                scene.subpixel_sprites[index].tile.texture_id
            } else {
                scene.monochrome_sprites[index].tile.texture_id
            };
            {
                let range = index..index + 1;
                let batch = if subpixel {
                    PrimitiveBatch::SubpixelSprites { texture_id, range }
                } else {
                    PrimitiveBatch::MonochromeSprites { texture_id, range }
                };
                // AA, clipping, unknown geometry and transforms remain conservative.
                damage[strip].intersects_batch(scene, &batch)
            }
        };
        let mono = selection::select(
            &scene.monochrome_sprites,
            damage.len(),
            65_536,
            |strip, index| visible(strip, index, false),
        )?;
        let subpixel = selection::select(
            &scene.subpixel_sprites,
            damage.len(),
            65_536 - mono.instances.len(),
            |strip, index| visible(strip, index, true),
        )?;
        Some(Self { mono, subpixel })
    }

    pub(super) fn map_range(
        &self,
        range: Range<usize>,
        subpixel: bool,
        strip: usize,
    ) -> Range<usize> {
        if subpixel {
            self.subpixel.map_range(range, strip)
        } else {
            self.mono.map_range(range, strip)
        }
    }
}

impl DirectXRenderer {
    pub(super) fn prepare_damage_buffers(
        &mut self,
        scene: &Scene,
        damage: &[SceneDamageRect],
    ) -> Result<Option<GlyphSelection>> {
        // Limits are checked before uploads or target writes. Original scene
        // buffers and offsets are the conservative preparation-limit fallback.
        let Some(selection) = GlyphSelection::capture(scene, damage) else {
            self.upload_scene_buffers(scene)?;
            return Ok(None);
        };
        self.upload_scene_buffers_without_glyphs(scene)?;
        let devices = self.devices.as_ref().context("devices missing")?;
        if !selection.mono.instances.is_empty() {
            self.pipelines.mono_sprites.update_buffer(
                &devices.device,
                &devices.device_context,
                &selection.mono.instances,
            )?;
        }
        if !selection.subpixel.instances.is_empty() {
            self.pipelines.subpixel_sprites.update_buffer(
                &devices.device,
                &devices.device_context,
                &selection.subpixel.instances,
            )?;
        }
        Ok(Some(selection))
    }

    pub(super) fn draw_glyph_batch(
        &mut self,
        texture_id: AtlasTextureId,
        range: Range<usize>,
        selection: Option<(&GlyphSelection, usize)>,
        subpixel: bool,
    ) -> Result<()> {
        let range = selection.map_or_else(
            || range.clone(),
            |(selected, strip)| selected.map_range(range.clone(), subpixel, strip),
        );
        if range.is_empty() {
            return Ok(());
        }
        // All buffer maps finished before the first strip; only offsets change.
        if subpixel {
            self.draw_subpixel_sprites(texture_id, range.start, range.len())
        } else {
            self.draw_monochrome_sprites(texture_id, range.start, range.len())
        }
    }
}
