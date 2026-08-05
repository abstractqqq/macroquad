use rayquad::{audio, prelude::*};

fn main() {
    rayquad::Window::new("Audio", game());
}

async fn game() {
    set_pc_assets_folder("examples");

    let sound1 = audio::load_sound("sound.wav").await.unwrap();
    let sound2 = audio::load_sound("sound2.wav").await.unwrap();

    loop {
        clear_background(LIGHTGRAY);

        draw_text("Press 1 to play sound 1", 20.0, 40.0, 30.0, DARKGRAY);
        draw_text("Press 2 to play sound 2", 20.0, 80.0, 30.0, DARKGRAY);

        if is_key_pressed(KeyCode::Key1) {
            warn!("play 1!");
            audio::play_sound_once(&sound1);
        }
        if is_key_pressed(KeyCode::Key2) {
            warn!("play 2!");
            audio::play_sound_once(&sound2);
        }
        next_frame().await
    }
}
