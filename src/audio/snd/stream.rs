//! Streaming playback: files are decoded incrementally by a worker thread (one per playing
//! stream) and handed to the audio thread as small 44.1 kHz stereo chunks, so only about a
//! second of audio is ever resident in memory.
//!
//! The audio thread never touches the disk or a decoder, and never allocates or frees memory
//! for a stream: it only does `try_recv` on the chunk queue, returns spent buffers through a
//! preallocated channel, and requests seeks through an atomic. If the worker falls behind
//! (underrun) the voice fades to silence and fades back in when data arrives. A seek keeps
//! playing the old audio until the new position is decoded, then crossfades.

use super::vorbis::OggStream;
use super::wav::WavStream;
use super::{mixer::RAMP_FRAMES, Error, StreamSource};

use core::num::NonZeroU32;
use foldhash::{HashMap, HashMapExt};
use std::collections::VecDeque;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Source frames decoded per chunk.
const CHUNK_FRAMES: usize = 4096;
/// Chunks buffered ahead of the audio thread (about 1 s at 44.1 kHz), enough to ride out
/// disk stalls such as asset loading on the same drive.
const QUEUE_CHUNKS: usize = 11;

pub(super) enum StreamDecoder {
    Wav(WavStream),
    Ogg(OggStream),
}

impl StreamDecoder {
    pub(super) fn open(source: &StreamSource) -> Result<Self, Error> {
        let mut header = Vec::with_capacity(12);
        match source {
            StreamSource::File(path) => {
                std::fs::File::open(path)?
                    .take(12)
                    .read_to_end(&mut header)?;
            }
            StreamSource::Bytes(bytes) => {
                header.extend_from_slice(&bytes[..bytes.len().min(12)]);
            }
        }

        if header.len() >= 12 && &header[0..4] == b"RIFF" && &header[8..12] == b"WAVE" {
            Ok(StreamDecoder::Wav(WavStream::open(source)?))
        } else if header.starts_with(b"OggS") {
            Ok(StreamDecoder::Ogg(match source {
                StreamSource::File(path) => OggStream::open_file(path)?,
                StreamSource::Bytes(bytes) => OggStream::open_memory(bytes.clone())?,
            }))
        } else {
            Err(Error::UnsupportedFormat)
        }
    }

    pub(super) fn channels(&self) -> u16 {
        match self {
            StreamDecoder::Wav(d) => d.channels(),
            StreamDecoder::Ogg(d) => d.channels,
        }
    }

    pub(super) fn sample_rate(&self) -> u32 {
        match self {
            StreamDecoder::Wav(d) => d.sample_rate(),
            StreamDecoder::Ogg(d) => d.sample_rate,
        }
    }

    pub(super) fn total_frames(&self) -> usize {
        match self {
            StreamDecoder::Wav(d) => d.total_frames,
            StreamDecoder::Ogg(d) => d.total_frames,
        }
    }

    pub(super) fn duration(&self) -> f32 {
        self.total_frames() as f32 / self.sample_rate() as f32
    }

    /// Appends up to `max_frames` interleaved frames to `out`, returns frames decoded.
    fn decode_frames(&mut self, max_frames: usize, out: &mut Vec<f32>) -> Result<usize, Error> {
        match self {
            StreamDecoder::Wav(d) => d.decode_frames(max_frames, out),
            StreamDecoder::Ogg(d) => d.decode_frames(max_frames, out),
        }
    }

    fn seek_frame(&mut self, frame: usize) -> Result<(), Error> {
        match self {
            StreamDecoder::Wav(d) => d.seek_frame(frame),
            StreamDecoder::Ogg(d) => d.seek_frame(frame),
        }
    }
}

/// Incremental version of `mixer::to_stereo_44k`: converts mono/stereo at any rate to stereo
/// 44.1 kHz with linear interpolation, carrying the fractional position across chunks.
struct Resampler {
    ch: usize,
    rate: u64,
    buf: Vec<f32>,
    idx: usize,
    rem: u64,
}

impl Resampler {
    const OUT_RATE: u64 = 44100;

    fn new(channels: u16, rate: u32) -> Self {
        Resampler {
            ch: channels as usize,
            rate: rate as u64,
            buf: Vec::new(),
            idx: 0,
            rem: 0,
        }
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.idx = 0;
        self.rem = 0;
    }

