// SPDX-License-Identifier: Apache-2.0
//! Verified integer vertical translations over an opaque, flat backdrop.
use super::*;
use crate::color::BackgroundTag;

const MAX_CANDIDATES: usize = 8;
const SAMPLE_GLYPHS: usize = 128;
const MAX_SEARCH_GLYPHS: usize = 16_384;
const MAX_GLYPH_MATCHES: usize = 262_144;
const MAX_SCANLINE_REFERENCES: usize = 2_000_000;

/// A pixel-copy plan established by exact ordered scanline comparisons.
#[derive(Debug)]
pub struct SceneScrollPlan {
    /// Opaque rectangle containing both source and destination pixels.
    pub region: SceneDamageRect,
    /// Destination Y minus source Y, in integer physical pixels.
    pub dy: i32,
    /// Disjoint destination rectangles proven equal to previous translated pixels.
    pub copied: Vec<SceneDamageRect>,
    /// Disjoint rectangles requiring ordinary clear and scene replay.
    pub redraw: Vec<SceneDamageRect>,
}

struct Scanlines {
    records: Vec<Vec<usize>>,
    unsupported: Vec<bool>,
}

impl SceneSnapshot {
    /// Find a bounded, exact scroll-copy opportunity. Appearance, atlas content
    /// and resource validity are additional requirements of the renderer.
    pub fn scroll_since(&self, previous: &Self) -> Option<SceneScrollPlan> {
        if !self.supported
            || !previous.supported
            || self.viewport != previous.viewport
            || self.viewport.iter().any(|size| *size > 8192)
        {
            return None;
        }
        let SceneDamage::Partial(damage) = self.damage_since(previous) else {
            return None;
        };
        let mut best = None;
        let mut best_area = 0u64;
        for (region, backdrop, old_backdrop) in self.backdrops(previous) {
            if rect_area(damage) * 2 < rect_area(region) {
                continue;
            }
            // A backdrop may span static UI beyond the terminal. Only inspect
            // and repair its intersection with the ordinary changed rectangle.
            let Some(region) = intersection(region, damage) else {
                continue;
            };
            let region = self.edge_interior(previous, region, backdrop, old_backdrop);
            if rect_area(region) <= best_area {
                continue;
            }
            let Some(current_rows) = self.scanlines(region, backdrop) else {
                continue;
            };
            let Some(previous_rows) = previous.scanlines(region, old_backdrop) else {
                continue;
            };
            let supported = |rows: &Scanlines| {
                rows.unsupported
                    .iter()
                    .filter(|unsupported| !**unsupported)
                    .count() as u64
            };
            let width = (region.right - region.left) as u64;
            if supported(&current_rows).min(supported(&previous_rows)) * width * 2
                < rect_area(region)
            {
                continue;
            }
            let shifts = self.shifts(previous, region);
            for dy in shifts {
                // No shift can copy rows whose translated source is outside.
                let maximum = (region.bottom - region.top - dy.unsigned_abs()) as u64 * width;
                if maximum <= best_area {
                    continue;
                }
                let equal: Vec<_> = (region.top..region.bottom)
                    .map(|y| {
                        let old_y = y as i32 - dy;
                        if old_y < region.top as i32 || old_y >= region.bottom as i32 {
                            return false;
                        }
                        let a = (y - region.top) as usize;
                        let b = (old_y as u32 - region.top) as usize;
                        !current_rows.unsupported[a]
                            && !previous_rows.unsupported[b]
                            && current_rows.records[a].len() == previous_rows.records[b].len()
                            && current_rows.records[a]
                                .iter()
                                .zip(&previous_rows.records[b])
                                .all(|(a, b)| {
                                    equivalent(&self.records[*a], &previous.records[*b], region, dy)
                                })
                    })
                    .collect();
                let area = equal.iter().filter(|equal| **equal).count() as u64
                    * (region.right - region.left) as u64;
                // Copying and replay setup must buy substantial rasterization work.
                if area <= best_area || area * 2 < rect_area(region) {
                    continue;
                }
                let copied = runs(region, &equal, true);
                let mut redraw = runs(region, &equal, false);
                redraw.extend(outside(damage, region));
                if copied.len() + redraw.len() > 64 {
                    continue;
                }
                best_area = area;
                best = Some(SceneScrollPlan {
                    region,
                    dy,
                    copied,
                    redraw,
                });
            }
        }
        best
    }

