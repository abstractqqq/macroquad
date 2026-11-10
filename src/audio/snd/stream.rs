//! Streaming playback: files are decoded incrementally by a worker thread (one per playing
//! stream) and handed to the audio thread as small 44.1 kHz stereo chunks, so only a few
//! hundred milliseconds of audio are ever resident in memory.
//!
//! The audio thread never touches the disk or a decoder: it only does `try_recv` on the chunk
//! queue. If the worker falls behind (underrun) the voice simply outputs silence until data
//! arrives.

use super::vorbis::OggStream;
use super::wav::WavStream;
use super::{mixer::RAMP_FRAMES, Error, StreamSource};

use core::num::NonZeroU32;
use foldhash::{HashMap, HashMapExt};
use std::collections::VecDeque;
use std::io::Read;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Source frames decoded per chunk.
const CHUNK_FRAMES: usize = 4096;
/// Chunks buffered ahead of the audio thread (about 0.4 s at 44.1 kHz).
const QUEUE_CHUNKS: usize = 4;

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

enum Command {
    /// Seek to `seconds`; chunks produced afterwards carry the new generation.
    Seek(f64, u32),
}

enum Chunk {
    Data(u32, Vec<f32>),
    End(u32),
}

/// Chunk buffers travel worker -> audio thread -> worker so that neither allocation nor
/// deallocation happens on the audio thread in steady state. The return channel is bounded
/// (preallocated, `try_send` never allocates); a full channel just drops the buffer.
const RECYCLE_SLOTS: usize = QUEUE_CHUNKS + 2;

/// Stores runtime errors produced by background stream decoder threads so callers can
/// inspect them without touching the real-time audio thread.
pub(super) struct StreamErrors {
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
        self.by_play.insert(play_id, (sound_id, message));
    }

    pub(super) fn clear_sound(&mut self, sound_id: NonZeroU32) {
        self.by_sound.remove(&sound_id);
        self.by_play.retain(|_, (sid, _)| *sid != sound_id);
    }

    pub(super) fn clear_sound_error(&mut self, sound_id: NonZeroU32) {
        self.by_sound.remove(&sound_id);
    }

    pub(super) fn get_play(&self, play_id: NonZeroU32) -> Option<String> {
        self.by_play.get(&play_id).map(|(_, msg)| msg.clone())
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
) -> (Receiver<Chunk>, Sender<Command>, mpsc::SyncSender<Vec<f32>>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (chunk_tx, chunk_rx) = mpsc::sync_channel(QUEUE_CHUNKS);
    let (recycle_tx, recycle_rx) = mpsc::sync_channel(RECYCLE_SLOTS);

    // If spawning fails the closure (and `chunk_tx`) is dropped, which the audio thread sees
    // as a disconnected queue and treats as the end of the stream.
    let worker_errors = Arc::clone(&errors);
    if let Err(err) = std::thread::Builder::new()
        .name("rayquad-audio-stream".to_string())
        .spawn(move || {
            worker(
                sound_id,
                play_id,
                source,
                looped,
                cmd_rx,
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

    (chunk_rx, cmd_tx, recycle_tx)
}

fn worker(
    sound_id: NonZeroU32,
    play_id: NonZeroU32,
    source: StreamSource,
    looped: bool,
    cmd_rx: Receiver<Command>,
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
        loop {
            match cmd_rx.try_recv() {
                Ok(Command::Seek(seconds, new_generation)) => {
                    generation = new_generation;
                    pending.clear();
                    resampler.reset();
                    decoded_since_seek = false;
                    if let Err(err) = decoder.seek_frame((seconds.max(0.0) * rate as f64) as usize) {
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
                Err(TryRecvError::Empty) => break,
                // The voice was stopped or dropped.
                Err(TryRecvError::Disconnected) => return,
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
    cmd: Sender<Command>,
    recycle: mpsc::SyncSender<Vec<f32>>,
    generation: u32,
    cur: Vec<f32>,
    pos: usize,
    ended: bool,
    last_l: f32,
    last_r: f32,
    ramp_last_l: f32,
    ramp_last_r: f32,
    ramp_remaining: usize,
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
        let (rx, cmd, recycle) = spawn_worker(sound_id, play_id, source, looped, errors);
        StreamState {
            sound_id,
            play_id,
            volume,
            paused: false,
            rx,
            cmd,
            recycle,
            generation: 0,
            cur: Vec::new(),
            pos: 0,
            ended: false,
            last_l: 0.0,
            last_r: 0.0,
            ramp_last_l: 0.0,
            ramp_last_r: 0.0,
            ramp_remaining: 0,
        }
    }

    /// `sample_offset` counts interleaved stereo samples at 44.1 kHz (as for buffered sounds).
    pub fn seek(&mut self, sample_offset: usize) {
        let seconds = (sample_offset / 2) as f64 / 44100.0;
        self.generation = self.generation.wrapping_add(1);
        let _ = self.cmd.send(Command::Seek(seconds, self.generation));
        self.cur.clear();
        self.pos = 0;
        self.ended = false;
        self.ramp_last_l = self.last_l;
        self.ramp_last_r = self.last_r;
        self.ramp_remaining = RAMP_FRAMES;
    }

    /// Adds this stream's output to `buffer` (interleaved stereo). Returns `false` once the
    /// stream has finished and should be removed.
    pub fn mix(&mut self, buffer: &mut [f32]) -> bool {
        let mut off = 0;
        while off < buffer.len() {
            if self.pos >= self.cur.len() {
                if self.ended {
                    return false;
                }
                match self.rx.try_recv() {
                    Ok(Chunk::Data(g, data)) => {
                        if g == self.generation {
                            let old = std::mem::replace(&mut self.cur, data);
                            let _ = self.recycle.try_send(old);
                            self.pos = 0;
                        } else {
                            let _ = self.recycle.try_send(data);
                        }
                    }
                    Ok(Chunk::End(g)) => {
                        if g == self.generation {
                            self.ended = true;
                        }
                    }
                    // Underrun: stay silent until the worker catches up.
                    Err(TryRecvError::Empty) => return true,
                    Err(TryRecvError::Disconnected) => self.ended = true,
                }
                continue;
            }

            let frames = ((buffer.len() - off) / 2).min((self.cur.len() - self.pos) / 2);
            if frames == 0 {
                break;
            }
            let n = frames * 2;
            let src = &self.cur[self.pos..self.pos + n];
            let dst = &mut buffer[off..off + n];
            let volume = self.volume;

            // Short crossfade after a seek (per frame, only for the first RAMP_FRAMES).
            let mut done = 0;
            while done < n && self.ramp_remaining > 0 {
                let factor = (RAMP_FRAMES - self.ramp_remaining) as f32 / RAMP_FRAMES as f32;
                dst[done] += (src[done] * factor + self.ramp_last_l * (1.0 - factor)) * volume;
                dst[done + 1] +=
                    (src[done + 1] * factor + self.ramp_last_r * (1.0 - factor)) * volume;
                self.ramp_remaining -= 1;
                done += 2;
            }

            // Steady state: a plain scaled add that the compiler can vectorize.
            for (d, s) in dst[done..].iter_mut().zip(&src[done..]) {
                *d += s * volume;
            }

            self.last_l = src[n - 2];
            self.last_r = src[n - 1];
            self.pos += frames * 2;
            off += frames * 2;
        }
        true
    }
}

