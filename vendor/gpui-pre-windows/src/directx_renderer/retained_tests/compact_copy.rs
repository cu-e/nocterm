// Nocterm regression tests, licensed under the upstream Apache-2.0 license.
use super::*;

#[::core::prelude::v1::test]
fn packed_strips_preserve_texture_order_offsets_and_preparation_fallback() {
    let devices = DirectXDevices::new().unwrap();
    for subpixel in [false, true] {
        let (mut renderer, _window) = renderer(&devices);
        renderer
            .resize(size(DevicePixels(1080), DevicePixels(680)))
            .unwrap();
        let tiles = super::scroll_copy::tiles(&renderer, subpixel);
        let bytes = vec![150; 1050 * 8 * if subpixel { 4 } else { 1 }];
        let second = renderer
            .atlas
            .get_or_insert_with(glyph_key(subpixel, false, 9001), &mut || {
                Ok(Some((
                    size(DevicePixels(1050), DevicePixels(8)),
                    Cow::Borrowed(&bytes),
                )))
            })
            .unwrap()
            .unwrap();
        assert_ne!(
            tiles[0].texture_id, second.texture_id,
            "oversized tile forces a second atlas texture"
        );
        let damage = SceneDamageRect {
            left: 320,
            top: 80,
            right: 384,
            bottom: 128,
        };
        let make = |changed: bool| {
            let mut scene = super::scroll_copy::dense_scene(&renderer, &tiles, 0, subpixel);
            let mut push = |order, index: usize, tile, visible: bool| {
                let bounds = rect(
                    match order {
                        14 => 600.0,
                        15 => 470.0,
                        _ if visible => 330.0,
                        _ => 780.0,
                    } + (index % 3) as f32 * 2.0,
                    95.0,
                    10.0,
                    10.0,
                );
                let color = if changed && visible {
                    // Distinct colors make overlapping alpha draws depend on
                    // both their instance order and their texture-batch order.
                    hsla(order as f32 * 0.057 + index as f32 * 0.001, 0.5, 0.7, 0.6)
                } else {
                    hsla(index as f32 * 0.002, 0.5, 0.6, 0.5)
                };
                let content_mask = ContentMask {
                    bounds: rect(0.0, 0.0, 1080.0, 680.0),
                };
                if subpixel {
                    scene.subpixel_sprites.push(SubpixelSprite {
                        order,
                        pad: 0,
                        bounds,
                        content_mask,
                        color,
                        tile,
                        transformation: TransformationMatrix::unit(),
                    });
                } else {
                    scene.monochrome_sprites.push(MonochromeSprite {
                        order,
                        pad: 0,
                        bounds,
                        content_mask,
                        color,
                        tile,
                        transformation: TransformationMatrix::unit(),
                    });
                }
            };
            // The alternating same-tile group has 65 visible runs. Each
            // strip retains their order within the single packed GPU upload.
            for index in 0..130 {
                push(10, index, tiles[0], index % 2 == 0);
            }
            for (order, tile) in [(11, second), (12, tiles[0]), (13, second)] {
                push(order, 0, tile, true);
                push(order, 1, tile, false);
            }
            push(14, 0, tiles[0], true); // changed in the disjoint second repair
            push(15, 1, tiles[0], false); // unchanged in the union's bounding-box gap
            // A static different-kind primitive creates a real batch boundary
            // before the fragmented group; it does not touch partial damage.
            scene.quads.push(Quad {
                order: 9,
                content_mask: ContentMask {
                    bounds: rect(0.0, 0.0, 1080.0, 680.0),
                },
                ..quad(rect(1000.0, 5.0, 5.0, 5.0), hsla(0.0, 0.0, 1.0, 1.0))
            });
            scene.finish();
            scene
        };
        let other_tiles = super::scroll_copy::tiles(&renderer, !subpixel);
        let (mono_tile, subpixel_tile) = if subpixel {
            (other_tiles[0], tiles[0])
        } else {
            (tiles[0], other_tiles[0])
        };
        let mut budget_scene = Scene::default();
        budget_scene.monochrome_sprites = vec![
            MonochromeSprite {
                order: 2,
                pad: 0,
                bounds: rect(330.0, 95.0, 8.0, 8.0),
                content_mask: ContentMask {
                    bounds: rect(0.0, 0.0, 1080.0, 680.0)
                },
                color: hsla(0.0, 0.0, 0.0, 1.0),
                tile: mono_tile,
                transformation: TransformationMatrix::unit(),
            };
            1030
        ];
        budget_scene.subpixel_sprites = vec![
            SubpixelSprite {
                order: 3,
                pad: 0,
                bounds: rect(330.0, 95.0, 8.0, 8.0),
                content_mask: ContentMask {
                    bounds: rect(0.0, 0.0, 1080.0, 680.0)
                },
                color: hsla(0.0, 0.0, 0.0, 1.0),
                tile: subpixel_tile,
                transformation: TransformationMatrix::unit(),
            };
            1030
        ];
        budget_scene.finish();
        let comparisons =
            (budget_scene.monochrome_sprites.len() + budget_scene.subpixel_sprites.len()) * 32;
        assert!(comparisons < 1_000_000 && comparisons > 65_536);
        assert!(
            super::super::culling::GlyphSelection::capture(&budget_scene, &[damage; 32]).is_none(),
            "combined mono+subpixel packed count is capped within valid comparison work"
        );
        let old = make(false);
        let current = make(true);
        let fallback = current
            .batches()
            .find_map(|batch| match batch {
                PrimitiveBatch::MonochromeSprites { texture_id, range }
                    if !subpixel && current.monochrome_sprites[range.start].order == 10 =>
                {
                    Some((texture_id, range))
                }
                PrimitiveBatch::SubpixelSprites { texture_id, range }
                    if subpixel && current.subpixel_sprites[range.start].order == 10 =>
                {
                    Some((texture_id, range))
                }
                _ => None,
            })
            .unwrap();
        renderer
            .render_to_image(&old, WindowBackgroundAppearance::Opaque)
            .unwrap();
        let second_damage = SceneDamageRect {
            left: 580,
            top: 80,
            right: 640,
            bottom: 128,
        };
        let empty = SceneDamageRect {
            left: 1000,
            top: 600,
            right: 1010,
            bottom: 610,
        };
        let rectangles = [
            empty,
            damage,
            empty,
            second_damage,
            SceneDamageRect {
                left: 330,
                top: 90,
                right: 360,
                bottom: 112,
            },
        ];
        let uploads = if subpixel {
            renderer.pipelines.subpixel_sprites.upload_count
        } else {
            renderer.pipelines.mono_sprites.upload_count
        };
        let selected = renderer
            .prepare_damage_buffers(&current, &rectangles)
            .unwrap()
            .unwrap();
        assert_eq!(
            selected.map_range(fallback.1.clone(), subpixel, 1).len(),
            65,
            "65 discontiguous visible instances retain original order in their strip"
        );
        let gap = if subpixel {
            current
                .subpixel_sprites
                .iter()
                .position(|glyph| glyph.order == 15)
        } else {
            current
                .monochrome_sprites
                .iter()
                .position(|glyph| glyph.order == 15)
        }
        .unwrap();
        assert_eq!(
            selected.map_range(gap..gap + 1, subpixel, 1).len(),
            0,
            "exact disjoint union excludes bbox-gap glyph"
        );
        let bbox = super::super::culling::GlyphSelection::capture(
            &current,
            &[SceneDamageRect {
                left: 320,
                top: 80,
                right: 640,
                bottom: 128,
            }],
        )
        .unwrap();
        assert_eq!(
            bbox.map_range(gap..gap + 1, subpixel, 0).len(),
            1,
            "gap fixture would be selected by a bounding-box substitution"
        );
        assert_eq!(selected.map_range(fallback.1.clone(), subpixel, 0), 0..0);
        let middle = selected.map_range(fallback.1.clone(), subpixel, 2);
        assert!(
            middle.is_empty() && middle.start > 0,
            "empty middle strip preserves absolute base"
        );
        assert!(
            selected
                .map_range(fallback.1.clone(), subpixel, 3)
                .is_empty(),
            "disjoint second strip excludes first-strip group"
        );
        let overlap = selected.map_range(fallback.1.clone(), subpixel, 4);
        assert_eq!(overlap.len(), 65);
        assert!(
            overlap.start > selected.map_range(fallback.1.clone(), subpixel, 1).end,
            "overlap duplicates use a later nonzero base"
        );
        for strip in 0..rectangles.len() {
            assert_eq!(selected.map_range(gap..gap + 1, subpixel, strip).len(), 0);
        }
        for (strip, rect) in rectangles.into_iter().enumerate() {
            renderer.pre_draw(&[1.0; 4], Some(rect)).unwrap();
            renderer
                .draw_batches(&current, Some(rect), Some((&selected, strip)))
                .unwrap();
        }
        let after = if subpixel {
            renderer.pipelines.subpixel_sprites.upload_count
        } else {
            renderer.pipelines.mono_sprites.upload_count
        };
        assert_eq!(
            after,
            uploads + 1,
            "all strips require only one glyph buffer upload"
        );
        let partial = renderer.read_target_to_image().unwrap();
        let full = renderer
            .render_to_image(&current, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_eq!(
            partial.as_raw(),
            full.as_raw(),
            "compacted batches and fallback must preserve exact pixels across strips/textures"
        );
        // A preparation-limit fallback after a compact upload must restore
        // original GPU arrays before using original batch offsets.
        renderer
            .prepare_damage_buffers(&old, &[damage])
            .unwrap()
            .unwrap();
        let fallback = renderer
            .prepare_damage_buffers(&old, &[damage; 65])
            .unwrap();
        assert!(fallback.is_none());
        let full_rect = SceneDamageRect {
            left: 0,
            top: 0,
            right: 1080,
            bottom: 680,
        };
        let before = renderer.read_target_to_image().unwrap();
        assert!(
            renderer
                .prepare_damage_buffers(&old, &[full_rect; 16])
                .unwrap()
                .is_none(),
            "duplicated packed instances exceed cap while original×rect comparisons remain bounded"
        );
        assert_eq!(
            renderer.read_target_to_image().unwrap().as_raw(),
            before.as_raw(),
            "preparation-limit fallback does not clear or draw the target"
        );
        for rect in [damage, second_damage] {
            renderer.pre_draw(&[1.0; 4], Some(rect)).unwrap();
            renderer.draw_batches(&old, Some(rect), None).unwrap();
        }
        let restored = renderer.read_target_to_image().unwrap();
        let expected = renderer
            .render_to_image(&old, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_eq!(
            restored.as_raw(),
            expected.as_raw(),
            "preparation fallback restores original buffers"
        );
        // Full rendering must also restore original buffers after compaction.
        renderer
            .prepare_damage_buffers(&current, &[damage])
            .unwrap()
            .unwrap();
        renderer
            .render_damage(&old, WindowBackgroundAppearance::Opaque, SceneDamage::Full)
            .unwrap();
        let restored = renderer.read_target_to_image().unwrap();
        let expected = renderer
            .render_to_image(&old, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_eq!(restored.as_raw(), expected.as_raw());
    }
}
