//! Low level audio: mixer plus per-platform output backends.
//!
//! Vendored from `quad-snd` 0.2.8 (https://github.com/not-fl3/quad-snd),
//! Copyright (c) Fedor Logachev, dual licensed MIT/Apache-2.0.
//! See `NOTICE.md` in this directory.

#![allow(warnings)]

mod error;

pub use error::Error;

#[cfg(target_os = "android")]
#[path = "opensles.rs"]
mod backend;

#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd"))]
#[path = "alsa.rs"]
mod backend;

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "coreaudio.rs"]
mod backend;

#[cfg(target_os = "windows")]
#[path = "wasapi.rs"]
mod backend;

#[cfg(target_arch = "wasm32")]
#[path = "web.rs"]
mod backend;

#[cfg(not(target_arch = "wasm32"))]
mod mixer;

#[cfg(not(target_arch = "wasm32"))]
mod wav;

#[cfg(not(target_arch = "wasm32"))]
mod vorbis;

pub use backend::{AudioContext, Playback, Sound};

pub struct PlaySoundParams {
    pub looped: bool,
    pub volume: f32,
}

impl Default for PlaySoundParams {
    fn default() -> PlaySoundParams {
        PlaySoundParams {
            looped: false,
            volume: 1.,
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
