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

const e = 2.7182818459045;
const pi = 3.141592653589793;

fn excitation(distance: f32) -> f32 {
    let sigma = 0.01;
    return 0.1 * pow(e, -0.5 * pow(distance / sigma, 2.0));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // XXX: this is not the same as the value from the vertex shader;
    // it is actually pixel coordinates.
    let frag_coord = vec2<u32>(in.clip_position.xy);

    var pos = in.pos;
    let aspect = config.window_size.x / config.window_size.y;
    if (aspect > 1.0) {
        pos.x *= aspect;
    } else {
        pos.y /= aspect;
    }

    let prev = textureLoad(tex_in, frag_coord);
    var next = prev * config.decay;

    var last_length = 0.0;

    for (var i: u32 = 1; i < config.sample_count; i++) {
        let start = samples[i - 1];
        let end = samples[i];

        let u = pos - start;
        let v = end - start;

        // Contribution from line
        let proj_position = dot(u, v) / dot(v, v);

        let proj = v * proj_position;

        var disp = u - proj;
        if (proj_position < 0.0) {
            // Clamp to start point
            disp = pos - start;
        } else if (proj_position > 1.0) {
            // Clamp to end point
            disp = pos - end;
        }
        next.x += excitation(length(disp)) / (20.0 * length(v));

        // Subtract contribution from start point - otherwise it will be
        // counted twice (previous and current line segment)
        next.x -= excitation(length(start - pos)) / (20.0 * max(length(v), last_length));
        last_length = length(v);
    }

    textureStore(tex_out, frag_coord, next);
    return vec4<f32>(0.0, next.x, 0.0, 1.0);
}
