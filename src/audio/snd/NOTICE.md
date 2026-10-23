# Vendored code: quad-snd

This directory is derived from `quad-snd` 0.2.8
(https://github.com/not-fl3/quad-snd, crates.io/crates/quad-snd).

- Author: Fedor Logachev <not.fl3@gmail.com>
- License: MIT OR Apache-2.0 (same as this crate; see `LICENSE-MIT` and
  `LICENSE-APACHE` at the repository root)

Changes from upstream:
- Files renamed (`*_snd.rs` -> backend name) and `crate::` paths rewritten.
- Platform backends: alsa (Linux/BSD), coreaudio (macOS/iOS), wasapi (Windows),
  opensles (Android), web (wasm32, expects `audio.js` glue).
- `audio.js` and `UPSTREAM_Cargo.toml.orig` are kept for reference only.
