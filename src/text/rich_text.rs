//! Optional Parley rich-text layout backed by the default Swash rasterizer.

use std::{collections::VecDeque, hash::BuildHasher, ops::Range, sync::Arc};

use foldhash::{fast::RandomState, HashMap, HashMapExt};
use parley::{
    fontique::{Blob, Collection, CollectionOptions, FontInfoOverride, SourceCache},
    FontContext, FontFamily, LayoutContext, PositionedLayoutItem, StyleProperty,
};

use crate::{
    color::{Color, WHITE},
    file::load_file,
    get_context,
    math::vec2,
    models::Vertex,
    quad_gl::{DrawMode, QuadGl},
    text::{
        atlas::{Atlas, SpriteKey},
        rasterizer::Rasterizer,
        renderer::BASE_FONT_SIZE,
        FontId,
    },
    texture::Texture2D,
    Error,
};

const FAMILY: &str = "macroquad-rich-text";

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct TextDimensions {
    pub width: f32,
    pub height: f32,
    pub offset_y: f32,
}

/// An immutable font handle for the optional rich-text renderer.
#[derive(Clone)]
pub struct Font {
    asset: crate::text::Font,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Font")
            .field("backend", &"Parley + Swash")
            .field("id", &self.id())
            .finish()
    }
}

impl Font {
    pub fn load_from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Ok(Self {
            asset: crate::text::Font::load_from_bytes(bytes)?,
        })
    }

    pub fn set_filter(&self, filter: miniquad::FilterMode) {
        set_font_filter(self, filter);
    }

    pub fn populate_font_cache(&self, text: &str, _font_size: u16) {
        warm_text_cache(self, text);
    }

    fn id(&self) -> FontId {
        self.asset.id()
    }

    fn bytes(&self) -> &[u8] {
        self.asset.bytes()
    }

    fn index(&self) -> usize {
        self.asset.index()
    }
}

#[derive(Debug, Clone, Copy)]
struct GlyphInfo {
    sprite: Option<SpriteKey>,
    left: i32,
    top: i32,
}

#[derive(Clone)]
struct PositionedGlyph {
    glyph_id: u16,
    x: f32,
    y: f32,
}

struct ShapedText {
    glyphs: Vec<PositionedGlyph>,
    dimensions: TextDimensions,
    line_ranges: Vec<Range<usize>>,
}

struct RichFontState {
    font_context: FontContext,
    layout_context: LayoutContext<()>,
    atlas: Atlas,
    glyphs: HashMap<u16, GlyphInfo>,
    frozen: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LayoutKey {
    font: FontId,
    text_hash: u64,
    max_width_bits: Option<u32>,
}

struct LayoutEntry {
    text: Arc<str>,
    shaped: Arc<ShapedText>,
}

struct LayoutCache {
    entries: HashMap<LayoutKey, Vec<LayoutEntry>>,
    order: VecDeque<(LayoutKey, Arc<str>)>,
    hash_state: RandomState,
    len: usize,
}

impl LayoutCache {
    const CAPACITY: usize = 256;

    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            hash_state: RandomState::default(),
            len: 0,
        }
    }

    fn key(&self, font: FontId, text: &str, max_width: Option<f32>) -> LayoutKey {
        LayoutKey {
            font,
            text_hash: self.hash_state.hash_one(text),
            max_width_bits: max_width.map(f32::to_bits),
        }
    }

    fn get(&self, font: FontId, text: &str, max_width: Option<f32>) -> Option<Arc<ShapedText>> {
        self.entries
            .get(&self.key(font, text, max_width))
            .and_then(|bucket| {
                bucket
                    .iter()
                    .find(|entry| entry.text.as_ref() == text)
                    .map(|entry| entry.shaped.clone())
            })
    }

    fn insert(
        &mut self,
        font: FontId,
        text: &str,
        max_width: Option<f32>,
        shaped: Arc<ShapedText>,
    ) {
        let key = self.key(font, text, max_width);
        if self
            .entries
            .get(&key)
            .is_some_and(|bucket| bucket.iter().any(|entry| entry.text.as_ref() == text))
        {
            return;
        }
        while self.len >= Self::CAPACITY {
            let Some((old_key, old_text)) = self.order.pop_front() else {
                break;
            };
            let mut remove_bucket = false;
            if let Some(bucket) = self.entries.get_mut(&old_key) {
                if let Some(index) = bucket
                    .iter()
                    .position(|entry| entry.text.as_ref() == old_text.as_ref())
                {
                    bucket.swap_remove(index);
                    self.len -= 1;
                }
                remove_bucket = bucket.is_empty();
            }
            if remove_bucket {
                self.entries.remove(&old_key);
            }
        }
        let text: Arc<str> = Arc::from(text);
        self.order.push_back((key, text.clone()));
        self.entries
            .entry(key)
            .or_default()
            .push(LayoutEntry { text, shaped });
        self.len += 1;
    }
}

