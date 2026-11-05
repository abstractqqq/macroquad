// roughly based on http://equalarea.com/paul/alsa-audio.html

use super::{error::Error, PlaySoundParams};

use core::num::NonZeroU32;

#[path = "alsa/alsa_sys.rs"]
mod sys;

use std::sync::mpsc;

pub use super::mixer::Playback;

mod consts {
    pub const DEVICES: &[&str] = &["default\0", "pipewire\0"];
    pub const RATE: u32 = 44100;
    pub const CHANNELS: u32 = 2;
    pub const PCM_BUFFER_SIZE: ::std::os::raw::c_ulong = 4096;
}

/// Opens and configures the PCM device. Returns an error (never panics) so the
/// audio thread can retry after suspend/resume or device hot-plug.
unsafe fn setup_pcm_device() -> Result<*mut sys::snd_pcm_t, &'static str> {
    let mut pcm_handle = std::ptr::null_mut();

    // Open the PCM device in playback mode
    if !consts::DEVICES.iter().any(|device| {
        sys::snd_pcm_open(
            &mut pcm_handle,
            device.as_ptr() as _,
            sys::SND_PCM_STREAM_PLAYBACK,
            0,
        ) >= 0
    }) {
        return Err("Can't open PCM device.");
    }

    let result = configure_pcm_device(pcm_handle);
    if let Err(e) = result {
        sys::snd_pcm_close(pcm_handle);
        return Err(e);
    }
    Ok(pcm_handle)
}

unsafe fn configure_pcm_device(pcm_handle: *mut sys::snd_pcm_t) -> Result<(), &'static str> {
    let mut hw_params: *mut sys::snd_pcm_hw_params_t = std::ptr::null_mut();
    sys::snd_pcm_hw_params_malloc(&mut hw_params);
    sys::snd_pcm_hw_params_any(pcm_handle, hw_params);

    if sys::snd_pcm_hw_params_set_access(pcm_handle, hw_params, sys::SND_PCM_ACCESS_RW_INTERLEAVED)
        < 0
    {
        return Err("Can't set interleaved mode");
    }

    if sys::snd_pcm_hw_params_set_format(pcm_handle, hw_params, sys::SND_PCM_FORMAT_FLOAT_LE) < 0 {
        return Err("Can't set SND_PCM_FORMAT_FLOAT_LE format");
    }
    if sys::snd_pcm_hw_params_set_buffer_size(pcm_handle, hw_params, consts::PCM_BUFFER_SIZE) < 0 {
        return Err("Cant's set buffer size");
    }
    if sys::snd_pcm_hw_params_set_channels(pcm_handle, hw_params, consts::CHANNELS) < 0 {
        return Err("Can't set channels number.");
    }

    let mut rate = consts::RATE;
    if sys::snd_pcm_hw_params_set_rate_near(pcm_handle, hw_params, &mut rate, std::ptr::null_mut())
        < 0
    {
        return Err("Can't set rate.");
    }

    // Write parameters
    if sys::snd_pcm_hw_params(pcm_handle, hw_params) < 0 {
        return Err("Can't set harware parameters.");
    }
    sys::snd_pcm_hw_params_free(hw_params);

    // tell ALSA to wake us up whenever AudioContext::PCM_BUFFER_SIZE or more frames
    //   of playback data can be delivered. Also, tell
    //   ALSA that we'll start the device ourselves.
    let mut sw_params: *mut sys::snd_pcm_sw_params_t = std::ptr::null_mut();

    if sys::snd_pcm_sw_params_malloc(&mut sw_params) < 0 {
        return Err("cannot allocate software parameters structure");
    }
    if sys::snd_pcm_sw_params_current(pcm_handle, sw_params) < 0 {
        return Err("cannot initialize software parameters structure");
    }

    // if sys::snd_pcm_sw_params_set_avail_min(
    //     pcm_handle,
    //     sw_params,
    //     AudioContext::PCM_BUFFER_SIZE,
    // ) < 0
    // {
    //     return Err("cannot set minimum available count");
    // }
    if sys::snd_pcm_sw_params_set_start_threshold(pcm_handle, sw_params, 0) < 0 {
        return Err("cannot set start mode");
    }
    if sys::snd_pcm_sw_params(pcm_handle, sw_params) < 0 {
        return Err("cannot set software parameters");
    }
    sys::snd_pcm_sw_params_free(sw_params);

    if sys::snd_pcm_prepare(pcm_handle) < 0 {
        return Err("cannot prepare audio interface for use");
    }

    Ok(())
}