    fn backdrops(&self, previous: &Self) -> Vec<(SceneDamageRect, usize, usize)> {
        // Examine at most 64 eligible backgrounds in each bounded snapshot.
        let collect =
            |snapshot: &Self| {
                snapshot
                    .records
                    .iter()
                    .enumerate()
                    .filter_map(|(index, record)| {
                        let Record::Quad(quad) = record else {
                            return None;
                        };
                        if !flat_style(quad) || quad.background.solid.a != 1.0 {
                            return None;
                        }
                        let region = opaque_region(quad, snapshot.viewport)?;
                        (rect_area(region) >= 1024 && region.bottom - region.top <= 8192)
                            .then_some((region, index, quad.background.solid))
                    })
                    .take(64)
                    .collect::<Vec<_>>()
            };
        let old = collect(previous);
        let mut candidates = Vec::new();
        // Prefer the deepest equivalent opaque layer in both scenes. Earlier
        // duplicate backgrounds can expose hidden unsupported layers in the
        // proof and consume all candidate slots without enabling any reuse.
        for (region, index, color) in collect(self).into_iter().rev() {
            if candidates
                .iter()
                .any(|(r, _, _, c)| *r == region && *c == color)
            {
                continue;
            }
            if let Some((_, old_index, _)) = old
                .iter()
                .rev()
                .find(|(r, _, c)| *r == region && *c == color)
            {
                candidates.push((region, index, *old_index, color));
            }
        }
        candidates.sort_by_key(|(region, index, _, _)| {
            (
                std::cmp::Reverse(rect_area(*region)),
                std::cmp::Reverse(*index),
            )
        });
        candidates.truncate(4);
        candidates
            .into_iter()
            .map(|(region, index, old_index, _)| (region, index, old_index))
            .collect()
    }

    fn edge_interior(
        &self,
        previous: &Self,
        region: SceneDamageRect,
        backdrop: usize,
        old_backdrop: usize,
    ) -> SceneDamageRect {
        let mut intervals = Vec::new();
        let width = region.right - region.left;
        for (snapshot, backdrop) in [(self, backdrop), (previous, old_backdrop)] {
            for record in snapshot.records.iter().skip(backdrop + 1) {
                let (bounds, safe) = record_bounds(record, snapshot.viewport);
                let Some(bounds) = bounds.filter(|_| !safe) else {
                    continue;
                };
                if !region.intersects(bounds) {
                    continue;
                }
                let height = bounds[3].min(region.bottom as f32) - bounds[1].max(region.top as f32);
                if height * 2.0 < (region.bottom - region.top) as f32 {
                    continue;
                }
                let left = bounds[0].floor().max(region.left as f32) as u32;
                let right = bounds[2].ceil().min(region.right as f32) as u32;
                if right - left > width / 2 {
                    continue;
                }
                intervals.push((left, right));
                if intervals.len() > 512 {
                    return region;
                }
            }
        }
        // Conservative AA coverage of narrow vertical decorations can overlap
        // one edge column while leaving the rest of the opaque pane untouched.
        // Crop those columns in both scenes; ordinary damage repairs them.
        intervals.sort_unstable();
        let mut left = region.left;
        for &(start, end) in &intervals {
            if start <= left {
                left = left.max(end);
            }
        }
        let mut right = region.right;
        for &(start, end) in intervals.iter().rev() {
            if end >= right {
                right = right.min(start);
            }
        }
        if left >= right || (right - left) * 2 < width {
            return region;
        }
        SceneDamageRect {
            left,
            right,
            ..region
        }
    }

    fn scanlines(&self, region: SceneDamageRect, backdrop: usize) -> Option<Scanlines> {
        let height = (region.bottom - region.top) as usize;
        let mut result = Scanlines {
            records: vec![Vec::new(); height],
            unsupported: vec![false; height],
        };
        let mut references = 0usize;
        // The backdrop completely replaces all earlier contributions in region.
        for (index, record) in self.records.iter().enumerate().skip(backdrop + 1) {
            let (bounds, safe) = record_bounds(record, self.viewport);
            let Some(bounds) = bounds else {
                if safe {
                    continue;
                }
                return None;
            };
            if !region.intersects(bounds) {
                continue;
            }
            let top = (bounds[1].floor().max(region.top as f32) as u32).min(region.bottom);
            let bottom = (bounds[3].ceil().min(region.bottom as f32) as u32).max(top);
            references = references.checked_add((bottom - top) as usize)?;
            if references > MAX_SCANLINE_REFERENCES {
                return None;
            }
            for y in top..bottom {
                let row = (y - region.top) as usize;
                if safe {
                    // Unsupported is monotonic: these rows can never authorize
                    // a copy, so avoid allocating their unused glyph lists.
                    if !result.unsupported[row] {
                        result.records[row].push(index);
                    }
                } else {
                    result.unsupported[row] = true;
                    result.records[row].clear();
                }
            }
        }
        Some(result)
    }