pub(crate) struct RichTextRenderer {
    fonts: HashMap<FontId, RichFontState>,
    rasterizer: Rasterizer,
    layouts: LayoutCache,
}

impl RichTextRenderer {
    pub(crate) fn new() -> Self {
        Self {
            fonts: HashMap::new(),
            rasterizer: Rasterizer::new(),
            layouts: LayoutCache::new(),
        }
    }

    fn register_font(
        &mut self,
        font: &Font,
        backend: &mut dyn miniquad::RenderingBackend,
        filter: miniquad::FilterMode,
    ) -> Result<(), Error> {
        if self.fonts.contains_key(&font.id()) {
            return Ok(());
        }
        let blob = Blob::new(Arc::new(font.bytes().to_vec()));
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        if collection
            .register_fonts(
                blob,
                Some(FontInfoOverride {
                    family_name: Some(FAMILY),
                    ..Default::default()
                }),
            )
            .is_empty()
        {
            return Err(Error::FontError(
                "Parley could not register the supplied font data",
            ));
        }
        self.fonts.insert(
            font.id(),
            RichFontState {
                font_context: FontContext {
                    collection,
                    source_cache: SourceCache::default(),
                },
                layout_context: LayoutContext::new(),
                atlas: Atlas::new(backend, filter),
                glyphs: HashMap::new(),
                frozen: false,
            },
        );
        Ok(())
    }

