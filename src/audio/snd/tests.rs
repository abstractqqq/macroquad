use super::mixer::load_samples_from_file;

use super::mixer::to_stereo_44k;

/// Every fixture is the same ~3.5 s clip, so each must load as this many stereo 44.1 kHz samples
/// (the resampled ones may be off by a frame or two).
const EXPECTED_LEN: usize = 155648;

const FIXTURES: &[&str] = &[
    "stereo_44k.wav",
    "stereo_48k.wav",
    "mono_44k.wav",
    "stereo_44k.ogg",
    "stereo_48k.ogg",
    "mono_44k.ogg",
];

/// Golden hashes from the original quad-snd 0.2.8 loader, for the paths that must stay bit-exact
/// (44.1 kHz WAV: no resampling and no lossy codec).
const EXACT: &[(&str, u64)] = &[
    ("stereo_44k.wav", 0xf05d87f8afe55ba1),
    ("mono_44k.wav", 0xf05d87f8afe55ba1),
];

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/src/audio/snd/testdata/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {}", path, e))
}

fn fnv1a(samples: &[f32]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for s in samples {
        for b in s.to_bits().to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

fn rms_diff(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let sum: f32 = a[..n].iter().zip(&b[..n]).map(|(x, y)| (x - y) * (x - y)).sum();
    (sum / n as f32).sqrt()
}

#[test]
fn exact_paths_match_golden() {
    for &(name, hash) in EXACT {
        let samples = load_samples_from_file(&fixture(name)).unwrap();
        assert_eq!(samples.len(), EXPECTED_LEN, "{name}");
        assert_eq!(fnv1a(&samples), hash, "{name}");
    }
}

#[test]
fn all_fixtures_agree_with_the_reference() {
    let reference = load_samples_from_file(&fixture("stereo_44k.wav")).unwrap();
    for name in FIXTURES {
        let samples = load_samples_from_file(&fixture(name)).unwrap();
        let rms = rms_diff(&samples, &reference);
        println!("{name:<16} len={} rms_diff={rms:.5}", samples.len());
        assert!(samples.len().abs_diff(EXPECTED_LEN) <= 4, "{name}: length {}", samples.len());
        // Resampling error stays tiny; lossy Vorbis is allowed more.
        let limit = if name.ends_with(".ogg") { 0.02 } else { 0.005 };
        assert!(rms < limit, "{name}: rms diff {rms} >= {limit}");
    }
}

#[test]
fn resampler_properties() {
    // Constant signal stays constant, mono is duplicated to both channels.
    let out = to_stereo_44k(vec![0.5; 4800], 1, 48000);
    assert_eq!(out.len(), 2 * 4410);
    assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));

    // A linear ramp is reproduced exactly by linear interpolation.
    let ramp: Vec<f32> = (0..4800).map(|i| i as f32 / 4800.0).collect();
    let out = to_stereo_44k(ramp, 1, 48000);
    for (i, frame) in out.chunks_exact(2).enumerate().take(4000) {
        let expected = (i as f32 * 48000.0 / 44100.0) / 4800.0;
        assert!((frame[0] - expected).abs() < 1e-4 && frame[0] == frame[1], "frame {i}");
    }

    // Upsampling works and never reads out of bounds, even for tiny inputs.
    assert_eq!(to_stereo_44k(vec![1.0], 1, 8000).len(), 2 * 5);
    assert!(to_stereo_44k(vec![], 2, 8000).is_empty());
    assert!(to_stereo_44k(vec![1.0], 2, 48000).is_empty()); // less than one stereo frame
}


#[test]
fn rejects_bad_input() {
    use super::Error;

    assert!(matches!(load_samples_from_file(b""), Err(Error::UnsupportedFormat)));
    assert!(matches!(
        load_samples_from_file(b"not an audio file at all"),
        Err(Error::UnsupportedFormat)
    ));

    // Truncated files must error, not panic.
    for name in ["stereo_44k.wav", "stereo_44k.ogg"] {
        let data = fixture(name);
        for cut in [4usize, 12, 30, 100, data.len() / 2] {
            let _ = load_samples_from_file(&data[..cut]);
        }
    }
    // Valid header but truncated in the sample data.
    let wav = fixture("stereo_44k.wav");
    assert!(load_samples_from_file(&wav[..wav.len() / 2]).is_err());
}

