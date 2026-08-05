# RayQuad

RayQuad is an opinionated fork of [Macroquad](https://github.com/not-fl3/macroquad)
that makes its API and resource model more like
[Raylib](https://github.com/raysan5/raylib). It keeps Macroquad's small,
cross-platform rendering foundation while favoring explicit, predictable game
resources over framework-managed runtime state.

RayQuad starts again at version `0.1.0` and is not API-compatible with upstream
Macroquad.

## Design differences

The main intentional differences from upstream Macroquad are:

- **No built-in immediate-mode UI:** the bundled UI module, UI feature, UI
  examples, and UI-based profiler were removed. Applications can choose an
  external UI library without RayQuad owning a second input and rendering path.
- **Raylib-style fonts:** loading a font constructs its complete immutable glyph
  atlas. Drawing scales that atlas and never rasterizes glyphs, grows a font
  texture, or uploads font data during gameplay. Use `FontLoadParams` with an
  explicit character repertoire or complete strings when the default repertoire
  is insufficient; complete strings collect ligatures and contextual forms.
- **Shader-friendly text:** text uses the currently active material, matching the
  Raylib pattern of activating a font shader around text drawing.
- **Explicit entry point:** applications call `Window::new` or
  `Window::from_config` directly; RayQuad has no companion procedural-macro
  crate.
- **Native-only for now:** RayQuad currently targets desktop and mobile platforms.

- **Repository cleanup and reorganization:** source files are grouped by subsystem,
  module entry points use `mod.rs`, and embedded library assets are stored under
  `assets/`.
- **Faster hash collections:** production uses of
  `std::collections::HashMap` and `HashSet` have been replaced with Foldhash.
- **Swash font backend:** text shaping and rasterization now use Swash. Rich-text
  layout is available through the optional `rich-text` Cargo feature, backed by
  Parley.
- **Ordered input handling:** keyboard state uses compact bitsets for membership
  checks together with `Vec` storage to preserve input-event order.

- Replaced the handwritten coroutine and texture generational stores with typed
  generational keys.
- Replaced the `color_u8!` macro with a typed `const fn` named `color_u8`.
- Updated coroutine memory telemetry to describe its SlotMap-based estimate.

## Features

* Native desktop and mobile support.
* Efficient 2D rendering with automatic geometry batching.
* Minimal amount of dependencies: build after `cargo clean` takes only 16s on x230(~6 years old laptop).
* Android and iOS support through Miniquad.

## Supported Platforms

* PC: Windows/Linux/macOS;
* Android;
* IOS.

## Build Instructions

### Setting Up a RayQuad Project

RayQuad is a normal Rust dependency, so an empty project may be created with:

```sh
# Create empty cargo project
cargo init --bin
```

Add RayQuad as a dependency to `Cargo.toml`:
```toml

[dependencies]
rayquad = "0.1"
```

Put some RayQuad code in `src/main.rs`:
```rust
use rayquad::prelude::*;

fn main() {
    Window::new("BasicShapes", game());
}

async fn game() {
    loop {
        clear_background(RED);

        draw_line(40.0, 40.0, 100.0, 200.0, 15.0, BLUE);
        draw_rectangle(screen_width() / 2.0 - 60.0, 100.0, 120.0, 60.0, GREEN);
        draw_circle(screen_width() - 30.0, screen_height() - 30.0, 15.0, YELLOW);

        draw_text("IT WORKS!", 20.0, 20.0, 30.0, DARKGRAY);

        next_frame().await
    }
}
```

`main` is now an ordinary synchronous Rust entry point. `Window::new` creates
the native window and runs the supplied application future. The async `game`
function contains initialization and the frame loop; `next_frame().await`
yields control to RayQuad until the next frame.

For custom window configuration, use `Window::from_config`:

```rust
use rayquad::prelude::*;

fn window_conf() -> Conf {
    Conf {
        window_title: "Configured RayQuad".to_owned(),
        window_width: 1280,
        window_height: 720,
        ..Default::default()
    }
}

fn main() {
    Window::from_config(window_conf(), game());
}

async fn game() {
    loop {
        clear_background(BLACK);
        draw_text("RayQuad", 30.0, 50.0, 36.0, WHITE);
        next_frame().await;
    }
}
```

RayQuad intentionally does not provide `#[rayquad::main]`. This avoids a
companion procedural-macro crate and leaves application startup visible in
normal Rust code.

And to run it natively:
```sh
cargo run
```

The repository's `examples` directory contains additional RayQuad examples.

### Linux

```sh
# ubuntu system dependencies
apt install pkg-config libx11-dev libxi-dev libgl1-mesa-dev libasound2-dev

# fedora system dependencies
dnf install libX11-devel libXi-devel mesa-libGL-devel alsa-lib-devel

# arch linux system dependencies
pacman -S pkg-config libx11 libxi mesa-libgl alsa-lib
```

### Windows

On windows both MSVC and GNU target are supported, no additional dependencies required.

Also cross-compilation to windows from linux is supported:

```sh
rustup target add x86_64-pc-windows-gnu

cargo run --target x86_64-pc-windows-gnu
```

### IOS (Untested)

To run on the simulator:

```sh
mkdir MyGame.app
cargo build --target x86_64-apple-ios --release
cp target/x86_64-apple-ios/release/mygame MyGame.app
# only if the game have any assets
cp -r assets MyGame.app
cat > MyGame.app/Info.plist << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
<key>CFBundleExecutable</key>
<string>mygame</string>
<key>CFBundleIdentifier</key>
<string>com.mygame</string>
<key>CFBundleName</key>
<string>mygame</string>
<key>CFBundleVersion</key>
<string>1</string>
<key>CFBundleShortVersionString</key>
<string>1.0</string>
</dict>
</plist>
EOF

xcrun simctl install booted MyGame.app/
xcrun simctl launch booted com.mygame
```

For details and instructions on provisioning for real iphone, check [https://macroquad.rs/articles/ios/](https://macroquad.rs/articles/ios/)

<details>
<summary>Tips</summary>
Adding the following snippet to your Cargo.toml ensures that all dependencies compile in release even in debug mode. In RayQuad, this makes image loading substantially faster while retaining quick application rebuilds.

```toml
[profile.dev.package.'*']
opt-level = 3
```
</details>

## async/await

RayQuad retains Macroquad's async application future. It requires no external
runtime or executor: `Window::new` polls the future as part of the native event
loop, and `next_frame().await` yields until the following frame.
