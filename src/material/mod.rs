//! Custom materials - shaders, uniforms.

use crate::{get_context, quad_gl::GlPipeline, texture::Texture2D, tobytes::ToBytes, Error};
use miniquad::{PipelineParams, UniformDesc};
use std::sync::Arc;

#[derive(PartialEq)]
pub struct GlPipelineGuarded(pub GlPipeline);

impl Drop for GlPipelineGuarded {
    fn drop(&mut self) {
        get_context().gl.delete_pipeline(self.0);
    }
}

/// Material instance loaded on GPU.
#[derive(Clone, PartialEq)]
pub struct Material {
    pub pipeline: Arc<GlPipelineGuarded>,
}

impl std::fmt::Debug for Material {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Material").finish()
    }
}

impl Material {
    /// Set GPU uniform value for this material.
    /// "name" should be from "uniforms" list used for material creation.
    /// Otherwise uniform value would be silently ignored.
    pub fn set_uniform<T>(&self, name: &str, uniform: T) {
        get_context().gl.set_uniform(self.pipeline.0, name, uniform);
    }

    pub fn set_uniform_array<T: ToBytes>(&self, name: &str, uniform: &[T]) {
        get_context()
            .gl
            .set_uniform_array(self.pipeline.0, name, uniform);
    }

    pub fn set_texture(&self, name: &str, texture: Texture2D) {
        get_context().gl.set_texture(self.pipeline.0, name, texture);
    }
}

/// Params used for material loading.
/// It is not possible to change material params at runtime, so this
/// struct is used only once - at "load_material".
#[derive(Default)]
pub struct MaterialParams {
    /// miniquad pipeline configuration for this material.
    /// Things like blending, culling, depth dest
    pub pipeline_params: PipelineParams,

    /// List of custom uniforms used in this material
    pub uniforms: Vec<UniformDesc>,

    /// List of textures used in this material
    pub textures: Vec<String>,
}

/// Creates custom material
///
/// For OpenGL and Metal examples check examples/custom_material.rs.
///
/// # Default variables for OpenGL backend:
/// ## Attributes (order doesn't matter, any could be skipped):
/// ```glsl
/// attribute vec3 position;
/// attribute vec4 color0;
/// attribute vec2 texcoord;
/// ```
/// ## Uniforms (order doesn't matter, any could be skipped):
/// ```glsl
/// uniform mat4 Model;
/// uniform mat4 Projection;
/// uniform float4 _Time;
/// ```
/// ## Textures (order doesn't matter, any could be skipped):
/// ```glsl
/// uniform sampler2D Texture;
/// uniform sampler2D _ScreenTexture; // check examples/screen_texture.rs to see how it works
/// ```
///
/// # Default variables for Metal backend:
/// ## Attributes (order doesn't matter, any could be skipped, should have exact index in attribute()):
/// ```msl
/// struct Vertex
/// {
///     float3 position    [[attribute(0)]];
///     float2 texcoord    [[attribute(1)]];
///     float4 color0      [[attribute(2)]];
/// };
/// ```
/// ## Uniforms (**order matters, all fields before needed one should present**):
/// **All uniforms are in the same buffer, so order matters also for custom additional uniforms**
/// ```msl
/// struct Uniforms
/// {
///     float4x4 Model;
///     float4x4 Projection;
///     float4 _Time;
///     ...
///     additional uniforms
/// };
/// // same for vertexShader
/// // Only buffer(0) is correct here
/// fragment float4 fragmentShader(..., constant Uniforms& u [[buffer(0)]], ...) {...}
/// ```
/// ## Textures (order doesn't matter, any could be skipped, should have exact indices in `texture()` and `sampler()`):
/// ```msl
/// // same for vertexShader
/// fragment float4 fragmentShader(...,
///     texture2d<float> Texture [[texture(0)]],
///     sampler TextureSmplr [[sampler(0)]],
///     _ScreenTexture is not working for metal for now
///     ...
/// ) {...}
/// ```
///
pub fn load_material(
    shader: crate::ShaderSource,
    params: MaterialParams,
) -> Result<Material, Error> {
    let context = &mut get_context();

    let pipeline = context.gl.make_pipeline(
        &mut context.quad_context,
        shader,
        params.pipeline_params,
        params.uniforms,
        params.textures,
    )?;

    Ok(Material {
        pipeline: Arc::new(GlPipelineGuarded(pipeline)),
    })
}

/// All following rayquad rendering calls will use the given material.
pub fn gl_use_material(material: &Material) {
    get_context().gl.pipeline(Some(material.pipeline.0));
}

/// Use default rayquad material.
pub fn gl_use_default_material() {
    get_context().gl.pipeline(None);
}

#[doc(hidden)]
pub mod shaders {
    type IncludeFilename = String;
    type IncludeContent = String;

    #[derive(Debug, Clone)]
    pub struct PreprocessorConfig {
        pub includes: Vec<(IncludeFilename, IncludeContent)>,
    }
    impl Default for PreprocessorConfig {
        fn default() -> PreprocessorConfig {
            PreprocessorConfig { includes: vec![] }
        }
    }

    impl PreprocessorConfig {}

    pub fn preprocess_shader(source: &str, config: &PreprocessorConfig) -> String {
        if !source.contains("#include") {
            return source.to_owned();
        }

        let mut res = source.to_owned();
        let mut search_from = 0;

        while let Some(rel_idx) = res[search_from..].find("#include") {
            let directive_start_ix = search_from + rel_idx;
            let after_directive = &res[directive_start_ix + "#include".len()..];
            let trimmed = after_directive.trim_start_matches(' ');
            assert!(
                trimmed.starts_with('"'),
                "Expected '\"' after #include directive"
            );
            let after_open_quote = &trimmed[1..];
            let close_quote_rel = after_open_quote
                .find('"')
                .expect("Unterminated #include filename");
            let filename = &after_open_quote[..close_quote_rel];
            let directive_end_ix =
                res.len() - after_open_quote.len() + close_quote_rel + 1;

            let (_, include_content) = config
                .includes
                .iter()
                .find(|(name, _)| name == filename)
                .unwrap_or_else(|| {
                    panic!("Include file {filename} in not on \"includes\" list")
                });

            let replacement_len = include_content.len();
            res.replace_range(directive_start_ix..directive_end_ix, include_content);
            search_from = directive_start_ix + replacement_len;
        }

        res
    }

    #[test]
    fn preprocessor_test() {
        let shader_string = r#"
#version blah blah

asd
asd

#include "hello.glsl"

qwe
"#;

        let preprocessed = r#"
#version blah blah

asd
asd

iii
jjj

qwe
"#;

        let result = preprocess_shader(
            shader_string,
            &PreprocessorConfig {
                includes: vec![("hello.glsl".to_string(), "iii\njjj".to_string())],
                ..Default::default()
            },
        );

        assert_eq!(result, preprocessed);
    }
}