    fn shifts(&self, previous: &Self, region: SceneDamageRect) -> Vec<i32> {
        let visible = |record: &Record, viewport| {
            record_bounds(record, viewport)
                .0
                .is_some_and(|bounds| region.intersects(bounds))
        };
        // Current anchors may occupy only the first dense text row. Search all
        // previous visible rows through an exact identity index, so a multi-line
        // wheel step does not hide its matching source below the anchor sample.
        let mut index = collections::FxHashMap::<[u32; 16], Vec<f32>>::default();
        let mut searched = 0;
        for record in &previous.records {
            let Some((key, y)) = glyph_identity(record) else {
                continue;
            };
            if !visible(record, previous.viewport) {
                continue;
            }
            searched += 1;
            if searched > MAX_SEARCH_GLYPHS {
                return Vec::new();
            }
            index.entry(key).or_default().push(y);
        }
        let mut anchors = 0;
        let mut matches = 0;
        let mut votes = std::collections::BTreeMap::<i32, usize>::new();
        for record in &self.records {
            let Some((key, y)) = glyph_identity(record) else {
                continue;
            };
            if !visible(record, self.viewport) {
                continue;
            }
            let Some(old_rows) = index.get(&key) else {
                continue;
            };
            let eligible = |old_y: &f32| {
                let delta = y - old_y;
                delta.fract() == 0.0
                    && delta != 0.0
                    && delta.abs() * 2.0 < (region.bottom - region.top) as f32
            };
            // Newly exposed text and static anchors cannot propose a shift.
            // Do not spend the bounded anchor sample on those records.
            let mut shifted = false;
            for old_y in old_rows {
                matches += 1;
                if matches > MAX_GLYPH_MATCHES {
                    return Vec::new();
                }
                if eligible(old_y) {
                    shifted = true;
                    break;
                }
            }
            if !shifted {
                continue;
            }
            anchors += 1;
            if anchors > SAMPLE_GLYPHS {
                break;
            }
            for old_y in old_rows {
                matches += 1;
                if matches > MAX_GLYPH_MATCHES {
                    return Vec::new();
                }
                let delta = y - old_y;
                if delta.fract() == 0.0
                    && delta != 0.0
                    && delta.abs() * 2.0 < (region.bottom - region.top) as f32
                {
                    *votes.entry(delta as i32).or_default() += 1;
                }
            }
        }
        let mut votes: Vec<_> = votes.into_iter().collect();
        votes.sort_by_key(|(dy, count)| (std::cmp::Reverse(*count), dy.abs(), *dy));
        votes
            .into_iter()
            .take(MAX_CANDIDATES)
            .map(|(dy, _)| dy)
            .collect()
    }
}

// Hashing selects candidates only. Pixel reuse still requires the independent
// exact ordered row proof; even a poor candidate can never authorize a copy.
fn glyph_identity(record: &Record) -> Option<([u32; 16], f32)> {
    let (kind, bounds, color, tile, transform) = match record {
        Record::Monochrome(p) => (0, p.bounds, p.color, p.tile, p.transformation),
        Record::Subpixel(p) => (1, p.bounds, p.color, p.tile, p.transformation),
        _ => return None,
    };
    if transform != TransformationMatrix::unit() {
        return None;
    }
    Some((
        [
            kind,
            tile.texture_id.index,
            tile.texture_id.kind as u32,
            tile.tile_id.0,
            tile.padding,
            tile.bounds.origin.x.0 as u32,
            tile.bounds.origin.y.0 as u32,
            tile.bounds.size.width.0 as u32,
            tile.bounds.size.height.0 as u32,
            bounds.origin.x.0.to_bits(),
            bounds.size.width.0.to_bits(),
            bounds.size.height.0.to_bits(),
            color.h.to_bits(),
            color.s.to_bits(),
            color.l.to_bits(),
            color.a.to_bits(),
        ],
        bounds.origin.y.0,
    ))
}

