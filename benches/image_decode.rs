//! PNG vs QOI decode benchmark.
//!
//! Decodes every `<name>.png` in the asset directory together with its `<name>.qoi` twin, checks
//! that both produce identical RGBA8 pixels, and reports file sizes and decode times.
//!
//! ```sh
//! cargo bench --bench image_decode                 # default dir: benches/assets/bg
//! cargo bench --bench image_decode -- path/to/dir  # custom dir
//! ```
//!
//! Create the `.qoi` files with e.g. `ffmpeg -i name.png name.qoi`.

use rayquad::texture::Image;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const ITERATIONS: usize = 15;

fn median_decode(bytes: &[u8], decode: fn(&[u8]) -> Image) -> (Duration, Image) {
    let mut times = Vec::with_capacity(ITERATIONS);
    let mut image = decode(bytes); // warm-up
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        image = std::hint::black_box(decode(std::hint::black_box(bytes)));
        times.push(start.elapsed());
    }
    times.sort();
    (times[ITERATIONS / 2], image)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() {
    let dir = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/assets/bg"));

    let mut pngs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .collect();
    pngs.sort();
    assert!(!pngs.is_empty(), "no .png files in {}", dir.display());

    println!(
        "{:<32} {:>9} {:>9} {:>9} {:>9} {:>9} {:>7}",
        "file", "dims", "png KiB", "qoi KiB", "png ms", "qoi ms", "speedup"
    );

    let (mut png_size, mut qoi_size) = (0usize, 0usize);
    let (mut png_time, mut qoi_time) = (Duration::ZERO, Duration::ZERO);
    let mut megapixels = 0.0;

    for png_path in &pngs {
        let qoi_path = png_path.with_extension("qoi");
        let png_bytes = std::fs::read(png_path).unwrap();
        let qoi_bytes = std::fs::read(&qoi_path)
            .unwrap_or_else(|e| panic!("missing {}: {e}", qoi_path.display()));

        let (png_t, png_img) =
            median_decode(&png_bytes, |b| Image::from_png_bytes(b).expect("PNG decode"));
        let (qoi_t, qoi_img) =
            median_decode(&qoi_bytes, |b| Image::from_qoi_bytes(b).expect("QOI decode"));

        assert_eq!(
            (png_img.width, png_img.height),
            (qoi_img.width, qoi_img.height),
            "{}: dimension mismatch",
            png_path.display()
        );
        assert!(
            png_img.bytes == qoi_img.bytes,
            "{}: PNG and QOI pixels differ",
            png_path.display()
        );

        png_size += png_bytes.len();
        qoi_size += qoi_bytes.len();
        png_time += png_t;
        qoi_time += qoi_t;
        megapixels += png_img.width as f64 * png_img.height as f64 / 1e6;

        println!(
            "{:<32} {:>9} {:>9.0} {:>9.0} {:>9.2} {:>9.2} {:>6.1}x",
            png_path.file_name().unwrap().to_string_lossy(),
            format!("{}x{}", png_img.width, png_img.height),
            png_bytes.len() as f64 / 1024.0,
            qoi_bytes.len() as f64 / 1024.0,
            ms(png_t),
            ms(qoi_t),
            png_t.as_secs_f64() / qoi_t.as_secs_f64(),
        );
    }

    println!();
    println!("files:      {}", pngs.len());
    println!(
        "total size: png {:.1} MiB, qoi {:.1} MiB (qoi/png = {:.2})",
        png_size as f64 / 1048576.0,
        qoi_size as f64 / 1048576.0,
        qoi_size as f64 / png_size as f64
    );
    println!(
        "total time: png {:.1} ms ({:.0} MP/s), qoi {:.1} ms ({:.0} MP/s), speedup {:.1}x",
        ms(png_time),
        megapixels / png_time.as_secs_f64(),
        ms(qoi_time),
        megapixels / qoi_time.as_secs_f64(),
        png_time.as_secs_f64() / qoi_time.as_secs_f64()
    );
}
