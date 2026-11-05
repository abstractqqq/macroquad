# Rayquad & Miniquad Optimization Status and Open Issues

This document tracks the performance, memory, and architectural optimizations completed across `rayquad` and its local `miniquad` workspace subcrate, as well as the deferred / platform-specific issues remaining to be addressed.

---

## Implemented Optimizations

### 1. Workspace Subcrate Migration & Dependency Synchronization
- **Local Subcrate**: Moved `miniquad` into [`miniquad/`](file:///home/tq/Projects/macroquad/miniquad/Cargo.toml) as a workspace member of [`Cargo.toml`](file:///home/tq/Projects/macroquad/Cargo.toml) (`publish = false`, `edition = "2021"`).
- **Zero Duplicate Dependencies**: Synced `foldhash = "0.2"`, `winapi = "0.3.8"`, `glam = "0.34"`, and `swash = "0.2"`, and removed unused NixOS [`shell.nix`](file:///home/tq/Projects/macroquad/miniquad) and external dev-dependencies (`quad-rand`, `fontdue`). `cargo tree --duplicates --workspace` reports **0 duplicate crates**.

### 2. Eliminated `Box<dyn ...>` Dynamic Dispatch on Hot Paths
- **Concrete `miniquad::Context` ([`miniquad/src/graphics.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics.rs#L1369-L1682), [`miniquad/src/lib.rs`](file:///home/tq/Projects/macroquad/miniquad/src/lib.rs#L140-L151), [`src/lib.rs`](file:///home/tq/Projects/macroquad/src/lib.rs#L216))**:
  - Replaced `pub type Context = dyn RenderingBackend;` and `new_rendering_backend() -> Box<dyn RenderingBackend>` with a concrete `miniquad::Context` returned and stored by value (`pub type Context = GlContext;` on non-Apple targets, and `pub enum Context { Gl(GlContext), Metal(MetalContext) }` on Apple targets).
  - Stored `quad_context: miniquad::Context` directly inline inside `rayquad::Context` (`get_quad_context() -> &'static mut miniquad::Context`), turning every `QuadGl` and texture/buffer/pipeline/draw call into a direct statically-dispatched call eligible for inlining.
- **Unboxed Generic `EventHandler` ([`miniquad/src/lib.rs`](file:///home/tq/Projects/macroquad/miniquad/src/lib.rs#L478-L534), [`miniquad/src/event.rs`](file:///home/tq/Projects/macroquad/miniquad/src/event.rs#L238-L293), [`miniquad/src/native/linux_x11.rs`](file:///home/tq/Projects/macroquad/miniquad/src/native/linux_x11.rs#L72-L76), [`miniquad/src/native/linux_wayland.rs`](file:///home/tq/Projects/macroquad/miniquad/src/native/linux_wayland.rs#L1052-L1055))**:
  - Changed `miniquad::start<H, F>(conf: conf::Conf, f: F)` where `H: 'static + EventHandler, F: 'static + FnOnce() -> H` so `Stage` is stored unboxed on the stack and dispatched statically on Linux (`linux_x11` and `linux_wayland`).
  - Implemented `EventHandler` for `Box<T: EventHandler + ?Sized>` for backwards compatibility.

### 3. `rayquad` $\leftrightarrow$ `miniquad` Boundary Optimizations
- **Cached Window State in `rayquad::Context` & Eliminated Per-Frame `NATIVE_DISPLAY` Locks**:
  - Cached `dpi_scale` and `blocking_event_loop` alongside `screen_width` and `screen_height` in [`rayquad::Context`](file:///home/tq/Projects/macroquad/src/lib.rs#L178-L181) (initialized in [`Context::new`](file:///home/tq/Projects/macroquad/src/lib.rs#L237-L249) and updated on resize in [`Stage::resize_event`](file:///home/tq/Projects/macroquad/src/lib.rs#L422-L431)).
  - Updated [`window::screen_width`](file:///home/tq/Projects/macroquad/src/window/mod.rs#L58-L61), [`window::screen_height`](file:///home/tq/Projects/macroquad/src/window/mod.rs#L63-L67), [`window::screen_dpi_scale`](file:///home/tq/Projects/macroquad/src/window/mod.rs#L69-L71), [`window::request_new_screen_size`](file:///home/tq/Projects/macroquad/src/window/mod.rs#L76-L78), [`input::mouse_position`](file:///home/tq/Projects/macroquad/src/input/mod.rs#L383-L388), [`Context::pixel_perfect_projection_matrix`](file:///home/tq/Projects/macroquad/src/lib.rs#L368-L379), [`QuadGl::draw`](file:///home/tq/Projects/macroquad/src/quad_gl.rs#L780-L783), [`GrabScreenTexture::end`](file:///home/tq/Projects/macroquad/src/quad_gl.rs#L271-L273), and [`text::camera_font_scale`](file:///home/tq/Projects/macroquad/src/text/mod.rs#L1063-L1065) to read directly from `rayquad::Context` with **zero `NATIVE_DISPLAY` `Mutex` locks** during frame execution.
  - Removed the unconditional per-frame `miniquad::window::set_cursor_grab(get_context().cursor_grabbed)` from [`Stage::update`](file:///home/tq/Projects/macroquad/src/lib.rs#L574-L577), which was locking `NATIVE_DISPLAY`, sending a `Request::SetCursorGrab` over the `mpsc` channel, and triggering `XUngrabPointer` + `XFlush` on X11 every single frame. Instead, cursor grab is applied on change in [`set_cursor_grab`](file:///home/tq/Projects/macroquad/src/input/mod.rs#L371-L375) and re-applied on window restore in [`Stage::window_restored_event`](file:///home/tq/Projects/macroquad/src/lib.rs#L631-L636).
- **Eliminated `Bindings` Heap Allocations in `QuadGl` ([`src/quad_gl.rs`](file:///home/tq/Projects/macroquad/src/quad_gl.rs#L100-L105), [`src/quad_gl.rs`](file:///home/tq/Projects/macroquad/src/quad_gl.rs#L617-L621), [`src/quad_gl.rs`](file:///home/tq/Projects/macroquad/src/quad_gl.rs#L844-L869))**:
  - Replaced `draw_calls_bindings: Vec<Bindings>` with `Vec<DrawCallBindings>` storing flat `Copy` buffer handles `(vertex_buffer: BufferId, index_buffer: BufferId)`, and passed a stack-allocated `[TextureId; MAX_SHADERSTAGE_IMAGES]` slice directly to `ctx.apply_bindings_from_slice(...)`.
  - Replaced `CameraSnapshotter::bindings: Bindings` (and its unused 1x1 dummy texture allocation) with `vertex_buffer: BufferId, index_buffer: BufferId` and `ctx.apply_bindings_from_slice(...)`.
  - Fixed [`QuadGl::update_drawcall_capacity`](file:///home/tq/Projects/macroquad/src/quad_gl.rs#L1086-L1127) to allocate `PackedVertex` buffers when `feature = "3d"` is enabled.
- **Zero-Allocation `ctx.info()` & Fast `ctx.texture_size()` ([`miniquad/src/graphics.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics.rs#L1101), [`miniquad/src/graphics/gl.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L515-L526), [`src/texture/mod.rs`](file:///home/tq/Projects/macroquad/src/texture/mod.rs#L724-L745))**:
  - Changed `RenderingBackend::info(&self)` to return `&ContextInfo` instead of cloning `ContextInfo` (and heap-allocating `gl_version_string: String` on every call).
  - Overrode `texture_size` in `GlContext` and `MetalContext` to read `(width, height)` directly without copying `Texture` / `TextureParams`, and updated `Texture2D::update` / `Texture2D::update_from_bytes` to call `ctx.texture_update_part` directly after checking dimensions instead of querying `texture_size` twice.
- **Eliminated Redundant Clones in `make_pipeline` ([`src/quad_gl.rs`](file:///home/tq/Projects/macroquad/src/quad_gl.rs#L518-L580), [`src/quad_gl.rs`](file:///home/tq/Projects/macroquad/src/quad_gl.rs#L679-L718))**:
  - Replaced `shader::uniforms() -> Vec<...>` and $3 \times O(N)$ `uniforms.insert(0, ...)` shifts with a `const UNIFORMS` array chained into `uniforms.into_iter()`, moving `UniformDesc::name` `String`s directly into `Box<str>` without cloning.
- **Single-Lock, Zero-Clone Dropped Files ([`miniquad/src/lib.rs`](file:///home/tq/Projects/macroquad/miniquad/src/lib.rs#L377-L383), [`src/lib.rs`](file:///home/tq/Projects/macroquad/src/lib.rs#L571-L584))**:
  - Added `miniquad::window::take_dropped_files()` to take ownership of dropped file `PathBuf`s and `Vec<u8>` buffers in a single `NATIVE_DISPLAY` lock acquisition with zero buffer clones.
- **Direct Desktop File Loading ([`src/file/mod.rs`](file:///home/tq/Projects/macroquad/src/file/mod.rs#L45-L56))**:
  - On desktop platforms (`linux`, `windows`, `macos`), `load_file` now calls `std::fs::read` directly without allocating `Arc<str>`, `Arc<Mutex<Option<...>>>`, and polling `FileLoadingFuture`.

### 4. Memory Savings & Struct Layout Optimizations
- **Discarded Unused Heap Strings (`ContextInfo::gl_version_string` & `GlxExtensions::extensions_string`)**:
  - Removed the unused `gl_version_string: String` field from [`miniquad::ContextInfo`](file:///home/tq/Projects/macroquad/miniquad/src/graphics.rs#L1079-L1085), making `ContextInfo`, `Features`, and `GlslSupport` 9-byte `Copy` structs and borrowing the OpenGL version C-string as `&str` during context initialization in [`gl_info`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L533-L607).
  - Removed the unused `extensions_string: String` field from [`GlxExtensions`](file:///home/tq/Projects/macroquad/miniquad/src/native/linux_x11/glx.rs#L197-L202), parsing extensions directly from a borrowed `&str`.
- **Long-Lived Immutable Slices (`Vec<T>` $\rightarrow$ `Box<[T]>` / `Arc<[f32]>`)**:
  - Converted `ShaderInternal::images` and `ShaderInternal::uniforms` ([`miniquad/src/graphics/gl.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L220-L224)), `PipelineInternal::layout` ([`miniquad/src/graphics/gl.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L486-L491)), and `RenderPassInternal::color_textures` / `resolves` ([`miniquad/src/graphics/gl.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L502-L508)) from `Vec<T>` / `Option<Vec<T>>` to `Box<[T]>`.
  - Converted `ShapedText::glyphs` and `ShapedText::clusters` ([`src/text/renderer.rs`](file:///home/tq/Projects/macroquad/src/text/renderer.rs#L37-L44)) and `TextLayout::cluster_colors` ([`src/text/mod.rs`](file:///home/tq/Projects/macroquad/src/text/mod.rs#L747-L761)) to `Box<[T]>`, and skipped allocating `cluster_colors` (`Box::default()`) when `color_spans.is_empty()` ([`src/text/mod.rs`](file:///home/tq/Projects/macroquad/src/text/mod.rs#L844-L855)).
  - Changed `AudioMessage::AddSound` ([`src/audio/snd/mixer.rs`](file:///home/tq/Projects/macroquad/src/audio/snd/mixer.rs#L11-L18)) from `Vec<f32>` to `Arc<[f32]>`, converting `samples.into()` on the loading thread ([`MixerControl::load`](file:///home/tq/Projects/macroquad/src/audio/snd/mixer.rs#L254-L260)) instead of allocating and copying on the real-time audio callback thread.
- **Compacted `VertexAttributeInternal` (40 B $\rightarrow$ 24 B) ([`miniquad/src/graphics/gl/cache.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl/cache.rs#L3-L13))**:
  - Narrowed `offset: u32`, `divisor: u32`, `buffer_index: u16`, `stride: u8`, and `size: u8`, shrinking `Option<CachedAttribute>` in `GlCache::attributes` and `PipelineInternal::layout` from 48 bytes to 28 bytes (42% reduction).
- **Compacted `DrawCall` & `GlPipeline` (168 B $\rightarrow$ 128 B) ([`src/quad_gl.rs`](file:///home/tq/Projects/macroquad/src/quad_gl.rs#L48-L67))**:
  - Changed `GlPipeline(pub usize)` to `GlPipeline(pub u8)`, moved `model: glam::Mat4` to the start of `DrawCall` to eliminate 16-byte alignment padding, and narrowed `vertices_count`, `indices_count`, `vertices_start`, and `indices_start` from `usize` to `u32` (fits in 2 CPU cache lines).
- **Compacted `GlyphMetric` & `PositionedGlyph` ([`src/text/mod.rs`](file:///home/tq/Projects/macroquad/src/text/mod.rs#L86-L103), [`src/text/renderer.rs`](file:///home/tq/Projects/macroquad/src/text/renderer.rs#L10-L24))**:
  - Narrowed `left` and `top` glyph bearing offsets from `i32` to `i16`, shrinking `GlyphMetric` from 20 bytes to 16 bytes and `PositionedGlyph` from 48 bytes to 40 bytes while preserving binary compatibility with `.rqfn` baked font assets.
- **Zero-`Vec<char>` Shader Preprocessor ([`src/material/mod.rs`](file:///home/tq/Projects/macroquad/src/material/mod.rs#L168-L205))**:
  - Rewrote `preprocess_shader` to operate directly on UTF-8 `&str` lines with a fast path returning `source.to_string()` when `!source.contains("#include")`.

### 5. Cross-Platform Clipboard Refinement & Removal of `Box<dyn Clipboard>`
- **Concrete `PlatformClipboard` & `Result<(), ClipboardError>` API ([`miniquad/src/native.rs`](file:///home/tq/Projects/macroquad/miniquad/src/native.rs#L83-L147), [`miniquad/src/lib.rs`](file:///home/tq/Projects/macroquad/miniquad/src/lib.rs#L354-L366), [`src/window/mod.rs`](file:///home/tq/Projects/macroquad/src/window/mod.rs#L89-L97))**:
  - Replaced `Box<dyn Clipboard>` and `trait Clipboard` with an inline `PlatformClipboard` (`enum PlatformClipboard { X11(X11Clipboard), Wayland(WaylandClipboard) }` on Linux and zero-sized concrete types on Windows, macOS, iOS, Android, and WASM).
  - Changed `clipboard_set(data: &str) -> Result<(), ClipboardError>` so callers receive explicit error feedback, and exposed `clipboard_get`, `clipboard_set`, and `ClipboardError` in `rayquad::window` / `rayquad::prelude`.
- **Platform Clipboard Fixes**:
  - **Linux X11 ([`miniquad/src/native/linux_x11/clipboard.rs`](file:///home/tq/Projects/macroquad/miniquad/src/native/linux_x11/clipboard.rs))**: Added `TARGETS` and standard text atom negotiation, replaced the unbounded `XNextEvent` loop (which swallowed input events and hung if no owner responded) with `XCheckTypedWindowEvent` and a bounded timeout, added a self-paste fast path via `XGetSelectionOwner`, and removed `static mut MESSAGE`.
  - **Linux Wayland ([`miniquad/src/native/linux_wayland/clipboard.rs`](file:///home/tq/Projects/macroquad/miniquad/src/native/linux_wayland/clipboard.rs))**: Added standard Wayland MIME type negotiation (`text/plain;charset=utf-8`, `text/plain`, `UTF8_STRING`, `TEXT`, `STRING`), moved `ClipboardContext` into `WaylandPayload` (eliminating `static mut CLIPBOARD`), added a self-paste fast path, destroyed replaced `wl_data_offer` proxies, and fixed the pipe `fds[0]` leak on read errors.
  - **macOS ([`miniquad/src/native/macos.rs`](file:///home/tq/Projects/macroquad/miniquad/src/native/macos.rs#L1055-L1094))**: Wired the `NSPasteboard` implementation into `MacosClipboard`.
  - **Windows ([`miniquad/src/native/windows/clipboard.rs`](file:///home/tq/Projects/macroquad/miniquad/src/native/windows/clipboard.rs))**: Fixed `GlobalSize` byte-to-`u16` length calculation, added `GlobalUnlock` / `GlobalFree` cleanup, and eliminated intermediate `Vec<u16>` and `format!` allocations.
  - **WASM ([`miniquad/src/native/wasm.rs`](file:///home/tq/Projects/macroquad/miniquad/src/native/wasm.rs#L197-L216))**: Updated the local clipboard cache on `set()` so subsequent `get()` calls return the newly set text.

### 6. Previously Completed GL & Linux Backend Fixes in `miniquad`
- **`ResourceManager<T>` generational/free-list storage** in [`miniquad/src/lib.rs`](file:///home/tq/Projects/macroquad/miniquad/src/lib.rs#L29-L78) and [`miniquad/src/graphics/gl.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L515-L520) preventing unbounded vector growth and GPU resource leaks (`glDetachShader` + `glDeleteShader`, `glDeleteBuffers`, `glDeleteFramebuffers`, `glDeleteProgram`, cached `readback_fbo`).
- **Linux X11 & Wayland**: `foldhash` cursor/touch maps, 256-entry X11 keycode lookup table, cached X11 cursor state, cached Wayland XKB modifier indices, and fixed Wayland keyboard repeat timer resource leak.

---

## Deferred / Open Issues

### 1. `NATIVE_DISPLAY` `Mutex` in `miniquad` (Cross-Platform)
- **Location**: [`miniquad/src/lib.rs`](file:///home/tq/Projects/macroquad/miniquad/src/lib.rs#L101-L124) (`static NATIVE_DISPLAY: OnceLock<Mutex<native::NativeDisplayData>>`)
- **Status**: All per-frame/hot-path locks from `rayquad` have been eliminated by caching `screen_width`, `screen_height`, `dpi_scale`, and `blocking_event_loop` in `rayquad::Context`.
- **Remaining Opportunity**: Inside `miniquad` itself, `OnceLock<Mutex<NativeDisplayData>>` and `mpsc::Sender<Request>` can optionally be replaced with `thread_local! { RefCell<Option<NativeDisplayData>> }` and a plain `Vec<Request>` since `miniquad`'s event loop is single-threaded.

### 2. `Box<dyn Fn(Request) + Send>` on Android in `NativeDisplayData`
- **Location**: [`miniquad/src/native.rs`](file:///home/tq/Projects/macroquad/miniquad/src/native.rs#L20), [`miniquad/src/native/android.rs`](file:///home/tq/Projects/macroquad/miniquad/src/native/android.rs#L512)
- **Problem**: On Android, `native_requests` is typed as `Box<dyn Fn(Request) + Send>`, even though the closure in `android.rs` can be a non-capturing function.
- **Proposed Fix**:
  - Change the field type on Android to a plain function pointer `pub native_requests: fn(Request)` and initialize it with `native_requests: process_request`.

### 3. Apple Metal Backend (`miniquad/src/graphics/metal.rs`)
- **Synchronous `waitUntilCompleted` Pipeline Stall**: [`commit_frame`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/metal.rs#L1185) calls `[commandBuffer waitUntilCompleted]` after presenting the drawable, serializing CPU and GPU execution and preventing triple-buffered pipelining. Should use a 3-slot `dispatch_semaphore_t` signaled via `addCompletedHandler:`.
- **Uniform Buffer Frame Index Rotation Per Draw Call**: [`apply_uniforms_from_bytes`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/metal.rs#L1135) increments `current_frame_index = (current_frame_index + 1) % 3` on every draw call instead of once per frame in `commit_frame()`.
- **Buffer Update `didModifyRange` Bug**: [`buffer_update`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/metal.rs#L747) copies into `buffer.raw[buffer.next_value]` without advancing `next_value` on subsequent mid-frame updates, and on macOS calls `didModifyRange` with `NSRange::new(0, data.size)` instead of `NSRange::new(0, (data.size * data.element_size) as u64)`.
- **Resource Leaks**: `buffers`, `textures`, `passes`, `shaders`, and `pipelines` in `MetalContext` use append-only `Vec`s instead of `ResourceManager`, and `delete_shader` / `delete_pipeline` are no-ops.

### 4. Android Backend (`miniquad/src/native/android.rs`)
- **Asset Loading Memory Leak & Double Copy**: [`load_asset`](file:///home/tq/Projects/macroquad/miniquad/src/native/android.rs#L655-L674) allocates a buffer via `libc::malloc`, reads into it, copies it into a Rust `Vec<u8>`, and never calls `libc::free(buf)`. Should read directly into a pre-sized `Vec<u8>` and drop the `malloc`.
- **Background Busy-Wait Spin Loop**: In [`MainThread::next_event`](file:///home/tq/Projects/macroquad/miniquad/src/native/android.rs#L207-L234), when the app is backgrounded (`SurfaceDestroyed`), the loop spins at 100% CPU calling `thread::yield_now()` instead of blocking on `rx.recv()` or `Condvar`.
- **Uncached JNI Lookups**: [`attach_jni_env`](file:///home/tq/Projects/macroquad/miniquad/src/native/android.rs#L533-L547) calls `GetObjectClass` and `GetMethodID` on every clipboard or keyboard request instead of caching method IDs at startup.

