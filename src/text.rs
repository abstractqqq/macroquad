//! Functions to load immutable fonts, warm text resources, and draw shaped text.

use std::{ops::Range, sync::Arc};

use swash::FontRef;

use crate::{
    color::{Color, WHITE},
    file::load_file,
    get_context,
    math::vec3,
    Error,
};

pub(crate) mod atlas;
pub(crate) mod rasterizer;
pub(crate) mod renderer;

use renderer::TextRenderer;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct TextDimensions {
    pub width: f32,
    pub height: f32,
    pub offset_y: f32,
}

struct FontData {
    bytes: Arc<[u8]>,
    index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FontId(usize);

/// An immutable font asset.
#[derive(Clone)]
pub struct Font {
    data: Arc<FontData>,
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
    pub(crate) fn load_from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let font = FontRef::from_index(bytes, 0).ok_or(Error::FontError(
            "Swash could not parse the supplied font data",
        ))?;
        if font.variations().len() != 0 {
            return Err(Error::FontError("Variable fonts are not supported"));
        }
        Ok(Self {
            data: Arc::new(FontData {
                bytes: Arc::from(bytes),
                index: 0,
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

    pub(crate) fn ascent(&self, font_size: f32) -> f32 {
        self.font_ref().metrics(&[]).scale(font_size).ascent
    }

    pub(crate) fn descent(&self, font_size: f32) -> f32 {
        -self.font_ref().metrics(&[]).scale(font_size).descent
    }

    pub fn ascii_character_list() -> Vec<char> {
        (0..255).filter_map(char::from_u32).collect()
    }

    pub fn latin_character_list() -> Vec<char> {
        "qwertyuiopasdfghjklzxcvbnmQWERTYUIOPASDFGHJKLZXCVBNM1234567890!@#$%^&*(){}[].,:"
            .chars()
            .collect()
    }

    /// Warms nominal glyphs for compatibility with the previous API.
    ///
    /// The size is ignored because the immutable atlas uses a single base
    /// raster size and scales it while drawing.
    pub fn populate_font_cache(&self, characters: &[char], _size: u16) {
        warm_font_cache(self, characters);
    }

    pub fn set_filter(&mut self, filter: miniquad::FilterMode) {
        let context = get_context();
        context
            .text_renderer
            .set_filter(self, &mut *context.quad_context, filter);
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
    font: Font,
    shaped: Arc<renderer::ShapedText>,
    cluster_colors: Vec<Option<Color>>,
    dimensions: TextDimensions,
    font_size: f32,
    scale_x: f32,
    scale_y: f32,
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
    let bytes = load_file(path)
        .await
        .map_err(|_| Error::FontError("The Font file couldn't be loaded"))?;
    load_ttf_font_from_bytes(&bytes)
}

pub fn load_ttf_font_from_bytes(bytes: &[u8]) -> Result<Font, Error> {
    let font = Font::load_from_bytes(bytes)?;
    let context = get_context();
    context.text_renderer.register_font(
        &font,
        &mut *context.quad_context,
        context.default_filter_mode,
    );
    Ok(font)
}

pub fn load_default_font() -> Result<Font, Error> {
    load_ttf_font_from_bytes(include_bytes!("ProggyClean.ttf"))
}

/// Rasterizes nominal glyphs for a known character set into the font atlas.
pub fn warm_font_cache(font: &Font, characters: &[char]) {
    let context = get_context();
    context.text_renderer.warm_characters(font, characters);
}

/// Shapes text and rasterizes every glyph produced by that text.
pub fn warm_text_cache(font: &Font, text: impl AsRef<str>) {
    let context = get_context();
    context.text_renderer.warm_text(font, text.as_ref());
}

/// Warms a collection of complete strings, including ligature glyphs.
pub fn warm_texts_cache<'a>(font: &Font, texts: impl IntoIterator<Item = &'a str>) {
    let context = get_context();
    for text in texts {
        context.text_renderer.warm_text(font, text);
    }
}

/// Uploads pending atlas changes and prevents new glyph insertion.
pub fn freeze_font_cache(font: &Font) {
    let context = get_context();
    context
        .text_renderer
        .freeze(font, &mut *context.quad_context);
}

pub fn is_font_cache_frozen(font: &Font) -> bool {
    get_context().text_renderer.is_frozen(font)
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
        text_renderer: &mut TextRenderer,
        filter: miniquad::FilterMode,
    ) -> Self {
        let default_font = Font::load_from_bytes(include_bytes!("ProggyClean.ttf")).unwrap();
        text_renderer.register_font(&default_font, ctx, filter);
        text_renderer.warm_characters(&default_font, &Font::ascii_character_list());
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