fn rect_area(rect: SceneDamageRect) -> u64 {
    (rect.right - rect.left) as u64 * (rect.bottom - rect.top) as u64
}

fn integral(bounds: Bounds<ScaledPixels>) -> bool {
    edges(bounds).is_some_and(|edges| {
        edges
            .iter()
            .all(|v| v.fract() == 0.0 && v.abs() <= 65_536.0)
    })
}

fn flat(quad: &Quad) -> bool {
    flat_style(quad) && integral(quad.bounds) && integral(quad.content_mask.bounds)
}

fn flat_style(quad: &Quad) -> bool {
    quad.background.tag == BackgroundTag::Solid
        && quad.corner_radii == Corners::all(ScaledPixels(0.0))
        && quad.border_widths == Edges::all(ScaledPixels(0.0))
}

fn opaque_region(quad: &Quad, viewport: [u32; 2]) -> Option<SceneDamageRect> {
    if flat(quad) {
        return clipped_rect(quad.bounds, quad.content_mask, viewport);
    }
    let b = edges(quad.bounds)?;
    let m = edges(quad.content_mask.bounds)?;
    // Fractional quads are only accepted as opaque backdrops inside this
    // guarded integer interior. AA fringes and mask boundaries stay outside
    // the copy region and are repaired with ordinary scene damage. Fractional
    // quads after the backdrop still reject their entire conservative coverage.
    let rect = SceneDamageRect {
        left: (b[0].max(m[0]) + 1.0).ceil().max(0.0) as u32,
        top: (b[1].max(m[1]) + 1.0).ceil().max(0.0) as u32,
        right: (b[2].min(m[2]) - 1.0)
            .floor()
            .min(viewport[0] as f32)
            .max(0.0) as u32,
        bottom: (b[3].min(m[3]) - 1.0)
            .floor()
            .min(viewport[1] as f32)
            .max(0.0) as u32,
    };
    (rect.left < rect.right && rect.top < rect.bottom).then_some(rect)
}

fn clipped_rect(
    bounds: Bounds<ScaledPixels>,
    mask: ContentMask<ScaledPixels>,
    viewport: [u32; 2],
) -> Option<SceneDamageRect> {
    let bounds = edges(bounds)?;
    let mask = edges(mask.bounds)?;
    let rect = SceneDamageRect {
        left: bounds[0].max(mask[0]).max(0.0) as u32,
        top: bounds[1].max(mask[1]).max(0.0) as u32,
        right: bounds[2].min(mask[2]).min(viewport[0] as f32).max(0.0) as u32,
        bottom: bounds[3].min(mask[3]).min(viewport[1] as f32).max(0.0) as u32,
    };
    (rect.left < rect.right && rect.top < rect.bottom).then_some(rect)
}

fn sprite_safe(
    bounds: Bounds<ScaledPixels>,
    mask: ContentMask<ScaledPixels>,
    transformation: TransformationMatrix,
    viewport: [u32; 2],
) -> bool {
    // With integral bounds inside an <=8192px viewport, clip-space rounding
    // stays below D3D's rasterizer subpixel grid. Integer translation therefore
    // preserves rasterized triangle geometry and the local atlas UV phase.
    integral(bounds)
        && edges(bounds).is_some_and(|b| b[0] >= 0.0 && b[1] >= 0.0
            && b[2] <= viewport[0] as f32 && b[3] <= viewport[1] as f32)
        && integral(mask.bounds)
        && transformation == TransformationMatrix::unit()
        // Avoid changing clip-generated vertices and their interpolated UVs.
        && edges(bounds).zip(edges(mask.bounds)).is_some_and(|(b, m)| {
            m[0] <= b[0] && m[1] <= b[1] && m[2] >= b[2] && m[3] >= b[3]
        })
}

