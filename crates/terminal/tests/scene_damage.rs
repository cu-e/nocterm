//! Exercise the vendored damage API without upstream GPUI's unavailable
//! font fixtures. Both this adapter and GPUI unit tests use the same cases.

use gpui_kit::{
    AtlasTextureId, AtlasTextureKind, AtlasTile, Bounds, ContentMask, DevicePixels, Hsla,
    MonochromeSprite, PaintSurface, Path, PathId, PathVertex, Pixels, PrimitiveBatch, Quad,
    Radians, ScaledPixels, Scene, SceneDamage, SceneDamageRect, SceneSnapshot, Shadow, TileId,
    TransformationMatrix, Underline, bounds, point, size,
};

#[path = "../../../vendor/gpui-pre/src/scene/damage/tests.rs"]
mod tests;
