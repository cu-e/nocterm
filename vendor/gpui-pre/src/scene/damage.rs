// SPDX-License-Identifier: Apache-2.0
// Nocterm additions to GPUI's Apache-2.0 scene implementation.
//! Exact, tile-local scene comparisons for retained platform renderers.

use super::*;

mod scroll;
pub use scroll::SceneScrollPlan;

const TILE_SIZE: u32 = 64;
const MAX_TILES: usize = 65_536;
const MAX_RECORDS: usize = 50_000;
const MAX_TILE_REFERENCES: usize = 500_000;
const MAX_PATH_VERTICES: usize = 500_000;

/// Conservative damage to a retained image of the previous scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneDamage {
    /// The visible scene is identical.
    Unchanged,
    /// Comparison cannot establish a safe partial update.
    Full,
    /// Repaint this half-open rectangle, preserving pixels outside it.
    Partial(SceneDamageRect),
}

/// A half-open rectangle in physical viewport pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneDamageRect {
    /// Inclusive left edge.
    pub left: u32,
    /// Inclusive top edge.
    pub top: u32,
    /// Exclusive right edge.
    pub right: u32,
    /// Exclusive bottom edge.
    pub bottom: u32,
}

impl SceneDamageRect {
    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn intersects(self, bounds: Coverage) -> bool {
        bounds[0] < self.right as f32
            && bounds[1] < self.bottom as f32
            && bounds[2] > self.left as f32
            && bounds[3] > self.top as f32
    }

    /// Whether a batch can affect this rectangle. Unsupported geometry always
    /// returns true. Path batches are considered together: their intermediate
    /// copy may span gaps between paths when the batch has different orders.
    pub fn intersects_batch(&self, scene: &Scene, batch: &PrimitiveBatch) -> bool {
        match batch {
            PrimitiveBatch::Shadows(r) => scene.shadows[r.clone()]
                .iter()
                .any(|p| self.intersects_or_unknown(shadow_coverage(p))),
            PrimitiveBatch::Quads(r) => scene.quads[r.clone()]
                .iter()
                .any(|p| self.intersects_or_unknown(coverage(p.bounds, p.content_mask, 0.0))),
            PrimitiveBatch::Paths(r) => {
                let paths = &scene.paths[r.clone()];
                if paths.first().map(|p| p.order) == paths.last().map(|p| p.order) {
                    paths.iter().any(|p| {
                        self.intersects_or_unknown(coverage(p.bounds, p.content_mask, 0.0))
                    })
                } else {
                    self.intersects_or_unknown(path_coverage(paths))
                }
            }
            PrimitiveBatch::Underlines(r) => scene.underlines[r.clone()]
                .iter()
                .any(|p| self.intersects_or_unknown(coverage(p.bounds, p.content_mask, 0.0))),
            PrimitiveBatch::MonochromeSprites { range, .. } => {
                scene.monochrome_sprites[range.clone()].iter().any(|p| {
                    p.transformation != TransformationMatrix::unit()
                        || self.intersects_or_unknown(coverage(p.bounds, p.content_mask, 0.0))
                })
            }
            PrimitiveBatch::SubpixelSprites { range, .. } => {
                scene.subpixel_sprites[range.clone()].iter().any(|p| {
                    p.transformation != TransformationMatrix::unit()
                        || self.intersects_or_unknown(coverage(p.bounds, p.content_mask, 0.0))
                })
            }
            PrimitiveBatch::PolychromeSprites { range, .. } => scene.polychrome_sprites
                [range.clone()]
            .iter()
            .any(|p| self.intersects_or_unknown(coverage(p.bounds, p.content_mask, 0.0))),
            PrimitiveBatch::Surfaces(_) => true,
        }
    }

    fn intersects_or_unknown(self, bounds: Option<Coverage>) -> bool {
        bounds.is_none_or(|bounds| self.intersects(bounds))
    }
}

/// An owned scene signature. Primitive records are stored once, and each tile
/// stores indices in actual renderer order. Equality compares fields directly;
/// draw order numbers and path IDs are normalized because renumbering alone
/// does not change pixels. Unsupported scenes use the full-render fallback.
pub struct SceneSnapshot {
    viewport: [u32; 2],
    records: Vec<Record>,
    tiles: Vec<Vec<usize>>,
    supported: bool,
}

