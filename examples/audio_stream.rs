use rayquad::{audio, prelude::*};

fn main() {
    rayquad::Window::new("Audio streaming", game());
}

async fn game() {
    set_pc_assets_folder("examples");

    // Streamed from disk while playing: only a small buffer is decoded ahead of time,
    // so loading is instant and memory stays flat no matter how long the track is.
    // Any WAV/OGG works, `sound.wav` is just the file shipped with the examples.
    let bgm = audio::load_sound_stream("sound.wav").await.unwrap();
    let mut playback = None;
    let mut paused = false;

    loop {
        clear_background(LIGHTGRAY);

        draw_text("Space: start/restart looped BGM", 20.0, 40.0, 30.0, DARKGRAY);
        draw_text("P: pause/resume   S: stop   R: seek to 0", 20.0, 80.0, 30.0, DARKGRAY);
        draw_text(
            &format!("duration: {:.2}s", audio::sound_duration(&bgm)),
            20.0,
            120.0,
            30.0,
            DARKGRAY,
        );

        if is_key_pressed(KeyCode::Space) {
            audio::stop_sound(&bgm);
            paused = false;
            playback = Some(audio::play_sound(
                &bgm,
                audio::PlaySoundParams {
                    looped: true,
                    volume: 0.7,
                },
            ));
        }
        if let Some(pb) = playback.as_ref() {
            if is_key_pressed(KeyCode::P) {
                paused = !paused;
                if paused {
                    audio::pause_playback(pb);
                } else {
                    audio::resume_playback(pb);
                }
            }
            if is_key_pressed(KeyCode::R) {
                audio::seek_playback(pb, 0.0);
            }
        }
        if is_key_pressed(KeyCode::S) {
            audio::stop_sound(&bgm);
            playback = None;
        }

        next_frame().await
    }
}

