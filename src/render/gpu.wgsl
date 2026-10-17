struct Viewport {
    size: vec2<f32>,
    padding: vec2<f32>,
}

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var atlas: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
}

@vertex
fn vertex(
    @builtin(vertex_index) index: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) kind: u32,
) -> VertexOutput {
    let corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let corner = corners[index];
    let position = (rect.xy + corner * rect.zw) / viewport.size;
    var out: VertexOutput;
    out.position = vec4(position.x * 2.0 - 1.0, 1.0 - position.y * 2.0, 0.0, 1.0);
    out.uv = uv.xy + corner * uv.zw;
    out.color = color;
    out.kind = kind;
    return out;
}

fn shade(in: VertexOutput, channel: u32) -> vec4<f32> {
    if in.kind == 0u {
        return in.color;
    }
    let offset = select(vec2(0.0), vec2(0.0001), in.kind == 3u);
    let texel = textureLoad(atlas, vec2<i32>(in.uv + offset), 0);
    if in.kind >= 2u {
        return texel;
    }
    return vec4(in.color.rgb, texel[channel]);
}

// Separate channel writes preserve native RGB font coverage on all adapters.
@fragment fn red(in: VertexOutput) -> @location(0) vec4<f32> {
    return shade(in, 0u);
}
@fragment fn green(in: VertexOutput) -> @location(0) vec4<f32> {
    return shade(in, 1u);
}
@fragment fn blue(in: VertexOutput) -> @location(0) vec4<f32> {
    return shade(in, 2u);
}
