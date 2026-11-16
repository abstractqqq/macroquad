//! Benchmark for quad_gl draw-call submission: GPU buffer memory and frame timing.
//!
//! Run with:
//!     cargo run --release --example drawcall_benchmark -- <variant> [capacity]
//!
//! `variant` is a free-form label written to every CSV row, so results from
//! different revisions can be concatenated. `capacity` sets
//! `draw_call_vertex_capacity` (default 10000); the index capacity is half of it,
//! matching the `Conf` defaults.
//!
//! Every draw call alternates between two textures, so each one is a real batch
//! break. Use the same window environment when comparing revisions.

use rayquad::prelude::*;
use rayquad::quad_gl::DrawStats;
use rayquad::window::miniquad;
use std::time::{Duration, Instant};

const WARMUP_FRAMES: usize = 60;
const SAMPLE_FRAMES: usize = 240;
const DEFAULT_CAPACITY: usize = 10_000;

/// (draw calls per frame, quads per draw call)
const SWEEP: [(usize, usize); 10] = [
    (1, 1),
    (10, 1),
    (100, 1),
    (500, 1),
    (2000, 1),
    (1, 250),
    (10, 250),
    (100, 250),
    (500, 250),
    (2000, 250),
];

const SPIKE_DRAW_CALLS: usize = 2000;
const IDLE_DRAW_CALLS: usize = 10;
const IDLE_FRAMES: usize = 300;

#[derive(Default)]
struct Samples {
    record: Vec<f64>,
    submit: Vec<f64>,
    gpu: Vec<f64>,
    frame: Vec<f64>,
}

impl Samples {
    fn clear(&mut self) {
        self.record.clear();
        self.submit.clear();
        self.gpu.clear();
        self.frame.clear();
    }
}

fn percentile(samples: &[f64], percentile: usize) -> f64 {
    if samples.is_empty() {
        return f64::NAN;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    sorted[(sorted.len() - 1) * percentile / 100]
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

/// Resident set size in bytes, Linux only.
fn rss_bytes() -> usize {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|statm| statm.split_whitespace().nth(1)?.parse::<usize>().ok())
        .map_or(0, |pages| pages * 4096)
}

fn stats() -> DrawStats {
    unsafe { get_internal_gl().quad_gl.stats }
}

fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

struct Row<'a> {
    variant: &'a str,
    capacity: usize,
    workload: &'a str,
    draw_calls: usize,
    quads_per_dc: usize,
}

fn report(row: &Row, samples: &Samples, stats: &DrawStats) {
    println!(
        "{},{},{},{},{},{},{},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.2},{:.2},{:.1}",
        row.variant,
        row.capacity,
        row.workload,
        row.draw_calls,
        row.quads_per_dc,
        stats.draw_calls,
        stats.passes,
        percentile(&samples.record, 50),
        percentile(&samples.record, 95),
        percentile(&samples.submit, 50),
        percentile(&samples.submit, 95),
        percentile(&samples.gpu, 50),
        percentile(&samples.gpu, 95),
        percentile(&samples.frame, 50),
        percentile(&samples.frame, 95),
        percentile(&samples.frame, 99),
        mib(stats.buffer_bytes),
        mib(stats.buffer_bytes_peak),
        mib(rss_bytes()),
    );
}

fn record(draw_calls: usize, quads_per_dc: usize, red: &Texture2D, blue: &Texture2D) {
    let params = DrawTextureParams {
        dest_size: Some(vec2(4.0, 4.0)),
        ..Default::default()
    };
    for dc in 0..draw_calls {
        let texture = if dc & 1 == 0 { red } else { blue };
        for quad in 0..quads_per_dc {
            let index = dc * quads_per_dc + quad;
            let x = (index % 200) as f32 * 4.0;
            let y = (index / 200 % 150) as f32 * 4.0;
            draw_texture_ex(texture, x, y, WHITE, params.clone());
        }
    }
}

