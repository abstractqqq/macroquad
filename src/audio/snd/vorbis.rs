//! Ogg Vorbis decoder backed by stb_vorbis.

use super::{
    mixer::{check_channels, Decoded},
    Error,
};
use std::ffi::{c_int, c_uint, c_void};

#[repr(C)]
struct StbVorbis;

#[repr(C)]
struct StbVorbisInfo {
    sample_rate: c_uint,
    channels: c_int,
    setup_memory_required: c_uint,
    setup_temp_memory_required: c_uint,
    temp_memory_required: c_uint,
    max_frame_size: c_int,
}

extern "C" {
    fn stb_vorbis_open_memory(
        data: *const u8,
        len: c_int,
        error: *mut c_int,
        alloc_buffer: *const c_void,
    ) -> *mut StbVorbis;

    fn stb_vorbis_get_info(f: *mut StbVorbis) -> StbVorbisInfo;
    fn stb_vorbis_stream_length_in_samples(f: *mut StbVorbis) -> c_uint;
    fn stb_vorbis_get_samples_float_interleaved(
        f: *mut StbVorbis,
        channels: c_int,
        buffer: *mut f32,
        num_floats: c_int,
    ) -> c_int;
    fn stb_vorbis_close(f: *mut StbVorbis);
}

pub(super) fn decode_ogg(bytes: &[u8]) -> Result<Decoded, Error> {
    if bytes.len() > c_int::MAX as usize {
        return Err(Error::Decode("file too large".to_string()));
    }

    let mut err: c_int = 0;
    let vorbis = unsafe {
        stb_vorbis_open_memory(
            bytes.as_ptr(),
            bytes.len() as c_int,
            &mut err,
            std::ptr::null(),
        )
    };

    if vorbis.is_null() {
        return Err(Error::Decode(format!("stb_vorbis open error: {err}")));
    }

    struct VorbisGuard(*mut StbVorbis);
    impl Drop for VorbisGuard {
        fn drop(&mut self) {
            unsafe { stb_vorbis_close(self.0) };
        }
    }
    let _guard = VorbisGuard(vorbis);

    let info = unsafe { stb_vorbis_get_info(vorbis) };
    let channels = info.channels as u16;
    check_channels(channels)?;

    let total_samples = unsafe { stb_vorbis_stream_length_in_samples(vorbis) } as usize;
    let total_floats = total_samples.saturating_mul(channels as usize);
    let mut samples = vec![0.0f32; total_floats];

    let samples_read = unsafe {
        stb_vorbis_get_samples_float_interleaved(
            vorbis,
            channels as c_int,
            samples.as_mut_ptr(),
            total_floats as c_int,
        )
    } as usize;

    samples.truncate(samples_read * channels as usize);

    Ok(Decoded {
        channels,
        sample_rate: info.sample_rate,
        samples,
    })
}
