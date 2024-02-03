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
    let aspect = config.window_size.x / config.window_size.y;
    var screen_pos = in.pos;
    if (aspect > 1.0) {
        screen_pos.x *= aspect;
    } else {
        screen_pos.y /= aspect;
    }

    let h = atan2(screen_pos.y, screen_pos.x) + 3.14159;
    let s = pow(length(screen_pos), 0.5);
    if (s > 1.0) {
        discard;
    }

    let hp = degrees(h) / 60.0;
    let x = s * (1 - abs((hp % 2) - 1));
    var color = vec4<f32>(0, 0, 0, 0);

    switch (i32(hp)) {
        case 0: {
            color.r = s;
            color.g = x;
        }
        case 1: {
            color.r = x;
            color.g = s;
        }
        case 2: {
            color.g = s;
            color.b = x;
        }
        case 3: {
            color.g = x;
            color.b = s;
        }
        case 4: {
            color.b = s;
            color.r = x;
        }
        case 5: {
            color.b = x;
            color.r = s;
        }
        default: {}
    }

    let m = 1.0 - s;
    color += vec4<f32>(m, m, m, 0);
    return color;
}