/// Keep trying to open the device until it works (e.g. while the system is
/// still resuming from suspend or the default device is being re-created).
unsafe fn open_pcm_blocking() -> *mut sys::snd_pcm_t {
    loop {
        match setup_pcm_device() {
            Ok(handle) => return handle,
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }
}

/// Try to bring a failed PCM back (underrun, suspend/resume). If ALSA can't
/// recover it, the device is closed and reopened from scratch.
unsafe fn recover_or_reopen(
    pcm_handle: *mut sys::snd_pcm_t,
    err: ::std::os::raw::c_int,
) -> *mut sys::snd_pcm_t {
    if sys::snd_pcm_recover(pcm_handle, err, 1) >= 0 {
        return pcm_handle;
    }
    sys::snd_pcm_close(pcm_handle);
    open_pcm_blocking()
}

unsafe fn audio_thread(mut mixer: super::mixer::Mixer) {
    let mut buffer: Vec<f32> = vec![0.0; consts::PCM_BUFFER_SIZE as usize * 2];

    let mut pcm_handle = open_pcm_blocking();

    loop {
        // Wait for PCM to be ready for next write. A timeout keeps the thread
        // from hanging forever if the device disappears.
        let ready = sys::snd_pcm_wait(pcm_handle, 1000);
        if ready < 0 {
            // e.g. -ESTRPIPE after system suspend, -EPIPE after underrun
            pcm_handle = recover_or_reopen(pcm_handle, ready);
            continue;
        }
        if ready == 0 {
            // timeout
            continue;
        }

        let frames_to_deliver = consts::PCM_BUFFER_SIZE as i64;

        // ask mixer to fill the buffer
        mixer.fill_audio_buffer(&mut buffer, frames_to_deliver as usize);

        // send filled buffer back to alsa
        // No logging here: I/O on the audio thread can itself cause underruns.
        let frames_written = sys::snd_pcm_writei(
            pcm_handle,
            buffer.as_ptr() as *const _,
            frames_to_deliver as _,
        );
        if frames_written < 0 {
            pcm_handle = recover_or_reopen(pcm_handle, frames_written as _);
        }
        // A short write is not an error; the remainder of this buffer is dropped.
    }
}

pub struct AudioContext {
    pub(crate) mixer_ctrl: super::mixer::MixerControl,
}

impl AudioContext {
    pub fn new() -> AudioContext {
        use super::mixer::Mixer;

        let (mixer_builder, mixer_ctrl) = Mixer::new();
        std::thread::spawn(move || unsafe {
            audio_thread(mixer_builder.build());
        });

        AudioContext { mixer_ctrl }
    }
}

pub struct Sound {
    pub sound_id: NonZeroU32,
    pub duration: f32,
}

impl Sound {
    pub fn load(ctx: &AudioContext, data: &[u8]) -> Result<Sound, super::Error> {
        let (sound_id, duration) = ctx.mixer_ctrl.load(data)?;

        Ok(Sound { sound_id, duration })
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    pub fn play(&self, ctx: &AudioContext, params: PlaySoundParams) -> Playback {
        ctx.mixer_ctrl.play(self.sound_id, params)
    }

    pub fn stop(&self, ctx: &AudioContext) {
        ctx.mixer_ctrl.stop_all(self.sound_id);
    }

    pub fn pause(&self, ctx: &AudioContext) {
        ctx.mixer_ctrl.pause_all(self.sound_id);
    }

    pub fn resume(&self, ctx: &AudioContext) {
        ctx.mixer_ctrl.resume_all(self.sound_id);
    }

    pub fn seek(&self, ctx: &AudioContext, position: f32) {
        let sample_offset = (position.max(0.0) * 44100.0) as usize * 2;
        ctx.mixer_ctrl.seek_all(self.sound_id, sample_offset);
    }

    pub fn set_volume(&self, ctx: &AudioContext, volume: f32) {
        ctx.mixer_ctrl.set_volume_all(self.sound_id, volume);
    }

    pub fn delete(&self, ctx: &AudioContext) {
        ctx.mixer_ctrl.delete(self.sound_id);
    }
}