    /// Feeds `input` and appends all output frames that can be produced. With `eof` the last
    /// source frame is held for the tail and the state is reset afterwards.
    fn push(&mut self, input: &[f32], eof: bool, out: &mut Vec<f32>) {
        let ch = self.ch;

        // Already 44.1 kHz: no interpolation, no hold-back frame, just (up)mix to stereo.
        if self.rate == Self::OUT_RATE {
            if ch == 2 {
                out.extend_from_slice(input);
            } else {
                out.reserve(input.len() * 2);
                for &s in input {
                    out.extend_from_slice(&[s, s]);
                }
            }
            return;
        }

        self.buf.extend_from_slice(input);
        let frames = self.buf.len() / ch;

        while self.idx < frames {
            if self.idx + 1 >= frames && !eof {
                break; // need the next frame to interpolate
            }
            let i1 = (self.idx + 1).min(frames - 1);
            let t = self.rem as f32 * (1.0 / Self::OUT_RATE as f32);
            if ch == 1 {
                let (a, b) = (self.buf[self.idx], self.buf[i1]);
                let v = a + (b - a) * t;
                out.extend_from_slice(&[v, v]);
            } else {
                let (a, b) = (
                    &self.buf[self.idx * 2..self.idx * 2 + 2],
                    &self.buf[i1 * 2..i1 * 2 + 2],
                );
                out.push(a[0] + (b[0] - a[0]) * t);
                out.push(a[1] + (b[1] - a[1]) * t);
            }
            self.rem += self.rate;
            self.idx += (self.rem / Self::OUT_RATE) as usize;
            self.rem %= Self::OUT_RATE;
        }

        let drain = self.idx.min(frames);
        self.buf.drain(..drain * ch);
        self.idx -= drain;

        if eof {
            self.reset();
        }
    }
}

/// State shared between a `StreamState` and its worker.
struct Shared {
    /// Latest seek request: generation in the high 32 bits, target frame at 44.1 kHz in the
    /// low 32 bits. A single atomic, so the worker never sees a torn request and the audio
    /// thread never allocates to send one.
    seek: AtomicU64,
}

enum Chunk {
    Data(u32, Vec<f32>),
    End(u32),
}

/// Chunk buffers travel worker -> audio thread -> worker so that neither allocation nor
/// deallocation happens on the audio thread. The return channel is bounded (preallocated,
/// `try_send` never allocates). The worker only allocates a buffer when this channel is
/// empty, so at most `QUEUE_CHUNKS` (queued) + 1 (worker) + 2 (`cur` and `fade` on the
/// audio thread) buffers ever exist, and the channel can always take one back.
const RECYCLE_SLOTS: usize = QUEUE_CHUNKS + 4;

/// Stores runtime errors produced by background stream decoder threads so callers can
/// inspect them without touching the real-time audio thread.
///
/// Both maps are keyed by sound and hold one entry each, so their size is bounded by the
/// number of live sounds (entries go away on `Sound::delete`, which also runs on drop).
pub(super) struct StreamErrors {
    /// Latest failing playback of each sound.
    by_play: HashMap<NonZeroU32, (NonZeroU32, String)>,
    by_sound: HashMap<NonZeroU32, String>,
}

impl StreamErrors {
    pub(super) fn new() -> Self {
        Self {
            by_play: HashMap::new(),
            by_sound: HashMap::new(),
        }
    }

    pub(super) fn record(&mut self, sound_id: NonZeroU32, play_id: NonZeroU32, message: String) {
        self.by_sound.insert(sound_id, message.clone());
        self.by_play.insert(sound_id, (play_id, message));
    }

    pub(super) fn clear_sound(&mut self, sound_id: NonZeroU32) {
        self.by_sound.remove(&sound_id);
        self.by_play.remove(&sound_id);
    }

    pub(super) fn clear_sound_error(&mut self, sound_id: NonZeroU32) {
        self.by_sound.remove(&sound_id);
    }

    /// Only the latest failing playback of a sound is remembered.
    pub(super) fn get_play(&self, play_id: NonZeroU32) -> Option<String> {
        self.by_play
            .values()
            .find(|(pid, _)| *pid == play_id)
            .map(|(_, msg)| msg.clone())
    }

    pub(super) fn get_sound(&self, sound_id: NonZeroU32) -> Option<String> {
        self.by_sound.get(&sound_id).cloned()
    }
}

