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
    // y' * 16 + x' where
    //     x' = min(u32(8.0 * (x + 1.0)), 15)
    //     y' = min(u32(8.0 * (y + 1.0)), 15)
    chunks: array<Chunk4, 64>,

    window_size: vec2<f32>,
    line_radius: f32,
    decay: f32,
    sigma: f32,
    intensity: f32,
    total_time: f32,
};

struct Chunk4 {
    // (size << 16) | offset
    offset_size: vec4<u32>,
}

struct Line {
    // 2x16snorm
    start: u32,
    // 2x16snorm
    v: u32,
    time: f32,
}

@group(0) @binding(0)
var<uniform> config: Config;

@group(0) @binding(1)
var<storage> lines: array<Line>;

@group(1) @binding(0)
var tex_in: texture_storage_2d<r32float, read>;

@group(1) @binding(1)
var tex_out: texture_storage_2d<r32float, write>;

const e = 2.7182818459045;
const pi = 3.141592653589793;
const inv_sqrt_2pi = 0.3989422804014327;

fn excitation(distance: f32) -> f32 {
    return config.intensity * inv_sqrt_2pi / config.sigma
        * pow(e, -0.5 * pow(distance / config.sigma, 2.0));
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

    if (max(abs(pos.x), abs(pos.y)) > 1.1) {
        // Outside of the center square
        discard;
    }

    let chunk_x = clamp(i32(8.0 * (pos.x + 1.0)), 0, 15);
    let chunk_y = clamp(i32(8.0 * (pos.y + 1.0)), 0, 15);
    let i_chunk = chunk_y * 16 + chunk_x;
    let chunk_offset_size = config.chunks[i_chunk >> 2].offset_size[i_chunk & 3];
    let chunk_offset = chunk_offset_size & 0xffff;
    let chunk_size = chunk_offset_size >> 16;

    let prev = textureLoad(tex_in, frag_coord).x;
    var next = prev;
    var t = 0.0;

    for (var i: u32 = 0; i < chunk_size; i++) {
        let line = lines[chunk_offset + i];

        // Calculate decay for time before this line.
        let delta_t = line.time - t;
        next *= pow(config.decay, delta_t);
        t = line.time;

        // Contribution from line
        let start = unpack2x16snorm(line.start);
        let v = unpack2x16snorm(line.v);
        let u = pos - start;
        var disp = u;
        if dot(v, v) != 0.0 {
            let proj_position = dot(u, v) / dot(v, v);
            let proj = v * clamp(proj_position, 0.0, 1.0);
            disp -= proj;
        }

        let x = excitation(length(disp)) / (3 * config.sigma + length(v));
        if x == x {
            // Only finite numbers please
            next += x;
        }

    }
    next *= pow(config.decay, config.total_time - t);

    // Clipping
    next = clamp(next, 0.0, 2.0);

    textureStore(tex_out, frag_coord, vec4(next));
    return vec4<f32>(0.0, pow(next, 0.5), 0.0, 1.0);
}
