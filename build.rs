fn main() {
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target_arch != "wasm32" && std::env::var("CARGO_FEATURE_AUDIO").is_ok() {
        let res = cc::Build::new()
            .file("src/audio/snd/stb_vorbis.c")
            .define("STB_VORBIS_NO_STDIO", None)
            .define("STB_VORBIS_NO_PUSHDATA_API", None)
            .define("STB_VORBIS_NO_INTEGER_CONVERSION", None)
            .define("STB_VORBIS_MAX_CHANNELS", "2")
            .opt_level(3)
            .warnings(false)
            .try_compile("stb_vorbis");
        if let Err(e) = res {
            println!("cargo:warning=Failed to compile stb_vorbis with C compiler: {e}");
        }
    }
}
