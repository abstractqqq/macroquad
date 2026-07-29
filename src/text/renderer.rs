use std::{
    collections::{hash_map::DefaultHasher, VecDeque},
    hash::{Hash, Hasher},
    sync::Arc,
};

use foldhash::{HashMap, HashMapExt};
use swash::{
    shape::ShapeContext,
    text::{Codepoint, Script},
};

use crate::{
    color::Color,
    math::vec2,
    models::Vertex,
    quad_gl::{DrawMode, QuadGl},
    texture::Texture2D,
};

use super::{
    atlas::{Atlas, SpriteKey},
    rasterizer::Rasterizer,
    Font, FontId, TextDimensions,
};

pub(crate) const BASE_FONT_SIZE: f32 = 32.0;

#[derive(Debug, Clone, Copy)]
struct GlyphInfo {
    sprite: SpriteKey,
    left: i32,
    top: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PositionedGlyph {
    pub(crate) glyph_id: u16,
    pub(crate) x: f32,
    pub(crate) y: f32,
}

pub(crate) struct ShapedText {
    pub(crate) glyphs: Vec<PositionedGlyph>,
    pub(crate) dimensions: TextDimensions,
}

struct RenderFont {
    atlas: Atlas,
    glyphs: HashMap<u16, GlyphInfo>,
    frozen: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LayoutHash {
    font: FontId,
    text: u64,
}

struct LayoutEntry {
    text: Arc<str>,
    shaped: Arc<ShapedText>,
}

struct LayoutCache {
    entries: HashMap<LayoutHash, Vec<LayoutEntry>>,
    insertion_order: VecDeque<(LayoutHash, Arc<str>)>,
    len: usize,
}

impl LayoutCache {
    const CAPACITY: usize = 256;

    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
            len: 0,
        }
    }

    fn text_hash(text: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        hasher.finish()
    }

    fn get(&self, font: FontId, text: &str) -> Option<Arc<ShapedText>> {
        let key = LayoutHash {
            font,
            text: Self::text_hash(text),
        };
        self.entries.get(&key).and_then(|bucket| {
            bucket
                .iter()
                .find(|entry| entry.text.as_ref() == text)
                .map(|entry| entry.shaped.clone())
        })
    }

