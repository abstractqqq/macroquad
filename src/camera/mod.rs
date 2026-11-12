//! 2D and 3D camera.

use crate::{
    get_context,
    math::Rect,
    prelude::RenderPass,
    texture::{Image, RenderTarget},
    window::{screen_height, screen_width},
};
#[cfg(feature = "3d")]
use glam::Vec3;
use glam::{vec2, Mat4, Vec2};
#[cfg(feature = "3d")]
use glam::vec3;

pub trait Camera {
    fn matrix(&self) -> Mat4;
    fn depth_enabled(&self) -> bool;
    fn render_pass(&self) -> Option<RenderPass>;
    fn viewport(&self) -> Option<(i32, i32, i32, i32)>;
}

#[derive(Debug)]
pub struct Camera2D {
    /// Rotation in degrees.
    pub rotation: f32,
    /// Scaling, should be (1.0, 1.0) by default.
    pub zoom: Vec2,
    /// Rotation and zoom origin.
    pub target: Vec2,
    /// Displacement from target.
    pub offset: Vec2,

    /// If "render_target" is set - camera will render to texture.
    ///
    /// Otherwise to the screen.
    pub render_target: Option<RenderTarget>,

    /// Part of the screen to render to.
    ///
    /// None means the whole screen.
    ///
    /// Viewport do not affect camera space, just the render position on the screen.
    ///
    /// Useful for things like splitscreen.
    pub viewport: Option<(i32, i32, i32, i32)>,
}

impl Camera2D {
    /// Will make camera space equals given rect, y-down: the rect's top-left corner is at the
    /// top-left of the screen, or of the render target.
    ///
    /// The same rect works for both: a render target drawn with this camera is stored upright,
    /// so its texture can be drawn without `flip_y`.
    pub fn from_display_rect(rect: Rect) -> Camera2D {
        let target = vec2(rect.x + rect.w / 2., rect.y + rect.h / 2.);

        Camera2D {
            target,
            zoom: vec2(1. / rect.w * 2., 1. / rect.h * 2.),
            offset: vec2(0., 0.),
            rotation: 0.,

            render_target: None,
            viewport: None,
        }
    }
}

impl Default for Camera2D {
    fn default() -> Camera2D {
        Camera2D {
            zoom: vec2(1., 1.),
            offset: vec2(0., 0.),
            target: vec2(0., 0.),
            rotation: 0.,

            render_target: None,
            viewport: None,
        }
    }
}

impl Camera2D {
    /// The linear part of the camera transform: `(zoom_x, zoom_y, cos, sin)`.
    ///
    /// Zoom Y includes the flip applied when drawing to the screen (render targets are
    /// already upside down, so they are not flipped).
    ///
    /// KEEP IN SYNC: `matrix()`, `world_to_ndc()` and `ndc_to_world()` are closed-form
    /// versions of the same transform and must agree with each other. If you change
    /// one of them, change the others (a unit test checks this).
    fn basis(&self) -> (f32, f32, f32, f32) {
        let invert_y = if self.render_target.is_some() { 1.0 } else { -1.0 };
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        (self.zoom.x, self.zoom.y * invert_y, cos, sin)
    }

    /// World position to clip space. Same transform as `matrix()`.
    fn world_to_ndc(&self, point: Vec2) -> Vec2 {
        let (zx, zy, c, s) = self.basis();
        let d = point - self.target;
        vec2(
            zx * (c * d.x - s * d.y) + self.offset.x,
            zy * (s * d.x + c * d.y) + self.offset.y,
        )
    }

    /// Clip space to world position. Inverse of `world_to_ndc()`: undo the offset,
    /// undo the zoom, rotate back, then add the target. A zoom of zero on either
    /// axis is not invertible.
    fn ndc_to_world(&self, ndc: Vec2) -> Vec2 {
        let (zx, zy, c, s) = self.basis();
        let a = vec2((ndc.x - self.offset.x) / zx, (ndc.y - self.offset.y) / zy);
        vec2(
            c * a.x + s * a.y + self.target.x,
            -s * a.x + c * a.y + self.target.y,
        )
    }

    /// Screen rectangle (window space, y down) that this camera renders to.
    fn screen_rect(&self) -> Rect {
        match self.viewport() {
            Some((vx, vy, vw, vh)) => Rect {
                x: vx as f32,
                y: screen_height() - (vy + vh) as f32,
                w: vw as f32,
                h: vh as f32,
            },
            None => Rect {
                x: 0.0,
                y: 0.0,
                w: screen_width(),
                h: screen_height(),
            },
        }
    }
}

