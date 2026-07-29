//! Functions to load fonts, shape text with Swash, and draw text.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use foldhash::{HashMap, HashMapExt};
use glam::vec2;
use swash::{
    shape::ShapeContext,
    text::{Codepoint, Script},
    FontRef,
};

use crate::{
    color::{Color, WHITE},
    file::load_file,
    get_context, get_quad_context,
    math::vec3,
    texture::{draw_texture_ex, DrawTextureParams, Texture2D, TextureHandle},
    Error,
};

#[path = "text/atlas.rs"]
pub(crate) mod atlas;
#[path = "text/rasterizer.rs"]
mod rasterizer;

use atlas::{Atlas, SpriteKey};
use rasterizer::Rasterizer;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct TextDimensions {
    pub width: f32,
    pub height: f32,
    pub offset_y: f32,
}

#[derive(Clone)]
pub struct Font {
    inner: Arc<FontInner>,
}

struct FontInner {
    font_data: Arc<Vec<u8>>,
    font_index: usize,
    shape_context: Mutex<ShapeContext>,
    shaped: Mutex<ShapeCache>,
    rasterizer: Mutex<Rasterizer>,
    atlas: Arc<Mutex<Atlas>>,
    glyphs: Mutex<HashMap<GlyphCacheKey, GlyphInfo>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShapeCacheKey {
    text: String,
    pixel_size_bits: u32,
    max_width_bits: Option<u32>,
}

struct ShapeCache {
    entries: HashMap<ShapeCacheKey, Arc<ShapedText>>,
    insertion_order: VecDeque<ShapeCacheKey>,
}

impl ShapeCache {
    const CAPACITY: usize = 128;

    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
        }
    }

    fn insert(&mut self, key: ShapeCacheKey, shaped: Arc<ShapedText>) {
        if self.entries.contains_key(&key) {
            return;
        }
        if self.entries.len() == Self::CAPACITY {
            if let Some(oldest) = self.insertion_order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.insertion_order.push_back(key.clone());
        self.entries.insert(key, shaped);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GlyphCacheKey {
    glyph_id: u16,
    pixel_size: u16,
}

#[derive(Debug, Clone, Copy)]
struct GlyphInfo {
    sprite: Option<SpriteKey>,
    left: i32,
    top: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct CharacterInfo {
    pub offset_x: i32,
    pub offset_y: i32,
    pub advance: f32,
    pub sprite: SpriteKey,
}

#[derive(Clone)]
struct PositionedGlyph {
    glyph_id: u16,
    x: f32,
    y: f32,
    size: f32,
}

struct ShapedText {
    glyphs: Vec<PositionedGlyph>,
    dimensions: TextDimensions,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Font")
            .field("backend", &"Swash")
            .finish()
    }
}

impl Font {
    pub(crate) fn load_from_bytes(atlas: Arc<Mutex<Atlas>>, bytes: &[u8]) -> Result<Self, Error> {
        let font = FontRef::from_index(bytes, 0).ok_or(Error::FontError(
            "Swash could not parse the supplied font data",
        ))?;
        if font.variations().len() != 0 {
            return Err(Error::FontError("Variable fonts are not supported"));
        }

        Ok(Self {
            inner: Arc::new(FontInner {
                font_data: Arc::new(bytes.to_vec()),
                font_index: 0,
                shape_context: Mutex::new(ShapeContext::new()),
                shaped: Mutex::new(ShapeCache::new()),
                rasterizer: Mutex::new(Rasterizer::new()),
                atlas,
                glyphs: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn set_filter(&mut self, filter: miniquad::FilterMode) {
        self.inner.atlas.lock().unwrap().set_filter(filter);
    }

    pub fn ascii_character_list() -> Vec<char> {
        (0..255).filter_map(char::from_u32).collect()
    }

    pub fn latin_character_list() -> Vec<char> {
        "qwertyuiopasdfghjklzxcvbnmQWERTYUIOPASDFGHJKLZXCVBNM1234567890!@#$%^&*(){}[].,:"
            .chars()
            .collect()
    }

    pub fn populate_font_cache(&self, characters: &[char], font_size: u16) {
        let text: String = characters.iter().collect();
        let dpi = miniquad::window::dpi_scale();
        let pixel_size = (font_size as f32 * dpi).ceil();
        let shaped = self.shape(&text, pixel_size, None);
        for glyph in &shaped.glyphs {
            self.cache_positioned_glyph(glyph);
        }
    }

    pub(crate) fn set_atlas(&mut self, atlas: Arc<Mutex<Atlas>>) {
        self.inner = Arc::new(FontInner {
            font_data: self.inner.font_data.clone(),
            font_index: self.inner.font_index,
            shape_context: Mutex::new(ShapeContext::new()),
            shaped: Mutex::new(ShapeCache::new()),
            rasterizer: Mutex::new(Rasterizer::new()),
            atlas,
            glyphs: Mutex::new(HashMap::new()),
        });
    }

    pub(crate) fn ascent(&self, font_size: f32) -> f32 {
        FontRef::from_index(&self.inner.font_data, self.inner.font_index)
            .map(|font| font.metrics(&[]).scale(font_size).ascent)
            .unwrap_or_default()
    }

    pub(crate) fn descent(&self, font_size: f32) -> f32 {
        -FontRef::from_index(&self.inner.font_data, self.inner.font_index)
            .map(|font| font.metrics(&[]).scale(font_size).descent)
            .unwrap_or_default()
    }

    pub(crate) fn cache_glyph(&self, character: char, font_size: u16) {
        let shaped = self.shape(&character.to_string(), font_size as f32, None);
        for glyph in &shaped.glyphs {
            self.cache_positioned_glyph(glyph);
        }
    }

    pub(crate) fn get(&self, character: char, font_size: u16) -> Option<CharacterInfo> {
        let shaped = self.shape(&character.to_string(), font_size as f32, None);
        let glyph = shaped.glyphs.first()?;
        let info = self.cache_positioned_glyph(glyph);
        let sprite = info.sprite?;
        let height = self.inner.atlas.lock().unwrap().get(sprite)?.rect.h as i32;
        Some(CharacterInfo {
            offset_x: info.left,
            offset_y: info.top - height,
            advance: shaped.dimensions.width,
            sprite,
        })
    }

    pub(crate) fn measure_text(
        &self,
        text: impl AsRef<str>,
        font_size: u16,
        font_scale_x: f32,
        font_scale_y: f32,
        mut glyph_callback: impl FnMut(f32),
    ) -> TextDimensions {
        let dpi = miniquad::window::dpi_scale();
        let pixel_size = (font_size as f32 * dpi).ceil();
        for character in text.as_ref().chars() {
            let width = self
                .shape(&character.to_string(), pixel_size, None)
                .dimensions
                .width
                * font_scale_x
                / dpi;
            glyph_callback(width);
        }
        scale_dimensions(
            self.shape(text.as_ref(), pixel_size, None).dimensions,
            dpi,
            font_scale_x,
            font_scale_y,
        )
    }

    fn shape(&self, text: &str, pixel_size: f32, max_width: Option<f32>) -> Arc<ShapedText> {
        let key = ShapeCacheKey {
            text: text.to_owned(),
            pixel_size_bits: pixel_size.to_bits(),
            max_width_bits: max_width.map(f32::to_bits),
        };
        if let Some(shaped) = self.inner.shaped.lock().unwrap().entries.get(&key).cloned() {
            return shaped;
        }

        if text.is_empty() {
            let shaped = Arc::new(ShapedText {
                glyphs: Vec::new(),
                dimensions: TextDimensions::default(),
            });
            self.inner
                .shaped
                .lock()
                .unwrap()
                .insert(key, shaped.clone());
            return shaped;
        }

        // Swash shapes one uniform run at a time. Text intentionally uses
        // one script, one font and left-to-right direction per explicit line.
        let font = FontRef::from_index(&self.inner.font_data, self.inner.font_index)
            .expect("font was validated when Font was created");
        let metrics = font.metrics(&[]).scale(pixel_size);
        // Swash exposes both ascent and descent as positive distances from
        // the baseline, so they must be added to obtain the line advance.
        let line_height = metrics.ascent + metrics.descent + metrics.leading;
        let first_baseline = metrics.ascent;
        let mut shape_context = self.inner.shape_context.lock().unwrap();
        let mut width = 0.0_f32;
        let mut glyphs = Vec::new();
        let lines: Vec<_> = text.split('\n').collect();
        for (line_index, line) in lines.iter().enumerate() {
            let script = line
                .chars()
                .map(Codepoint::script)
                .find(|script| {
                    !matches!(script, Script::Common | Script::Inherited | Script::Unknown)
                })
                .unwrap_or(Script::Latin);
            let mut pen_x = 0.0;
            let baseline_y = line_index as f32 * line_height;
            let mut shaper = shape_context
                .builder(font)
                .script(script)
                .size(pixel_size)
                .build();
            shaper.add_str(line);
            shaper.shape_with(|cluster| {
                for glyph in cluster.glyphs {
                    glyphs.push(PositionedGlyph {
                        glyph_id: glyph.id,
                        x: pen_x + glyph.x,
                        y: baseline_y + glyph.y,
                        size: pixel_size,
                    });
                    pen_x += glyph.advance;
                }
            });
            width = width.max(pen_x);
        }
        let dimensions = TextDimensions {
            width,
            height: line_height * lines.len() as f32,
            offset_y: first_baseline,
        };
        let shaped = Arc::new(ShapedText { glyphs, dimensions });
        self.inner
            .shaped
            .lock()
            .unwrap()
            .insert(key, shaped.clone());
        shaped
    }

    fn cache_positioned_glyph(&self, glyph: &PositionedGlyph) -> GlyphInfo {
        let key = GlyphCacheKey {
            glyph_id: glyph.glyph_id,
            pixel_size: glyph.size.ceil() as u16,
        };
        if let Some(info) = self.inner.glyphs.lock().unwrap().get(&key).copied() {
            return info;
        }

        let rendered = self.inner.rasterizer.lock().unwrap().rasterize(
            &self.inner.font_data,
            self.inner.font_index,
            glyph.glyph_id,
            glyph.size,
        );
        let info = if let Some(rendered) = rendered {
            let sprite = if rendered.image.width == 0 || rendered.image.height == 0 {
                None
            } else {
                let mut atlas = self.inner.atlas.lock().unwrap();
                let sprite = atlas.new_unique_id();
                atlas.cache_sprite(sprite, rendered.image);
                Some(sprite)
            };
            GlyphInfo {
                sprite,
                left: rendered.left,
                top: rendered.top,
            }
        } else {
            GlyphInfo {
                sprite: None,
                left: 0,
                top: 0,
            }
        };
        self.inner.glyphs.lock().unwrap().insert(key, info);
        info
    }
}

#[derive(Debug, Clone, Copy)]
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

pub async fn load_ttf_font(path: &str) -> Result<Font, Error> {
    let bytes = load_file(path).await?;
    load_ttf_font_from_bytes(&bytes)
}

pub fn load_ttf_font_from_bytes(bytes: &[u8]) -> Result<Font, Error> {
    let atlas = Arc::new(Mutex::new(Atlas::new(
        get_quad_context(),
        miniquad::FilterMode::Linear,
    )));
    let mut font = Font::load_from_bytes(atlas, bytes)?;
    font.set_filter(get_context().default_filter_mode);
    Ok(font)
}

pub fn load_default_font() -> Result<Font, Error> {
    load_ttf_font_from_bytes(include_bytes!("ProggyClean.ttf"))
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
    let font = params
        .font
        .unwrap_or(&get_context().fonts_storage.default_font);
    let dpi = miniquad::window::dpi_scale();
    let pixel_size = (params.font_size as f32 * dpi).ceil();
    let shaped = font.shape(text, pixel_size, None);
    let scale_x = params.font_scale * params.font_scale_aspect;
    let scale_y = params.font_scale;
    let cos = params.rotation.cos();
    let sin = params.rotation.sin();

    let glyph_infos: Vec<_> = shaped
        .glyphs
        .iter()
        .map(|glyph| font.cache_positioned_glyph(glyph))
        .collect();
    let mut atlas = font.inner.atlas.lock().unwrap();
    let texture = Texture2D {
        texture: TextureHandle::Unmanaged(atlas.texture()),
    };

    for (glyph, info) in shaped.glyphs.iter().zip(glyph_infos) {
        let Some(sprite_id) = info.sprite else {
            continue;
        };
        let Some(sprite) = atlas.get(sprite_id) else {
            continue;
        };
        let logical_x = (glyph.x + info.left as f32) * scale_x / dpi;
        let logical_y = (glyph.y - info.top as f32) * scale_y / dpi;
        let dest_x = x + logical_x * cos - logical_y * sin;
        let dest_y = y + logical_x * sin + logical_y * cos;
        draw_texture_ex(
            &texture,
            dest_x,
            dest_y,
            params.color,
            DrawTextureParams {
                dest_size: Some(vec2(
                    sprite.rect.w * scale_x / dpi,
                    sprite.rect.h * scale_y / dpi,
                )),
                source: Some(sprite.rect),
                rotation: params.rotation,
                pivot: Some(vec2(dest_x, dest_y)),
                ..Default::default()
            },
        );
    }

    scale_dimensions(shaped.dimensions, dpi, scale_x, scale_y)
}

pub fn measure_text(
    text: impl AsRef<str>,
    font: Option<&Font>,
    font_size: u16,
    font_scale: f32,
) -> TextDimensions {
    let font = font.unwrap_or(&get_context().fonts_storage.default_font);
    let dpi = miniquad::window::dpi_scale();
    let shaped = font.shape(text.as_ref(), (font_size as f32 * dpi).ceil(), None);
    scale_dimensions(shaped.dimensions, dpi, font_scale, font_scale)
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
    let font = font.unwrap_or(&get_context().fonts_storage.default_font);
    let line_advance = line_distance_factor.unwrap() * font_size as f32 * font_scale;
    let mut dimensions = TextDimensions::default();
    for (index, line) in text.split('\n').enumerate() {
        let line_dimensions = measure_text(line, Some(font), font_size, font_scale);
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
    let font = font.unwrap_or(&get_context().fonts_storage.default_font);
    let dpi = miniquad::window::dpi_scale();
    let pixel_size = (font_size as f32 * dpi).ceil();
    let max_width = maximum_line_length * dpi / font_scale;
    let mut output = String::with_capacity(text.len());
    let mut line = String::new();
    for ch in text.chars() {
        if ch == '\n' {
            output.push_str(&line);
            output.push('\n');
            line.clear();
            continue;
        }
        line.push(ch);
        if font.shape(&line, pixel_size, None).dimensions.width > max_width {
            line.pop();
            if !line.is_empty() {
                output.push_str(&line);
                output.push('\n');
                line.clear();
            }
            line.push(ch);
        }
    }
    output.push_str(&line);
    output
}

fn scale_dimensions(
    dimensions: TextDimensions,
    dpi: f32,
    scale_x: f32,
    scale_y: f32,
) -> TextDimensions {
    TextDimensions {
        width: dimensions.width * scale_x / dpi,
        height: dimensions.height * scale_y / dpi,
        offset_y: dimensions.offset_y * scale_y / dpi,
    }
}

#[allow(dead_code)]
fn require_font_send() {
    fn require_send<T: Send>() {}
    require_send::<Font>();
}

impl Default for Font {
    fn default() -> Self {
        get_default_font()
    }
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
    pub(crate) fn new(ctx: &mut dyn miniquad::RenderingBackend) -> Self {
        let atlas = Arc::new(Mutex::new(Atlas::new(ctx, miniquad::FilterMode::Linear)));
        let default_font = Font::load_from_bytes(atlas, include_bytes!("ProggyClean.ttf")).unwrap();
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
