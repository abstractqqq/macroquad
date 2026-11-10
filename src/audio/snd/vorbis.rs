//! Ogg Vorbis decoder backed by stb_vorbis.

use super::{
    mixer::{check_channels, Decoded},
    Error,
};
use std::ffi::{c_char, c_int, c_uint, c_void, CString};
use std::path::Path;
use std::sync::Arc;

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

const VORBIS_FILE_OPEN_FAILURE: c_int = 6;

extern "C" {
    fn stb_vorbis_open_memory(
        data: *const u8,
        len: c_int,
        error: *mut c_int,
        alloc_buffer: *const c_void,
    ) -> *mut StbVorbis;

    fn stb_vorbis_open_filename(
        filename: *const c_char,
        error: *mut c_int,
        alloc_buffer: *const c_void,
    ) -> *mut StbVorbis;

    fn stb_vorbis_get_info(f: *mut StbVorbis) -> StbVorbisInfo;
    fn stb_vorbis_get_error(f: *mut StbVorbis) -> c_int;
    fn stb_vorbis_stream_length_in_samples(f: *mut StbVorbis) -> c_uint;
    fn stb_vorbis_get_samples_float_interleaved(
        f: *mut StbVorbis,
        channels: c_int,
        buffer: *mut f32,
        num_floats: c_int,
    ) -> c_int;
    fn stb_vorbis_seek(f: *mut StbVorbis, sample_number: c_uint) -> c_int;
    fn stb_vorbis_seek_start(f: *mut StbVorbis) -> c_int;
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

pub(super) struct OggStream {
    vorbis: *mut StbVorbis,
    /// Keeps the backing slice alive when `vorbis` was opened via `stb_vorbis_open_memory`.
    _backing: Option<Arc<[u8]>>,
    pub(super) channels: u16,
    pub(super) sample_rate: u32,
    pub(super) total_frames: usize,
}

// Safety: Each `StbVorbis` handle is exclusively owned by a single `OggStream`
// and never accessed concurrently across threads.
unsafe impl Send for OggStream {}

impl Drop for OggStream {
    fn drop(&mut self) {
        if !self.vorbis.is_null() {
            unsafe { stb_vorbis_close(self.vorbis) };
        }
    }
}

impl OggStream {
    pub(super) fn open_file(path: &Path) -> Result<Self, Error> {
        let s = path.to_str().ok_or_else(|| {
            Error::IOError(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "OGG stream path is not valid UTF-8",
            ))
        })?;
        let c_path = CString::new(s).map_err(|_| {
            Error::IOError(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "OGG stream path contains NUL byte",
            ))
        })?;

        let mut err: c_int = 0;
        let vorbis =
            unsafe { stb_vorbis_open_filename(c_path.as_ptr(), &mut err, std::ptr::null()) };
        if vorbis.is_null() {
            if err == VORBIS_FILE_OPEN_FAILURE {
                return Err(Error::IOError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("failed to open OGG file: {}", path.display()),
                )));
            }
            return Err(Error::Decode(format!("stb_vorbis open error: {err}")));
        }

        Self::from_raw(vorbis, None)
    }

    pub(super) fn open_memory(bytes: Arc<[u8]>) -> Result<Self, Error> {
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

        Self::from_raw(vorbis, Some(bytes))
    }

    fn from_raw(vorbis: *mut StbVorbis, backing: Option<Arc<[u8]>>) -> Result<Self, Error> {
        let info = unsafe { stb_vorbis_get_info(vorbis) };
        let channels = info.channels as u16;
        let sample_rate = info.sample_rate;
        let total_frames = unsafe { stb_vorbis_stream_length_in_samples(vorbis) } as usize;
        let stream_err = unsafe { stb_vorbis_get_error(vorbis) };

        let stream = OggStream {
            vorbis,
            _backing: backing,
            channels,
            sample_rate,
            total_frames,
        };

        check_channels(channels)?;
        if sample_rate == 0 {
            return Err(Error::Decode("sample rate is zero".to_string()));
        }
        if total_frames == 0 && stream_err != 0 {
            return Err(Error::Decode(format!(
                "stb_vorbis stream length error: {stream_err}"
            )));
        }

        Ok(stream)
    }

    pub(super) fn decode_frames(
        &mut self,
        max_frames: usize,
        out: &mut Vec<f32>,
    ) -> Result<usize, Error> {
        if max_frames == 0 {
            return Ok(0);
        }
        let ch = self.channels as usize;
        let num_floats = max_frames * ch;
        let start_len = out.len();
        out.resize(start_len + num_floats, 0.0);

        let frames_read = unsafe {
            stb_vorbis_get_samples_float_interleaved(
                self.vorbis,
                self.channels as c_int,
                out[start_len..].as_mut_ptr(),
                num_floats as c_int,
            )
        };

        if frames_read <= 0 {
            out.truncate(start_len);
            let err = unsafe { stb_vorbis_get_error(self.vorbis) };
            if err != 0 {
                return Err(Error::Decode(format!("stb_vorbis decode error: {err}")));
            }
            return Ok(0);
        }

        let frames_read = frames_read as usize;
        out.truncate(start_len + frames_read * ch);
        Ok(frames_read)
    }

    pub(super) fn seek_frame(&mut self, frame: usize) -> Result<(), Error> {
        let ok = unsafe {
            if frame == 0 {
                stb_vorbis_seek_start(self.vorbis)
            } else {
                let target = frame.min(self.total_frames) as c_uint;
                stb_vorbis_seek(self.vorbis, target)
            }
        };
        if ok == 0 {
            let err = unsafe { stb_vorbis_get_error(self.vorbis) };
            return Err(Error::Decode(format!("stb_vorbis seek error: {err}")));
        }
        Ok(())
    }
}
