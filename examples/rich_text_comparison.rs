use macroquad::{prelude::*, rich_text};

const SAMPLE: &str = "AVATAR ffi office\nMacroquad text rendering";
const LONG_SENTENCE: &str = "Macroquad is a simple and easy-to-use game library for Rust that supports\n desktop, HTML5, Android, and iOS while keeping the application loop pleasantly small.";
const LONG_MULTILINE: &str = "Macroquad is a simple and easy-to-use game library for Rust.\nThis second line compares multiline shaping, glyph caching, atlas lookup,\nand draw-command submission between Swash-only and Parley plus Swash.";

#[derive(Default)]
struct Average {
    value_us: f64,
    displayed_us: f64,
    samples: u64,
    next_display_update: f64,
}

impl Average {
    fn record(&mut self, elapsed_seconds: f64, now: f64) -> f64 {
        let elapsed_us = elapsed_seconds * 1_000_000.0;
        self.samples += 1;
        self.value_us += (elapsed_us - self.value_us) / self.samples.min(120) as f64;
        if self.samples == 1 || now >= self.next_display_update {
            self.displayed_us = self.value_us;
            self.next_display_update = now + 1.0;
        }
        self.displayed_us
    }
}

#[macroquad::main("Swash-only vs Parley + Swash")]
async fn main() {
    let started = get_time();
    let swash_font =
        load_ttf_font_from_bytes(include_bytes!("../assets/fonts/ProggyClean.ttf")).unwrap();
    let swash_load_us = (get_time() - started) * 1_000_000.0;

    let started = get_time();
    let rich_font =
        rich_text::load_ttf_font_from_bytes(include_bytes!("../assets/fonts/ProggyClean.ttf"))
            .unwrap();
    let rich_load_us = (get_time() - started) * 1_000_000.0;

    let mut draw_text_average = Average::default();
    let mut draw_rich_average = Average::default();
    let mut multiline_average = Average::default();
    let mut multiline_rich_average = Average::default();
    let mut measure_average = Average::default();
    let mut measure_rich_average = Average::default();

    loop {
        clear_background(Color::from_rgba(25, 27, 32, 255));

        let half = screen_width() * 0.5;
        draw_line(half, 0.0, half, screen_height(), 2.0, DARKGRAY);
        draw_text("Swash-only timings", 24.0, 36.0, 26.0, SKYBLUE);
        draw_text("Parley + Swash rich-text", half + 24.0, 36.0, 26.0, LIME);

        let started = get_time();
        draw_text(LONG_SENTENCE, 24.0, 82.0, 18.0, WHITE);
        let now = get_time();
        let draw_text_us = draw_text_average.record(now - started, now);

        let started = get_time();
        rich_text::draw_text(LONG_SENTENCE, half + 24.0, 82.0, &rich_font, 18.0, WHITE);
        let now = get_time();
        let draw_rich_us = draw_rich_average.record(now - started, now);

        let started = get_time();
        draw_multiline_text_ex(
            LONG_MULTILINE,
            24.0,
            170.0,
            Some(1.0),
            TextParams {
                font: Some(&swash_font),
                font_size: 18,
                color: WHITE,
                ..Default::default()
            },
        );
        let now = get_time();
        let multiline_us = multiline_average.record(now - started, now);

        let started = get_time();
        rich_text::draw_multiline_text_ex(
            LONG_MULTILINE,
            half + 24.0,
            170.0,
            rich_text::TextParams::new(&rich_font, 18, WHITE),
        );
        let now = get_time();
        let multiline_rich_us = multiline_rich_average.record(now - started, now);

        let started = get_time();
        let _ = measure_text(LONG_SENTENCE, Some(&swash_font), 18, 1.0);
        let now = get_time();
        let measure_us = measure_average.record(now - started, now);

        let started = get_time();
        let _ = rich_text::measure_text(LONG_SENTENCE, &rich_font, 18, 1.0);
        let now = get_time();
        let measure_rich_us = measure_rich_average.record(now - started, now);

        draw_text(
            &format!("draw_text: {draw_text_us:.2} us (updated every 1s)"),
            24.0,
            280.0,
            19.0,
            SKYBLUE,
        );
        draw_text(
            &format!("rich_text::draw_text: {draw_rich_us:.2} us (updated every 1s)"),
            half + 24.0,
            280.0,
            19.0,
            LIME,
        );
        draw_text(
            &format!("draw_multiline_text_ex: {multiline_us:.2} us (updated every 1s)"),
            24.0,
            312.0,
            19.0,
            SKYBLUE,
        );
        draw_text(
            &format!(
                "rich_text::draw_multiline_text_ex: {multiline_rich_us:.2} us (updated every 1s)"
            ),
            half + 24.0,
            312.0,
            19.0,
            LIME,
        );
        draw_text(
            &format!("load_ttf_font_from_bytes: {swash_load_us:.2} us (one shot)"),
            24.0,
            344.0,
            19.0,
            SKYBLUE,
        );
        draw_text(
            &format!("rich_text::load_ttf_font_from_bytes: {rich_load_us:.2} us (one shot)"),
            half + 24.0,
            344.0,
            19.0,
            LIME,
        );
        draw_text(
            &format!("measure_text: {measure_us:.2} us (updated every 1s)"),
            24.0,
            376.0,
            19.0,
            SKYBLUE,
        );
        draw_text(
            &format!("rich_text::measure_text: {measure_rich_us:.2} us (updated every 1s)"),
            half + 24.0,
            376.0,
            19.0,
            LIME,
        );

        let font_size = 32;
        let baseline = 460.0;
        let dimensions_swash = measure_text(SAMPLE, Some(&swash_font), font_size, 1.0);
        let dimensions_rich = rich_text::measure_text(SAMPLE, &rich_font, font_size, 1.0);
        draw_multiline_text_ex(
            SAMPLE,
            24.0,
            baseline,
            Some(1.2),
            TextParams {
                font: Some(&swash_font),
                font_size,
                color: WHITE,
                ..Default::default()
            },
        );
        rich_text::draw_multiline_text_ex(
            SAMPLE,
            half + 24.0,
            baseline,
            rich_text::TextParams::new(&rich_font, font_size, WHITE),
        );
        draw_rectangle_lines(
            24.0,
            baseline - dimensions_swash.offset_y,
            dimensions_swash.width,
            dimensions_swash.height,
            1.0,
            SKYBLUE,
        );
        draw_rectangle_lines(
            half + 24.0,
            baseline - dimensions_rich.offset_y,
            dimensions_rich.width,
            dimensions_rich.height,
            1.0,
            LIME,
        );

        draw_text(
            "First sample is cold-cache; rolling average approaches cached steady state. Timings measure CPU work and draw-command submission, not GPU completion.",
            24.0,
            screen_height() - 24.0,
            16.0,
            GRAY,
        );

        next_frame().await;
    }
}
