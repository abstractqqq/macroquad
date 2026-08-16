//! Functions to construct immutable atlas-backed fonts and draw shaped text.

use std::{ops::Range, sync::Arc};

use swash::{
    shape::ShapeContext,
    text::{Codepoint, Script},
    FontRef,
};

use crate::{
    color::{Color, WHITE},
    file::load_file,
    get_context,
    math::vec3,
    Error,
};

pub mod font_atlas;
pub(crate) mod rasterizer;
pub mod renderer;

use renderer::TextRenderer;

use self::{
    font_atlas::{FontAtlas, FontAtlasBuilder},
    rasterizer::Rasterizer,
};

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct TextDimensions {
    pub width: f32,
    pub height: f32,
    pub offset_y: f32,
}

pub struct FontData {
    pub bytes: Arc<[u8]>,
    pub index: usize,
    pub atlas: Option<FontAtlas>,
    pub glyphs: Vec<(u16, GlyphInfo)>,
    pub raster_size: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct GlyphInfo {
    pub rect: Option<crate::math::Rect>,
    pub left: i32,
    pub top: i32,
}

/// Parameters used to construct a complete immutable font atlas.
#[derive(Debug, Clone)]
pub struct FontLoadParams {
    /// Characters whose nominal glyphs will be stored in the atlas.
    ///
    /// Setting this field replaces the default ASCII repertoire. Extend
    /// [`Font::ascii_character_list`] when the font must also render general
    /// labels, numbers, or dynamically formatted values.
    pub characters: Vec<char>,
    /// Complete strings used to collect ligatures and contextual glyph forms.
    pub texts: Vec<String>,
    /// Canonical raster size. Drawing scales these cached glyphs.
    pub raster_size: u16,
    /// Texture filtering used by the finished atlas.
    pub filter: miniquad::FilterMode,
}

impl Default for FontLoadParams {
    fn default() -> Self {
        Self {
            characters: Font::ascii_character_list(),
            texts: Vec::new(),
            raster_size: renderer::BASE_FONT_SIZE as u16,
            filter: miniquad::FilterMode::Linear,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontId(pub usize);

/// An immutable font asset.
#[derive(Clone)]
pub struct Font {
    pub data: Arc<FontData>,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Font")
            .field("backend", &"Swash")
            .field("id", &self.id())
            .finish()
    }
}

impl Font {
    pub(crate) fn load_from_bytes(
        bytes: &[u8],
        params: FontLoadParams,
        ctx: &mut dyn miniquad::RenderingBackend,
    ) -> Result<Self, Error> {
        let font = FontRef::from_index(bytes, 0).ok_or(Error::FontError(
            "Swash could not parse the supplied font data",
        ))?;
        if font.variations().len() != 0 {
            return Err(Error::FontError("Variable fonts are not supported"));
        }
        if params.raster_size == 0 {
            return Err(Error::FontError(
                "Font raster size must be greater than zero",
            ));
        }

        let mut atlas = FontAtlasBuilder::new(params.filter);
        let mut glyphs = Vec::new();
        let mut rasterizer = Rasterizer::new();
        let mut glyph_ids = Vec::with_capacity(params.characters.len() + 1);
        glyph_ids.push(0);
        for character in params.characters {
            let glyph_id = font.charmap().map(character);
            if glyph_id != 0 && !glyph_ids.contains(&glyph_id) {
                glyph_ids.push(glyph_id);
            }
        }
        let mut shape_context = ShapeContext::new();
        for text in &params.texts {
            let script = text
                .chars()
                .map(Codepoint::script)
                .find(|script| {
                    !matches!(script, Script::Common | Script::Inherited | Script::Unknown)
                })
                .unwrap_or(Script::Latin);
            let mut shaper = shape_context
                .builder(font)
                .script(script)
                .size(renderer::BASE_FONT_SIZE)
                .build();
            shaper.add_str(text);
            shaper.shape_with(|cluster| {
                for glyph in cluster.glyphs {
                    if !glyph_ids.contains(&glyph.id) {
                        glyph_ids.push(glyph.id);
                    }
                }
            });
        }
        for glyph_id in glyph_ids {
            let Some(rendered) =
                rasterizer.rasterize(bytes, 0, glyph_id, params.raster_size as f32)
            else {
                continue;
            };
            let rect = if rendered.image.width == 0 || rendered.image.height == 0 {
                None
            } else {
                Some(atlas.insert(&rendered.image).ok_or(Error::FontError(
                    "The immutable font atlas is too small for the requested characters",
                ))?)
            };
            glyphs.push((
                glyph_id,
                GlyphInfo {
                    rect,
                    left: rendered.left,
                    top: rendered.top,
                },
            ));
        }
        glyphs.sort_unstable_by_key(|(glyph_id, _)| *glyph_id);
        let atlas = atlas.finish(ctx);
        Ok(Self {
            data: Arc::new(FontData {
                bytes: Arc::from(bytes),
                index: 0,
                atlas: Some(atlas),
                glyphs,
                raster_size: params.raster_size as f32,
            }),
        })
    }

    pub(crate) fn id(&self) -> FontId {
        FontId(Arc::as_ptr(&self.data) as usize)
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.data.bytes
    }

    pub(crate) fn index(&self) -> usize {
        self.data.index
    }

    pub(crate) fn font_ref(&self) -> FontRef<'_> {
        FontRef::from_index(self.bytes(), self.index())
            .expect("font was validated when it was loaded")
    }

    pub(crate) fn atlas(&self) -> &FontAtlas {
        self.data
            .atlas
            .as_ref()
            .expect("renderable fonts always have an atlas")
    }

    pub(crate) fn glyph(&self, glyph_id: u16) -> Option<GlyphInfo> {
        self.data
            .glyphs
            .binary_search_by_key(&glyph_id, |(glyph_id, _)| *glyph_id)
            .ok()
            .or_else(|| {
                self.data
                    .glyphs
                    .binary_search_by_key(&0, |(glyph_id, _)| *glyph_id)
                    .ok()
            })
            .map(|index| self.data.glyphs[index].1)
    }

    pub(crate) fn raster_size(&self) -> f32 {
        self.data.raster_size
    }

    #[cfg(test)]
    pub(crate) fn load_for_test(bytes: &[u8]) -> Result<Self, Error> {
        FontRef::from_index(bytes, 0).ok_or(Error::FontError(
            "Swash could not parse the supplied font data",
        ))?;
        Ok(Self {
            data: Arc::new(FontData {
                bytes: Arc::from(bytes),
                index: 0,
                atlas: None,
                glyphs: Vec::new(),
                raster_size: renderer::BASE_FONT_SIZE,
            }),
        })
    }

    pub fn ascii_character_list() -> Vec<char> {
        (' '..='~').collect()
    }

    pub fn latin_character_list() -> Vec<char> {
        "qwertyuiopasdfghjklzxcvbnmQWERTYUIOPASDFGHJKLZXCVBNM1234567890!@#$%^&*(){}[].,:"
            .chars()
            .collect()
    }
}

#[cfg(test)]
mod font_tests {
    use super::Font;

