//! Minimal QOI ("Quite OK Image") decoder. Spec: https://qoiformat.org/qoi-specification.pdf
//!
//! Always decodes to RGBA8. 3-channel files get an opaque alpha channel. The colorspace byte is
//! informational only and is ignored.

use crate::Error;

pub(crate) const QOI_MAGIC: &[u8; 4] = b"qoif";
const HEADER_LEN: usize = 14;

const OP_INDEX: u8 = 0x00;
const OP_DIFF: u8 = 0x40;
const OP_LUMA: u8 = 0x80;
const OP_RUN: u8 = 0xc0;
const OP_RGB: u8 = 0xfe;
const OP_RGBA: u8 = 0xff;
const MASK_2: u8 = 0xc0;

#[inline(always)]
fn hash(px: [u8; 4]) -> usize {
    (px[0] as usize * 3 + px[1] as usize * 5 + px[2] as usize * 7 + px[3] as usize * 11) & 63
}

/// Decodes QOI bytes into `(width, height, rgba8)`.
pub(crate) fn decode_qoi_rgba8(bytes: &[u8]) -> Result<(u16, u16, Vec<u8>), Error> {
    if bytes.len() < HEADER_LEN || &bytes[0..4] != QOI_MAGIC {
        return Err(Error::QoiError("missing QOI header"));
    }
    let width = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let height = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let channels = bytes[12];
    if channels != 3 && channels != 4 {
        return Err(Error::QoiError("QOI channel count must be 3 or 4"));
    }
    if width == 0 || height == 0 {
        return Err(Error::QoiError("QOI image has zero width or height"));
    }
    let width = u16::try_from(width)
        .map_err(|_| Error::QoiError("QOI image width exceeds 65535 pixels"))?;
    let height = u16::try_from(height)
        .map_err(|_| Error::QoiError("QOI image height exceeds 65535 pixels"))?;

    let pixel_count = width as usize * height as usize;
    // Every op encodes at least one pixel in at least one byte, except runs (up to 62 pixels per
    // byte). Reject headers that claim far more pixels than the payload could possibly encode, so
    // a tiny file cannot trigger a huge allocation.
    let payload = &bytes[HEADER_LEN..];
    if pixel_count > payload.len().saturating_mul(62) {
        return Err(Error::QoiError("QOI data is too short for its dimensions"));
    }

    let mut out = vec![0u8; pixel_count * 4];
    let mut index = [[0u8; 4]; 64];
    let mut px = [0u8, 0, 0, 255];
    let mut pos = 0usize;
    let mut written = 0usize;
    const EARLY_END: Error = Error::QoiError("QOI data ended early");

    while written < out.len() {
        // The longest op is 5 bytes. Files end with an 8-byte marker, so this fast path covers
        // every op but the very last few; the slow path below re-checks lengths per op.
        let op: &[u8] = match payload.get(pos..pos + 5) {
            Some(op) => op,
            None => payload.get(pos..).filter(|rest| !rest.is_empty()).ok_or(EARLY_END)?,
        };
        let b1 = op[0];

        if b1 == OP_RGB {
            let s = op.get(1..4).ok_or(EARLY_END)?;
            px[0] = s[0];
            px[1] = s[1];
            px[2] = s[2];
            pos += 4;
        } else if b1 == OP_RGBA {
            px.copy_from_slice(op.get(1..5).ok_or(EARLY_END)?);
            pos += 5;
        } else {
            match b1 & MASK_2 {
                OP_INDEX => {
                    px = index[b1 as usize];
                    pos += 1;
                }
                OP_DIFF => {
                    px[0] = px[0].wrapping_add(((b1 >> 4) & 0x03).wrapping_sub(2));
                    px[1] = px[1].wrapping_add(((b1 >> 2) & 0x03).wrapping_sub(2));
                    px[2] = px[2].wrapping_add((b1 & 0x03).wrapping_sub(2));
                    pos += 1;
                }
                OP_LUMA => {
                    let b2 = *op.get(1).ok_or(EARLY_END)?;
                    let vg = (b1 & 0x3f).wrapping_sub(32);
                    px[0] = px[0].wrapping_add(vg.wrapping_sub(8).wrapping_add(b2 >> 4));
                    px[1] = px[1].wrapping_add(vg);
                    px[2] = px[2].wrapping_add(vg.wrapping_sub(8).wrapping_add(b2 & 0x0f));
                    pos += 2;
                }
                _ => {
                    // OP_RUN: 0xfe and 0xff were handled above. The pixel is unchanged, so the
                    // index needs no update.
                    debug_assert_eq!(b1 & MASK_2, OP_RUN);
                    pos += 1;
                    let run_end = (written + ((b1 & 0x3f) as usize + 1) * 4).min(out.len());
                    for dst in out[written..run_end].chunks_exact_mut(4) {
                        dst.copy_from_slice(&px);
                    }
                    written = run_end;
                    continue;
                }
            }
        }
        index[hash(px) & 63] = px;
        out[written..written + 4].copy_from_slice(&px);
        written += 4;
    }

    Ok((width, height, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: u32, h: u32, channels: u8) -> Vec<u8> {
        let mut v = QOI_MAGIC.to_vec();
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.push(channels);
        v.push(0);
        v
    }

    const END: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 1];

    #[test]
    fn decodes_every_op() {
        let mut f = header(6, 1, 4);
        f.extend_from_slice(&[OP_RGBA, 10, 20, 30, 40]); // (10,20,30,40)
        f.push(OP_DIFF | (3 << 4) | (1 << 2) | 2); // +1,-1,+0 -> (11,19,30,40)
        f.extend_from_slice(&[OP_LUMA | (32 + 5), (8 + 1) << 4 | (8 - 2)]); // dg=5 -> (17,24,33,40)
        f.push(OP_RUN | 1); // 2 more copies
        f.push(OP_INDEX | hash([10, 20, 30, 40]) as u8); // back to first pixel
        f.extend_from_slice(&END);

        let (w, h, px) = decode_qoi_rgba8(&f).unwrap();
        assert_eq!((w, h), (6, 1));
        assert_eq!(
            px,
            [
                10, 20, 30, 40, 11, 19, 30, 40, 17, 24, 33, 40, 17, 24, 33, 40, 17, 24, 33, 40, 10,
                20, 30, 40
            ]
        );
    }

    #[test]
    fn rgb_op_keeps_alpha() {
        let mut f = header(1, 1, 3);
        f.extend_from_slice(&[OP_RGB, 1, 2, 3]);
        f.extend_from_slice(&END);
        assert_eq!(decode_qoi_rgba8(&f).unwrap().2, [1, 2, 3, 255]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(decode_qoi_rgba8(b"qoif").is_err());
        assert!(decode_qoi_rgba8(&header(1, 1, 5)).is_err());
        assert!(decode_qoi_rgba8(&header(0, 1, 4)).is_err());
        assert!(decode_qoi_rgba8(&header(70000, 1, 4)).is_err());
        // Truncated mid-op.
        let mut f = header(2, 1, 4);
        f.extend_from_slice(&[OP_RGBA, 1, 2]);
        assert!(decode_qoi_rgba8(&f).is_err());
        // Huge claimed size with a tiny payload must fail before allocating.
        let mut f = header(65535, 65535, 4);
        f.extend_from_slice(&END);
        assert!(decode_qoi_rgba8(&f).is_err());
    }
}