/// Records one frame, awaits it, and stores its timings when `sample` is set.
async fn frame(
    draw_calls: usize,
    quads_per_dc: usize,
    red: &Texture2D,
    blue: &Texture2D,
    previous_frame: &mut Instant,
    samples: Option<&mut Samples>,
) {
    clear_background(BLACK);
    let record_start = Instant::now();
    record(draw_calls, quads_per_dc, red, blue);
    let record_elapsed = record_start.elapsed();

    next_frame().await;

    // next_frame() returns after quad_gl submitted this frame, so `stats`
    // describes the frame recorded above.
    let now = Instant::now();
    if let Some(samples) = samples {
        let stats = stats();
        samples.record.push(micros(record_elapsed));
        samples.submit.push(stats.submit_seconds * 1_000_000.0);
        if let Some(ns) = stats.gpu_ns {
            samples.gpu.push(ns as f64 / 1000.0);
        }
        samples.frame.push(micros(now.duration_since(*previous_frame)));
    }
    *previous_frame = now;
}

fn window_conf() -> rayquad::conf::Conf {
    let capacity = capacity();
    rayquad::conf::Conf {
        miniquad_conf: miniquad::conf::Conf {
            window_title: "quad_gl draw-call benchmark".to_owned(),
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
        draw_call_vertex_capacity: capacity,
        draw_call_index_capacity: capacity / 2,
        ..Default::default()
    }
}

fn capacity() -> usize {
    std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_CAPACITY)
}

fn main() {
    rayquad::Window::from_config(window_conf(), benchmark());
}

async fn benchmark() {
    let variant = std::env::args().nth(1).unwrap_or_else(|| "unnamed".to_owned());
    let capacity = capacity();

    unsafe { get_internal_gl().quad_gl.stats.gpu_timing = true };

    let red = Texture2D::from_image(&Image::gen_image_color(1, 1, RED));
    let blue = Texture2D::from_image(&Image::gen_image_color(1, 1, BLUE));

    println!(
        "variant,capacity,workload,draw_calls,quads_per_dc,dc_actual,passes,\
         record_p50_us,record_p95_us,submit_p50_us,submit_p95_us,gpu_p50_us,gpu_p95_us,\
         frame_p50_us,frame_p95_us,frame_p99_us,buffer_mib,buffer_peak_mib,rss_mib"
    );

    let mut samples = Samples::default();
    let mut previous_frame = Instant::now();

    // Spike first, while buffers are still at their startup size: one frame
    // with many draw calls, then a long idle stretch with few.
    for frame_index in 0..WARMUP_FRAMES {
        let _ = frame_index;
        frame(IDLE_DRAW_CALLS, 1, &red, &blue, &mut previous_frame, None).await;
    }
    samples.clear();
    for _ in 0..SAMPLE_FRAMES {
        let s = Some(&mut samples);
        frame(IDLE_DRAW_CALLS, 1, &red, &blue, &mut previous_frame, s).await;
    }
    let row = |workload| Row {
        variant: &variant,
        capacity,
        workload,
        draw_calls: IDLE_DRAW_CALLS,
        quads_per_dc: 1,
    };
    report(&row("spike_before"), &samples, &stats());

    frame(SPIKE_DRAW_CALLS, 1, &red, &blue, &mut previous_frame, None).await;

    samples.clear();
    for _ in 0..IDLE_FRAMES {
        let s = Some(&mut samples);
        frame(IDLE_DRAW_CALLS, 1, &red, &blue, &mut previous_frame, s).await;
    }
    report(&row("spike_after"), &samples, &stats());

    for (draw_calls, quads_per_dc) in SWEEP {
        for _ in 0..WARMUP_FRAMES {
            frame(draw_calls, quads_per_dc, &red, &blue, &mut previous_frame, None).await;
        }
        samples.clear();
        for _ in 0..SAMPLE_FRAMES {
            let s = Some(&mut samples);
            frame(draw_calls, quads_per_dc, &red, &blue, &mut previous_frame, s).await;
        }
        let row = Row {
            variant: &variant,
            capacity,
            workload: "sweep",
            draw_calls,
            quads_per_dc,
        };
        report(&row, &samples, &stats());
    }

    miniquad::window::quit();
}
