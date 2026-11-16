//! SDF fonts: one atlas, sharp at any size, scale and rotation, with game-side outlines.
//!
//! Rayquad bakes and loads SDF atlases but does not ship the shader. The game owns it, so it
//! can add outlines, shadows or glows however it likes. This example contains a reference
//! shader. The atlas contract it relies on:
//!
//! - The distance is in the alpha channel. `0.5` is the glyph edge, higher is inside.
//! - The value changes by `0.5 / spread` per atlas texel, where `spread` comes from
//!   `FontRenderMode::Sdf { spread }` (see `Font::render_mode`).
//! - Vertex color is the text color, as with bitmap fonts.
//!
//! For shipping, bake once with `FontData::bake_from_ttf_bytes` + `serialize_baked`
//! (see `examples/bake_font.rs`) and load with `load_baked_font_from_bytes`. This example
//! bakes at startup to stay self-contained.

use rayquad::prelude::*;
use rayquad::window::miniquad::*;

const VERTEX: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;

varying lowp vec2 uv;
varying lowp vec4 color;

uniform mat4 Model;
uniform mat4 Projection;

void main() {
    gl_Position = Projection * Model * vec4(position, 1);
    color = color0 / 255.0;
    uv = texcoord;
}"#;

const FRAGMENT: &str = r#"#version 100
#extension GL_OES_standard_derivatives : enable
precision mediump float;

varying lowp vec4 color;
varying lowp vec2 uv;

uniform sampler2D Texture;
// Drawn behind the fill. Alpha 0 disables the outline.
uniform vec4 outline_color;
// Outline thickness in distance units: `texels * 0.5 / spread`. Must stay below 0.5.
uniform float outline_width;

void main() {
    float d = texture2D(Texture, uv).a;
    // Distance change across one screen pixel: keeps edges about 1 px soft at any scale.
    float w = fwidth(d) * 0.5;
    float fill = smoothstep(0.5 - w, 0.5 + w, d) * color.a;
    float edge = 0.5 - outline_width;
    float outline = smoothstep(edge - w, edge + w, d) * outline_color.a * (1.0 - fill);
    float alpha = fill + outline;
    vec3 rgb = (color.rgb * fill + outline_color.rgb * outline) / max(alpha, 0.0001);
    gl_FragColor = vec4(rgb, alpha);
}"#;

const METAL: &str = r#"
#include <metal_stdlib>
using namespace metal;

// Same layout as the default material: Model, Projection, _Time, then custom uniforms.
struct Uniforms
{
    float4x4 Model;
    float4x4 Projection;
    float4 _Time;
    float4 outline_color;
    float outline_width;
};

struct Vertex
{
    float3 position    [[attribute(0)]];
    float2 texcoord    [[attribute(1)]];
    float4 color0      [[attribute(2)]];
};

struct RasterizerData
{
    float4 position [[position]];
    float4 color [[user(locn0)]];
    float2 uv [[user(locn1)]];
};

vertex RasterizerData vertexShader(Vertex v [[stage_in]], constant Uniforms& u [[buffer(0)]])
{
    RasterizerData out;
    out.position = u.Model * u.Projection * float4(v.position, 1);
    out.color = v.color0 / 255.0;
    out.uv = v.texcoord;
    return out;
}

fragment float4 fragmentShader(RasterizerData in [[stage_in]],
                               constant Uniforms& u [[buffer(0)]],
                               texture2d<float> tex [[texture(0)]],
                               sampler texSmplr [[sampler(0)]])
{
    float d = tex.sample(texSmplr, in.uv).a;
    float w = fwidth(d) * 0.5;
    float fill = smoothstep(0.5 - w, 0.5 + w, d) * in.color.a;
    float edge = 0.5 - u.outline_width;
    float outline = smoothstep(edge - w, edge + w, d) * u.outline_color.a * (1.0 - fill);
    float alpha = fill + outline;
    float3 rgb = (in.color.rgb * fill + u.outline_color.rgb * outline) / max(alpha, 0.0001);
    return float4(rgb, alpha);
}
"#;

const SPREAD: u8 = 6;
const LATIN: &str = "Sharp at any size";
const DIALOG: &str = "你好，旅行者。今天的风很大。";