#[derive(PartialEq)]
enum Record {
    Shadow(Shadow),
    Quad(Quad),
    Paths {
        paths: Vec<Path<ScaledPixels>>,
        individual_copies: bool,
    },
    Underline(Underline),
    Monochrome(MonochromeSprite),
    Subpixel(SubpixelSprite),
    Polychrome(PolychromeSprite),
}

impl SceneSnapshot {
    /// Capture a finished scene for a physical-pixel viewport. Bounded storage
    /// prevents complex or excessively large scenes from causing unbounded
    /// comparison work; those scenes fall back to full rendering.
    pub fn capture(scene: &Scene, viewport: [u32; 2]) -> Self {
        let mut snapshot = Self {
            viewport,
            records: Vec::new(),
            tiles: Vec::new(),
            supported: false,
        };
        let columns = viewport[0].div_ceil(TILE_SIZE) as usize;
        let rows = viewport[1].div_ceil(TILE_SIZE) as usize;
        let Some(tile_count) = columns.checked_mul(rows).filter(|n| *n <= MAX_TILES) else {
            return snapshot;
        };
        if viewport.contains(&0)
            || !scene.surfaces.is_empty()
            || [
                scene.shadows.len(),
                scene.quads.len(),
                scene.paths.len(),
                scene.underlines.len(),
                scene.monochrome_sprites.len(),
                scene.subpixel_sprites.len(),
                scene.polychrome_sprites.len(),
            ]
            .into_iter()
            .try_fold(0usize, usize::checked_add)
            .is_none_or(|n| n > MAX_RECORDS)
            || scene
                .paths
                .iter()
                .map(|p| p.vertices.len())
                .try_fold(0usize, usize::checked_add)
                .is_none_or(|n| n > MAX_PATH_VERTICES)
        {
            return snapshot;
        }
        snapshot.tiles.resize_with(tile_count, Vec::new);
        let mut references = 0;
        for batch in scene.batches() {
            let ok = match batch {
                PrimitiveBatch::Shadows(range) => scene.shadows[range].iter().all(|p| {
                    let mut normalized = *p;
                    normalized.order = 0;
                    normalized.pad = 0;
                    snapshot.insert(
                        Record::Shadow(normalized),
                        shadow_coverage(p),
                        &mut references,
                    )
                }),
                PrimitiveBatch::Quads(range) => scene.quads[range].iter().all(|p| {
                    let mut normalized = *p;
                    normalized.order = 0;
                    snapshot.insert(
                        Record::Quad(normalized),
                        coverage(p.bounds, p.content_mask, 0.0),
                        &mut references,
                    )
                }),
                PrimitiveBatch::Paths(range) => {
                    let paths = &scene.paths[range];
                    let individual_copies =
                        paths.first().map(|p| p.order) == paths.last().map(|p| p.order);
                    let normalized = paths
                        .iter()
                        .cloned()
                        .map(|mut p| {
                            p.order = 0;
                            p.id = PathId(0);
                            p
                        })
                        .collect();
                    snapshot.insert(
                        Record::Paths {
                            paths: normalized,
                            individual_copies,
                        },
                        path_coverage(paths),
                        &mut references,
                    )
                }
                PrimitiveBatch::Underlines(range) => scene.underlines[range].iter().all(|p| {
                    let mut normalized = *p;
                    normalized.order = 0;
                    normalized.pad = 0;
                    snapshot.insert(
                        Record::Underline(normalized),
                        coverage(p.bounds, p.content_mask, 0.0),
                        &mut references,
                    )
                }),
                PrimitiveBatch::MonochromeSprites { range, .. } => {
                    scene.monochrome_sprites[range].iter().all(|p| {
                        let mut normalized = *p;
                        normalized.order = 0;
                        normalized.pad = 0;
                        snapshot.insert(
                            Record::Monochrome(normalized),
                            sprite_coverage(p.bounds, p.content_mask, p.transformation),
                            &mut references,
                        )
                    })
                }
                PrimitiveBatch::SubpixelSprites { range, .. } => {
                    scene.subpixel_sprites[range].iter().all(|p| {
                        let mut normalized = *p;
                        normalized.order = 0;
                        normalized.pad = 0;
                        snapshot.insert(
                            Record::Subpixel(normalized),
                            sprite_coverage(p.bounds, p.content_mask, p.transformation),
                            &mut references,
                        )
                    })
                }
                PrimitiveBatch::PolychromeSprites { range, .. } => {
                    scene.polychrome_sprites[range].iter().all(|p| {
                        let mut normalized = *p;
                        normalized.order = 0;
                        normalized.pad = 0;
                        snapshot.insert(
                            Record::Polychrome(normalized),
                            coverage(p.bounds, p.content_mask, 0.0),
                            &mut references,
                        )
                    })
                }
                PrimitiveBatch::Surfaces(_) => false,
            };
            if !ok {
                snapshot.records.clear();
                snapshot.tiles.clear();
                return snapshot;
            }
        }
        snapshot.supported = true;
        snapshot
    }