fn report_stream_error(
    errors: &Arc<Mutex<StreamErrors>>,
    sound_id: NonZeroU32,
    play_id: NonZeroU32,
    message: String,
) {
    crate::logging::error!("audio stream error: {message}");
    errors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record(sound_id, play_id, message);
}

fn spawn_worker(
    sound_id: NonZeroU32,
    play_id: NonZeroU32,
    source: StreamSource,
    looped: bool,
    errors: Arc<Mutex<StreamErrors>>,
) -> (Receiver<Chunk>, Arc<Shared>, mpsc::SyncSender<Vec<f32>>) {
    let shared = Arc::new(Shared {
        seek: AtomicU64::new(0),
    });
    let (chunk_tx, chunk_rx) = mpsc::sync_channel(QUEUE_CHUNKS);
    let (recycle_tx, recycle_rx) = mpsc::sync_channel(RECYCLE_SLOTS);

    // If spawning fails the closure (and `chunk_tx`) is dropped, which the audio thread sees
    // as a disconnected queue and treats as the end of the stream.
    let worker_errors = Arc::clone(&errors);
    let worker_shared = Arc::clone(&shared);
    if let Err(err) = std::thread::Builder::new()
        .name("rayquad-audio-stream".to_string())
        .spawn(move || {
            worker(
                sound_id,
                play_id,
                source,
                looped,
                worker_shared,
                chunk_tx,
                recycle_rx,
                worker_errors,
            )
        })
    {
        report_stream_error(
            &errors,
            sound_id,
            play_id,
            format!("failed to spawn audio stream thread: {err}"),
        );
    }

    (chunk_rx, shared, recycle_tx)
}

fn worker(
    sound_id: NonZeroU32,
    play_id: NonZeroU32,
    source: StreamSource,
    looped: bool,
    shared: Arc<Shared>,
    chunk_tx: mpsc::SyncSender<Chunk>,
    recycle_rx: Receiver<Vec<f32>>,
    errors: Arc<Mutex<StreamErrors>>,
) {
    let mut decoder = match StreamDecoder::open(&source) {
        Ok(d) => d,
        Err(err) => {
            report_stream_error(
                &errors,
                sound_id,
                play_id,
                format!("failed to open audio stream: {err}"),
            );
            return;
        }
    };
    let rate = decoder.sample_rate();
    let mut resampler = Resampler::new(decoder.channels(), rate);

    let mut generation = 0u32;
    let mut pending: VecDeque<Chunk> = VecDeque::new();
    let mut finished = false;
    let mut decoded_since_seek = false;
    let mut raw = Vec::new();

    loop {
        // The voice was stopped or dropped: only this thread still holds `shared`.
        if Arc::strong_count(&shared) == 1 {
            return;
        }

        let request = shared.seek.load(Ordering::Acquire);
        let new_generation = (request >> 32) as u32;
        if new_generation != generation {
            generation = new_generation;
            pending.clear();
            resampler.reset();
            decoded_since_seek = false;
            let target_44k = request as u32 as u64;
            let target = (target_44k * rate as u64 / Resampler::OUT_RATE) as usize;
            if let Err(err) = decoder.seek_frame(target) {
                report_stream_error(
                    &errors,
                    sound_id,
                    play_id,
                    format!("audio stream seek error: {err}"),
                );
                finished = true;
                pending.push_back(Chunk::End(generation));
            } else {
                finished = false;
            }
        }

        if pending.is_empty() && !finished {
            raw.clear();
            let mut out = recycle_rx.try_recv().unwrap_or_default();
            out.clear();
            match decoder.decode_frames(CHUNK_FRAMES, &mut raw) {
                Ok(frames) if frames > 0 => {
                    decoded_since_seek = true;
                    resampler.push(&raw, false, &mut out);
                }
                Ok(_) => {
                    resampler.push(&[], true, &mut out);
                    if looped && decoded_since_seek {
                        if let Err(err) = decoder.seek_frame(0) {
                            report_stream_error(
                                &errors,
                                sound_id,
                                play_id,
                                format!("audio stream loop rewind error: {err}"),
                            );
                            finished = true;
                        } else {
                            decoded_since_seek = false;
                        }
                    } else {
                        finished = true;
                    }
                }
                Err(err) => {
                    report_stream_error(
                        &errors,
                        sound_id,
                        play_id,
                        format!("audio stream decode error: {err}"),
                    );
                    finished = true;
                }
            }
            if !out.is_empty() {
                pending.push_back(Chunk::Data(generation, out));
            }
            if finished {
                pending.push_back(Chunk::End(generation));
            }
        }

        match pending.pop_front() {
            Some(chunk) => match chunk_tx.try_send(chunk) {
                Ok(()) => {}
                Err(TrySendError::Full(chunk)) => {
                    pending.push_front(chunk);
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TrySendError::Disconnected(_)) => return,
            },
            // Finished and everything delivered: idle until a seek or the voice goes away.
            None if finished => std::thread::sleep(Duration::from_millis(10)),
            None => {}
        }
    }
}

