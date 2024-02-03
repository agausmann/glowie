struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) pos: vec2<f32>,
};

var<private> vertices: array<vec2<f32>, 4> = array(
    vec2(1.0, 1.0),
    vec2(-1.0, 1.0),
    vec2(1.0, -1.0),
    vec2(-1.0, -1.0),
);

@vertex
fn vs_main(
    @builtin(vertex_index) in_vertex_index: u32,
) -> VertexOutput {

    var out: VertexOutput;
    let vert = vertices[in_vertex_index].xy;
    out.pos = vert;
    out.clip_position = vec4<f32>(vert, 0.0, 1.0);
    return out;
}

struct Config {
    window_size: vec2<f32>,
    sample_count: u32,
    line_radius: f32,
    decay: f32,
};

@group(0) @binding(0)
var<uniform> config: Config;

@group(0) @binding(1)
var<storage> samples: array<vec2<f32>>;

@group(1) @binding(0)
var tex_in: texture_storage_2d<r32float, read>;

@group(1) @binding(1)
var tex_out: texture_storage_2d<r32float, write>;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // XXX: this is not the same as the value from the vertex shader;
    // it is actually pixel coordinates.
    let frag_coord = vec2<u32>(in.clip_position.xy);

    let prev = textureLoad(tex_in, frag_coord);
    var next = prev * config.decay;
    if (next.x < 0.01) {
        next.x = 0.1;
    }

    textureStore(tex_out, frag_coord, next);

    return vec4<f32>(0.0, next.x, 0.0, 1.0);
}