    #[test]
    fn ascii_character_list_is_the_printable_ascii_repertoire() {
        let characters = Font::ascii_character_list();

        assert_eq!(characters.len(), 95);
        assert_eq!(characters.first(), Some(&' '));
        assert_eq!(characters.last(), Some(&'~'));
    }
}

impl Default for Font {
    fn default() -> Self {
        get_default_font()
    }
}

#[derive(Debug, Clone)]
pub struct TextParams<'a> {
    pub font: Option<&'a Font>,
    pub font_size: u16,
    pub font_scale: f32,
    pub font_scale_aspect: f32,
    pub rotation: f32,
    pub color: Color,
}

impl Default for TextParams<'_> {
    fn default() -> Self {
        Self {
            font: None,
            font_size: 20,
            font_scale: 1.0,
            font_scale_aspect: 1.0,
            rotation: 0.0,
            color: WHITE,
        }
    }
}

/// Parameters used to shape a reusable [`TextLayout`].
#[derive(Debug, Clone)]
pub struct TextLayoutParams<'a> {
    pub font: Option<&'a Font>,
    pub font_size: u16,
    pub font_scale: f32,
    pub font_scale_aspect: f32,
    /// Maximum rendered width. `None` disables automatic wrapping.
    pub max_width: Option<f32>,
}

