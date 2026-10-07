// Nocterm regression tests, licensed under the upstream Apache-2.0 license.
use super::*;
use gpui::{
    AtlasKey, Corners, FontId, GlyphId, MonochromeSprite, Path, RenderGlyphParams, Shadow,
    SubpixelSprite, TransformationMatrix, Underline, px,
};
use std::borrow::Cow;

fn glyph_key(subpixel: bool, id: u32) -> AtlasKey {
    RenderGlyphParams {
        font_id: FontId(0),
        glyph_id: GlyphId(id),
        font_size: px(8.),
        subpixel_variant: point(0, 0),
        scale_factor: 1.,
        is_emoji: false,
        subpixel_rendering: subpixel,
        dilation: 0,
    }
    .into()
}

#[test]
fn mono_subpixel_clipping_transform_and_alpha_order_match_full_pixels() -> Result<()> {
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    for subpixel in [false, true] {
        renderer.core.retained.reset();
        let bytes = vec![180; 64 * if subpixel { 4 } else { 1 }];
        let tile = renderer
            .core
            .atlas
            .get_or_insert_with(glyph_key(subpixel, 1), &mut || {
                Ok(Some((viewport(8, 8), Cow::Borrowed(&bytes))))
            })?
            .unwrap();
        for transformed in [false, true] {
            renderer.core.retained.reset();
            for offset in [0., 3.25, 15., 3.25] {
                let mut scene = scene(false);
                for (index, x) in [60.5, 64.0, 80.0, 170.0].into_iter().enumerate() {
                    let mask = ContentMask {
                        bounds: rect(62.25, 20., 150., 90.),
                    };
                    let bounds = rect(x + offset, 65.25, 16., 16.);
                    let color = hsla(index as f32 * 0.17, 0.7, 0.6, 0.5);
                    let transformation = if index == 3 && transformed {
                        TransformationMatrix {
                            translation: [-91., 0.],
                            ..TransformationMatrix::unit()
                        }
                    } else {
                        TransformationMatrix::unit()
                    };
                    if subpixel {
                        scene.subpixel_sprites.push(SubpixelSprite {
                            order: index as u32 + 3,
                            pad: 0,
                            bounds,
                            content_mask: mask,
                            color,
                            tile,
                            transformation,
                        });
                    } else {
                        scene.monochrome_sprites.push(MonochromeSprite {
                            order: index as u32 + 3,
                            pad: 0,
                            bounds,
                            content_mask: mask,
                            color,
                            tile,
                            transformation,
                        });
                    }
                }
                scene.finish();
                assert_matches(
                    &mut renderer,
                    &scene,
                    viewport(256, 192),
                    false,
                    wgpu::Color::BLACK,
                )?;
            }
            if transformed {
                assert_eq!(renderer.core.retained.last_damage, None);
                assert!(renderer.core.retained.snapshot.is_none());
            } else {
                assert!(matches!(
                    renderer.core.retained.last_damage,
                    Some(SceneDamage::Partial(_))
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn path_pass_reopening_restores_damage_scissor_with_shadow_and_wavy_underline() -> Result<()> {
    let _gpu = gpu_test();
    let mut renderer = WgpuHeadlessRenderer::new()?;
    for offset in [0., 4.25, 12., 0.] {
        let mut scene = scene(false);
        scene.shadows.push(Shadow {
            order: 1,
            blur_radius: ScaledPixels(5.),
            bounds: rect(55. + offset, 40., 42., 30.),
            corner_radii: Corners::all(ScaledPixels(4.)),
            content_mask: ContentMask {
                bounds: rect(57., 38., 54., 48.),
            },
            color: hsla(0., 0., 0., 0.4),
            element_bounds: rect(60. + offset, 45., 32., 20.),
            element_corner_radii: Corners::all(ScaledPixels(4.)),
            inset: 0,
            pad: 0,
        });
        scene.underlines.push(Underline {
            order: 5,
            pad: 0,
            bounds: rect(40., 92., 80., 5.),
            content_mask: ContentMask {
                bounds: rect(0., 0., 256., 192.),
            },
            color: hsla(0.8, 0.8, 0.6, 0.7),
            thickness: ScaledPixels(1.5 + offset / 16.),
            wavy: true.into(),
        });
        for (order, start_x, shift) in [(3, 64., offset), (3, 120., 0.), (7, 220., 0.)] {
            let mut path = Path::new(point(px(start_x + shift), px(52.)));
            path.line_to(point(px(start_x + shift + 25.), px(58.)));
            path.line_to(point(px(start_x + shift + 7.), px(80.)));
            path.line_to(point(px(start_x + shift), px(52.)));
            let mut path = path.scale(1.);
            path.order = order;
            path.content_mask = ContentMask {
                bounds: rect(0., 0., 256., 192.),
            };
            path.color = hsla(start_x / 256., 0.8, 0.6, 0.5).into();
            scene.paths.push(path);
        }
        // Replay after paths reaches beyond partial damage; scissor must survive reopening.
        scene
            .quads
            .push(quad(6, rect(0., 0., 256., 192.), hsla(0.6, 0.4, 0.5, 0.2)));
        scene.finish();
        assert_matches(
            &mut renderer,
            &scene,
            viewport(256, 192),
            false,
            wgpu::Color::TRANSPARENT,
        )?;
    }
    assert!(matches!(
        renderer.core.retained.last_damage,
        Some(SceneDamage::Partial(_))
    ));
    Ok(())
}