impl Camera for Camera2D {
    /// World space to clip space (NDC):
    ///
    /// `ndc = offset + S * R * (world - target)`
    ///
    /// where `R` is the rotation and `S = diag(zoom.x, zoom.y * flip)`. This is the same
    /// as `T(offset) * S * R * T(-target)`, written out by hand so no matrix multiplies
    /// are needed. Follows raylib's `GetCameraMatrix2D` except that there is no separate
    /// orthographic projection, so `offset` is in NDC units, not pixels.
    ///
    /// KEEP IN SYNC with `basis()`, `world_to_ndc()` and `ndc_to_world()`.
    fn matrix(&self) -> Mat4 {
        let (zx, zy, c, s) = self.basis();
        let (tx, ty) = (self.target.x, self.target.y);

        Mat4::from_cols(
            glam::vec4(zx * c, zy * s, 0.0, 0.0),
            glam::vec4(-zx * s, zy * c, 0.0, 0.0),
            glam::vec4(0.0, 0.0, 1.0, 0.0),
            glam::vec4(
                self.offset.x - zx * (c * tx - s * ty),
                self.offset.y - zy * (s * tx + c * ty),
                0.0,
                1.0,
            ),
        )
    }

    fn depth_enabled(&self) -> bool {
        false
    }

    fn render_pass(&self) -> Option<RenderPass> {
        self.render_target.as_ref().map(|rt| rt.render_pass.clone())
    }

    fn viewport(&self) -> Option<(i32, i32, i32, i32)> {
        self.viewport
    }
}

impl Camera2D {
    /// Returns the screen space position for a 2d camera world space position.
    ///
    /// Screen position in window space - from (0, 0) to (screen_width, screen_height()).
    /// When a viewport is set, returns coordinates within that viewport's screen region.
    pub fn world_to_screen(&self, point: Vec2) -> Vec2 {
        let dims = self.screen_rect();
        let ndc = self.world_to_ndc(point);

        vec2(
            (ndc.x / 2. + 0.5) * dims.w + dims.x,
            (0.5 - ndc.y / 2.) * dims.h + dims.y,
        )
    }

    /// Returns the world space position for a 2d camera screen space position.
    ///
    /// Point is a screen space position, often mouse x and y.
    pub fn screen_to_world(&self, point: Vec2) -> Vec2 {
        let dims = self.screen_rect();
        let ndc = vec2(
            (point.x - dims.x) / dims.w * 2. - 1.,
            1. - (point.y - dims.y) / dims.h * 2.,
        );

        self.ndc_to_world(ndc)
    }
}

#[cfg(feature = "3d")]
#[derive(Debug, Clone, Copy)]
pub enum Projection {
    Perspective,
    Orthographics,
}

#[cfg(feature = "3d")]
#[derive(Debug)]
pub struct Camera3D {
    /// Camera position.
    pub position: Vec3,
    /// Camera target it looks-at.
    pub target: Vec3,
    /// Camera up vector (rotation over its axis).
    pub up: Vec3,
    /// Camera field-of-view aperture in Y (radians)
    /// in perspective, used as near plane width in orthographic.
    pub fovy: f32,
    /// Screen aspect ratio.
    ///
    /// By default aspect is calculated with screen_width() / screen_height() on each frame.
    pub aspect: Option<f32>,
    /// Camera projection type, perspective or orthographics.
    pub projection: Projection,

    /// If "render_target" is set - camera will render to texture.
    ///
    /// Otherwise to the screen.
    pub render_target: Option<RenderTarget>,

    /// Part of the screen to render to.
    ///
    /// None means the whole screen.
    ///
    /// Viewport do not affect camera space, just the render position on the screen.
    ///
    /// Useful for things like splitscreen.
    pub viewport: Option<(i32, i32, i32, i32)>,

    /// Camera near plane
    pub z_near: f32,
    /// Camera far plane
    pub z_far: f32,
}

#[cfg(feature = "3d")]
impl Default for Camera3D {
    fn default() -> Camera3D {
        Camera3D {
            position: vec3(0., -10., 0.),
            target: vec3(0., 0., 0.),
            aspect: None,
            up: vec3(0., 0., 1.),
            fovy: 45.0_f32.to_radians(),
            projection: Projection::Perspective,
            render_target: None,
            viewport: None,
            z_near: 0.01,
            z_far: 10000.0,
        }
    }
}

#[cfg(feature = "3d")]
impl Camera for Camera3D {
    fn matrix(&self) -> Mat4 {
        let (width, height) = if let Some(rt) = &self.render_target {
            (rt.texture.width(), rt.texture.height())
        } else {
            (screen_width(), screen_height())
        };
        let aspect = self.aspect.unwrap_or(width / height);

        match self.projection {
            Projection::Perspective => {
                glam::camera::rh::proj::opengl::perspective(self.fovy, aspect, self.z_near, self.z_far)
                    * glam::camera::rh::view::look_at_mat4(self.position, self.target, self.up)
            }
            Projection::Orthographics => {
                let top = self.fovy / 2.0;
                let right = top * aspect;

                glam::camera::rh::proj::opengl::orthographic(-right, right, -top, top, self.z_near, self.z_far)
                    * glam::camera::rh::view::look_at_mat4(self.position, self.target, self.up)
            }
        }
    }

