//! Minimal RIFF/WAVE decoder: PCM 8/16/24/32-bit and IEEE float 32-bit, mono or stereo.
//!
//! Works directly on the in-memory file, so converting to `f32` is a tight loop over the
//! data chunk that the compiler can vectorize.

use super::{
    mixer::{check_channels, Decoded},
    Error, StreamSource,
};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::sync::Arc;

const FORMAT_PCM: u16 = 0x0001;
const FORMAT_FLOAT: u16 = 0x0003;
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

#[derive(Clone, Copy, Debug)]
struct Format {
    tag: u16,
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    bits: u16,
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn parse_fmt(chunk: &[u8]) -> Result<Format, Error> {
    if chunk.len() < 16 {
        return Err(Error::Decode("WAV fmt chunk too short".to_string()));
    }
    let mut tag = u16_at(chunk, 0);
    if tag == FORMAT_EXTENSIBLE {
        // cbSize(2) validBits(2) channelMask(4) subFormat GUID(16), the first 2 bytes of the
        // GUID are the real format tag.
        if chunk.len() < 26 {
            return Err(Error::Decode("WAV extensible fmt chunk too short".to_string()));
        }
        tag = u16_at(chunk, 24);
    }
    Ok(Format {
        tag,
        channels: u16_at(chunk, 2),
        sample_rate: u32_at(chunk, 4),
        block_align: u16_at(chunk, 12),
        bits: u16_at(chunk, 14),
    })
}

fn validate_fmt(fmt: &Format) -> Result<(), Error> {
    check_channels(fmt.channels)?;
    if fmt.sample_rate == 0 {
        return Err(Error::Decode("sample rate is zero".to_string()));
    }

    let bytes_per_sample = (fmt.bits / 8) as usize;
    let supported = matches!(
        (fmt.tag, fmt.bits),
        (FORMAT_PCM, 8 | 16 | 24 | 32) | (FORMAT_FLOAT, 32)
    );
    if !supported {
        return Err(Error::UnsupportedSampleFormat(format!(
            "WAV format tag {:#x}, {}-bit",
            fmt.tag, fmt.bits
        )));
    }
    if fmt.block_align as usize != bytes_per_sample * fmt.channels as usize {
        return Err(Error::Decode("WAV block alignment does not match format".to_string()));
    }
    Ok(())
}

fn pcm_bytes_to_f32(data: &[u8], fmt: &Format, out: &mut Vec<f32>) {
    match (fmt.tag, fmt.bits) {
        // 8-bit WAV is unsigned.
        (FORMAT_PCM, 8) => out.extend(
            data.iter()
                .map(|&b| (b as f32 - 128.0) * (1.0 / 128.0)),
        ),
        (FORMAT_PCM, 16) => out.extend(
            data.chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 * (1.0 / 32768.0)),
        ),
        (FORMAT_PCM, 24) => out.extend(data.chunks_exact(3).map(|c| {
            // Sign extend through the top byte.
            let v = (c[0] as i32) | (c[1] as i32) << 8 | (c[2] as i8 as i32) << 16;
            v as f32 * (1.0 / 8388608.0)
        })),
        (FORMAT_PCM, 32) => out.extend(
            data.chunks_exact(4)
                .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 * (1.0 / 2147483648.0)),
        ),
        (FORMAT_FLOAT, 32) => out.extend(
            data.chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])),
        ),
        _ => unreachable!(),
    }
}

pub(super) fn decode_wav(bytes: &[u8]) -> Result<Decoded, Error> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(Error::Decode("not a RIFF/WAVE file".to_string()));
    }

    // Walk the chunks: skip anything that is not `fmt ` / `data` (LIST, fact, ...).
    let mut pos = 12;
    let mut format: Option<Format> = None;
    let data = loop {
        if pos + 8 > bytes.len() {
            return Err(Error::Decode("WAV file has no data chunk".to_string()));
        }
        let id = &bytes[pos..pos + 4];
        let size = u32_at(bytes, pos + 4) as usize;
        let body = pos + 8;
        let remaining = bytes.len() - body;

        if id == b"data" {
            // Streamed files may leave the size as 0 or 0xFFFFFFFF, use everything that follows.
            let len = if size == 0 || size == u32::MAX as usize {
                remaining
            } else if size > remaining {
                return Err(Error::Decode("WAV data chunk is truncated".to_string()));
            } else {
                size
            };
            break &bytes[body..body + len];
        }

        if id == b"fmt " {
            if size > remaining {
                return Err(Error::Decode("WAV fmt chunk is truncated".to_string()));
            }
            format = Some(parse_fmt(&bytes[body..body + size])?);
        }

        // Chunks are padded to an even number of bytes.
        pos = body.saturating_add(size).saturating_add(size & 1);
    };

    let fmt = format.ok_or_else(|| Error::Decode("WAV data chunk before fmt chunk".to_string()))?;
    validate_fmt(&fmt)?;

    // Ignore a trailing partial frame.
    let num_frames = data.len() / fmt.block_align as usize;
    let data = &data[..num_frames * fmt.block_align as usize];

    let mut samples = Vec::with_capacity(num_frames * fmt.channels as usize);
    pcm_bytes_to_f32(data, &fmt, &mut samples);

    Ok(Decoded {
        channels: fmt.channels,
        sample_rate: fmt.sample_rate,
        samples,
    })
}

