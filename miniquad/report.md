# Miniquad Performance & Optimization Report

This report summarizes the optimization and bug-hunting audit of the `miniquad` codebase, split into **Implemented Fixes** and **Remaining / Deferred Issues** (also tracked in [`../issues.md`](file:///home/tq/Projects/macroquad/issues.md)).

---

## Implemented Fixes

### 1. Eliminated `Box<dyn ...>` Dynamic Dispatch & Boundary Overhead
- **Concrete `miniquad::Context` ([`src/graphics.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics.rs#L1369-L1682), [`src/lib.rs`](file:///home/tq/Projects/macroquad/miniquad/src/lib.rs#L140-L151))**:
  - Replaced `pub type Context = dyn RenderingBackend;` and `window::new_rendering_backend() -> Box<dyn RenderingBackend>` with a concrete `Context` returned by value (`GlContext` on non-Apple targets, `enum Context { Gl(GlContext), Metal(MetalContext) }` on Apple targets).
- **Generic `miniquad::start` ([`src/lib.rs`](file:///home/tq/Projects/macroquad/miniquad/src/lib.rs#L478-L534), [`src/event.rs`](file:///home/tq/Projects/macroquad/miniquad/src/event.rs#L238-L293))**:
  - Changed `pub fn start<H: 'static + EventHandler, F: 'static + FnOnce() -> H>(conf: conf::Conf, f: F)` so `EventHandler` implementations are unboxed and statically dispatched on Linux (`linux_x11` and `linux_wayland`).
- **Zero-Allocation `RenderingBackend::info(&self) -> &ContextInfo` ([`src/graphics.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics.rs#L1079-L1085), [`src/graphics/gl.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L533-L607), [`src/graphics/metal.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/metal.rs#L419-L440))**:
  - Removed the unused `gl_version_string: String` field from `ContextInfo` (making `ContextInfo` a 9-byte `Copy` struct), borrowed the OpenGL version string as `&str` during init, and changed `info(&self)` to return `&ContextInfo` by reference.
- **Direct `texture_size` Lookup ([`src/graphics/gl.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L515-L526), [`src/graphics/metal.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/metal.rs#L589-L593))**:
  - Overrode `texture_size(&self, texture: TextureId) -> (u32, u32)` in `GlContext` and `MetalContext` to read `(width, height)` directly without copying the full `Texture` / `TextureParams` struct.
- **Single-Lock Dropped Files Take ([`src/lib.rs`](file:///home/tq/Projects/macroquad/miniquad/src/lib.rs#L377-L383))**:
  - Added `window::take_dropped_files()` to move `dropped_files.paths` and `dropped_files.bytes` out of `NATIVE_DISPLAY` in a single lock acquisition with zero buffer clones.

### 2. OpenGL Backend (`src/graphics/gl.rs`, `src/graphics/gl/cache.rs`) & Resource Management (`src/lib.rs`)
- **Compacted `VertexAttributeInternal` & Long-Lived `Box<[T]>` Slices**:
  - Narrowed [`VertexAttributeInternal`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl/cache.rs#L3-L13) from 40 bytes to 24 bytes (`Option<CachedAttribute>` from 48 bytes to 28 bytes).
  - Converted [`ShaderInternal::images`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L220-L224), [`ShaderInternal::uniforms`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L220-L224), [`PipelineInternal::layout`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L486-L491), and [`RenderPassInternal::color_textures`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L502-L508) / `resolves` from `Vec<T>` to `Box<[T]>`.
- **`ResourceManager<T>` Free-List Reuse**:
  - Added `ResourceManager<T>` in [`src/lib.rs`](file:///home/tq/Projects/macroquad/miniquad/src/lib.rs#L29-L78) and used it for `shaders`, `pipelines`, `passes`, and `buffers` in [`src/graphics/gl.rs`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L529-L532), preventing unbounded vector growth when resources are created and deleted dynamically.
- **GPU Resource Leak Fixes**:
  - Detached and deleted vertex/fragment shader objects (`glDetachShader` + `glDeleteShader`) after linking in [`load_shader_internal`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L619-L622).
  - Properly deleted GL framebuffers (`glDeleteFramebuffers`) in [`delete_render_pass`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L1209-L1228).
  - Reused a cached `readback_fbo: Option<GLuint>` in [`Texture::read_pixels`](file:///home/tq/Projects/macroquad/miniquad/src/graphics/gl.rs#L422-L426) instead of allocating and leaking a new FBO on every readback.

### 3. Linux X11 & Wayland Backends (`src/native/linux_x11.rs`, `src/native/linux_wayland.rs`)
- **X11**:
  - Cached cursor icon and visibility state to avoid redundant `XDefineCursor` calls.
  - Added a 256-entry `keycode_cache` lookup table in [`X11Display`](file:///home/tq/Projects/macroquad/miniquad/src/native/linux_x11.rs#L50) (invalidated on `MappingNotify`) to avoid calling `XKeycodeToKeysym` on every key press/release.
  - Replaced `std::collections::HashMap` with `foldhash::HashMap` for `cursor_cache`.
- **Wayland**:
  - Fixed a timerfd file descriptor leak in keyboard key repeat handling (`extensions/timer.rs`).
  - Cached XKB modifier indices (`ctrl`, `alt`, `shift`, `logo`) on keymap initialization instead of querying `xkb_ state_mod_name_is_active` with C-strings on every key event.
  - Replaced `std::collections::HashMap` with `foldhash::HashMap` for `touch_positions`.

---

## Remaining / Deferred Issues

See [`../issues.md`](file:///home/tq/Projects/macroquad/issues.md) for full details on the remaining deferred items:
1. **`NATIVE_DISPLAY` `Mutex` (`src/lib.rs`)**: All hot-path queries from `rayquad` are now served from `rayquad::Context`; `miniquad`'s internal `OnceLock<Mutex<NativeDisplayData>>` can optionally be converted to `thread_local!` + `RefCell` with a `Vec<Request>` queue.
2. **`Box<dyn Clipboard>` (`src/native.rs`)**: Replace with a concrete platform clipboard type / Linux enum.
3. **`Box<dyn Fn(Request) + Send>` on Android (`src/native.rs`)**: Replace with `fn(Request)`.
4. **Apple Metal Backend (`src/graphics/metal.rs`)**: Replace synchronous `waitUntilCompleted` stall with semaphore-based triple buffering, rotate `current_frame_index` once per frame instead of per draw call, fix `didModifyRange` byte length, and use `ResourceManager` for Metal resources.
5. **Android Backend (`src/native/android.rs`)**: Fix `libc::malloc` leak and double copy in `load_asset`, replace background `SurfaceDestroyed` busy-wait spin loop with blocking wait, and cache JNI method IDs.

