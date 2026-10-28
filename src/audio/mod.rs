//! Loading and playing sounds.

#![allow(dead_code)]

use crate::{file::load_file, get_context, Error};
use std::sync::Arc;

#[cfg(feature = "audio")]
mod snd;

#[cfg(feature = "audio")]
use snd::{AudioContext as QuadSndContext, Sound as QuadSndSound};

#[cfg(feature = "audio")]
pub use snd::{PlaySoundParams, Playback};

#[cfg(not(feature = "audio"))]
mod dummy_audio {
    use crate::audio::{PlaySoundParams, Playback};

    pub struct AudioContext {}

    impl AudioContext {
        pub fn new() -> AudioContext {
            AudioContext {}
        }

        #[cfg(target_os = "android")]
        pub fn pause(&mut self) {}

        #[cfg(target_os = "android")]
        pub fn resume(&mut self) {}
    }

    pub struct Sound {}

    impl Sound {
        pub fn load(_ctx: &mut AudioContext, _data: &[u8]) -> Result<Sound, String> {
            Ok(Sound {})
        }

        pub fn duration(&self) -> f32 {
            0.0
        }

        pub fn play(&self, _ctx: &mut AudioContext, _params: PlaySoundParams) -> Playback {
            eprintln!("warn: rayquad's \"audio\" feature disabled.");
            Playback {}
        }

        pub fn stop(&self, _ctx: &AudioContext) {}

        pub fn pause(&self, _ctx: &AudioContext) {}

        pub fn resume(&self, _ctx: &AudioContext) {}

        pub fn seek(&self, _ctx: &AudioContext, _position: f32) {}

        pub fn set_volume(&self, _ctx: &AudioContext, _volume: f32) {}

        #[allow(dead_code)]
        pub fn is_loaded(&self) -> bool {
            true
        }

        pub fn delete(&self, _ctx: &AudioContext) {}
    }
}

#[cfg(not(feature = "audio"))]
use dummy_audio::{AudioContext as QuadSndContext, Sound as QuadSndSound};

#[cfg(not(feature = "audio"))]
#[derive(Clone, Copy, Debug)]
pub struct Playback {}

#[cfg(not(feature = "audio"))]
impl Playback {
    pub fn stop(self, _ctx: &QuadSndContext) {}
    pub fn pause(&self, _ctx: &QuadSndContext) {}
    pub fn resume(&self, _ctx: &QuadSndContext) {}
    pub fn seek(&self, _ctx: &QuadSndContext, _position: f32) {}
    pub fn set_volume(&self, _ctx: &QuadSndContext, _volume: f32) {}
}

#[cfg(not(feature = "audio"))]
pub struct PlaySoundParams {
    pub looped: bool,
    pub volume: f32,
}

pub struct AudioContext {
    pub native_ctx: QuadSndContext,
}

impl AudioContext {
    pub fn new() -> AudioContext {
        AudioContext {
            native_ctx: QuadSndContext::new(),
        }
    }

    #[cfg(target_os = "android")]
    pub fn pause(&mut self) {
        self.native_ctx.pause()
    }

    #[cfg(target_os = "android")]
    pub fn resume(&mut self) {
        self.native_ctx.resume()
    }
}

pub struct QuadSndSoundGuarded(pub QuadSndSound);

impl Drop for QuadSndSoundGuarded {
    fn drop(&mut self) {
        let ctx = &get_context().audio_context;
        self.0.delete(&ctx.native_ctx);
    }
}

#[derive(Clone)]
pub struct Sound(pub Arc<QuadSndSoundGuarded>);

impl Sound {
    /// Get the sound duration in seconds.
    pub fn duration(&self) -> f32 {
        self.0 .0.duration()
    }

    /// Pause all playing instances of this sound.
    pub fn pause(&self) {
        let ctx = &get_context().audio_context;
        self.0 .0.pause(&ctx.native_ctx);
    }

    /// Resume all paused instances of this sound.
    pub fn resume(&self) {
        let ctx = &get_context().audio_context;
        self.0 .0.resume(&ctx.native_ctx);
    }

    /// Seek all playing instances of this sound to `position` (in seconds).
    pub fn seek(&self, position: f32) {
        let ctx = &get_context().audio_context;
        self.0 .0.seek(&ctx.native_ctx, position);
    }

    /// Stop all playing instances of this sound.
    pub fn stop(&self) {
        let ctx = &get_context().audio_context;
        self.0 .0.stop(&ctx.native_ctx);
    }

    /// Set volume for all instances of this sound.
    pub fn set_volume(&self, volume: f32) {
        let ctx = &get_context().audio_context;
        self.0 .0.set_volume(&ctx.native_ctx, volume);
    }
}

impl std::fmt::Debug for Sound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sound")
            .field("duration", &self.duration())
            .finish()
    }
}

/// Load audio file.
///
/// Attempts to automatically detect the format of the source of data.
pub async fn load_sound(path: &str) -> Result<Sound, Error> {
    let data = load_file(path).await?;

    load_sound_from_bytes(&data).await
}

/// Load audio data.
///
/// Attempts to automatically detect the format of the source of data.
pub async fn load_sound_from_bytes(data: &[u8]) -> Result<Sound, Error> {
    let sound = {
        let ctx = &mut get_context().audio_context;
        QuadSndSound::load(&mut ctx.native_ctx, data)
            .map_err(|e| Error::AudioError(e.to_string()))?
    };

    Ok(Sound(Arc::new(QuadSndSoundGuarded(sound))))
}

pub fn play_sound_once(sound: &Sound) -> Playback {
    let ctx = &mut get_context().audio_context;

    sound.0 .0.play(
        &mut ctx.native_ctx,
        PlaySoundParams {
            looped: false,
            volume: 1.0,
        },
    )
}

pub fn play_sound(sound: &Sound, params: PlaySoundParams) -> Playback {
    let ctx = &mut get_context().audio_context;
    sound.0 .0.play(&mut ctx.native_ctx, params)
}

pub fn stop_sound(sound: &Sound) {
    let ctx = &get_context().audio_context;
    sound.0 .0.stop(&ctx.native_ctx);
}

pub fn pause_sound(sound: &Sound) {
    let ctx = &get_context().audio_context;
    sound.0 .0.pause(&ctx.native_ctx);
}

pub fn resume_sound(sound: &Sound) {
    let ctx = &get_context().audio_context;
    sound.0 .0.resume(&ctx.native_ctx);
}

pub fn seek_sound(sound: &Sound, position: f32) {
    let ctx = &get_context().audio_context;
    sound.0 .0.seek(&ctx.native_ctx, position);
}

pub fn sound_duration(sound: &Sound) -> f32 {
    sound.duration()
}

pub fn set_sound_volume(sound: &Sound, volume: f32) {
    let ctx = &get_context().audio_context;
    sound.0 .0.set_volume(&ctx.native_ctx, volume);
}

pub fn stop_playback(playback: Playback) {
    let ctx = &get_context().audio_context;
    playback.stop(&ctx.native_ctx);
}

pub fn pause_playback(playback: &Playback) {
    let ctx = &get_context().audio_context;
    playback.pause(&ctx.native_ctx);
}

pub fn resume_playback(playback: &Playback) {
    let ctx = &get_context().audio_context;
    playback.resume(&ctx.native_ctx);
}

pub fn seek_playback(playback: &Playback, position: f32) {
    let ctx = &get_context().audio_context;
    playback.seek(&ctx.native_ctx, position);
}

pub fn set_playback_volume(playback: &Playback, volume: f32) {
    let ctx = &get_context().audio_context;
    playback.set_volume(&ctx.native_ctx, volume);
}
