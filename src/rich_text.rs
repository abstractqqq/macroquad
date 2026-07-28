//! Optional Parley + Swash rich-text pipeline, independent from `crate::text`.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use foldhash::{HashMap, HashMapExt};
use glam::vec2;
use parley::{
    fontique::{Blob, Collection, CollectionOptions, FontInfoOverride, SourceCache},
    FontContext, FontFamily, LayoutContext, PositionedLayoutItem, StyleProperty,
};

use crate::{
    color::{Color, WHITE},
    file::load_file,
    get_quad_context,
    texture::{draw_texture_ex, DrawTextureParams, Texture2D, TextureHandle},
    Error,
};

#[path = "rich_text/atlas.rs"]
mod atlas;
#[path = "rich_text/rasterizer.rs"]
mod rasterizer;

use atlas::Atlas;
use rasterizer::Rasterizer;

const FAMILY: &str = "macroquad-rich-text";

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
    layout: Mutex<LayoutState>,
    shaped: Mutex<ShapeCache>,
    rasterizer: Mutex<Rasterizer>,
    atlas: Mutex<Atlas>,
    glyphs: Mutex<HashMap<GlyphCacheKey, GlyphInfo>>,
}

struct LayoutState {
    font_context: FontContext,
    layout_context: LayoutContext<()>,
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
    font_data_id: u64,
    font_index: u32,
    glyph_id: u32,
    pixel_size: u16,
    normalized_coords: Vec<i16>,
}

#[derive(Debug, Clone, Copy)]
struct GlyphInfo {
    sprite: Option<u64>,
    left: i32,
    top: i32,
}

#[derive(Clone)]
struct PositionedGlyph {
    font: parley::FontData,
    glyph_id: u32,
    x: f32,
    y: f32,
    size: f32,
    normalized_coords: Vec<i16>,
}

struct ShapedText {
    glyphs: Vec<PositionedGlyph>,
    dimensions: TextDimensions,
    line_ranges: Vec<std::ops::Range<usize>>,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Font")
            .field("backend", &"Parley + Swash")
            .finish()
    }
}

impl Font {
    pub fn load_from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let blob = Blob::new(Arc::new(bytes.to_vec()));
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        let registered = collection.register_fonts(
            blob,
            Some(FontInfoOverride {
                family_name: Some(FAMILY),
                ..Default::default()
            }),
        );
        if registered.is_empty() {
            return Err(Error::FontError(
                "Parley could not register the supplied font data",
            ));
        }

        let font_context = FontContext {
            collection,
            source_cache: SourceCache::default(),
        };
        let atlas = Atlas::new(get_quad_context(), miniquad::FilterMode::Linear);
        Ok(Self {
            inner: Arc::new(FontInner {
                layout: Mutex::new(LayoutState {
                    font_context,
                    layout_context: LayoutContext::new(),
                }),
                shaped: Mutex::new(ShapeCache::new()),
                rasterizer: Mutex::new(Rasterizer::new()),
                atlas: Mutex::new(atlas),
                glyphs: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn set_filter(&self, filter: miniquad::FilterMode) {
        self.inner.atlas.lock().unwrap().set_filter(filter);
    }

    pub fn populate_font_cache(&self, text: &str, font_size: u16) {
        let dpi = miniquad::window::dpi_scale();
        let pixel_size = (font_size as f32 * dpi).ceil();
        let shaped = self.shape(text, pixel_size, None);
        for glyph in &shaped.glyphs {
            self.cache_glyph(glyph);
        }
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
                line_ranges: Vec::new(),
            });
            self.inner
                .shaped
                .lock()
                .unwrap()
                .insert(key, shaped.clone());
            return shaped;
        }

        let mut state = self.inner.layout.lock().unwrap();
        let LayoutState {
            font_context,
            layout_context,
        } = &mut *state;
        let mut builder = layout_context.ranged_builder(font_context, text, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named(FAMILY)));
        builder.push_default(StyleProperty::FontSize(pixel_size));
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);

