// SPDX-License-Identifier: Apache-2.0
// Nocterm opt-in storage reuse; classic capture and comparison remain unchanged.
use super::*;

#[cfg(test)]
mod tests;

/// Bounded reusable storage for an exact scene snapshot. This is opt-in;
/// `SceneSnapshot::capture` keeps its original allocation and fallback behavior.
pub struct ReusableSceneSnapshot {
    snapshot: SceneSnapshot,
    index_capacity: usize,
    path_capacity: usize,
    vertex_capacity: usize,
}

fn grow_bounded<T>(values: &mut Vec<T>, needed: usize, limit: usize) -> bool {
    if needed > limit || values.capacity() > limit {
        return false;
    }
    if values.capacity() >= needed {
        return true;
    }
    let capacity = values
        .capacity()
        .saturating_mul(2)
        .max(4)
        .min(limit)
        .max(needed);
    values.try_reserve_exact(capacity - values.len()).is_ok() && values.capacity() <= limit
}

impl ReusableSceneSnapshot {
    fn empty(viewport: [u32; 2]) -> Self {
        Self {
            snapshot: SceneSnapshot {
                viewport,
                records: Vec::new(),
                tiles: Vec::new(),
                supported: false,
            },
            index_capacity: 0,
            path_capacity: 0,
            vertex_capacity: 0,
        }
    }
    fn rejected(self) -> Self {
        Self::empty(self.snapshot.viewport)
    }