    fn depth_enabled(&self) -> bool {
        true
    }

    fn render_pass(&self) -> Option<RenderPass> {
        self.render_target.as_ref().map(|rt| rt.render_pass.clone())
    }

    fn viewport(&self) -> Option<(i32, i32, i32, i32)> {
        self.viewport
    }
}

/// Set active 2D or 3D camera.
pub fn set_camera(camera: &dyn Camera) {
    let context = get_context();

    // flush previous camera draw calls
    context.perform_render_passes();

    context
        .gl
        .render_pass(camera.render_pass().map(|rt| rt.raw_miniquad_id()));

    context.gl.viewport(camera.viewport());
    context.gl.depth_test(camera.depth_enabled());
    context.camera_matrix = Some(camera.matrix());
}

/// Reset default 2D camera mode.
pub fn set_default_camera() {
    let context = get_context();

    // flush previous camera draw calls
    context.perform_render_passes();

    context.gl.render_pass(None);
    context.gl.viewport(None);
    context.gl.depth_test(false);
    context.camera_matrix = None;
}

/// Runs `draw` with `camera` rendering into `target`, then reads the target back as an [`Image`].
///
/// Camera state (camera, viewport, render pass) is saved before and restored after, so this
/// can be called anywhere, including inside another camera. `camera.render_target` is
/// replaced by `target`.
///
/// The returned image is the target's texture data. With a y-down camera such as
/// [`Camera2D::from_display_rect`], row 0 is the top.
///
/// This is a blocking GPU readback: fine for thumbnails or save-game previews, too slow to
/// run every frame.
///
/// It is only a convenience. The same thing can be done procedurally:
///
/// ```ignore
/// push_camera_state();
/// set_camera(&Camera2D { render_target: Some(target.clone()), ..camera });
/// draw_scene();
/// pop_camera_state(); // flushes the draw calls into the target
/// let image = target.texture.get_texture_data();
/// ```
pub fn render_to_image(target: &RenderTarget, camera: Camera2D, draw: impl FnOnce()) -> Image {
    push_camera_state();
    set_camera(&Camera2D {
        render_target: Some(target.clone()),
        ..camera
    });
    draw();
    pop_camera_state();

    target.texture.get_texture_data()
}

pub(crate) struct CameraState {
    render_pass: Option<miniquad::RenderPass>,
    depth_test: bool,
    viewport: Option<(i32, i32, i32, i32)>,
    matrix: Option<Mat4>,
}

pub fn push_camera_state() {
    let context = get_context();

    let camera_state = CameraState {
        render_pass: context.gl.get_active_render_pass(),
        depth_test: context.gl.is_depth_test_enabled(),
        viewport: context.gl.get_viewport_override(),
        matrix: context.camera_matrix,
    };
    context.camera_stack.push(camera_state);
}

pub fn pop_camera_state() {
    let context = get_context();

    if let Some(camera_state) = context.camera_stack.pop() {
        context.perform_render_passes();

        context.gl.render_pass(camera_state.render_pass);
        context.gl.viewport(camera_state.viewport);
        context.gl.depth_test(camera_state.depth_test);
        context.camera_matrix = camera_state.matrix;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::vec3;

    /// The original, straightforward construction (raylib's GetCameraMatrix2D).
    fn reference_matrix(c: &Camera2D) -> Mat4 {
        let invert_y = if c.render_target.is_some() { 1.0 } else { -1.0 };
        Mat4::from_translation(vec3(c.offset.x, c.offset.y, 0.0))
            * (Mat4::from_scale(vec3(c.zoom.x, c.zoom.y * invert_y, 1.0))
                * Mat4::from_axis_angle(vec3(0., 0., 1.), c.rotation.to_radians())
                * Mat4::from_translation(vec3(-c.target.x, -c.target.y, 0.0)))
    }

    #[test]
    fn closed_form_matches_matrix_and_roundtrips() {
        let cam = Camera2D {
            rotation: 37.0,
            zoom: vec2(0.004, -0.0025),
            target: vec2(120.0, -45.0),
            offset: vec2(0.1, -0.2),
            ..Default::default()
        };
        assert!(cam.matrix().abs_diff_eq(reference_matrix(&cam), 1e-5));

        for p in [vec2(0., 0.), vec2(300., 200.), vec2(-50., 75.)] {
            let ndc = cam.world_to_ndc(p);
            let via_matrix = cam.matrix().transform_point3(vec3(p.x, p.y, 0.));
            assert!((ndc - vec2(via_matrix.x, via_matrix.y)).length() < 1e-4);
            assert!((cam.ndc_to_world(ndc) - p).length() < 1e-2);
        }
    }
}
