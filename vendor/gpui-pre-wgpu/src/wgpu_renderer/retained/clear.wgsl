// Nocterm additions, licensed under the upstream Apache-2.0 license.
@group(0) @binding(0) var<uniform> clear_color: vec4<f32>;
@vertex fn vs_clear(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs_clear() -> @location(0) vec4<f32> { return clear_color; }
