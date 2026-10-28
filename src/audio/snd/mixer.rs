use super::vorbis::decode_ogg;
use super::wav::decode_wav;
use super::{AudioContext, Error, PlaySoundParams};

use core::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

enum AudioMessage {
    AddSound(NonZeroU32, Vec<f32>),
    Play(NonZeroU32, NonZeroU32, bool, f32),
    Stop(NonZeroU32),
    StopAll(NonZeroU32),
    SetVolume(NonZeroU32, f32),
    SetVolumeAll(NonZeroU32, f32),
    Delete(NonZeroU32),
    Pause(NonZeroU32),
    PauseAll(NonZeroU32),
    Resume(NonZeroU32),
    ResumeAll(NonZeroU32),
    Seek(NonZeroU32, usize),
    SeekAll(NonZeroU32, usize),
}

pub const RAMP_FRAMES: usize = 128; // ~2.9ms at 44.1kHz micro-ramp to eliminate clicks/pops

#[derive(Debug)]
pub struct SoundState {
    pub sound_id: NonZeroU32,
    pub play_id: NonZeroU32,
    sample: usize,
    data: Arc<[f32]>,
    looped: bool,
    volume: f32,
    paused: bool,
    last_l: f32,
    last_r: f32,
    ramp_last_l: f32,
    ramp_last_r: f32,
    ramp_remaining: usize,
}

impl SoundState {
    fn new(sound_id: NonZeroU32, play_id: NonZeroU32, data: Arc<[f32]>, looped: bool, volume: f32) -> Self {
        Self {
            sound_id,
            play_id,
            sample: 0,
            data,
            looped,
            volume,
            paused: false,
            last_l: 0.0,
            last_r: 0.0,
            ramp_last_l: 0.0,
            ramp_last_r: 0.0,
            ramp_remaining: 0,
        }
    }

    pub fn seek(&mut self, sample_offset: usize) {
        self.ramp_last_l = self.last_l;
        self.ramp_last_r = self.last_r;
        let target = if self.looped && !self.data.is_empty() {
            sample_offset % self.data.len()
        } else {
            sample_offset.min(self.data.len())
        };
        self.sample = target & !1;
        self.ramp_remaining = RAMP_FRAMES;
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.paused = false;
    }

    fn get_samples(&mut self, n: usize) -> &[f32] {
        let data = &self.data[self.sample..];

        self.sample += n;

        match data.get(..n) {
            Some(data) => data,
            None => data,
        }
    }

    fn rewind(&mut self) {
        self.sample = 0;
    }
}

pub struct Mixer {
    rx: mpsc::Receiver<AudioMessage>,
    sounds: Vec<Option<Arc<[f32]>>>,
    mixer_state: Vec<SoundState>,
}

pub struct MixerBuilder {
    rx: mpsc::Receiver<AudioMessage>,
}

pub struct MixerControl {
    tx: mpsc::Sender<AudioMessage>,
    sound_id: AtomicU32,
    play_id: AtomicU32,
}

fn next_id(counter: &AtomicU32) -> NonZeroU32 {
    let mut cur = counter.load(Ordering::Relaxed);
    loop {
        let next = if cur == u32::MAX { 1 } else { cur + 1 };
        match counter.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(val) => return NonZeroU32::new(val).unwrap_or(NonZeroU32::MIN),
            Err(actual) => cur = actual,
        }
    }
}

pub struct Playback {
    pub play_id: NonZeroU32,
}

impl Playback {
    pub fn stop(self, ctx: &AudioContext) {
        ctx.mixer_ctrl.send(AudioMessage::Stop(self.play_id));
    }

    pub fn pause(&self, ctx: &AudioContext) {
        ctx.mixer_ctrl.pause(self.play_id);
    }

    pub fn resume(&self, ctx: &AudioContext) {
        ctx.mixer_ctrl.resume(self.play_id);
    }

