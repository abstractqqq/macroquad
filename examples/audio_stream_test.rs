use rayquad::{audio, prelude::*};

fn main() {
    rayquad::Window::new("Audio Streaming Test (WAV & OGG)", game());
}

enum Track {
    Wav,
    Ogg,
}

async fn game() {
    set_pc_assets_folder("assets/audio");

    let wav_sound = audio::load_sound_stream("test.wav").await.unwrap();
    let ogg_sound = audio::load_sound_stream("test.ogg").await.unwrap();

    let mut active_track: Option<Track> = None;
    let mut playback: Option<audio::Playback> = None;
    let mut paused = false;
    let mut looped = true;
    let mut volume = 0.8f32;

    let start_track = |track: Track,
                       wav: &audio::Sound,
                       ogg: &audio::Sound,
                       looped: bool,
                       volume: f32|
     -> (Option<Track>, Option<audio::Playback>) {
        audio::stop_sound(wav);
        audio::stop_sound(ogg);
        let sound = match track {
            Track::Wav => wav,
            Track::Ogg => ogg,
        };
        let pb = audio::play_sound(sound, audio::PlaySoundParams { looped, volume });
        (Some(track), Some(pb))
    };

    loop {
        clear_background(LIGHTGRAY);

        let track_name = match active_track {
            Some(Track::Wav) => "test.wav",
            Some(Track::Ogg) => "test.ogg",
            None => "none",
        };
        let status = if active_track.is_none() {
            "stopped"
        } else if paused {
            "paused"
        } else {
            "playing"
        };

        draw_text(
            &format!("Track: {track_name} ({status}) | Loop: {looped} | Vol: {volume:.2}"),
            20.0,
            40.0,
            26.0,
            BLACK,
        );
        draw_text(
            &format!(
                "Durations - WAV: {:.2}s | OGG: {:.2}s",
                audio::sound_duration(&wav_sound),
                audio::sound_duration(&ogg_sound)
            ),
            20.0,
            75.0,
            24.0,
            DARKGRAY,
        );

        draw_text("1: Stream test.wav", 20.0, 130.0, 24.0, DARKGRAY);
        draw_text("2: Stream test.ogg", 20.0, 165.0, 24.0, DARKGRAY);
        draw_text("P: Pause / Resume", 20.0, 200.0, 24.0, DARKGRAY);
        draw_text("S: Stop", 20.0, 235.0, 24.0, DARKGRAY);
        draw_text("L: Toggle loop (takes effect on next play)", 20.0, 270.0, 24.0, DARKGRAY);
        draw_text("0 / 3 / 5: Seek to 0s / 25s / 50s (near end)", 20.0, 305.0, 24.0, DARKGRAY);
        draw_text("Up / Down: Volume +/-", 20.0, 340.0, 24.0, DARKGRAY);

        if let Some(err) = playback.as_ref().and_then(audio::playback_error) {
            draw_text(&format!("Stream error: {err}"), 20.0, 390.0, 24.0, RED);
        }

        if is_key_pressed(KeyCode::Key1) {
            paused = false;
            (active_track, playback) = start_track(Track::Wav, &wav_sound, &ogg_sound, looped, volume);
        }
        if is_key_pressed(KeyCode::Key2) {
            paused = false;
            (active_track, playback) = start_track(Track::Ogg, &wav_sound, &ogg_sound, looped, volume);
        }
        if is_key_pressed(KeyCode::L) {
            looped = !looped;
        }
        if is_key_pressed(KeyCode::S) {
            audio::stop_sound(&wav_sound);
            audio::stop_sound(&ogg_sound);
            active_track = None;
            playback = None;
            paused = false;
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
            if is_key_pressed(KeyCode::Key0) {
                audio::seek_playback(pb, 0.0);
            }
            if is_key_pressed(KeyCode::Key3) {
                audio::seek_playback(pb, 25.0);
            }
            if is_key_pressed(KeyCode::Key5) {
                audio::seek_playback(pb, 50.0);
            }
            if is_key_pressed(KeyCode::Up) {
                volume = (volume + 0.1).min(1.0);
                audio::set_playback_volume(pb, volume);
            }
            if is_key_pressed(KeyCode::Down) {
                volume = (volume - 0.1).max(0.0);
                audio::set_playback_volume(pb, volume);
            }
        }

        next_frame().await
    }
}

