//! End-to-end benchmark for quad_gl's per-frame uniform and texture arenas.
//!
//! Run with:
//!     cargo run --release --example arena_benchmark -- 2000
//!
//! The optional argument is the number of rectangles recorded per frame. The
//! benchmark warms each workload up, samples it, prints CSV-friendly results,
//! and exits automatically. Use the same count and window environment when
//! comparing revisions.

use rayquad::prelude::*;
use rayquad::window::miniquad::{self, *};
use std::time::{Duration, Instant};

const WARMUP_FRAMES: usize = 60;
const SAMPLE_FRAMES: usize = 240;
const DEFAULT_DRAWS_PER_FRAME: usize = 2_000;

#[derive(Clone, Copy, Debug)]
enum Workload {
    Batched,
    UniformChanges,
    TextureChanges,
}

impl Workload {
    const ALL: [Self; 3] = [Self::Batched, Self::UniformChanges, Self::TextureChanges];

    const fn name(self) -> &'static str {
        match self {
            Self::Batched => "batched",
            Self::UniformChanges => "uniform_changes",
            Self::TextureChanges => "texture_changes",
        }
    }
}

struct Samples {
    record: Vec<Duration>,
    frame: Vec<Duration>,
}

impl Samples {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            record: Vec::with_capacity(capacity),
            frame: Vec::with_capacity(capacity),
        }
    }

    fn clear(&mut self) {
        self.record.clear();
        self.frame.clear();
    }
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let index = (sorted.len() - 1) * percentile / 100;
    sorted[index]
}

fn mean(samples: &[Duration]) -> Duration {
    let total: f64 = samples.iter().map(Duration::as_secs_f64).sum();
    Duration::from_secs_f64(total / samples.len() as f64)
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

fn report(workload: Workload, draws: usize, samples: &Samples) {
    let record_mean = mean(&samples.record);
    let frame_mean = mean(&samples.frame);

    println!(
        "{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.1}",
        workload.name(),
        draws,
        samples.record.len(),
        micros(record_mean),
        micros(percentile(&samples.record, 50)),
        micros(percentile(&samples.record, 95)),
        micros(percentile(&samples.record, 99)),
        micros(percentile(&samples.frame, 50)),
        micros(percentile(&samples.frame, 95)),
        1.0 / frame_mean.as_secs_f64(),
    );
}

fn window_conf() -> rayquad::conf::Conf {
    rayquad::conf::Conf {
        miniquad_conf: miniquad::conf::Conf {
            window_title: "quad_gl arena benchmark".to_owned(),
            window_width: 800,
            window_height: 600,
            window_resizable: false,
            platform: miniquad::conf::Platform {
                // Avoid measuring display synchronization where supported.
                swap_interval: Some(0),
                ..Default::default()
            },
            ..Default::default()
        },
        // Keep per-draw GPU buffers small. quad_gl currently allocates one pair
        // of these buffers for every cached draw-call slot.
        draw_call_vertex_capacity: 64,
        draw_call_index_capacity: 96,
        ..Default::default()
    }
}

fn main() {
    rayquad::Window::from_config(window_conf(), benchmark());
}

async fn benchmark() {
    let draws = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_DRAWS_PER_FRAME);

    let red = Texture2D::from_image(&Image::gen_image_color(1, 1, RED));
    let blue = Texture2D::from_image(&Image::gen_image_color(1, 1, BLUE));
    let backend = unsafe { get_internal_gl().quad_context.info().backend };
    let material = load_material(
        match backend {
            Backend::OpenGl => ShaderSource::Glsl {
                vertex: GLSL_VERTEX,
                fragment: GLSL_FRAGMENT,
            },
            Backend::Metal => ShaderSource::Msl {
                program: MSL_SHADER,
            },
        },
        MaterialParams {
            uniforms: vec![UniformDesc::new("Tint", UniformType::Float4)],
            textures: vec!["Mask".to_owned()],
            ..Default::default()
        },
    )
    .expect("benchmark material should compile");

    println!(
        "quad_gl arena benchmark: draws/frame={draws}, warmup={WARMUP_FRAMES}, samples={SAMPLE_FRAMES}"
    );
    println!(
        "workload,draws,samples,record_mean_us,record_p50_us,record_p95_us,record_p99_us,frame_p50_us,frame_p95_us,mean_fps"
    );

    let mut samples = Samples::with_capacity(SAMPLE_FRAMES);

    for workload in Workload::ALL {
        samples.clear();
        let mut previous_frame = Instant::now();

        for frame_index in 0..WARMUP_FRAMES + SAMPLE_FRAMES {
            clear_background(BLACK);
            gl_use_material(&material);

            // Establish state outside the timed region when it is not the
            // variable being measured by this workload.
            if matches!(workload, Workload::Batched | Workload::UniformChanges) {
                material.set_texture("Mask", red.clone());
            }
            if matches!(workload, Workload::Batched | Workload::TextureChanges) {
                material.set_uniform("Tint", vec4(1.0, 1.0, 1.0, 1.0));
            }

            let record_start = Instant::now();
            for draw_index in 0..draws {
                match workload {
                    Workload::Batched => {}
                    Workload::UniformChanges => {
                        let channel = (draw_index & 255) as f32 / 255.0;
                        material.set_uniform("Tint", vec4(channel, 1.0 - channel, 0.5, 1.0));
                    }
                    Workload::TextureChanges => {
                        let texture = if draw_index & 1 == 0 { &red } else { &blue };
                        material.set_texture("Mask", texture.clone());
                    }
                }

                let column = draw_index % 100;
                let row = draw_index / 100;
                draw_rectangle(
                    column as f32 * 8.0,
                    (row % 75) as f32 * 8.0,
                    6.0,
                    6.0,
                    WHITE,
                );
            }
            let record_elapsed = record_start.elapsed();
            gl_use_default_material();

            next_frame().await;

            let now = Instant::now();
            if frame_index >= WARMUP_FRAMES {
                samples.record.push(record_elapsed);
                samples.frame.push(now.duration_since(previous_frame));
            }
            previous_frame = now;
        }

        report(workload, draws, &samples);
    }

    miniquad::window::quit();
}

