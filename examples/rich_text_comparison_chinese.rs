use rayquad::{prelude::*, rich_text};

const CHINESE: &str =
    "任务更新: a宏观四方是一个简单易用的 Rust 游戏引擎，支持桌面、网页、\n安卓和苹果平台。中文字体渲染性能测试。";
const CHINESE_MULTILINE: &str = "宏观四方是一个简单易用的 Rust 游戏引擎。: 123\n这个示例比较字体加载、文本测量、字形缓存和绘制命令提交。\n天地玄黄，宇宙洪荒，日月盈昃，辰宿列张。";
const COLORED_PREFIX: &str = "文字为白色，";
const COLORED_BLUE: &str = "这部分是蓝色，";
const COLORED_SUFFIX: &str = "然后恢复白色。";

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

fn window_conf() -> Conf {
    Conf {
        window_title: "Chinese Swash-only vs Parley + Swash".to_owned(),
        window_width: 1400,
        window_height: 760,
        ..Default::default()
    }
}

fn main() {
    rayquad::Window::from_config(window_conf(), game());
}

async fn game() {
    let font_bytes = include_bytes!("chinese.ttf");
    let sample_texts = [
        CHINESE,
        CHINESE_MULTILINE,
        COLORED_PREFIX,
        COLORED_BLUE,
        COLORED_SUFFIX,
    ];
    let mut characters = Font::ascii_character_list();
    characters.extend(sample_texts.concat().chars());
    let texts = sample_texts.map(str::to_owned).to_vec();

    let started = get_time();
    let swash_font = load_ttf_font_from_bytes_ex(
        font_bytes,
        FontLoadParams {
            characters: characters.clone(),
            texts: texts.clone(),
            shaping: FontShapingOptions::new(false, false, false),
            ..Default::default()
        },
    )
    .unwrap();
    let swash_load_us = (get_time() - started) * 1_000_000.0;
    set_default_font(swash_font.clone());
    let started = get_time();
    let rich_font = rich_text::load_ttf_font_from_bytes_ex(
        font_bytes,
        FontLoadParams {
            characters,
            texts,
            shaping: FontShapingOptions::new(false, false, false),
            ..Default::default()
        },
    )
    .unwrap();
    let rich_load_us = (get_time() - started) * 1_000_000.0;

    let mut draw_average = Average::default();
    let mut draw_rich_average = Average::default();
    let mut multiline_average = Average::default();
    let mut multiline_rich_average = Average::default();
    let mut measure_average = Average::default();
    let mut measure_rich_average = Average::default();

    loop {
        clear_background(Color::from_rgba(25, 27, 32, 255));
        let half = screen_width() * 0.5;
        draw_line(half, 0.0, half, screen_height(), 2.0, DARKGRAY);
        draw_text("Swash-only + chinese.ttf", 24.0, 38.0, 27.0, SKYBLUE);
        draw_text(
            "Parley + Swash + chinese.ttf",
            half + 24.0,
            38.0,
            27.0,
            LIME,
        );

        let started = get_time();
        draw_text_ex(
            CHINESE,
            24.0,
            100.0,
            TextParams {
                font: Some(&swash_font),
                font_size: 26,
                color: WHITE,
                ..Default::default()
            },
        );
        let now = get_time();
        let draw_us = draw_average.record(now - started, now);

        let started = get_time();
        rich_text::draw_text(CHINESE, half + 24.0, 100.0, &rich_font, 26.0, WHITE);
        let now = get_time();
        let draw_rich_us = draw_rich_average.record(now - started, now);

        let started = get_time();
        draw_multiline_text_ex(
            CHINESE_MULTILINE,
            24.0,
            190.0,
            None,
            TextParams {
                font: Some(&swash_font),
                font_size: 24,
                color: WHITE,
                ..Default::default()
            },
        );
        let now = get_time();
        let multiline_us = multiline_average.record(now - started, now);

        let started = get_time();
        rich_text::draw_multiline_text_ex(
            CHINESE_MULTILINE,
            half + 24.0,
            190.0,
            rich_text::TextParams::new(&rich_font, 24, WHITE),
        );
        let now = get_time();
        let multiline_rich_us = multiline_rich_average.record(now - started, now);

        draw_text("Inline color comparison:", 24.0, 320.0, 19.0, GRAY);
        draw_text("Inline color comparison:", half + 24.0, 320.0, 19.0, GRAY);

        let colored_y = 355.0;
        let colored_size = 24;
        let segments = [
            (COLORED_PREFIX, WHITE),
            (COLORED_BLUE, BLUE),
            (COLORED_SUFFIX, WHITE),
        ];

        let mut colored_x = 24.0;
        for (text, color) in segments {
            draw_text_ex(
                text,
                colored_x,
                colored_y,
                TextParams {
                    font: Some(&swash_font),
                    font_size: colored_size,
                    color,
                    ..Default::default()
                },
            );
            colored_x += measure_text(text, Some(&swash_font), colored_size, 1.0).width;
        }

        let mut colored_x_rich = half + 24.0;
        for (text, color) in segments {
            rich_text::draw_text(
                text,
                colored_x_rich,
                colored_y,
                &rich_font,
                colored_size as f32,
                color,
            );
            colored_x_rich += rich_text::measure_text(text, &rich_font, colored_size, 1.0).width;
        }

        let started = get_time();
        let _ = measure_text(CHINESE, Some(&swash_font), 26, 1.0);
        let now = get_time();
        let measure_us = measure_average.record(now - started, now);

        let started = get_time();
        let _ = rich_text::measure_text(CHINESE, &rich_font, 26, 1.0);
        let now = get_time();
        let measure_rich_us = measure_rich_average.record(now - started, now);

        let rows = [
            (
                format!("font load: {swash_load_us:.2} us"),
                format!("font load: {rich_load_us:.2} us"),
            ),
            (
                format!("Chinese draw: {draw_us:.2} us (1s update)"),
                format!("Chinese rich draw: {draw_rich_us:.2} us (1s update)"),
            ),
            (
                format!("Chinese multiline: {multiline_us:.2} us (1s update)"),
                format!("Chinese rich multiline: {multiline_rich_us:.2} us (1s update)"),
            ),
            (
                format!("Chinese measure: {measure_us:.2} us (1s update)"),
                format!("Chinese rich measure: {measure_rich_us:.2} us (1s update)"),
            ),
        ];
        for (index, (left, right)) in rows.iter().enumerate() {
            let y = 420.0 + index as f32 * 38.0;
            draw_text(left, 24.0, y, 21.0, SKYBLUE);
            draw_text(right, half + 24.0, y, 21.0, LIME);
        }

        draw_text(
            "Fonts are rasterized and uploaded during loading; drawing only shapes text and submits cached glyphs.",
            24.0,
            screen_height() - 44.0,
            19.0,
            GRAY,
        );
        draw_text(
            "Times are CPU work through draw-command submission, not GPU completion.",
            24.0,
            screen_height() - 18.0,
            19.0,
            GRAY,
        );

        draw_fps();
        next_frame().await;
    }
}