impl Default for TextLayoutParams<'_> {
    fn default() -> Self {
        Self {
            font: None,
            font_size: 20,
            font_scale: 1.0,
            font_scale_aspect: 1.0,
            max_width: None,
        }
    }
}

/// A color override for a UTF-8 byte range in prepared text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextColorSpan {
    pub range: Range<usize>,
    pub color: Color,
}

/// Draw-time parameters for a reusable [`TextLayout`].
#[derive(Debug, Clone)]
pub struct TextLayoutDrawParams {
    pub rotation: f32,
    pub color: Color,
    /// Number of complete shaping clusters to reveal.
    pub visible_clusters: Option<usize>,
}

impl Default for TextLayoutDrawParams {
    fn default() -> Self {
        Self {
            rotation: 0.0,
            color: WHITE,
            visible_clusters: None,
        }
    }
}

/// Shaped and positioned text that can be drawn repeatedly without layout work.
#[derive(Clone)]
pub struct TextLayout {
    pub font: Font,
    pub shaped: Arc<renderer::ShapedText>,
    pub cluster_colors: Vec<Option<Color>>,
    pub dimensions: TextDimensions,
    pub font_size: f32,
    pub scale_x: f32,
    pub scale_y: f32,
}

impl std::fmt::Debug for TextLayout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextLayout")
            .field("font", &self.font)
            .field("dimensions", &self.dimensions)
            .field("clusters", &self.cluster_count())
            .finish()
    }
}

impl TextLayout {
    pub fn dimensions(&self) -> TextDimensions {
        self.dimensions
    }

    pub fn cluster_count(&self) -> usize {
        self.shaped.clusters.len()
    }

    /// Returns the UTF-8 byte range represented by a reveal cluster.
    pub fn cluster_range(&self, index: usize) -> Option<Range<usize>> {
        self.shaped
            .clusters
            .get(index)
            .map(|cluster| cluster.source.clone())
    }
}

/// Shapes text once for repeated measurement and drawing.
///
/// Color spans do not affect shaping. When spans overlap, the last matching
/// span wins.
pub fn prepare_text_layout(
    text: impl AsRef<str>,
    params: TextLayoutParams<'_>,
    color_spans: &[TextColorSpan],
) -> TextLayout {
    let text = text.as_ref();
    let context = get_context();
    let font = params
        .font
        .cloned()
        .unwrap_or_else(|| context.fonts_storage.default_font.clone());
    let scale_x = params.font_scale * params.font_scale_aspect;
    let scale_y = params.font_scale;
    let base_scale = params.font_size as f32 / renderer::BASE_FONT_SIZE;
    let max_width = params.max_width.map(|width| width / (base_scale * scale_x));
    let shaped = context.text_renderer.shape(&font, text, max_width);
    let cluster_colors = shaped
        .clusters
        .iter()
        .map(|cluster| {
            color_spans
                .iter()
                .rev()
                .find(|span| {
                    span.range.start < cluster.source.end && cluster.source.start < span.range.end
                })
                .map(|span| span.color)
        })
        .collect();
    let dimensions = TextDimensions {
        width: shaped.dimensions.width * base_scale * scale_x,
        height: shaped.dimensions.height * base_scale * scale_y,
        offset_y: shaped.dimensions.offset_y * base_scale * scale_y,
    };
    TextLayout {
        font,
        shaped,
        cluster_colors,
        dimensions,
        font_size: params.font_size as f32,
        scale_x,
        scale_y,
    }
}

/// Draws all clusters in a prepared layout.
pub fn draw_text_layout(layout: &TextLayout, x: f32, y: f32, color: Color) -> TextDimensions {
    draw_text_layout_ex(
        layout,
        x,
        y,
        TextLayoutDrawParams {
            color,
            ..Default::default()
        },
    )
}