enum WavReader {
    File(BufReader<std::fs::File>),
    Bytes { data: Arc<[u8]>, pos: usize },
}

impl WavReader {
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), Error> {
        match self {
            WavReader::File(r) => r
                .read_exact(buf)
                .map_err(|e| Error::Decode(format!("WAV read error: {e}"))),
            WavReader::Bytes { data, pos } => {
                if *pos + buf.len() > data.len() {
                    return Err(Error::Decode("WAV unexpected EOF".to_string()));
                }
                buf.copy_from_slice(&data[*pos..*pos + buf.len()]);
                *pos += buf.len();
                Ok(())
            }
        }
    }

    fn seek_to(&mut self, target: u64) -> Result<(), Error> {
        match self {
            WavReader::File(r) => {
                r.seek(SeekFrom::Start(target))
                    .map_err(|e| Error::Decode(format!("WAV seek error: {e}")))?;
                Ok(())
            }
            WavReader::Bytes { data, pos } => {
                if target > data.len() as u64 {
                    return Err(Error::Decode("WAV seek past EOF".to_string()));
                }
                *pos = target as usize;
                Ok(())
            }
        }
    }
}

pub(super) struct WavStream {
    reader: WavReader,
    fmt: Format,
    data_offset: u64,
    pub(super) total_frames: usize,
    current_frame: usize,
    raw_buf: Vec<u8>,
}

impl WavStream {
    pub(super) fn open(source: &StreamSource) -> Result<Self, Error> {
        let (reader, total_len) = match source {
            StreamSource::File(path) => {
                let file = std::fs::File::open(path)?;
                let len = file.metadata()?.len();
                (WavReader::File(BufReader::with_capacity(16 * 1024, file)), len)
            }
            StreamSource::Bytes(data) => {
                let len = data.len() as u64;
                (
                    WavReader::Bytes {
                        data: Arc::clone(data),
                        pos: 0,
                    },
                    len,
                )
            }
        };
        Self::from_reader(reader, total_len)
    }

    fn from_reader(mut reader: WavReader, total_len: u64) -> Result<Self, Error> {
        if total_len < 12 {
            return Err(Error::Decode("not a RIFF/WAVE file".to_string()));
        }
        let mut riff_hdr = [0u8; 12];
        reader.read_exact(&mut riff_hdr)?;
        if &riff_hdr[0..4] != b"RIFF" || &riff_hdr[8..12] != b"WAVE" {
            return Err(Error::Decode("not a RIFF/WAVE file".to_string()));
        }

        let mut pos: u64 = 12;
        let mut format: Option<Format> = None;
        let (data_offset, data_len) = loop {
            if pos + 8 > total_len {
                return Err(Error::Decode("WAV file has no data chunk".to_string()));
            }
            reader.seek_to(pos)?;
            let mut chunk_hdr = [0u8; 8];
            reader.read_exact(&mut chunk_hdr)?;
            let id = &chunk_hdr[0..4];
            let size = u32_at(&chunk_hdr, 4) as u64;
            let body = pos + 8;
            let remaining = total_len - body;

            if id == b"data" {
                let len = if size == 0 || size == u32::MAX as u64 {
                    remaining
                } else if size > remaining {
                    return Err(Error::Decode("WAV data chunk is truncated".to_string()));
                } else {
                    size
                };
                break (body, len);
            }

            if id == b"fmt " {
                if size > remaining {
                    return Err(Error::Decode("WAV fmt chunk is truncated".to_string()));
                }
                let mut fmt_buf = vec![0u8; size as usize];
                reader.read_exact(&mut fmt_buf)?;
                format = Some(parse_fmt(&fmt_buf)?);
            }

            pos = body.saturating_add(size).saturating_add(size & 1);
        };

        let fmt = format.ok_or_else(|| Error::Decode("WAV data chunk before fmt chunk".to_string()))?;
        validate_fmt(&fmt)?;

        let total_frames = (data_len / fmt.block_align as u64) as usize;

        Ok(WavStream {
            reader,
            fmt,
            data_offset,
            total_frames,
            current_frame: 0,
            raw_buf: Vec::new(),
        })
    }

    pub(super) fn channels(&self) -> u16 {
        self.fmt.channels
    }

    pub(super) fn sample_rate(&self) -> u32 {
        self.fmt.sample_rate
    }

    pub(super) fn decode_frames(
        &mut self,
        max_frames: usize,
        out: &mut Vec<f32>,
    ) -> Result<usize, Error> {
        let avail = self.total_frames.saturating_sub(self.current_frame);
        let frames = max_frames.min(avail);
        if frames == 0 {
            return Ok(0);
        }

        let byte_count = frames * self.fmt.block_align as usize;
        self.raw_buf.resize(byte_count, 0);
        if let Err(err) = self.reader.read_exact(&mut self.raw_buf) {
            self.current_frame = self.total_frames;
            return Err(err);
        }
        self.current_frame += frames;

        out.reserve(frames * self.fmt.channels as usize);
        pcm_bytes_to_f32(&self.raw_buf, &self.fmt, out);
        Ok(frames)
    }

    pub(super) fn seek_frame(&mut self, frame: usize) -> Result<(), Error> {
        let target = frame.min(self.total_frames);
        let byte_offset = self.data_offset + (target as u64) * (self.fmt.block_align as u64);
        self.reader.seek_to(byte_offset)?;
        self.current_frame = target;
        Ok(())
    }
}
