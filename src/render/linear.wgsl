@group(0) @binding(0) var frame: texture_2d<f32>;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corners = array<vec2<f32>, 3>(
        vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0),
    );
    return vec4(corners[index], 0.0, 1.0);
}

@fragment
fn encode(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let color = textureLoad(frame, vec2<i32>(position.xy), 0);
    if color.a <= 0.0 {
        return vec4(0.0);
    }
    let straight = clamp(color.rgb / color.a, vec3(0.0), vec3(1.0));
    let encoded = select(
        1.055 * pow(straight, vec3(1.0 / 2.4)) - 0.055,
        straight * 12.92,
        straight <= vec3(0.0031308),
    );
    // Wayland's default alpha convention premultiplies encoded RGB.
    return vec4(encoded * color.a, color.a);
}
