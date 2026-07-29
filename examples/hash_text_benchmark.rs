use std::{
    collections::hash_map::DefaultHasher,
    hash::{BuildHasher, Hash, Hasher},
    hint::black_box,
    time::{Duration, Instant},
};

use foldhash::fast::RandomState as FoldRandomState;

const SAMPLES: usize = 9;
const TARGET_BYTES_PER_SAMPLE: usize = 128 * 1024 * 1024;
const MIN_ITERATIONS: usize = 100_000;

fn default_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn time_default(text: &str, iterations: usize) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(default_hash(black_box(text)));
    }
    start.elapsed()
}

fn time_foldhash(text: &str, iterations: usize, state: &FoldRandomState) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(state.hash_one(black_box(text)));
    }
    start.elapsed()
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn benchmark(name: &str, text: &str, fold_state: &FoldRandomState) {
    let iterations = (TARGET_BYTES_PER_SAMPLE / text.len().max(1)).max(MIN_ITERATIONS);

    // Warm both implementations before collecting samples.
    black_box(default_hash(black_box(text)));
    black_box(fold_state.hash_one(black_box(text)));

    let mut default_samples = Vec::with_capacity(SAMPLES);
    let mut foldhash_samples = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        // Alternate order to reduce systematic effects from running one hasher first.
        if sample % 2 == 0 {
            default_samples.push(time_default(text, iterations));
            foldhash_samples.push(time_foldhash(text, iterations, fold_state));
        } else {
            foldhash_samples.push(time_foldhash(text, iterations, fold_state));
            default_samples.push(time_default(text, iterations));
        }
    }

    let default = median(default_samples);
    let foldhash = median(foldhash_samples);
    let default_ns = default.as_secs_f64() * 1e9 / iterations as f64;
    let foldhash_ns = foldhash.as_secs_f64() * 1e9 / iterations as f64;

    println!(
        "{name:<8} {:>6} B  {iterations:>9} iters  DefaultHasher: {default_ns:>9.2} ns  \
         foldhash: {foldhash_ns:>9.2} ns  speedup: {:>5.2}x",
        text.len(),
        default_ns / foldhash_ns,
    );
}

fn main() {
    let short = "Score: 000042";
    let medium = "The quick brown fox jumps over the lazy dog. ".repeat(4);
    let long = "Macroquad text layout cache benchmark input. ".repeat(96);
    let fold_state = FoldRandomState::default();

    println!("Median of {SAMPLES} samples; each sample hashes about 128 MiB.");
    benchmark("short", short, &fold_state);
    benchmark("medium", &medium, &fold_state);
    benchmark("long", &long, &fold_state);
}
