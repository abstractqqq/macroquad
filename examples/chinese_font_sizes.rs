use macroquad::prelude::*;

const SAMPLE: &str = "你好，世界！Macroquad 中文字体";

#[macroquad::main("Chinese font sizes")]
async fn main() {
    let font = load_ttf_font("examples/chinese.ttf").await.unwrap();

    // Font atlases use a single base raster size. The requested size is kept
    // here to make the initialization intent explicit, but cache warming does
    // not create a separate 32px atlas.
    let characters: Vec<char> = SAMPLE.chars().collect();
    font.populate_font_cache(&characters, 32);

    let layout_16 = prepare_text_layout(
        SAMPLE,
        TextLayoutParams {
            font: Some(&font),
            font_size: 16,
            ..Default::default()
        },
        &[],
    );
    let layout_24 = prepare_text_layout(
        SAMPLE,
        TextLayoutParams {
            font: Some(&font),
            font_size: 24,
            ..Default::default()
        },
        &[],
    );
    let layout_32 = prepare_text_layout(
        SAMPLE,
        TextLayoutParams {
            font: Some(&font),
            font_size: 32,
            ..Default::default()
        },
        &[],
    );
    let layout_48 = prepare_text_layout(
        SAMPLE,
        TextLayoutParams {
            font: Some(&font),
            font_size: 48,
            ..Default::default()
        },
        &[],
    );

    loop {
        clear_background(Color::from_rgba(24, 26, 32, 255));

        draw_text("Font cache initialized at size 32", 32.0, 40.0, 24.0, GRAY);

        draw_text("16 px", 32.0, 90.0, 20.0, SKYBLUE);
        draw_text_layout(&layout_16, 120.0, 90.0, WHITE);

        draw_text("24 px", 32.0, 150.0, 20.0, SKYBLUE);
        draw_text_layout(&layout_24, 120.0, 150.0, WHITE);

        draw_text("32 px", 32.0, 220.0, 20.0, SKYBLUE);
        draw_text_layout(&layout_32, 120.0, 220.0, WHITE);

        draw_text("48 px", 32.0, 310.0, 20.0, SKYBLUE);
        draw_text_layout(&layout_48, 120.0, 310.0, WHITE);

        next_frame().await;
    }
}