    fn shape(&mut self, font: &Font, text: &str, max_width: Option<f32>) -> Arc<ShapedText> {
        if let Some(shaped) = self.layouts.get(font.id(), text, max_width) {
            return shaped;
        }
        if text.is_empty() {
            return Arc::new(ShapedText {
                glyphs: Vec::new(),
                dimensions: TextDimensions::default(),
                line_ranges: Vec::new(),
            });
        }
        let state = self
            .fonts
            .get_mut(&font.id())
            .expect("rich font must be registered before layout");
        let mut builder =
            state
                .layout_context
                .ranged_builder(&mut state.font_context, text, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named(FAMILY)));
        builder.push_default(StyleProperty::FontSize(BASE_FONT_SIZE));
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
                    glyphs.extend(run.positioned_glyphs().map(|glyph| PositionedGlyph {
                        glyph_id: glyph.id as u16,
                        x: glyph.x,
                        y: glyph.y - first_baseline,
                    }));
                }
            }
        }
        let shaped = Arc::new(ShapedText {
            glyphs,
            dimensions,
            line_ranges,
        });
        self.layouts
            .insert(font.id(), text, max_width, shaped.clone());
        shaped
    }

    fn ensure_glyph(&mut self, font: &Font, glyph_id: u16) -> bool {
        let Some(state) = self.fonts.get_mut(&font.id()) else {
            return false;
        };
        if state.glyphs.contains_key(&glyph_id) {
            return true;
        }
        if state.frozen {
            return false;
        }
        let Some(rendered) =
            self.rasterizer
                .rasterize(font.bytes(), font.index(), glyph_id, BASE_FONT_SIZE)
        else {
            return false;
        };
        let sprite = if rendered.image.width == 0 || rendered.image.height == 0 {
            None
        } else {
            let sprite = state.atlas.new_unique_id();
            state.atlas.cache_sprite(sprite, rendered.image);
            Some(sprite)
        };
        state.glyphs.insert(
            glyph_id,
            GlyphInfo {
                sprite,
                left: rendered.left,
                top: rendered.top,
            },
        );
        true
    }

    fn warm_text(&mut self, font: &Font, text: &str) {
        let shaped = self.shape(font, text, None);
        for glyph in &shaped.glyphs {
            self.ensure_glyph(font, glyph.glyph_id);
        }
    }

    fn warm_characters(&mut self, font: &Font, characters: &[char]) {
        for character in characters {
            let mut buffer = [0; 4];
            self.warm_text(font, character.encode_utf8(&mut buffer));
        }
    }

    fn freeze(&mut self, font: &Font, backend: &mut dyn miniquad::RenderingBackend) {
        self.ensure_glyph(font, 0);
        if let Some(state) = self.fonts.get_mut(&font.id()) {
            state.atlas.flush(backend);
            state.frozen = true;
        }
    }

    fn is_frozen(&self, font: &Font) -> bool {
        self.fonts.get(&font.id()).is_some_and(|state| state.frozen)
    }

    fn set_filter(
        &mut self,
        font: &Font,
        backend: &mut dyn miniquad::RenderingBackend,
        filter: miniquad::FilterMode,
    ) {
        if let Some(state) = self.fonts.get_mut(&font.id()) {
            state.atlas.set_filter_with(backend, filter);
        }
    }

    fn measure(
        &mut self,
        font: &Font,
        text: &str,
        requested_size: f32,
        scale_x: f32,
        scale_y: f32,
    ) -> TextDimensions {
        let dimensions = self.shape(font, text, None).dimensions;
        let base_scale = requested_size / BASE_FONT_SIZE;
        TextDimensions {
            width: dimensions.width * base_scale * scale_x,
            height: dimensions.height * base_scale * scale_y,
            offset_y: dimensions.offset_y * base_scale * scale_y,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        font: &Font,
        text: &str,
        x: f32,
        y: f32,
        requested_size: f32,
        scale_x: f32,
        scale_y: f32,
        rotation: f32,
        color: Color,
        gl: &mut QuadGl,
        backend: &mut dyn miniquad::RenderingBackend,
    ) -> TextDimensions {
        let shaped = self.shape(font, text, None);
        for glyph in &shaped.glyphs {
            self.ensure_glyph(font, glyph.glyph_id);
        }
        let Some(state) = self.fonts.get_mut(&font.id()) else {
            return TextDimensions::default();
        };
        state.atlas.flush(backend);
        let texture = Texture2D::unmanaged(state.atlas.texture_id());
        let (atlas_width, atlas_height) = state.atlas.image_size();
        let base_scale = requested_size / BASE_FONT_SIZE;
        let draw_scale_x = base_scale * scale_x;
        let draw_scale_y = base_scale * scale_y;
        let cos = rotation.cos();
        let sin = rotation.sin();
        let indices = [0, 1, 2, 0, 2, 3];

        gl.texture(Some(&texture));
        gl.draw_mode(DrawMode::Triangles);
        for glyph in &shaped.glyphs {
            let Some(info) = state
                .glyphs
                .get(&glyph.glyph_id)
                .or_else(|| state.glyphs.get(&0))
            else {
                continue;
            };
            let Some(sprite_key) = info.sprite else {
                continue;
            };
            let Some(sprite) = state.atlas.get(sprite_key) else {
                continue;
            };
            let logical_x = (glyph.x + info.left as f32) * draw_scale_x;
            let logical_y = (glyph.y - info.top as f32) * draw_scale_y;
            let dest_x = x + logical_x * cos - logical_y * sin;
            let dest_y = y + logical_x * sin + logical_y * cos;
            let width = sprite.rect.w * draw_scale_x;
            let height = sprite.rect.h * draw_scale_y;
            let points = [
                vec2(dest_x, dest_y),
                vec2(dest_x + width * cos, dest_y + width * sin),
                vec2(
                    dest_x + width * cos - height * sin,
                    dest_y + width * sin + height * cos,
                ),
                vec2(dest_x - height * sin, dest_y + height * cos),
            ];
            let sx = sprite.rect.x / atlas_width;
            let sy = sprite.rect.y / atlas_height;
            let sw = sprite.rect.w / atlas_width;
            let sh = sprite.rect.h / atlas_height;
            let vertices = [
                Vertex::new(points[0].x, points[0].y, 0.0, sx, sy, color),
                Vertex::new(points[1].x, points[1].y, 0.0, sx + sw, sy, color),
                Vertex::new(points[2].x, points[2].y, 0.0, sx + sw, sy + sh, color),
                Vertex::new(points[3].x, points[3].y, 0.0, sx, sy + sh, color),
            ];
            gl.geometry(&vertices, &indices);
        }
        TextDimensions {
            width: shaped.dimensions.width * draw_scale_x,
            height: shaped.dimensions.height * draw_scale_y,
            offset_y: shaped.dimensions.offset_y * draw_scale_y,
        }
    }
}