/// Run with: cargo test --release --features audio -- --ignored --nocapture bench_loader
#[test]
#[ignore]
fn bench_loader() {
    const ITERS: u32 = 200;
    for name in FIXTURES {
        let data = fixture(name);
        let start = std::time::Instant::now();
        for _ in 0..ITERS {
            std::hint::black_box(load_samples_from_file(std::hint::black_box(&data)).unwrap());
        }
        println!("{:<16} {:>8.1?} per load", name, start.elapsed() / ITERS);
    }
}

// ---- WAV decoder (wav.rs), checked against `hound` as the reference implementation ----

mod wav {
    use super::super::wav::decode_wav;
    use super::super::Error;

    /// Deterministic pseudo random values covering the full range, including the extremes.
    fn noise(n: usize, bits: u32) -> Vec<i32> {
        let (min, max) = (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1);
        let mut x: u64 = 0x2545F4914F6CDD1D;
        let mut v: Vec<i32> = (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (min + (x % (max - min + 1) as u64) as i64) as i32
            })
            .collect();
        v[0] = min as i32;
        v[1] = max as i32;
        v
    }

    fn write_with_hound(spec: hound::WavSpec, ints: &[i32], floats: &[f32]) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        let mut w = hound::WavWriter::new(&mut cursor, spec).unwrap();
        match spec.sample_format {
            hound::SampleFormat::Int => ints.iter().for_each(|&s| w.write_sample(s).unwrap()),
            hound::SampleFormat::Float => floats.iter().for_each(|&s| w.write_sample(s).unwrap()),
        }
        w.finalize().unwrap();
        cursor.into_inner()
    }

    fn hound_reference(bytes: &[u8]) -> Vec<f32> {
        let mut r = hound::WavReader::new(std::io::Cursor::new(bytes)).unwrap();
        let spec = r.spec();
        match spec.sample_format {
            hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap()).collect(),
            hound::SampleFormat::Int => {
                let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
                r.samples::<i32>().map(|s| s.unwrap() as f32 * scale).collect()
            }
        }
    }

    #[test]
    fn matches_hound_for_all_formats() {
        for channels in [1u16, 2] {
            for bits in [8u16, 16, 24, 32] {
                let spec = hound::WavSpec {
                    channels,
                    sample_rate: 22050,
                    bits_per_sample: bits,
                    sample_format: hound::SampleFormat::Int,
                };
                let wav = write_with_hound(spec, &noise(1000 * channels as usize, bits as u32), &[]);
                let ours = decode_wav(&wav).unwrap();
                assert_eq!((ours.channels, ours.sample_rate), (channels, 22050));
                assert_eq!(ours.samples, hound_reference(&wav), "{bits}-bit, {channels} ch");
            }
            let floats: Vec<f32> = (0..1000 * channels as usize)
                .map(|i| ((i as f32) * 0.37).sin())
                .collect();
            let spec = hound::WavSpec {
                channels,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            };
            let wav = write_with_hound(spec, &[], &floats);
            assert_eq!(decode_wav(&wav).unwrap().samples, floats, "float, {channels} ch");
        }
    }

    // ---- hand built files for container quirks ----

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut c = id.to_vec();
        c.extend((body.len() as u32).to_le_bytes());
        c.extend(body);
        if body.len() % 2 == 1 {
            c.push(0); // pad byte, not counted in the size
        }
        c
    }

    fn fmt_body(tag: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
        let block = channels * bits / 8;
        let mut b = Vec::new();
        b.extend(tag.to_le_bytes());
        b.extend(channels.to_le_bytes());
        b.extend(rate.to_le_bytes());
        b.extend((rate * block as u32).to_le_bytes());
        b.extend(block.to_le_bytes());
        b.extend(bits.to_le_bytes());
        b
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut f = b"RIFF".to_vec();
        f.extend((body.len() as u32 + 4).to_le_bytes());
        f.extend(b"WAVE");
        f.extend(body);
        f
    }

    const PCM16_DATA: [i16; 4] = [0, 16384, -16384, -32768];

    fn pcm16_bytes() -> Vec<u8> {
        PCM16_DATA.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn expected_pcm16() -> Vec<f32> {
        PCM16_DATA.iter().map(|&s| s as f32 / 32768.0).collect()
    }

    #[test]
    fn skips_unknown_and_odd_sized_chunks() {
        let wav = riff(&[
            chunk(b"fmt ", &fmt_body(1, 2, 44100, 16)),
            chunk(b"LIST", b"odd"), // 3 bytes + padding
            chunk(b"fact", &[0; 4]),
            chunk(b"data", &pcm16_bytes()),
            chunk(b"junk", b"trailing chunk after data"),
        ]);
        assert_eq!(decode_wav(&wav).unwrap().samples, expected_pcm16());
    }

    #[test]
    fn wave_format_extensible() {
        // cbSize(22) validBits(16) channelMask(3) subFormat = PCM GUID
        let mut fmt = fmt_body(0xFFFE, 2, 44100, 16);
        fmt.extend(22u16.to_le_bytes());
        fmt.extend(16u16.to_le_bytes());
        fmt.extend(3u32.to_le_bytes());
        fmt.extend([1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
        let wav = riff(&[chunk(b"fmt ", &fmt), chunk(b"data", &pcm16_bytes())]);
        assert_eq!(decode_wav(&wav).unwrap().samples, expected_pcm16());
    }

    #[test]
    fn streamed_data_size_uses_rest_of_file() {
        for size in [0u32, u32::MAX] {
            let mut wav = riff(&[
                chunk(b"fmt ", &fmt_body(1, 2, 44100, 16)),
                chunk(b"data", &pcm16_bytes()),
            ]);
            let size_at = wav.len() - pcm16_bytes().len() - 4;
            wav[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
            assert_eq!(decode_wav(&wav).unwrap().samples, expected_pcm16(), "size {size:#x}");
        }
    }

    #[test]
    fn trailing_partial_frame_is_ignored() {
        let mut data = pcm16_bytes();
        data.extend([0x12, 0x34, 0x56]); // 3 stray bytes, a frame is 4
        let wav = riff(&[chunk(b"fmt ", &fmt_body(1, 2, 44100, 16)), chunk(b"data", &data)]);
        assert_eq!(decode_wav(&wav).unwrap().samples, expected_pcm16());
    }

    #[test]
    fn rejects_malformed_and_unsupported() {
        let fmt16 = chunk(b"fmt ", &fmt_body(1, 2, 44100, 16));
        let data = chunk(b"data", &pcm16_bytes());

        // truncated data chunk
        let wav = riff(&[fmt16.clone(), data.clone()]);
        assert!(matches!(decode_wav(&wav[..wav.len() - 3]), Err(Error::Decode(_))));
        // no data chunk / data before fmt / too short for a header
        assert!(matches!(decode_wav(&riff(&[fmt16.clone()])), Err(Error::Decode(_))));
        assert!(matches!(decode_wav(&riff(&[data.clone(), fmt16.clone()])), Err(Error::Decode(_))));
        assert!(matches!(decode_wav(b"RIFF"), Err(Error::Decode(_))));
        // ADPCM, 12-bit, 64-bit float
        for (tag, bits) in [(2u16, 4u16), (1, 12), (3, 64)] {
            let fmt = chunk(b"fmt ", &fmt_body(tag, 2, 44100, bits));
            let r = decode_wav(&riff(&[fmt, data.clone()]));
            assert!(matches!(r, Err(Error::UnsupportedSampleFormat(_))), "tag {tag} bits {bits}");
        }
        // 6 channels, zero sample rate, inconsistent block align
        let r = decode_wav(&riff(&[chunk(b"fmt ", &fmt_body(1, 6, 44100, 16)), data.clone()]));
        assert!(matches!(r, Err(Error::UnsupportedChannels(6))));
        let r = decode_wav(&riff(&[chunk(b"fmt ", &fmt_body(1, 2, 0, 16)), data.clone()]));
        assert!(matches!(r, Err(Error::Decode(_))));
        let mut bad = fmt_body(1, 2, 44100, 16);
        bad[12] = 7; // block align
        assert!(matches!(
            decode_wav(&riff(&[chunk(b"fmt ", &bad), data.clone()])),
            Err(Error::Decode(_))
        ));
    }

    #[test]
    fn repo_fixtures_match_hound() {
        for name in ["stereo_44k.wav", "stereo_48k.wav", "mono_44k.wav"] {
            let bytes = super::fixture(name);
            assert_eq!(decode_wav(&bytes).unwrap().samples, hound_reference(&bytes), "{name}");
        }
    }
}

mod mixer_playback {
    use super::super::mixer::Mixer;
    use super::super::PlaySoundParams;
    use super::fixture;

    #[test]
    fn duration_is_accurate() {
        let (_builder, ctrl) = Mixer::new();
        let (_id, duration) = ctrl.load(&fixture("stereo_44k.wav")).unwrap();
        // 155648 samples = 77824 stereo frames @ 44100 Hz = 1.7647166 s
        let expected = (super::EXPECTED_LEN / 2) as f32 / 44100.0;
        assert!((duration - expected).abs() < 1e-4);
    }

    #[test]
    fn pause_and_resume() {
        let (builder, ctrl) = Mixer::new();
        let mut mixer = builder.build();

        let (sound_id, _) = ctrl.load(&fixture("stereo_44k.wav")).unwrap();
        let pb = ctrl.play(sound_id, PlaySoundParams { looped: true, volume: 1.0 });

        let mut buf = vec![0.0f32; 1000];
        mixer.fill_audio_buffer(&mut buf, 500);
        let has_sound = buf.iter().any(|&s| s.abs() > 0.01);
        assert!(has_sound, "Playing sound should produce non-zero samples");

        // Pause
        ctrl.pause(pb.play_id);
        mixer.fill_audio_buffer(&mut buf, 500);
        let is_silent = buf.iter().all(|&s| s == 0.0);
        assert!(is_silent, "Paused sound should produce silence");

        // Resume
        ctrl.resume(pb.play_id);
        mixer.fill_audio_buffer(&mut buf, 500);
        let has_sound_again = buf.iter().any(|&s| s.abs() > 0.01);
        assert!(has_sound_again, "Resumed sound should produce non-zero samples again");
    }

    #[test]
    fn seek_with_micro_ramp_continuity() {
        let (builder, ctrl) = Mixer::new();
        let mut mixer = builder.build();

        let (sound_id, _) = ctrl.load(&fixture("stereo_44k.wav")).unwrap();
        let pb = ctrl.play(sound_id, PlaySoundParams { looped: true, volume: 1.0 });

        // Render first buffer
        let mut buf1 = vec![0.0f32; 1000];
        mixer.fill_audio_buffer(&mut buf1, 500);
        let last_l = buf1[998];
        let last_r = buf1[999];

        // Seek far ahead into track
        ctrl.seek(pb.play_id, 40000);

        // Render next buffer
        let mut buf2 = vec![0.0f32; 1000];
        mixer.fill_audio_buffer(&mut buf2, 500);

        // The very first frame of buf2 must match last_l and last_r with C0 continuity
        assert!((buf2[0] - last_l).abs() < 1e-5, "Left channel C0 continuity preserved on seek");
        assert!((buf2[1] - last_r).abs() < 1e-5, "Right channel C0 continuity preserved on seek");
    }
}

