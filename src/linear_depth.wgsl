// Turns the camera's depth buffer into linear view depth in metres.
//
// The guest camera is reversed-Z with an infinite far plane (see camera.rs): the buffer holds
// d = near / t for a surface t metres in front of the camera, and 0 where nothing was drawn. So
// t = near / d exactly. "Nothing here" is written as -1 and becomes +inf on the CPU (WGSL has no
// infinity literal).

struct Params {
    near: f32,
    far: f32,
    _pad0: f32,
    _pad1: f32,
}

@group(0) @binding(0) var scene_depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> params: Params;

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let d = textureLoad(scene_depth, vec2<i32>(in.position.xy), 0);
    var t = -1.0;
    if d > 0.0 {
        t = params.near / d;
    }
    return vec4<f32>(t, 0.0, 0.0, 1.0);
}