    pub fn seek(&self, ctx: &AudioContext, position: f32) {
        let sample_offset = (position.max(0.0) * 44100.0) as usize * 2;
        ctx.mixer_ctrl.seek(self.play_id, sample_offset);
    }

    pub fn set_volume(&self, ctx: &AudioContext, volume: f32) {
        ctx.mixer_ctrl
            .send(AudioMessage::SetVolume(self.play_id, volume));
    }
}

impl MixerControl {
    pub fn load(&self, data: &[u8]) -> Result<(NonZeroU32, f32), super::Error> {
        let samples = load_samples_from_file(data)?;
        let duration = (samples.len() / 2) as f32 / 44100.0;

        let sound_id = next_id(&self.sound_id);

        self.tx
            .send(super::mixer::AudioMessage::AddSound(sound_id, samples))
            .unwrap_or_else(|_| println!("Audio thread died"));

        Ok((sound_id, duration))
    }

    pub fn play(&self, sound_id: NonZeroU32, params: PlaySoundParams) -> Playback {
        let play_id = next_id(&self.play_id);

        self.send(AudioMessage::Play(
            sound_id,
            play_id,
            params.looped,
            params.volume,
        ));

        Playback { play_id }
    }

    pub fn stop(&self, play_id: NonZeroU32) {
        self.send(AudioMessage::Stop(play_id));
    }

    pub fn stop_all(&self, sound_id: NonZeroU32) {
        self.send(AudioMessage::StopAll(sound_id));
    }

    pub fn pause(&self, play_id: NonZeroU32) {
        self.send(AudioMessage::Pause(play_id));
    }

    pub fn pause_all(&self, sound_id: NonZeroU32) {
        self.send(AudioMessage::PauseAll(sound_id));
    }

    pub fn resume(&self, play_id: NonZeroU32) {
        self.send(AudioMessage::Resume(play_id));
    }

    pub fn resume_all(&self, sound_id: NonZeroU32) {
        self.send(AudioMessage::ResumeAll(sound_id));
    }

    pub fn seek(&self, play_id: NonZeroU32, sample_offset: usize) {
        self.send(AudioMessage::Seek(play_id, sample_offset));
    }

    pub fn seek_all(&self, sound_id: NonZeroU32, sample_offset: usize) {
        self.send(AudioMessage::SeekAll(sound_id, sample_offset));
    }

    pub fn set_volume_all(&self, sound_id: NonZeroU32, volume: f32) {
        self.send(AudioMessage::SetVolumeAll(sound_id, volume));
    }

    pub fn delete(&self, sound_id: NonZeroU32) {
        self.send(AudioMessage::Delete(sound_id));
    }

    fn send(&self, message: AudioMessage) {
        self.tx
            .send(message)
            .unwrap_or_else(|_| println!("Audio thread died"))
    }
}

impl MixerBuilder {
    pub fn build(self) -> Mixer {
        Mixer {
            rx: self.rx,
            sounds: Vec::new(),
            mixer_state: vec![],
        }
    }
}

impl Mixer {
    pub fn new() -> (MixerBuilder, MixerControl) {
        let (tx, rx) = mpsc::channel();

        (
            MixerBuilder { rx },
            MixerControl {
                tx,
                sound_id: AtomicU32::new(1),
                play_id: AtomicU32::new(1),
            },
        )
    }

