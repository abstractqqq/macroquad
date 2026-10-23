//! Minimal RIFF/WAVE decoder: PCM 8/16/24/32-bit and IEEE float 32-bit, mono or stereo.
//!
//! Works directly on the in-memory file, so converting to `f32` is a tight loop over the
//! data chunk that the compiler can vectorize.

use super::{
    mixer::{check_channels, Decoded},
    Error,
};

const FORMAT_PCM: u16 = 0x0001;
const FORMAT_FLOAT: u16 = 0x0003;
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

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
    check_channels(fmt.channels)?;
    if fmt.sample_rate == 0 {
        return Err(Error::Decode("sample rate is zero".to_string()));
    }

    let bytes_per_sample = (fmt.bits / 8) as usize;
    let supported = match (fmt.tag, fmt.bits) {
        (FORMAT_PCM, 8 | 16 | 24 | 32) | (FORMAT_FLOAT, 32) => true,
        _ => false,
    };
    if !supported {
        return Err(Error::UnsupportedSampleFormat(format!(
            "WAV format tag {:#x}, {}-bit",
            fmt.tag, fmt.bits
        )));
    }
    if fmt.block_align as usize != bytes_per_sample * fmt.channels as usize {
        return Err(Error::Decode("WAV block alignment does not match format".to_string()));
    }

    // Ignore a trailing partial frame.
    let data = &data[..data.len() / fmt.block_align as usize * fmt.block_align as usize];

    let samples: Vec<f32> = match (fmt.tag, fmt.bits) {
        // 8-bit WAV is unsigned.
        (FORMAT_PCM, 8) => data
            .iter()
            .map(|&b| (b as f32 - 128.0) * (1.0 / 128.0))
            .collect(),
        (FORMAT_PCM, 16) => data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 * (1.0 / 32768.0))
            .collect(),
        (FORMAT_PCM, 24) => data
            .chunks_exact(3)
            .map(|c| {
                // Sign extend through the top byte.
                let v = (c[0] as i32) | (c[1] as i32) << 8 | (c[2] as i8 as i32) << 16;
                v as f32 * (1.0 / 8388608.0)
            })
            .collect(),
        (FORMAT_PCM, 32) => data
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 * (1.0 / 2147483648.0))
            .collect(),
        (FORMAT_FLOAT, 32) => data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        _ => unreachable!(),
    };

    Ok(Decoded {
        channels: fmt.channels,
        sample_rate: fmt.sample_rate,
        samples,
    })
}