    fn insert(&mut self, font: FontId, text: &str, shaped: Arc<ShapedText>) {
        let key = LayoutHash {
            font,
            text: Self::text_hash(text),
        };
        if self
            .entries
            .get(&key)
            .is_some_and(|bucket| bucket.iter().any(|entry| entry.text.as_ref() == text))
        {
            return;
        }
        while self.len >= Self::CAPACITY {
            let Some((old_key, old_text)) = self.insertion_order.pop_front() else {
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
        let owned: Arc<str> = Arc::from(text);
        self.insertion_order.push_back((key, owned.clone()));
        self.entries.entry(key).or_default().push(LayoutEntry {
            text: owned,
            shaped,
        });
        self.len += 1;
    }
}

pub(crate) struct TextRenderer {
    shape_context: ShapeContext,
    rasterizer: Rasterizer,
    fonts: HashMap<FontId, RenderFont>,
    layouts: LayoutCache,
}

impl TextRenderer {
    pub(crate) fn new() -> Self {
        Self {
            shape_context: ShapeContext::new(),
            rasterizer: Rasterizer::new(),
            fonts: HashMap::new(),
            layouts: LayoutCache::new(),
        }
    }

    pub(crate) fn register_font(
        &mut self,
        font: &Font,
        ctx: &mut dyn miniquad::RenderingBackend,
        filter: miniquad::FilterMode,
    ) {
        if self.fonts.contains_key(&font.id()) {
            return;
        }
        let mut atlas = Atlas::new(ctx, filter);
        atlas.set_filter_with(ctx, filter);
        self.fonts.insert(
            font.id(),
            RenderFont {
                atlas,
                glyphs: HashMap::new(),
                frozen: false,
            },
        );
    }

    pub(crate) fn set_filter(
        &mut self,
        font: &Font,
        ctx: &mut dyn miniquad::RenderingBackend,
        filter: miniquad::FilterMode,
    ) {
        if let Some(render_font) = self.fonts.get_mut(&font.id()) {
            render_font.atlas.set_filter_with(ctx, filter);
        }
    }

    pub(crate) fn shape(&mut self, font: &Font, text: &str) -> Arc<ShapedText> {
        if let Some(shaped) = self.layouts.get(font.id(), text) {
            return shaped;
        }
        if text.is_empty() {
            return Arc::new(ShapedText {
                glyphs: Vec::new(),
                dimensions: TextDimensions::default(),
            });
        }

        let font_ref = font.font_ref();
        let metrics = font_ref.metrics(&[]).scale(BASE_FONT_SIZE);
        let line_height = metrics.ascent + metrics.descent + metrics.leading;
        let mut glyphs = Vec::new();
        let mut width = 0.0_f32;
        let mut line_count = 0usize;

        for (line_index, line) in text.split('\n').enumerate() {
            line_count += 1;
            let script = line
                .chars()
                .map(Codepoint::script)
                .find(|script| {
                    !matches!(script, Script::Common | Script::Inherited | Script::Unknown)
                })
                .unwrap_or(Script::Latin);
            let mut pen_x = 0.0;
            let baseline_y = line_index as f32 * line_height;
            let mut shaper = self
                .shape_context
                .builder(font_ref)
                .script(script)
                .size(BASE_FONT_SIZE)
                .build();
            shaper.add_str(line);
            shaper.shape_with(|cluster| {
                for glyph in cluster.glyphs {
                    glyphs.push(PositionedGlyph {
                        glyph_id: glyph.id,
                        x: pen_x + glyph.x,
                        y: baseline_y + glyph.y,
                    });
                    pen_x += glyph.advance;
                }
            });
            width = width.max(pen_x);
        }

        let shaped = Arc::new(ShapedText {
            glyphs,
            dimensions: TextDimensions {
                width,
                height: line_height * line_count as f32,
                offset_y: metrics.ascent,
            },
        });
        self.layouts.insert(font.id(), text, shaped.clone());
        shaped
    }

    fn ensure_glyph(&mut self, font: &Font, glyph_id: u16) -> bool {
        let Some(render_font) = self.fonts.get_mut(&font.id()) else {
            return false;
        };
        if render_font.glyphs.contains_key(&glyph_id) {
            return true;
        }
        if render_font.frozen {
            return false;
        }
        let Some(rendered) =
            self.rasterizer
                .rasterize(font.bytes(), font.index(), glyph_id, BASE_FONT_SIZE)
        else {
            return false;
        };
        if rendered.image.width == 0 || rendered.image.height == 0 {
            return false;
        }
        let sprite = render_font.atlas.new_unique_id();
        render_font.atlas.cache_sprite(sprite, rendered.image);
        render_font.glyphs.insert(
            glyph_id,
            GlyphInfo {
                sprite,
                left: rendered.left,
                top: rendered.top,
            },
        );
        true
    }

    pub(crate) fn warm_text(&mut self, font: &Font, text: &str) {
        let shaped = self.shape(font, text);
        for glyph in &shaped.glyphs {
            self.ensure_glyph(font, glyph.glyph_id);
        }
    }

    pub(crate) fn warm_characters(&mut self, font: &Font, characters: &[char]) {
        for character in characters {
            let mut buffer = [0; 4];
            self.warm_text(font, character.encode_utf8(&mut buffer));
        }
    }

    pub(crate) fn freeze(&mut self, font: &Font, ctx: &mut dyn miniquad::RenderingBackend) {
        self.ensure_glyph(font, 0);
        if let Some(render_font) = self.fonts.get_mut(&font.id()) {
            render_font.atlas.flush(ctx);
            render_font.frozen = true;
        }
    }

    pub(crate) fn is_frozen(&self, font: &Font) -> bool {
        self.fonts
            .get(&font.id())
            .is_some_and(|render_font| render_font.frozen)
    }

    pub(crate) fn measure(
        &mut self,
        font: &Font,
        text: &str,
        requested_pixel_size: f32,
        scale_x: f32,
        scale_y: f32,
    ) -> TextDimensions {
        let dimensions = self.shape(font, text).dimensions;
        let base_scale = requested_pixel_size / BASE_FONT_SIZE;
        TextDimensions {
            width: dimensions.width * base_scale * scale_x,
            height: dimensions.height * base_scale * scale_y,
            offset_y: dimensions.offset_y * base_scale * scale_y,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        font: &Font,
        text: &str,
        x: f32,
        y: f32,
        requested_pixel_size: f32,
        scale_x: f32,
        scale_y: f32,
        rotation: f32,
        color: Color,
        gl: &mut QuadGl,
        backend: &mut dyn miniquad::RenderingBackend,
    ) -> TextDimensions {
        let shaped = self.shape(font, text);
        for glyph in &shaped.glyphs {
            self.ensure_glyph(font, glyph.glyph_id);
        }

        let Some(render_font) = self.fonts.get_mut(&font.id()) else {
            return TextDimensions::default();
        };
        render_font.atlas.flush(backend);
        let texture = Texture2D::unmanaged(render_font.atlas.texture_id());
        let (atlas_width, atlas_height) = render_font.atlas.image_size();
        let base_scale = requested_pixel_size / BASE_FONT_SIZE;
        let draw_scale_x = base_scale * scale_x;
        let draw_scale_y = base_scale * scale_y;
        let cos = rotation.cos();
        let sin = rotation.sin();
        let indices = [0, 1, 2, 0, 2, 3];

        gl.texture(Some(&texture));
        gl.draw_mode(DrawMode::Triangles);
        for glyph in &shaped.glyphs {
            let Some(info) = render_font
                .glyphs
                .get(&glyph.glyph_id)
                .or_else(|| render_font.glyphs.get(&0))
            else {
                continue;
            };
            let Some(sprite) = render_font.atlas.get(info.sprite) else {
                continue;
            };
            let logical_x = (glyph.x + info.left as f32) * draw_scale_x;
            let logical_y = (glyph.y - info.top as f32) * draw_scale_y;
            let dest_x = x + logical_x * cos - logical_y * sin;
            let dest_y = y + logical_x * sin + logical_y * cos;
            let width = sprite.rect.w * draw_scale_x;
            let height = sprite.rect.h * draw_scale_y;
            let p = [
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
                Vertex::new(p[0].x, p[0].y, 0.0, sx, sy, color),
                Vertex::new(p[1].x, p[1].y, 0.0, sx + sw, sy, color),
                Vertex::new(p[2].x, p[2].y, 0.0, sx + sw, sy + sh, color),
                Vertex::new(p[3].x, p[3].y, 0.0, sx, sy + sh, color),
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