    pub fn fill_audio_buffer(&mut self, buffer: &mut [f32], frames: usize) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                AudioMessage::AddSound(id, data) => {
                    let idx = (id.get() - 1) as usize;
                    if idx >= self.sounds.len() {
                        self.sounds.resize(idx + 1, None);
                    }
                    self.sounds[idx] = Some(data.into());
                }
                AudioMessage::Play(sound_id, play_id, looped, volume) => {
                    let idx = (sound_id.get() - 1) as usize;
                    if let Some(Some(data)) = self.sounds.get(idx) {
                        self.mixer_state.push(SoundState::new(
                            sound_id,
                            play_id,
                            data.clone(),
                            looped,
                            volume,
                        ));
                    }
                }
                AudioMessage::Stop(play_id) => {
                    if let Some(i) = self.mixer_state.iter().position(|s| s.play_id == play_id) {
                        self.mixer_state.swap_remove(i);
                    }
                }
                AudioMessage::StopAll(sound_id) => {
                    for i in (0..self.mixer_state.len()).rev() {
                        if self.mixer_state[i].sound_id == sound_id {
                            self.mixer_state.swap_remove(i);
                        }
                    }
                }
                AudioMessage::Pause(play_id) => {
                    if let Some(sound) = self.mixer_state.iter_mut().find(|s| s.play_id == play_id) {
                        sound.pause();
                    }
                }
                AudioMessage::PauseAll(sound_id) => {
                    for sound in self.mixer_state.iter_mut().filter(|s| s.sound_id == sound_id) {
                        sound.pause();
                    }
                }
                AudioMessage::Resume(play_id) => {
                    if let Some(sound) = self.mixer_state.iter_mut().find(|s| s.play_id == play_id) {
                        sound.resume();
                    }
                }
                AudioMessage::ResumeAll(sound_id) => {
                    for sound in self.mixer_state.iter_mut().filter(|s| s.sound_id == sound_id) {
                        sound.resume();
                    }
                }
                AudioMessage::Seek(play_id, sample_offset) => {
                    if let Some(sound) = self.mixer_state.iter_mut().find(|s| s.play_id == play_id) {
                        sound.seek(sample_offset);
                    }
                }
                AudioMessage::SeekAll(sound_id, sample_offset) => {
                    for sound in self.mixer_state.iter_mut().filter(|s| s.sound_id == sound_id) {
                        sound.seek(sample_offset);
                    }
                }
                AudioMessage::SetVolume(play_id, volume) => {
                    if let Some(sound) = self.mixer_state.iter_mut().find(|s| s.play_id == play_id)
                    {
                        sound.volume = volume;
                    }
                }
                AudioMessage::SetVolumeAll(sound_id, volume) => {
                    for sound in self
                        .mixer_state
                        .iter_mut()
                        .filter(|s| s.sound_id == sound_id)
                    {
                        sound.volume = volume;
                    }
                }
                AudioMessage::Delete(sound_id) => {
                    for i in (0..self.mixer_state.len()).rev() {
                        if self.mixer_state[i].sound_id == sound_id {
                            self.mixer_state.swap_remove(i);
                        }
                    }
                    let idx = (sound_id.get() - 1) as usize;
                    if let Some(slot) = self.sounds.get_mut(idx) {
                        *slot = None;
                    }
                }
            }
        }

        // zeroize the buffer
        buffer.fill(0.0);

        // Note: Doing manual iteration so we can remove sounds that finished playing
        let mut i = 0;

        while let Some(sound) = self.mixer_state.get_mut(i) {
            if sound.paused {
                i += 1;
                continue;
            }

            let data = Arc::clone(&sound.data);
            let volume = sound.volume;
            let mut buf_offset = 0;

            loop {
                let remainder = buffer.len() - buf_offset;
                let start = sound.sample;
                let available = data.len().saturating_sub(start);
                let count = remainder.min(available);
                sound.sample += count;
                let samples = &data[start..start + count];

                if count >= 2 {
                    sound.last_l = samples[count - 2];
                    sound.last_r = samples[count - 1];
                }

                if sound.ramp_remaining > 0 {
                    let mut s_idx = 0;
                    while s_idx + 1 < count && sound.ramp_remaining > 0 {
                        let factor = (RAMP_FRAMES - sound.ramp_remaining) as f32 / RAMP_FRAMES as f32;
                        let s_l = samples[s_idx] * factor + sound.ramp_last_l * (1.0 - factor);
                        let s_r = samples[s_idx + 1] * factor + sound.ramp_last_r * (1.0 - factor);
                        buffer[buf_offset + s_idx] += s_l * volume;
                        buffer[buf_offset + s_idx + 1] += s_r * volume;
                        sound.ramp_remaining -= 1;
                        s_idx += 2;
                    }
                    for (b, s) in buffer[buf_offset + s_idx..buf_offset + count].iter_mut().zip(&samples[s_idx..]) {
                        *b += s * volume;
                    }
                } else {
                    for (b, s) in buffer[buf_offset..buf_offset + count].iter_mut().zip(samples) {
                        *b += s * volume;
                    }
                }

                buf_offset += count;

                if buf_offset >= buffer.len() {
                    break;
                }

                if sound.looped {
                    sound.rewind();
                    continue;
                }

                break;
            }

            if buf_offset < buffer.len() {
                self.mixer_state.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
}

/// Interleaved samples in the source file's own channel count and sample rate.
pub(super) struct Decoded {
    pub(super) channels: u16,
    pub(super) sample_rate: u32,
    pub(super) samples: Vec<f32>,
}

pub(super) fn check_channels(channels: u16) -> Result<(), Error> {
    if channels == 1 || channels == 2 {
        Ok(())
    } else {
        Err(Error::UnsupportedChannels(channels))
    }
}


/// Convert interleaved mono/stereo samples at `rate` Hz to stereo at 44100 Hz.
///
/// Mono expansion and linear-interpolation resampling happen in one pass. Source positions
/// are tracked with exact integer arithmetic (no f32 index drift on long files).
pub(super) fn to_stereo_44k(mut samples: Vec<f32>, channels: u16, rate: u32) -> Vec<f32> {
    const OUT_RATE: u64 = 44100;

    let ch = channels as usize;
    let in_frames = samples.len() / ch;
    if in_frames == 0 {
        return Vec::new();
    }

    if rate == OUT_RATE as u32 {
        if ch == 2 {
            samples.truncate(in_frames * 2);
            return samples;
        }
        let mut stereo = Vec::with_capacity(in_frames * 2);
        for &s in &samples[..in_frames] {
            stereo.extend_from_slice(&[s, s]);
        }
        return stereo;
    }

    let out_frames = (in_frames as u64 * OUT_RATE / rate as u64) as usize;
    let last = in_frames - 1;
    let mut out = Vec::with_capacity(out_frames * 2);

    // Source position of output frame `i` is `i * rate / OUT_RATE`, kept as (idx, rem / OUT_RATE).
    let (mut idx, mut rem) = (0usize, 0u64);
    for _ in 0..out_frames {
        let i1 = (idx + 1).min(last);
        let t = rem as f32 * (1.0 / OUT_RATE as f32);
        if ch == 1 {
            let (a, b) = (samples[idx], samples[i1]);
            let v = a + (b - a) * t;
            out.extend_from_slice(&[v, v]);
        } else {
            let (a, b) = (&samples[idx * 2..idx * 2 + 2], &samples[i1 * 2..i1 * 2 + 2]);
            out.push(a[0] + (b[0] - a[0]) * t);
            out.push(a[1] + (b[1] - a[1]) * t);
        }
        rem += rate as u64;
        idx += (rem / OUT_RATE) as usize;
        rem %= OUT_RATE;
    }
    out
}

/// Parse ogg/wav and get resampled to 44100, 2 channel data
pub fn load_samples_from_file(bytes: &[u8]) -> Result<Vec<f32>, Error> {
    let decoded = if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        decode_wav(bytes)?
    } else if bytes.starts_with(b"OggS") {
        decode_ogg(bytes)?
    } else {
        return Err(Error::UnsupportedFormat);
    };

    if decoded.sample_rate == 0 {
        return Err(Error::Decode("sample rate is zero".to_string()));
    }

    Ok(to_stereo_44k(
        decoded.samples,
        decoded.channels,
        decoded.sample_rate,
    ))
}
