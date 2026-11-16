//! Example showing how to bake a TTF into production `.bin` + `.png` assets
//! and load them with zero runtime font parsing or shaping overhead.

use rayquad::prelude::*;

#[cfg(feature = "font-rasterize")]
fn bake_assets() {
    println!("Baking DancingScript font into .bin and .png assets...");
    let ttf_bytes = include_bytes!("DancingScriptRegular.ttf");
    let (font_data, image) = rayquad::text::FontData::bake_from_ttf_bytes(
        ttf_bytes,
        FontLoadParams {
            characters: Font::ascii_character_list(),
            raster_size: 48,
            filter: miniquad::FilterMode::Linear,
        },
    )
    .expect("Failed to bake font from TTF");

    let bin_bytes = font_data.serialize_baked();
    std::fs::write("target/dancing_script.bin", bin_bytes)
        .expect("Failed to write target/dancing_script.bin");

    let file = std::fs::File::create("target/dancing_script.png")
        .expect("Failed to create target/dancing_script.png");
    let mut encoder = png::Encoder::new(file, image.width.into(), image.height.into());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("Failed to write png header");
    writer
        .write_image_data(&image.bytes)
        .expect("Failed to write png image bytes");

    println!("Successfully exported target/dancing_script.bin and target/dancing_script.png!");
}

fn main() {
    #[cfg(feature = "font-rasterize")]
    bake_assets();

    rayquad::Window::new("Bake Font Demo", game());
}

async fn game() {
    // Load the baked font assets
    let bin_bytes = std::fs::read("target/dancing_script.bin")
        .expect("Please ensure target/dancing_script.bin was generated");
    let png_bytes = std::fs::read("target/dancing_script.png")
        .expect("Please ensure target/dancing_script.png was generated");

    let font = rayquad::text::load_baked_font_from_bytes(&bin_bytes, &png_bytes, miniquad::FilterMode::Linear)
        .expect("Failed to load baked font");

    loop {
        clear_background(DARKGRAY);

        draw_text("Pre-baked Font (.bin + .png): Zero Shaping & Zero Font Parsing!", 20.0, 40.0, 22.0, YELLOW);
        draw_text_ex(
            "Hello from pre-baked DancingScript!",
            40.0,
            120.0,
            TextParams {
                font: Some(&font),
                font_size: 36,
                color: WHITE,
                ..Default::default()
            },
        );

        next_frame().await;
    }
}