fn register(font: &Font) -> Result<(), Error> {
    let context = get_context();
    context.rich_text_renderer.register_font(
        font,
        &mut *context.quad_context,
        context.default_filter_mode,
    )
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
    let bytes = load_file(path)
        .await
        .map_err(|_| Error::FontError("The Font file couldn't be loaded"))?;
    load_ttf_font_from_bytes(&bytes)
}

pub fn load_ttf_font_from_bytes(bytes: &[u8]) -> Result<Font, Error> {
    let font = Font::load_from_bytes(bytes)?;
    register(&font)?;
    Ok(font)
}

pub fn load_default_font() -> Result<Font, Error> {
    load_ttf_font_from_bytes(include_bytes!("../../assets/fonts/ProggyClean.ttf"))
}

pub fn set_font_filter(font: &Font, filter: miniquad::FilterMode) {
    let context = get_context();
    let _ = context.rich_text_renderer.register_font(
        font,
        &mut *context.quad_context,
        context.default_filter_mode,
    );
    context
        .rich_text_renderer
        .set_filter(font, &mut *context.quad_context, filter);
}

pub fn warm_font_cache(font: &Font, characters: &[char]) {
    if register(font).is_ok() {
        get_context()
            .rich_text_renderer
            .warm_characters(font, characters);
    }
}

pub fn warm_text_cache(font: &Font, text: impl AsRef<str>) {
    if register(font).is_ok() {
        get_context()
            .rich_text_renderer
            .warm_text(font, text.as_ref());
    }
}

pub fn warm_texts_cache<'a>(font: &Font, texts: impl IntoIterator<Item = &'a str>) {
    if register(font).is_ok() {
        let context = get_context();
        for text in texts {
            context.rich_text_renderer.warm_text(font, text);
        }
    }
}

pub fn freeze_font_cache(font: &Font) {
    if register(font).is_ok() {
        let context = get_context();
        context
            .rich_text_renderer
            .freeze(font, &mut *context.quad_context);
    }
}

pub fn is_font_cache_frozen(font: &Font) -> bool {
    get_context().rich_text_renderer.is_frozen(font)
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
    if text.is_empty() || register(params.font).is_err() {
        return TextDimensions::default();
    }
    let context = get_context();
    context.rich_text_renderer.draw(
        params.font,
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
    font: &Font,
    font_size: u16,
    font_scale: f32,
) -> TextDimensions {
    if register(font).is_err() {
        return TextDimensions::default();
    }
    get_context().rich_text_renderer.measure(
        font,
        text.as_ref(),
        font_size as f32,
        font_scale,
        font_scale,
    )
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
    if register(font).is_err() {
        return text.to_owned();
    }
    let base_width = maximum_line_length * BASE_FONT_SIZE / (font_size as f32 * font_scale);
    let shaped = get_context()
        .rich_text_renderer
        .shape(font, text, Some(base_width));
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

pub fn default_text_params(font: &Font) -> TextParams<'_> {
    TextParams::new(font, 16, WHITE)
}