/// A playing stream, owned by the mixer / audio thread.
pub struct StreamState {
    pub sound_id: NonZeroU32,
    pub play_id: NonZeroU32,
    pub volume: f32,
    pub paused: bool,
    rx: Receiver<Chunk>,
    shared: Arc<Shared>,
    recycle: mpsc::SyncSender<Vec<f32>>,
    generation: u32,
    /// A seek was requested and no chunk of the new generation has arrived yet. Old audio
    /// keeps playing meanwhile.
    seek_pending: bool,
    /// The queue ran dry (underrun or end): output faded / is fading to silence.
    starved: bool,
    cur: Vec<f32>,
    pos: usize,
    ended: bool,
    /// Last frame written to the output, before volume.
    out_l: f32,
    out_r: f32,
    /// Crossfade source: the rest of the old audio after a seek (may be empty). Once it runs
    /// out its last frame is held.
    fade: Vec<f32>,
    fade_pos: usize,
    hold_l: f32,
    hold_r: f32,
    ramp_remaining: usize,
    /// Fade to silence after an underrun or at the end.
    tail_l: f32,
    tail_r: f32,
    tail_remaining: usize,
}

impl StreamState {
    pub(super) fn new(
        sound_id: NonZeroU32,
        play_id: NonZeroU32,
        source: StreamSource,
        looped: bool,
        volume: f32,
        errors: Arc<Mutex<StreamErrors>>,
    ) -> Self {
        let (rx, shared, recycle) = spawn_worker(sound_id, play_id, source, looped, errors);
        StreamState {
            sound_id,
            play_id,
            volume,
            paused: false,
            rx,
            shared,
            recycle,
            generation: 0,
            seek_pending: false,
            starved: false,
            cur: Vec::new(),
            pos: 0,
            ended: false,
            out_l: 0.0,
            out_r: 0.0,
            fade: Vec::new(),
            fade_pos: 0,
            hold_l: 0.0,
            hold_r: 0.0,
            ramp_remaining: 0,
            tail_l: 0.0,
            tail_r: 0.0,
            tail_remaining: 0,
        }
    }

    /// `sample_offset` counts interleaved stereo samples at 44.1 kHz (as for buffered sounds).
    /// Real-time safe: no allocation, no locking.
    pub fn seek(&mut self, sample_offset: usize) {
        self.generation = self.generation.wrapping_add(1);
        let frame = (sample_offset / 2).min(u32::MAX as usize) as u64;
        self.shared
            .seek
            .store(((self.generation as u64) << 32) | frame, Ordering::Release);
        self.seek_pending = true;
        self.ended = false;
    }

    /// Hands a buffer back to the worker. The channel has room for every buffer that can
    /// exist (see `RECYCLE_SLOTS`), so this only drops (frees) once the worker has exited.
    fn recycle(&self, buf: Vec<f32>) {
        if buf.capacity() != 0 {
            let _ = self.recycle.try_send(buf);
        }
    }