        let first_baseline = layout
            .lines()
            .next()
            .map(|line| line.metrics().baseline)
            .unwrap_or(0.0);
        let dimensions = TextDimensions {
            width: layout.width(),
            height: layout.height(),
            offset_y: first_baseline,
        };
        let line_ranges = layout.lines().map(|line| line.text_range()).collect();
        let mut glyphs = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    let font = run.run().font().clone();
                    let size = run.run().font_size();
                    let normalized_coords = run.run().normalized_coords().to_vec();
                    glyphs.extend(run.positioned_glyphs().map(|glyph| PositionedGlyph {
                        font: font.clone(),
                        glyph_id: glyph.id,
                        x: glyph.x,
                        y: glyph.y - first_baseline,
                        size,
                        normalized_coords: normalized_coords.clone(),
                    }));
                }
            }
        }
        let shaped = Arc::new(ShapedText {
            glyphs,
            dimensions,
            line_ranges,
        });
        self.inner
            .shaped
            .lock()
            .unwrap()
            .insert(key, shaped.clone());
        shaped
    }

    fn cache_glyph(&self, glyph: &PositionedGlyph) -> GlyphInfo {
        let key = GlyphCacheKey {
            font_data_id: glyph.font.data.id(),
            font_index: glyph.font.index,
            glyph_id: glyph.glyph_id,
            pixel_size: glyph.size.ceil() as u16,
            normalized_coords: glyph.normalized_coords.clone(),
        };
        if let Some(info) = self.inner.glyphs.lock().unwrap().get(&key).copied() {
            return info;
        }

        let rendered = self.inner.rasterizer.lock().unwrap().rasterize(
            &glyph.font,
            glyph.glyph_id,
            glyph.size,
            &glyph.normalized_coords,
        );
        let info = if let Some(rendered) = rendered {
            let sprite = if rendered.image.width == 0 || rendered.image.height == 0 {
                None
            } else {
                Some(self.inner.atlas.lock().unwrap().insert(rendered.image))
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
    pub font: &'a Font,
    pub font_size: u16,
    pub font_scale: f32,
    pub font_scale_aspect: f32,
    pub rotation: f32,
    pub color: Color,
}

impl<'a> TextParams<'a> {
    pub fn new(font: &'a Font, font_size: u16, color: Color) -> Self {
        Self {
            font,
            font_size,
            font_scale: 1.0,
            font_scale_aspect: 1.0,
            rotation: 0.0,
            color,
        }
    }
}

pub async fn load_ttf_font(path: &str) -> Result<Font, Error> {
    let bytes = load_file(path).await?;
    load_ttf_font_from_bytes(&bytes)
}

pub fn load_ttf_font_from_bytes(bytes: &[u8]) -> Result<Font, Error> {
    Font::load_from_bytes(bytes)
}

pub fn load_default_font() -> Result<Font, Error> {
    load_ttf_font_from_bytes(include_bytes!("ProggyClean.ttf"))
}

pub fn draw_text(
    text: impl AsRef<str>,
    x: f32,
    y: f32,
    font: &Font,
    font_size: f32,
    color: Color,
) -> TextDimensions {
    draw_text_ex(text, x, y, TextParams::new(font, font_size as u16, color))
}

pub fn draw_text_ex(
    text: impl AsRef<str>,
    x: f32,
    y: f32,
    params: TextParams<'_>,
) -> TextDimensions {
    let text = text.as_ref();
    let dpi = miniquad::window::dpi_scale();
    let pixel_size = (params.font_size as f32 * dpi).ceil();
    let shaped = params.font.shape(text, pixel_size, None);
    let scale_x = params.font_scale * params.font_scale_aspect;
    let scale_y = params.font_scale;
    let cos = params.rotation.cos();
    let sin = params.rotation.sin();

    let glyph_infos: Vec<_> = shaped
        .glyphs
        .iter()
        .map(|glyph| params.font.cache_glyph(glyph))
        .collect();
    let mut atlas = params.font.inner.atlas.lock().unwrap();
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
    font: &Font,
    font_size: u16,
    font_scale: f32,
) -> TextDimensions {
    let dpi = miniquad::window::dpi_scale();
    let shaped = font.shape(text.as_ref(), (font_size as f32 * dpi).ceil(), None);
    scale_dimensions(shaped.dimensions, dpi, font_scale, font_scale)
}

pub fn measure_multiline_text(
    text: &str,
    font: &Font,
    font_size: u16,
    font_scale: f32,
) -> TextDimensions {
    measure_text(text, font, font_size, font_scale)
}

pub fn draw_multiline_text(
    text: &str,
    x: f32,
    y: f32,
    font: &Font,
    font_size: f32,
    color: Color,
) -> TextDimensions {
    draw_text(text, x, y, font, font_size, color)
}

pub fn draw_multiline_text_ex(
    text: &str,
    x: f32,
    y: f32,
    params: TextParams<'_>,
) -> TextDimensions {
    draw_text_ex(text, x, y, params)
}

pub fn wrap_text(
    text: &str,
    font: &Font,
    font_size: u16,
    font_scale: f32,
    maximum_line_length: f32,
) -> String {
    let dpi = miniquad::window::dpi_scale();
    let pixel_size = (font_size as f32 * dpi).ceil();
    let max_width = maximum_line_length * dpi / font_scale;
    let shaped = font.shape(text, pixel_size, Some(max_width));
    if shaped.line_ranges.len() <= 1 {
        return text.to_owned();
    }

    let mut output = String::with_capacity(text.len() + shaped.line_ranges.len() - 1);
    let mut start = 0;
    for range in shaped.line_ranges.iter().take(shaped.line_ranges.len() - 1) {
        let end = range.end.min(text.len());
        output.push_str(&text[start..end]);
        if !output.ends_with('\n') {
            output.push('\n');
        }
        start = end;
    }
    output.push_str(&text[start..]);
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

pub fn default_text_params(font: &Font) -> TextParams<'_> {
    TextParams::new(font, 16, WHITE)
}
