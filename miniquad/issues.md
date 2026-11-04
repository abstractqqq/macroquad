# Miniquad Rust Efficiency & Performance Audit Report

This report ranks the most impactful efficiency and performance improvements across the Rust codebase (`src/`), ordered from **High** to **Low** potential gain, and tracks their implementation status.

## Progress Summary

| # | Area | Priority | Status |
|---|---|---|---|
| **1** | Metal Backend (`src/graphics/metal.rs`) | **HIGH** | **Untouched** (requires macOS/iOS hardware to test) |
| **2** | OpenGL Hot Draw Path & `ResourceManager` (`src/lib.rs`, `src/graphics/gl.rs`, `src/graphics/gl/cache.rs`) | **HIGH** | **Completed** |
| **3** | Android Backend (`src/native/android.rs`, `src/fs.rs`) | **MEDIUM-HIGH** | **Untouched** (requires Android target/device to test) |
| **4** | Linux (Wayland & X11) Input & SHM (`src/native/linux_wayland*`, `src/native/linux_x11.rs`) | **MEDIUM** | **Completed** (Linux items) |
| **5** | OpenGL Buffer Upload, Readback FBO Reuse, Desktop FS, and `foldhash` (`src/graphics/gl.rs`, `src/fs.rs`) | **MEDIUM-LOW** | **Completed** (GL & Desktop/Linux items) |

---

## 1. [HIGH] Metal Backend: Synchronous CPU-GPU Pipeline Stall (`waitUntilCompleted`) & Broken In-Flight Uniform Buffer Rotation
**Status:** *Untouched (deferred until macOS hardware is available)*