    /// Drains the queue up to the next chunk of the current generation. Returns `true` if
    /// one was installed as `cur`.
    fn poll(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(Chunk::Data(g, data)) if g == self.generation => {
                    self.install(data);
                    return true;
                }
                Ok(Chunk::Data(_, data)) => self.recycle(data),
                Ok(Chunk::End(g)) if g == self.generation => {
                    self.ended = true;
                    if self.seek_pending {
                        // Seeked to (or past) the end: drop the old audio and fade out.
                        self.seek_pending = false;
                        let old = std::mem::take(&mut self.cur);
                        self.recycle(old);
                        self.pos = 0;
                    }
                    return false;
                }
                Ok(Chunk::End(_)) => {}
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    return false;
                }
            }
        }
    }

    fn install(&mut self, data: Vec<f32>) {
        let old = std::mem::replace(&mut self.cur, data);
        let old_pos = std::mem::replace(&mut self.pos, 0);
        if !(self.seek_pending || self.starved) {
            self.recycle(old);
            return;
        }

        // Discontinuity: crossfade from what was last played into the new data.
        let prev_fade = std::mem::take(&mut self.fade);
        self.recycle(prev_fade);
        self.fade_pos = 0;
        if self.seek_pending && !self.starved && self.ramp_remaining == 0 {
            // Seek during normal playback: blend with the continuing old audio.
            self.fade = old;
            self.fade_pos = old_pos;
        } else {
            // After an underrun, or a seek mid-crossfade: fade from the last output frame.
            self.recycle(old);
        }
        self.hold_l = self.out_l;
        self.hold_r = self.out_r;
        self.ramp_remaining = RAMP_FRAMES;
        self.seek_pending = false;
        self.starved = false;
        self.tail_remaining = 0;
    }

    /// Next frame of the crossfade source.
    fn fade_frame(&mut self) -> (f32, f32) {
        if self.fade_pos + 1 < self.fade.len() {
            self.hold_l = self.fade[self.fade_pos];
            self.hold_r = self.fade[self.fade_pos + 1];
            self.fade_pos += 2;
        }
        (self.hold_l, self.hold_r)
    }

    /// No data available: fade the last output frame to silence over `RAMP_FRAMES`.
    fn fade_out(&mut self, buffer: &mut [f32]) {
        if !self.starved {
            self.starved = true;
            self.tail_l = self.out_l;
            self.tail_r = self.out_r;
            self.tail_remaining = RAMP_FRAMES;
            self.ramp_remaining = 0;
            let fade = std::mem::take(&mut self.fade);
            self.recycle(fade);
            self.fade_pos = 0;
        }
        let volume = self.volume;
        for frame in buffer.chunks_exact_mut(2) {
            if self.tail_remaining == 0 {
                break;
            }
            self.tail_remaining -= 1;
            let gain = self.tail_remaining as f32 / RAMP_FRAMES as f32;
            let (l, r) = (self.tail_l * gain, self.tail_r * gain);
            frame[0] += l * volume;
            frame[1] += r * volume;
            self.out_l = l;
            self.out_r = r;
        }
    }

    /// Adds this stream's output to `buffer` (interleaved stereo). Returns `false` once the
    /// stream has finished and should be removed.
    pub fn mix(&mut self, buffer: &mut [f32]) -> bool {
        // Switch to the seek target as soon as it is decoded, even mid-chunk.
        if self.seek_pending {
            self.poll();
        }

        let mut off = 0;
        while off < buffer.len() {
            if self.pos >= self.cur.len() {
                if !self.ended && self.poll() {
                    continue;
                }
                // Underrun or end of stream.
                self.fade_out(&mut buffer[off..]);
                return !(self.ended && self.tail_remaining == 0);
            }

            let frames = ((buffer.len() - off) / 2).min((self.cur.len() - self.pos) / 2);
            if frames == 0 {
                break;
            }
            let n = frames * 2;
            let volume = self.volume;

            // Crossfade after a seek or underrun (per frame, only for the first RAMP_FRAMES).
            let mut done = 0;
            while done < n && self.ramp_remaining > 0 {
                let factor = (RAMP_FRAMES - self.ramp_remaining) as f32 / RAMP_FRAMES as f32;
                let (fl, fr) = self.fade_frame();
                let i = self.pos + done;
                let l = self.cur[i] * factor + fl * (1.0 - factor);
                let r = self.cur[i + 1] * factor + fr * (1.0 - factor);
                buffer[off + done] += l * volume;
                buffer[off + done + 1] += r * volume;
                self.out_l = l;
                self.out_r = r;
                self.ramp_remaining -= 1;
                done += 2;
            }
            if self.ramp_remaining == 0 && self.fade.capacity() != 0 {
                let fade = std::mem::take(&mut self.fade);
                self.recycle(fade);
                self.fade_pos = 0;
            }

            // Steady state: a plain scaled add that the compiler can vectorize.
            let src = &self.cur[self.pos + done..self.pos + n];
            for (d, s) in buffer[off + done..off + n].iter_mut().zip(src) {
                *d += s * volume;
            }
            if done < n {
                self.out_l = self.cur[self.pos + n - 2];
                self.out_r = self.cur[self.pos + n - 1];
            }

            self.pos += n;
            off += n;
        }
        true
    }
}