fn load_font(
    path: &str,
    characters: Vec<char>,
    raster_size: u16,
    render_mode: FontRenderMode,
) -> Font {
    let bytes = std::fs::read(path).expect("run from the repository root");
    load_ttf_font_from_bytes_ex(
        &bytes,
        FontLoadParams {
            characters,
            raster_size,
            render_mode,
            ..Default::default()
        },
    )
    .unwrap()
}

fn sdf_material() -> Material {
    let ctx = unsafe { get_internal_gl().quad_context };
    load_material(
        match ctx.info().backend {
            Backend::OpenGl => ShaderSource::Glsl {
                vertex: VERTEX,
                fragment: FRAGMENT,
            },
            Backend::Metal => ShaderSource::Msl { program: METAL },
        },
        MaterialParams {
            uniforms: vec![
                UniformDesc::new("outline_color", UniformType::Float4),
                UniformDesc::new("outline_width", UniformType::Float1),
            ],
            pipeline_params: PipelineParams {
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::Value(BlendValue::SourceAlpha),
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap()
}

/// Selects the SDF material with an optional outline (`texels` thick at the raster size).
fn use_sdf(material: &Material, outline: Option<(Color, f32)>) {
    gl_use_material(material);
    let (color, texels) = outline.unwrap_or((Color::new(0.0, 0.0, 0.0, 0.0), 0.0));
    material.set_uniform("outline_color", [color.r, color.g, color.b, color.a]);
    material.set_uniform("outline_width", texels * 0.5 / f32::from(SPREAD));
}

fn text(font: &Font, s: &str, x: f32, y: f32, size: u16, rotation: f32, color: Color) {
    draw_text_ex(
        s,
        x,
        y,
        TextParams {
            font: Some(font),
            font_size: size,
            rotation,
            color,
            ..Default::default()
        },
    );
}

fn main() {
    rayquad::Window::from_config(
        Conf {
            window_title: "SDF text".to_owned(),
            window_width: 1300,
            window_height: 860,
            ..Default::default()
        },
        game(),
    );
}

async fn game() {
    let latin_chars = Font::ascii_character_list();
    let sdf_mode = FontRenderMode::Sdf { spread: SPREAD };
    let latin_bitmap = load_font(
        "examples/DancingScriptRegular.ttf",
        latin_chars.clone(),
        48,
        FontRenderMode::Bitmap,
    );
    // A single-channel SDF cannot represent strokes thinner than about one atlas texel.
    // This script font has hairlines, so it is baked larger. CJK body text is fine at 48.
    let latin_sdf = load_font("examples/DancingScriptRegular.ttf", latin_chars, 96, sdf_mode);
    let cjk_sdf = load_font("examples/chinese.ttf", DIALOG.chars().collect(), 48, sdf_mode);
    let material = sdf_material();

    loop {
        clear_background(Color::from_rgba(40, 44, 52, 255));
        draw_text(
            "Left: bitmap atlas baked at 48 px. Right: SDF atlas baked at 96 px.",
            20.0,
            30.0,
            20.0,
            LIGHTGRAY,
        );

        let mut y = 70.0;
        for size in [12u16, 16, 24, 48, 96] {
            y += f32::from(size) * 1.1;
            text(&latin_bitmap, LATIN, 20.0, y, size, 0.0, WHITE);
            use_sdf(&material, None);
            text(&latin_sdf, LATIN, 660.0, y, size, 0.0, WHITE);
            gl_use_default_material();
        }

        // Game-side effects: the outline is just a second threshold in the shader.
        use_sdf(&material, Some((BLACK, 2.0)));
        text(&cjk_sdf, DIALOG, 20.0, y + 90.0, 40, 0.0, YELLOW);
        use_sdf(&material, Some((Color::from_rgba(200, 40, 80, 255), 3.0)));
        text(&latin_sdf, "Outlined", 20.0, y + 170.0, 64, 0.0, WHITE);
        use_sdf(&material, None);
        text(
            &latin_sdf,
            "Rotated",
            900.0,
            y + 150.0,
            56,
            get_time() as f32 * 0.5,
            SKYBLUE,
        );
        gl_use_default_material();

        next_frame().await;
    }
}