const GLSL_VERTEX: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;

varying lowp vec2 uv;
varying lowp vec4 color;

uniform mat4 Model;
uniform mat4 Projection;

void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    uv = texcoord;
    color = color0 / 255.0;
}
"#;

const GLSL_FRAGMENT: &str = r#"#version 100
precision lowp float;

varying lowp vec2 uv;
varying lowp vec4 color;

uniform sampler2D Texture;
uniform sampler2D Mask;
uniform vec4 Tint;

void main() {
    gl_FragColor = color * Tint * texture2D(Texture, uv) * texture2D(Mask, uv);
}
"#;

const MSL_SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Vertex {
    float3 position [[attribute(0)]];
    float2 texcoord [[attribute(1)]];
    float4 color0 [[attribute(2)]];
};

struct Uniforms {
    float4x4 Projection;
    float4x4 Model;
    float4 Time;
    float4 Tint;
};

struct RasterizerData {
    float4 position [[position]];
    float2 uv;
    float4 color;
};

vertex RasterizerData vertexShader(Vertex v [[stage_in]],
                                   constant Uniforms& u [[buffer(0)]]) {
    RasterizerData out;
    out.position = u.Projection * u.Model * float4(v.position, 1.0);
    out.uv = v.texcoord;
    out.color = v.color0 / 255.0;
    return out;
}

fragment float4 fragmentShader(RasterizerData in [[stage_in]],
                               constant Uniforms& u [[buffer(0)]],
                               texture2d<float> texture [[texture(0)]],
                               sampler texture_sampler [[sampler(0)]],
                               texture2d<float> mask [[texture(2)]],
                               sampler mask_sampler [[sampler(2)]]) {
    return in.color * u.Tint * texture.sample(texture_sampler, in.uv)
        * mask.sample(mask_sampler, in.uv);
}
"#;