fn record_bounds(record: &Record, viewport: [u32; 2]) -> (Option<Coverage>, bool) {
    let exact = |bounds, mask| {
        clipped_rect(bounds, mask, viewport)
            .map(|r| [r.left as f32, r.top as f32, r.right as f32, r.bottom as f32])
    };
    match record {
        Record::Quad(p) if flat(p) => (exact(p.bounds, p.content_mask), true),
        Record::Monochrome(p)
            if sprite_safe(p.bounds, p.content_mask, p.transformation, viewport) =>
        {
            (exact(p.bounds, p.content_mask), true)
        }
        Record::Subpixel(p)
            if sprite_safe(p.bounds, p.content_mask, p.transformation, viewport) =>
        {
            (exact(p.bounds, p.content_mask), true)
        }
        Record::Quad(p) => (coverage(p.bounds, p.content_mask, 0.0), false),
        Record::Shadow(p) => (shadow_coverage(p), false),
        Record::Paths { paths, .. } => (path_coverage(paths), false),
        Record::Underline(p) => (coverage(p.bounds, p.content_mask, 0.0), false),
        Record::Monochrome(p) => (
            sprite_coverage(p.bounds, p.content_mask, p.transformation),
            false,
        ),
        Record::Subpixel(p) => (
            sprite_coverage(p.bounds, p.content_mask, p.transformation),
            false,
        ),
        Record::Polychrome(p) => (coverage(p.bounds, p.content_mask, 0.0), false),
    }
}

fn same_glyph<'a>(
    a: &'a Record,
    b: &'a Record,
) -> Option<(Bounds<ScaledPixels>, Bounds<ScaledPixels>)> {
    let (a_bounds, b_bounds, same) = match (a, b) {
        (Record::Monochrome(a), Record::Monochrome(b)) => (
            a.bounds,
            b.bounds,
            a.tile == b.tile && a.color == b.color && a.transformation == b.transformation,
        ),
        (Record::Subpixel(a), Record::Subpixel(b)) => (
            a.bounds,
            b.bounds,
            a.tile == b.tile && a.color == b.color && a.transformation == b.transformation,
        ),
        _ => return None,
    };
    (same && a_bounds.origin.x == b_bounds.origin.x && a_bounds.size == b_bounds.size)
        .then_some((a_bounds, b_bounds))
}

fn equivalent(a: &Record, b: &Record, region: SceneDamageRect, dy: i32) -> bool {
    match (a, b) {
        (Record::Quad(a), Record::Quad(b)) => {
            let clipped = |p: &Quad| {
                edges(p.bounds)
                    .zip(edges(p.content_mask.bounds))
                    .map(|(b, m)| {
                        (
                            b[0].max(m[0]).max(region.left as f32),
                            b[2].min(m[2]).min(region.right as f32),
                        )
                    })
            };
            a.background.solid == b.background.solid && clipped(a) == clipped(b)
        }
        _ => same_glyph(a, b).is_some_and(|(a, b)| a.origin.y.0 == b.origin.y.0 + dy as f32),
    }
}

fn runs(region: SceneDamageRect, equal: &[bool], wanted: bool) -> Vec<SceneDamageRect> {
    let mut result = Vec::new();
    let mut start = None;
    for (row, matches) in equal.iter().copied().chain([!wanted]).enumerate() {
        if matches == wanted {
            start.get_or_insert(row as u32);
        } else if let Some(top) = start.take() {
            result.push(SceneDamageRect {
                top: region.top + top,
                bottom: region.top + row as u32,
                ..region
            });
        }
    }
    result
}

fn outside(damage: SceneDamageRect, region: SceneDamageRect) -> Vec<SceneDamageRect> {
    let top = damage.top.max(region.top);
    let bottom = damage.bottom.min(region.bottom);
    let mut result = Vec::new();
    for rect in [
        SceneDamageRect {
            bottom: damage.bottom.min(region.top),
            ..damage
        },
        SceneDamageRect {
            top: damage.top.max(region.bottom),
            ..damage
        },
        SceneDamageRect {
            top,
            bottom,
            right: damage.right.min(region.left),
            ..damage
        },
        SceneDamageRect {
            top,
            bottom,
            left: damage.left.max(region.right),
            ..damage
        },
    ] {
        if rect.top < rect.bottom && rect.left < rect.right {
            result.push(rect);
        }
    }
    result
}

fn intersection(a: SceneDamageRect, b: SceneDamageRect) -> Option<SceneDamageRect> {
    let rect = SceneDamageRect {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    };
    (rect.left < rect.right && rect.top < rect.bottom).then_some(rect)
}