/// Draws a prepared layout, optionally limiting it to a typewriter prefix.
pub fn draw_text_layout_ex(
    layout: &TextLayout,
    x: f32,
    y: f32,
    params: TextLayoutDrawParams,
) -> TextDimensions {
    let context = get_context();
    context.text_renderer.draw_shaped(
        &layout.font,
        &layout.shaped,
        x,
        y,
        layout.font_size,
        layout.scale_x,
        layout.scale_y,
        params.rotation,
        params.color,
        Some(&layout.cluster_colors),
        params
            .visible_clusters
            .unwrap_or_else(|| layout.cluster_count()),
        &mut context.gl,
        &mut *context.quad_context,
    )
}

pub async fn load_ttf_font(path: &str) -> Result<Font, Error> {
    load_ttf_font_ex(path, FontLoadParams::default()).await
}

pub async fn load_ttf_font_ex(path: &str, params: FontLoadParams) -> Result<Font, Error> {
    let bytes = load_file(path)
        .await
        .map_err(|_| Error::FontError("The Font file couldn't be loaded"))?;
    load_ttf_font_from_bytes_ex(&bytes, params)
}

pub fn load_ttf_font_from_bytes(bytes: &[u8]) -> Result<Font, Error> {
    load_ttf_font_from_bytes_ex(bytes, FontLoadParams::default())
}

pub fn load_ttf_font_from_bytes_ex(bytes: &[u8], params: FontLoadParams) -> Result<Font, Error> {
    let context = get_context();
    Font::load_from_bytes(bytes, params, &mut *context.quad_context)
}

pub fn load_default_font() -> Result<Font, Error> {
    load_ttf_font_from_bytes(include_bytes!("../../assets/fonts/ProggyClean.ttf"))
}

pub fn draw_text(
    text: impl AsRef<str>,
    x: f32,
    y: f32,
    font_size: f32,
    color: Color,
) -> TextDimensions {
    draw_text_ex(
        text,
        x,
        y,
        TextParams {
            font_size: font_size as u16,
            color,
            ..Default::default()
        },
    )
}

pub fn draw_text_ex(
    text: impl AsRef<str>,
    x: f32,
    y: f32,
    params: TextParams<'_>,
) -> TextDimensions {
    let text = text.as_ref();
    if text.is_empty() {
        return TextDimensions::default();
    }
    let context = get_context();
    let font = params
        .font
        .cloned()
        .unwrap_or_else(|| context.fonts_storage.default_font.clone());
    context.text_renderer.draw(
        &font,
        text,
        x,
        y,
        params.font_size as f32,
        params.font_scale * params.font_scale_aspect,
        params.font_scale,
        params.rotation,
        params.color,
        &mut context.gl,
        &mut *context.quad_context,
    )
}

pub fn measure_text(
    text: impl AsRef<str>,
    font: Option<&Font>,
    font_size: u16,
    font_scale: f32,
) -> TextDimensions {
    let context = get_context();
    let font = font
        .cloned()
        .unwrap_or_else(|| context.fonts_storage.default_font.clone());
    context.text_renderer.measure(
        &font,
        text.as_ref(),
        font_size as f32,
        font_scale,
        font_scale,
    )
}

pub fn measure_multiline_text(
    text: &str,
    font: Option<&Font>,
    font_size: u16,
    font_scale: f32,
    line_distance_factor: Option<f32>,
) -> TextDimensions {
    if line_distance_factor.is_none() {
        return measure_text(text, font, font_size, font_scale);
    }
    let line_advance = line_distance_factor.unwrap() * font_size as f32 * font_scale;
    let mut dimensions = TextDimensions::default();
    for (index, line) in text.split('\n').enumerate() {
        let line_dimensions = measure_text(line, font, font_size, font_scale);
        dimensions.width = dimensions.width.max(line_dimensions.width);
        dimensions.offset_y = dimensions.offset_y.max(line_dimensions.offset_y);
        dimensions.height = line_dimensions.height + index as f32 * line_advance;
    }
    dimensions
}