    /// Compare exact visible signatures with the previous retained frame.
    /// The union of changed tiles covers removals, insertions, and overlapping
    /// ordering changes without dirtying unrelated tiles after renumbering.
    pub fn damage_since(&self, previous: &Self) -> SceneDamage {
        if !self.supported || !previous.supported || self.viewport != previous.viewport {
            return SceneDamage::Full;
        }
        let columns = self.viewport[0].div_ceil(TILE_SIZE) as usize;
        let mut damage: Option<SceneDamageRect> = None;
        for (index, (current, old)) in self.tiles.iter().zip(&previous.tiles).enumerate() {
            if current.len() == old.len()
                && current
                    .iter()
                    .zip(old)
                    .all(|(a, b)| self.records[*a] == previous.records[*b])
            {
                continue;
            }
            let left = (index % columns) as u32 * TILE_SIZE;
            let top = (index / columns) as u32 * TILE_SIZE;
            let rect = SceneDamageRect {
                left,
                top,
                right: (left + TILE_SIZE).min(self.viewport[0]),
                bottom: (top + TILE_SIZE).min(self.viewport[1]),
            };
            damage = Some(damage.map_or(rect, |old| old.union(rect)));
        }
        damage.map_or(SceneDamage::Unchanged, SceneDamage::Partial)
    }

    fn insert(&mut self, record: Record, bounds: Option<Coverage>, references: &mut usize) -> bool {
        let Some(bounds) = bounds.filter(|_| record.is_finite()) else {
            return false;
        };
        let left = bounds[0].max(0.0).min(self.viewport[0] as f32);
        let top = bounds[1].max(0.0).min(self.viewport[1] as f32);
        let right = bounds[2].max(0.0).min(self.viewport[0] as f32);
        let bottom = bounds[3].max(0.0).min(self.viewport[1] as f32);
        if right <= left || bottom <= top {
            return true;
        }
        let x0 = left.floor() as u32 / TILE_SIZE;
        let y0 = top.floor() as u32 / TILE_SIZE;
        let x1 = (right.ceil() as u32).div_ceil(TILE_SIZE);
        let y1 = (bottom.ceil() as u32).div_ceil(TILE_SIZE);
        let added = (x1 - x0) as usize * (y1 - y0) as usize;
        if self.records.len() >= MAX_RECORDS || *references + added > MAX_TILE_REFERENCES {
            return false;
        }
        *references += added;
        let index = self.records.len();
        self.records.push(record);
        let columns = self.viewport[0].div_ceil(TILE_SIZE) as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                self.tiles[y as usize * columns + x as usize].push(index);
            }
        }
        true
    }
}

type Coverage = [f32; 4];

fn edges(bounds: Bounds<ScaledPixels>) -> Option<Coverage> {
    let [x, y, w, h] = [
        bounds.origin.x.0,
        bounds.origin.y.0,
        bounds.size.width.0,
        bounds.size.height.0,
    ];
    (w >= 0.0 && h >= 0.0 && [x, y, w, h, x + w, y + h].iter().all(|v| v.is_finite())).then_some([
        x,
        y,
        x + w,
        y + h,
    ])
}