    /// Capture with a previous spare buffer. A viewport change releases every
    /// old capacity. Unsupported geometry, allocation or capacity exhaustion
    /// releases the buffer and returns an unsupported snapshot.
    pub fn capture(scene: &Scene, viewport: [u32; 2], spare: Option<Self>) -> Self {
        let mut buffer = spare
            .filter(|old| old.snapshot.viewport == viewport)
            .unwrap_or_else(|| Self::empty(viewport));
        buffer.snapshot.supported = false;
        buffer.snapshot.records.clear(); // Nested path vectors deliberately drop normally.
        buffer.path_capacity = 0;
        buffer.vertex_capacity = 0;
        for tile in &mut buffer.snapshot.tiles {
            tile.clear();
        }
        let columns = viewport[0].div_ceil(TILE_SIZE) as usize;
        let rows = viewport[1].div_ceil(TILE_SIZE) as usize;
        let Some(tile_count) = columns.checked_mul(rows).filter(|n| *n <= MAX_TILES) else {
            return buffer.rejected();
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
            return buffer.rejected();
        }
        if !grow_bounded(&mut buffer.snapshot.tiles, tile_count, MAX_TILES) {
            return buffer.rejected();
        }
        buffer.snapshot.tiles.resize_with(tile_count, Vec::new);
        let mut references = 0;
        for batch in scene.batches() {
            let ok = match batch {
                PrimitiveBatch::Shadows(range) => scene.shadows[range].iter().all(|p| {
                    let mut normalized = *p;
                    normalized.order = 0;
                    normalized.pad = 0;
                    buffer.insert(
                        Record::Shadow(normalized),
                        shadow_coverage(p),
                        &mut references,
                    )
                }),
                PrimitiveBatch::Quads(range) => scene.quads[range].iter().all(|p| {
                    let mut normalized = *p;
                    normalized.order = 0;
                    buffer.insert(
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
                    buffer.insert(
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
                    buffer.insert(
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
                        buffer.insert(
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
                        buffer.insert(
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
                        buffer.insert(
                            Record::Polychrome(normalized),
                            coverage(p.bounds, p.content_mask, 0.0),
                            &mut references,
                        )
                    })
                }
                PrimitiveBatch::Surfaces(_) => false,
            };
            if !ok {
                return buffer.rejected();
            }
        }

        buffer.snapshot.supported = true;
        buffer
    }

    /// Whether exact conservative capture completed within all storage limits.
    pub fn supports_partial_updates(&self) -> bool {
        self.snapshot.supported
    }

    /// Physical viewport shape; equal tile counts do not imply compatible shapes.
    pub fn viewport(&self) -> [u32; 2] {
        self.snapshot.viewport
    }

    /// Actual reserved bytes, including nested path vectors and vertices.
    pub fn reserved_bytes(&self) -> usize {
        self.snapshot.records.capacity() * std::mem::size_of::<Record>()
            + self.snapshot.tiles.capacity() * std::mem::size_of::<Vec<usize>>()
            + self.index_capacity * std::mem::size_of::<usize>()
            + self.path_capacity * std::mem::size_of::<Path<ScaledPixels>>()
            + self.vertex_capacity * std::mem::size_of::<PathVertex<ScaledPixels>>()
    }

    /// Derived finite byte ceiling for one snapshot's actual capacities.
    pub fn reserved_byte_limit() -> usize {
        MAX_RECORDS * std::mem::size_of::<Record>()
            + MAX_TILES * std::mem::size_of::<Vec<usize>>()
            + MAX_TILE_REFERENCES * std::mem::size_of::<usize>()
            + MAX_RECORDS * std::mem::size_of::<Path<ScaledPixels>>()
            + MAX_PATH_VERTICES * std::mem::size_of::<PathVertex<ScaledPixels>>()
    }

    fn insert(&mut self, record: Record, bounds: Option<Coverage>, references: &mut usize) -> bool {
        let Some(bounds) = bounds.filter(|_| record.is_finite()) else {
            return false;
        };
        let viewport = self.snapshot.viewport;
        let left = bounds[0].max(0.0).min(viewport[0] as f32);
        let top = bounds[1].max(0.0).min(viewport[1] as f32);
        let right = bounds[2].max(0.0).min(viewport[0] as f32);
        let bottom = bounds[3].max(0.0).min(viewport[1] as f32);
        if right <= left || bottom <= top {
            return true;
        }
        let x0 = left.floor() as u32 / TILE_SIZE;
        let y0 = top.floor() as u32 / TILE_SIZE;
        let x1 = (right.ceil() as u32).div_ceil(TILE_SIZE);
        let y1 = (bottom.ceil() as u32).div_ceil(TILE_SIZE);
        let added = (x1 - x0) as usize * (y1 - y0) as usize;
        let records = &mut self.snapshot.records;
        let needed_records = records.len() + 1;
        if records.len() >= MAX_RECORDS
            || *references + added > MAX_TILE_REFERENCES
            || !grow_bounded(records, needed_records, MAX_RECORDS)
        {
            return false;
        }
        if let Record::Paths { paths, .. } = &record {
            let vertices = paths
                .iter()
                .map(|path| path.vertices.capacity())
                .sum::<usize>();
            if self.path_capacity + paths.capacity() > MAX_RECORDS
                || self.vertex_capacity + vertices > MAX_PATH_VERTICES
            {
                return false;
            }
            self.path_capacity += paths.capacity();
            self.vertex_capacity += vertices;
        }
        *references += added;
        let index = records.len();
        records.push(record);
        let columns = viewport[0].div_ceil(TILE_SIZE) as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let tile = &mut self.snapshot.tiles[y as usize * columns + x as usize];
                let old = tile.capacity();
                let limit = old + MAX_TILE_REFERENCES - self.index_capacity;
                let needed_indices = tile.len() + 1;
                if !grow_bounded(tile, needed_indices, limit) {
                    return false;
                }
                self.index_capacity += tile.capacity() - old;
                tile.push(index);
            }
        }
        true
    }

    /// Exact comparison with a per-call memo. Both true and false equality
    /// results are reusable only for the same current/previous record pair.
    pub fn damage_since(&self, previous: &Self, memo: &mut SceneComparisonMemo) -> SceneDamage {
        memo.entries.clear(); // Never retain a result across calls or frames.
        let current = &self.snapshot;
        let previous = &previous.snapshot;
        if !current.supported || !previous.supported || current.viewport != previous.viewport {
            return SceneDamage::Full;
        }
        if !grow_bounded(&mut memo.entries, current.records.len(), MAX_RECORDS) {
            memo.entries = Vec::new();
            return SceneDamage::Full;
        }
        memo.entries.resize(current.records.len(), MemoEntry::EMPTY);
        let columns = current.viewport[0].div_ceil(TILE_SIZE) as usize;
        let mut damage: Option<SceneDamageRect> = None;
        for (index, (current_tile, old)) in current.tiles.iter().zip(&previous.tiles).enumerate() {
            if current_tile.len() == old.len()
                && current_tile.iter().zip(old).all(|(a, b)| {
                    let cached = &mut memo.entries[*a];
                    if cached.old_index != *b {
                        *cached = MemoEntry {
                            old_index: *b,
                            equal: current.records[*a] == previous.records[*b],
                        };
                    }
                    cached.equal
                })
            {
                continue;
            }
            let left = (index % columns) as u32 * TILE_SIZE;
            let top = (index / columns) as u32 * TILE_SIZE;
            let rect = SceneDamageRect {
                left,
                top,
                right: (left + TILE_SIZE).min(current.viewport[0]),
                bottom: (top + TILE_SIZE).min(current.viewport[1]),
            };
            damage = Some(damage.map_or(rect, |old| old.union(rect)));
        }
        damage.map_or(SceneDamage::Unchanged, SceneDamage::Partial)
    }
}

#[derive(Clone, Copy)]
struct MemoEntry {
    old_index: usize,
    equal: bool,
}
impl MemoEntry {
    const EMPTY: Self = Self {
        old_index: usize::MAX,
        equal: false,
    };
}

/// Bounded exact record-pair comparison storage. Entries reset on every call.
#[derive(Default)]
pub struct SceneComparisonMemo {
    entries: Vec<MemoEntry>,
}
impl SceneComparisonMemo {
    /// Actual reserved bytes, included alongside both owned snapshot buffers.
    pub fn reserved_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<MemoEntry>()
    }
    /// Maximum reserved memo bytes, with actual capacity at most 50,000 entries.
    pub fn reserved_byte_limit() -> usize {
        MAX_RECORDS * std::mem::size_of::<MemoEntry>()
    }
}