pub fn draw_multiline_text(
    text: impl AsRef<str>,
    x: f32,
    y: f32,
    font_size: f32,
    line_distance_factor: Option<f32>,
    color: Color,
) -> TextDimensions {
    draw_multiline_text_ex(
        text,
        x,
        y,
        line_distance_factor,
        TextParams {
            font_size: font_size as u16,
            color,
            ..Default::default()
        },
    )
}

pub fn draw_multiline_text_ex(
    text: impl AsRef<str>,
    mut x: f32,
    mut y: f32,
    line_distance_factor: Option<f32>,
    params: TextParams<'_>,
) -> TextDimensions {
    let text = text.as_ref();
    if line_distance_factor.is_none() {
        return draw_text_ex(text, x, y, params);
    }
    let line_advance = line_distance_factor.unwrap() * params.font_size as f32 * params.font_scale;
    let mut dimensions = TextDimensions::default();
    for (index, line) in text.split('\n').enumerate() {
        let line_dimensions = draw_text_ex(line, x, y, params.clone());
        dimensions.width = dimensions.width.max(line_dimensions.width);
        dimensions.offset_y = dimensions.offset_y.max(line_dimensions.offset_y);
        dimensions.height = line_dimensions.height + index as f32 * line_advance;
        x -= line_advance * params.rotation.sin();
        y += line_advance * params.rotation.cos();
    }
    dimensions
}

pub fn wrap_text(
    text: &str,
    font: Option<&Font>,
    font_size: u16,
    font_scale: f32,
    maximum_line_length: f32,
) -> String {
    let mut output = String::with_capacity(text.len());
    let mut line = String::new();
    for character in text.chars() {
        if character == '\n' {
            output.push_str(&line);
            output.push('\n');
            line.clear();
            continue;
        }
        line.push(character);
        if measure_text(&line, font, font_size, font_scale).width > maximum_line_length {
            line.pop();
            if !line.is_empty() {
                output.push_str(&line);
                output.push('\n');
                line.clear();
            }
            line.push(character);
        }
    }
    output.push_str(&line);
    output
}

pub fn get_text_center(
    text: impl AsRef<str>,
    font: Option<&Font>,
    font_size: u16,
    font_scale: f32,
    rotation: f32,
) -> crate::Vec2 {
    let measure = measure_text(text, font, font_size, font_scale);
    crate::Vec2::new(
        measure.width / 2.0 * rotation.cos() + measure.height / 2.0 * rotation.sin(),
        measure.width / 2.0 * rotation.sin() - measure.height / 2.0 * rotation.cos(),
    )
}

pub(crate) struct FontsStorage {
    default_font: Font,
}

impl FontsStorage {
    pub(crate) fn new(
        ctx: &mut dyn miniquad::RenderingBackend,
        _text_renderer: &mut TextRenderer,
        filter: miniquad::FilterMode,
    ) -> Self {
        let default_font = Font::load_from_bytes(
            include_bytes!("../../assets/fonts/ProggyClean.ttf"),
            FontLoadParams {
                filter,
                ..Default::default()
            },
            ctx,
        )
        .unwrap();
        Self { default_font }
    }
}

pub fn get_default_font() -> Font {
    get_context().fonts_storage.default_font.clone()
}

pub fn set_default_font(font: Font) {
    get_context().fonts_storage.default_font = font;
}

pub fn camera_font_scale(world_font_size: f32) -> (u16, f32, f32) {
    let context = get_context();
    let (screen_width, screen_height) = miniquad::window::screen_size();
    let camera_space = context
        .projection_matrix()
        .inverse()
        .transform_vector3(vec3(2.0, 2.0, 0.0));
    let (camera_width, camera_height) = (camera_space.x.abs(), camera_space.y.abs());
    let screen_font_size = world_font_size * screen_height / camera_height;
    (
        screen_font_size as u16,
        camera_height / screen_height,
        screen_height / screen_width * camera_width / camera_height,
    )
}

#[allow(dead_code)]
fn require_font_send() {
    fn require_send<T: Send>() {}
    require_send::<Font>();
}