fn coverage(
    bounds: Bounds<ScaledPixels>,
    mask: ContentMask<ScaledPixels>,
    margin: f32,
) -> Option<Coverage> {
    let bounds = edges(bounds)?;
    let mask = edges(mask.bounds)?;
    if !margin.is_finite() || margin < 0.0 {
        return None;
    }
    let clipped = [
        (bounds[0] - margin).max(mask[0]),
        (bounds[1] - margin).max(mask[1]),
        (bounds[2] + margin).min(mask[2]),
        (bounds[3] + margin).min(mask[3]),
    ];
    if clipped[2] <= clipped[0] || clipped[3] <= clipped[1] {
        return Some([0.0; 4]);
    }
    // Include a physical-pixel guard for fractional edges and antialiasing.
    let result = [
        clipped[0] - 1.0,
        clipped[1] - 1.0,
        clipped[2] + 1.0,
        clipped[3] + 1.0,
    ];
    result.iter().all(|v| v.is_finite()).then_some(result)
}

fn shadow_coverage(shadow: &Shadow) -> Option<Coverage> {
    if shadow.inset != 0 {
        coverage(shadow.element_bounds, shadow.content_mask, 0.0)
    } else {
        coverage(
            shadow.bounds,
            shadow.content_mask,
            3.0 * shadow.blur_radius.0,
        )
    }
}

fn sprite_coverage(
    bounds: Bounds<ScaledPixels>,
    mask: ContentMask<ScaledPixels>,
    transformation: TransformationMatrix,
) -> Option<Coverage> {
    (transformation == TransformationMatrix::unit())
        .then(|| coverage(bounds, mask, 0.0))
        .flatten()
}

fn path_coverage(paths: &[Path<ScaledPixels>]) -> Option<Coverage> {
    let mut result: Option<Coverage> = None;
    for path in paths {
        let bounds = coverage(path.bounds, path.content_mask, 0.0)?;
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
            continue;
        }
        result = Some(result.map_or(bounds, |old| {
            [
                old[0].min(bounds[0]),
                old[1].min(bounds[1]),
                old[2].max(bounds[2]),
                old[3].max(bounds[3]),
            ]
        }));
    }
    Some(result.unwrap_or([0.0; 4]))
}

fn color_finite(color: Hsla) -> bool {
    [color.h, color.s, color.l, color.a]
        .iter()
        .all(|v| v.is_finite())
}

fn background_finite(background: Background) -> bool {
    color_finite(background.solid)
        && background.gradient_angle_or_pattern_height.is_finite()
        && background
            .colors
            .iter()
            .all(|s| color_finite(s.color) && s.percentage.is_finite())
}

fn corners_finite(corners: Corners<ScaledPixels>) -> bool {
    [
        corners.top_left.0,
        corners.top_right.0,
        corners.bottom_left.0,
        corners.bottom_right.0,
    ]
    .iter()
    .all(|v| v.is_finite())
}

impl Record {
    fn is_finite(&self) -> bool {
        match self {
            Self::Quad(p) => {
                background_finite(p.background)
                    && color_finite(p.border_color)
                    && corners_finite(p.corner_radii)
                    && [
                        p.border_widths.top.0,
                        p.border_widths.right.0,
                        p.border_widths.bottom.0,
                        p.border_widths.left.0,
                    ]
                    .iter()
                    .all(|v| v.is_finite())
            }
            Self::Shadow(p) => {
                p.blur_radius.0.is_finite()
                    && p.blur_radius.0 >= 0.0
                    && edges(p.bounds).is_some()
                    && edges(p.element_bounds).is_some()
                    && color_finite(p.color)
                    && corners_finite(p.corner_radii)
                    && corners_finite(p.element_corner_radii)
            }
            Self::Underline(p) => color_finite(p.color) && p.thickness.0.is_finite(),
            Self::Monochrome(p) => color_finite(p.color),
            Self::Subpixel(p) => color_finite(p.color),
            Self::Polychrome(p) => p.opacity.is_finite() && corners_finite(p.corner_radii),
            Self::Paths { paths, .. } => paths.iter().all(|p| {
                background_finite(p.color)
                    && p.vertices.iter().all(|v| {
                        [
                            v.xy_position.x.0,
                            v.xy_position.y.0,
                            v.st_position.x,
                            v.st_position.y,
                        ]
                        .iter()
                        .all(|v| v.is_finite())
                            && edges(v.content_mask.bounds).is_some()
                    })
            }),
        }
    }
}

#[cfg(test)]
mod tests;