**Locations:**
- [`MetalContext::commit_frame`](src/graphics/metal.rs#L1451-L1472) (`src/graphics/metal.rs:1460-1461`)
- [`MetalContext::apply_uniforms_from_bytes`](src/graphics/metal.rs#L1239-L1285) (`src/graphics/metal.rs:1257`, `1273`)
- [`MetalContext::new_buffer`](src/graphics/metal.rs#L675-L716) (`src/graphics/metal.rs:695-710`)
- [`MetalContext::delete_shader`](src/graphics/metal.rs#L1442) / [`MetalContext::delete_pipeline`](src/graphics/metal.rs#L1445) (`src/graphics/metal.rs:1442-1447`)

### Issues
1. **Full GPU pipeline stall every frame (`waitUntilCompleted`)**:
   In `commit_frame` (`src/graphics/metal.rs:1460-1461`), immediately after committing the command buffer, the CPU blocks until the GPU finishes executing the entire frame:
   ```rust
   msg_send_![self.command_buffer.unwrap(), commit];
   msg_send_![self.command_buffer.unwrap(), waitUntilCompleted];
   ```
   This serializes CPU and GPU execution, preventing any overlap between CPU frame preparation and GPU rendering, and completely negates the purpose of `NUM_INFLIGHT_FRAMES = 3`.
2. **Uniform buffer frame index rotated per draw call instead of per frame**:
   In `apply_uniforms_from_bytes` (`src/graphics/metal.rs:1257`), `self.current_frame_index = (self.current_frame_index + 1) % NUM_INFLIGHT_FRAMES;` increments on *every uniform upload* (and also in `commit_frame` at line 1469), while `self.current_ub_offset` is shared across all 3 buffers.
3. **Incorrect `didModifyRange` on macOS managed uniform buffers**:
   At `src/graphics/metal.rs:1273`, `didModifyRange:NSRange::new(0, size as u64)` flushes bytes `[0..size]` instead of `NSRange::new(self.current_ub_offset as u64, size as u64)`, syncing the wrong memory region.
4. **Metal buffer & resource leaks**:
   - `new_buffer` (`src/graphics/metal.rs:695-710`) calls `retain` on an `MTLBuffer` returned by `newBufferWithBytes` / `newBufferWithLength`, which already has a retain count of `+1` per Objective-C conventions. `delete_buffer` (`src/graphics/metal.rs:439`) only sends a single `release`, leaking every deleted buffer.
   - `delete_shader` and `delete_pipeline` (`src/graphics/metal.rs:1442-1447`) are empty no-ops that never release GPU objects or remove entries from `self.shaders` / `self.pipelines`.

---

## 2. [HIGH] OpenGL Hot Draw Path: `HashMap` Resource Lookups, Uncached Pipeline/Sampler State, and End-of-Frame Cache Thrashing
**Status:** **Completed**

**Locations:**
- [`ResourceManager`](src/lib.rs#L29-L77) (`src/lib.rs:29-77`)
- [`GlContext::apply_pipeline`](src/graphics/gl.rs#L1326-L1388) (`src/graphics/gl.rs:1326-1388`)
- [`GlContext::apply_bindings_from_slice`](src/graphics/gl.rs#L1499-L1586) (`src/graphics/gl.rs:1499-1586`)
- [`GlCache::bind_texture`](src/graphics/gl/cache.rs#L95-L109) (`src/graphics/gl/cache.rs:95-109`)
- [`GlContext::end_render_pass`](src/graphics/gl.rs#L1727-L1764) & [`GlContext::commit_frame`](src/graphics/gl.rs#L1766) (`src/graphics/gl.rs:1727-1766`)

### Implemented Fixes
1. **Replaced `HashMap<usize, T>` in `ResourceManager<T>` with `Vec<Option<T>>` + free-list (`src/lib.rs:29-77`)**:
   Eliminated hashing on sequential integer handles for `shaders`, `pipelines`, `passes`, and `buffers`, turning lookups into direct $O(1)$ slice indexing and reusing freed slots via `free: Vec<usize>`.
2. **Switched remaining `HashMap` usages to `foldhash = "0.2.0"`**:
   Replaced `std::collections::HashMap` with `foldhash::HashMap` in `src/fs.rs`, `src/native/linux_wayland.rs`, `src/native/linux_x11.rs`, and `src/native/macos.rs`.
3. **Cached pipeline, program, scissor, depth, and front-face state (`src/graphics/gl.rs:1326-1388`, `src/graphics/gl/cache.rs:28-51`)**:
   - `apply_pipeline` now short-circuits immediately when `self.cache.cur_pipeline == Some(*pipeline)`.
   - `self.pipelines[pipeline.0]` is indexed once instead of 6 times per `apply_pipeline` call.
   - `glUseProgram`, `glEnable`/`glDisable(GL_SCISSOR_TEST)`, `glEnable`/`glDisable(GL_DEPTH_TEST)`, `glDepthFunc`, and `glFrontFace` are only called when their state actually changes.
4. **Eliminated redundant `glActiveTexture` and per-draw `glUniform1i` calls (`src/graphics/gl/cache.rs:95-120`, `src/graphics/gl.rs:628-635`, `1508-1519`)**:
   - Sampler uniform locations are now bound once (`glUniform1i`) when linking the shader in `load_shader_internal` instead of on every `apply_bindings_from_slice` call.
   - `GlCache::bind_texture` now tracks `active_texture` and skips both `glActiveTexture` and `glBindTexture` on a texture cache hit.
5. **Removed per-frame and per-pass buffer/texture unbinding (`src/graphics/gl.rs:927-946`, `1754-1766`)**:
   - `commit_frame` and `end_render_pass` no longer unbind all buffers and all 12 texture slots every frame; `delete_texture` selectively unbinds only slots holding the deleted texture.

---

## 3. [MEDIUM-HIGH] Android Backend: Unconditional Asset Memory Leak + Double Allocation, Background Spin-Loop, and Uncached JNI Lookups
**Status:** *Untouched (deferred until Android target/device testing is available)*

**Locations:**
- [`load_asset`](src/native/android.rs#L753-L781) & [`load_file_android`](src/fs.rs#L53-L76) (`src/native/android.rs:753-781`, `src/fs.rs:53-76`)
- [`MainThread::frame`](src/native/android.rs#L255-L266) & [`run`](src/native/android.rs#L542-L567) (`src/native/android.rs:255-266`, `542-567`)
- [`query_display_density`](src/native/android.rs#L330-L376) (`src/native/android.rs:330-376`)

### Issues
1. **100% Native Memory Leak + Double Allocation on Every Android Asset Load**:
   `load_asset` (`src/native/android.rs:773-780`) allocates `length` bytes via `libc::malloc` and never frees it, and `load_file_android` (`src/fs.rs:64-66`) copies that slice byte-by-byte into a second `Vec<u8>`.
2. **100% CPU Busy-Wait Spin Loop When App is Backgrounded (`SurfaceDestroyed`)**:
   When `self.surface.is_null()`, `eglSwapBuffers` is skipped and the main thread (`src/native/android.rs:542-567`) spins in an unthrottled `while !s.quit` loop calling `event_handler.update()` and `thread::yield_now()`.
3. **Repeated uncached JNI class/method/field lookups**:
   `load_asset`, `query_display_density`, and `AndroidClipboard` look up JNI classes, methods, and fields dynamically on every call.

---

## 4. [MEDIUM] Linux (Wayland & X11) Input & SHM Optimizations
**Status:** **Completed** *(Linux Wayland & X11 items completed)*

**Locations:**
- [`XkbKeymap`](src/native/linux_wayland/libxkbcommon.rs#L74-L115) (`src/native/linux_wayland/libxkbcommon.rs:74-115`)
- [`keyboard_handle_keymap`](src/native/linux_wayland.rs#L547-L578), [`keyboard_handle_key`](src/native/linux_wayland.rs#L604-L645), & [`generate_key_events`](src/native/linux_wayland.rs#L287-L312) (`src/native/linux_wayland.rs:287-312, 547-645`)
- [`create_shm_buffer`](src/native/linux_wayland/shm.rs#L39-L95) (`src/native/linux_wayland/shm.rs:77`)
- [`X11Display::translate_key`](src/native/linux_x11.rs#L60-L102) (`src/native/linux_x11.rs:50, 60-102, 257`)

### Implemented Fixes
1. **Wayland: Eliminated per-keypress `xkb_state_new` / `xkb_state_unref` allocations and duplicate key translations (`src/native/linux_wayland/libxkbcommon.rs:74-115`, `src/native/linux_wayland.rs:299, 564-575, 618-644`)**:
   - `XkbKeymap` now maintains a persistent unmodified `clean_state: *mut xkb_state` initialized once when the keymap is loaded in `keyboard_handle_keymap`.
   - `keyboard_handle_key` no longer translates the keycode and modifier state twice on key-down/repeat events (`state == 1 | 2`).
2. **X11: Cached scancode-to-`KeyCode` mappings (`src/native/linux_x11.rs:50, 60-102, 257`)**:
   - Previously, every key press, repeat, and release called `keycodes::translate_key`, which issued a synchronous `XGetKeyboardMapping` IPC round-trip to the X11 server followed by `XFree`.
   - `X11Display` now caches translated keycodes in `keycode_cache: [Option<KeyCode>; 256]` and invalidates the cache on X11 `MappingNotify` (event `34`).
3. **Wayland SHM: Replaced byte-by-byte pointer loop with `std::ptr::copy_nonoverlapping` (`src/native/linux_wayland/shm.rs:77`)**:
   - Copies shared-memory icon/cursor/decoration pixel buffers in a single bulk `memcpy`.

---

## 5. [MEDIUM-LOW] OpenGL Buffer Initialization, Readback FBO Reuse, and Desktop File Loading
**Status:** **Completed** *(Cross-platform GL & Desktop FS items completed)*

**Locations:**
- [`GlContext::new_buffer`](src/graphics/gl.rs#L1390-L1439) (`src/graphics/gl.rs:1420-1428`)
- [`Texture::read_pixels`](src/graphics/gl.rs#L411-L448) (`src/graphics/gl.rs:411-448`, `520`, `1047`)
- [`load_file_desktop`](src/fs.rs#L126-L130) (`src/fs.rs:126-130`)

### Implemented Fixes
1. **Single-step `glBufferData` upload in `GlContext::new_buffer` (`src/graphics/gl.rs:1420-1428`)**:
   - Passes the slice pointer directly to `glBufferData(gl_target, size as _, ptr, gl_usage)` instead of calling `glBufferData(..., null)` followed by `glBufferSubData`.
2. **Cached readback FBO in `GlContext` for `Texture::read_pixels` (`src/graphics/gl.rs:411-448, 520, 1047`)**:
   - Lazily creates and reuses `ctx.readback_fbo` instead of calling `glGenFramebuffers` and `glDeleteFramebuffers` on every `read_pixels` call.
3. **Single-allocation file read in `load_file_desktop` (`src/fs.rs:126-130`)**:
   - Uses `std::fs::read(path)` so the destination `Vec<u8>` is preallocated to the exact file size from metadata instead of growing incrementally from an empty `vec![]`.
