use std::{
    collections::{hash_map::DefaultHasher, VecDeque},
    hash::{Hash, Hasher},
    ops::Range,
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
    sprite: Option<SpriteKey>,
    left: i32,
    top: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PositionedGlyph {
    pub(crate) glyph_id: u16,
    pub(crate) x: f32,
    pub(crate) y: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct ShapedCluster {
    pub(crate) source: Range<usize>,
    pub(crate) glyphs: Range<usize>,
}

pub(crate) struct ShapedText {
    pub(crate) glyphs: Vec<PositionedGlyph>,
    pub(crate) clusters: Vec<ShapedCluster>,
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
    max_width_bits: Option<u32>,
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

    fn get(&self, font: FontId, text: &str, max_width: Option<f32>) -> Option<Arc<ShapedText>> {
        let key = LayoutHash {
            font,
            text: Self::text_hash(text),
            max_width_bits: max_width.map(f32::to_bits),
        };
        self.entries.get(&key).and_then(|bucket| {
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
        let key = LayoutHash {
            font,
            text: Self::text_hash(text),
            max_width_bits: max_width.map(f32::to_bits),
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

    pub(crate) fn shape(
        &mut self,
        font: &Font,
        text: &str,
        max_width: Option<f32>,
    ) -> Arc<ShapedText> {
        let max_width = max_width.filter(|width| width.is_finite() && *width > 0.0);
        if let Some(shaped) = self.layouts.get(font.id(), text, max_width) {
            return shaped;
        }
        if text.is_empty() {
            return Arc::new(ShapedText {
                glyphs: Vec::new(),
                clusters: Vec::new(),
                dimensions: TextDimensions::default(),
            });
        }

        let font_ref = font.font_ref();
        let metrics = font_ref.metrics(&[]).scale(BASE_FONT_SIZE);
        let line_height = metrics.ascent + metrics.descent + metrics.leading;
        let mut glyphs = Vec::new();
        let mut clusters = Vec::new();
        let mut width = 0.0_f32;
        let mut line_count = 0usize;
        let mut source_offset = 0usize;
        let explicit_line_count = text.split('\n').count();

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
            let mut baseline_y = (line_count - 1) as f32 * line_height;
            let mut shaper = self
                .shape_context
                .builder(font_ref)
                .script(script)
                .size(BASE_FONT_SIZE)
                .build();
            shaper.add_str(line);
            shaper.shape_with(|cluster| {
                let advance = cluster.advance();
                let wrapped =
                    max_width.is_some_and(|max_width| pen_x > 0.0 && pen_x + advance > max_width);
                if wrapped {
                    width = width.max(pen_x);
                    pen_x = 0.0;
                    line_count += 1;
                    baseline_y += line_height;
                }
                let glyph_start = glyphs.len();
                if wrapped && cluster.info.is_whitespace() {
                    clusters.push(ShapedCluster {
                        source: source_offset + cluster.source.start as usize
                            ..source_offset + cluster.source.end as usize,
                        glyphs: glyph_start..glyph_start,
                    });
                    return;
                }
                for glyph in cluster.glyphs {
                    glyphs.push(PositionedGlyph {
                        glyph_id: glyph.id,
                        x: pen_x + glyph.x,
                        y: baseline_y + glyph.y,
                    });
                    pen_x += glyph.advance;
                }
                clusters.push(ShapedCluster {
                    source: source_offset + cluster.source.start as usize
                        ..source_offset + cluster.source.end as usize,
                    glyphs: glyph_start..glyphs.len(),
                });
            });
            width = width.max(pen_x);
            source_offset += line.len();
            if line_index + 1 < explicit_line_count {
                clusters.push(ShapedCluster {
                    source: source_offset..source_offset + 1,
                    glyphs: glyphs.len()..glyphs.len(),
                });
                source_offset += 1;
            }
        }

        let shaped = Arc::new(ShapedText {
            glyphs,
            clusters,
            dimensions: TextDimensions {
                width,
                height: line_height * line_count as f32,
                offset_y: metrics.ascent,
            },
        });
        self.layouts
            .insert(font.id(), text, max_width, shaped.clone());
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
        let sprite = if rendered.image.width == 0 || rendered.image.height == 0 {
            None
        } else {
            let sprite = render_font.atlas.new_unique_id();
            render_font.atlas.cache_sprite(sprite, rendered.image);
            Some(sprite)
        };
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
        let shaped = self.shape(font, text, None);
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
        let dimensions = self.shape(font, text, None).dimensions;
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
        let shaped = self.shape(font, text, None);
        self.draw_shaped(
            font,
            &shaped,
            x,
            y,
            requested_pixel_size,
            scale_x,
            scale_y,
            rotation,
            color,
            None,
            shaped.clusters.len(),
            gl,
            backend,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_shaped(
        &mut self,
        font: &Font,
        shaped: &ShapedText,
        x: f32,
        y: f32,
        requested_pixel_size: f32,
        scale_x: f32,
        scale_y: f32,
        rotation: f32,
        color: Color,
        cluster_colors: Option<&[Option<Color>]>,
        visible_clusters: usize,
        gl: &mut QuadGl,
        backend: &mut dyn miniquad::RenderingBackend,
    ) -> TextDimensions {
        let visible_clusters = visible_clusters.min(shaped.clusters.len());
        let visible_glyphs = shaped
            .clusters
            .get(..visible_clusters)
            .and_then(|clusters| clusters.last())
            .map_or(0, |cluster| cluster.glyphs.end);
        for glyph in &shaped.glyphs[..visible_glyphs] {
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
        for (cluster_index, cluster) in shaped.clusters[..visible_clusters].iter().enumerate() {
            let cluster_color = cluster_colors
                .and_then(|colors| colors.get(cluster_index))
                .copied()
                .flatten()
                .unwrap_or(color);
            for glyph in &shaped.glyphs[cluster.glyphs.clone()] {
                let Some(info) = render_font
                    .glyphs
                    .get(&glyph.glyph_id)
                    .or_else(|| render_font.glyphs.get(&0))
                else {
                    continue;
                };
                let Some(sprite_key) = info.sprite else {
                    continue;
                };
                let Some(sprite) = render_font.atlas.get(sprite_key) else {
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
                    Vertex::new(p[0].x, p[0].y, 0.0, sx, sy, cluster_color),
                    Vertex::new(p[1].x, p[1].y, 0.0, sx + sw, sy, cluster_color),
                    Vertex::new(p[2].x, p[2].y, 0.0, sx + sw, sy + sh, cluster_color),
                    Vertex::new(p[3].x, p[3].y, 0.0, sx, sy + sh, cluster_color),
                ];
                gl.geometry(&vertices, &indices);
            }
        }

        TextDimensions {
            width: shaped.dimensions.width * draw_scale_x,
            height: shaped.dimensions.height * draw_scale_y,
            offset_y: shaped.dimensions.offset_y * draw_scale_y,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_font() -> Font {
        Font::load_from_bytes(include_bytes!("../ProggyClean.ttf")).unwrap()
    }

    #[test]
    fn combining_sequence_is_one_reveal_cluster() {
        let mut renderer = TextRenderer::new();
        let shaped = renderer.shape(&test_font(), "e\u{301}", None);

        assert_eq!(shaped.clusters.len(), 1);
        assert_eq!(shaped.clusters[0].source, 0.."e\u{301}".len());
    }

    #[test]
    fn explicit_newline_has_a_reveal_cluster() {
        let mut renderer = TextRenderer::new();
        let shaped = renderer.shape(&test_font(), "a\nb", None);

        assert_eq!(shaped.clusters.len(), 3);
        assert_eq!(shaped.clusters[1].source, 1..2);
        assert!(shaped.clusters[1].glyphs.is_empty());
        assert!(
            shaped.dimensions.height > renderer.shape(&test_font(), "ab", None).dimensions.height
        );
    }

    #[test]
    fn maximum_width_wraps_without_reshaping_each_character() {
        let mut renderer = TextRenderer::new();
        let font = test_font();
        let unwrapped = renderer.shape(&font, "abcdef", None);
        let wrapped = renderer.shape(&font, "abcdef", Some(unwrapped.dimensions.width / 2.0));

        assert!(wrapped.dimensions.height > unwrapped.dimensions.height);
        assert!(wrapped.dimensions.width < unwrapped.dimensions.width);
        assert_eq!(wrapped.clusters.len(), unwrapped.clusters.len());
    }
}
