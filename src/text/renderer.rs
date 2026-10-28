use std::{ops::Range, sync::Arc};

use crate::{
    color::Color,
    math::{vec2, Rect},
    models::Vertex,
    quad_gl::{DrawMode, QuadGl},
};

use super::{line_break::can_break_between, Font, TextDimensions};

pub(crate) const BASE_FONT_SIZE: f32 = 32.0;

#[derive(Clone, Copy)]
struct BreakCandidate {
    cluster_idx: usize,
    glyph_idx: usize,
    pen_x: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct PositionedGlyph {
    pub rect: Rect,
    pub left: i32,
    pub top: i32,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone)]
pub struct ShapedCluster {
    pub source: Range<u32>,
    pub glyphs: Range<u32>,
}

pub struct ShapedText {
    pub glyphs: Vec<PositionedGlyph>,
    pub clusters: Vec<ShapedCluster>,
    pub dimensions: TextDimensions,
}

pub(crate) struct TextRenderer {
    batch_vertices: Vec<Vertex>,
    batch_indices: Vec<u16>,
}

impl TextRenderer {
    pub(crate) fn new() -> Self {
        Self {
            batch_vertices: Vec::with_capacity(128),
            batch_indices: Vec::with_capacity(192),
        }
    }

    pub(crate) fn shape(
        &mut self,
        font: &Font,
        text: &str,
        max_width: Option<f32>,
    ) -> Arc<ShapedText> {
        let max_width = max_width.filter(|width| width.is_finite() && *width > 0.0);
        if text.is_empty() {
            return Arc::new(ShapedText {
                glyphs: Vec::new(),
                clusters: Vec::new(),
                dimensions: TextDimensions::default(),
            });
        }

        let character_count = text.chars().count();
        u32::try_from(text.len()).expect("Text exceeds 4 GiB");
        let line_height = font.data.ascent + font.data.descent + font.data.line_gap;
        let fallback = font.data.get_glyph('?');
        let mut glyphs: Vec<PositionedGlyph> = Vec::with_capacity(character_count);
        let mut clusters = Vec::with_capacity(character_count);
        let mut width = 0.0_f32;
        let mut line_count = 0usize;
        let mut source_offset = 0usize;
        let explicit_line_count = text.split('\n').count();

        for (line_index, line) in text.split('\n').enumerate() {
            line_count += 1;
            let mut pen_x = 0.0;
            let mut baseline_y = (line_count - 1) as f32 * line_height;
            let mut prev_char: Option<char> = None;
            let mut line_start_cluster = clusters.len();
            let mut last_break: Option<BreakCandidate> = None;

            for (char_byte_offset, ch) in line.char_indices() {
                if let Some(prev) = prev_char {
                    if can_break_between(prev, ch) {
                        last_break = Some(BreakCandidate {
                            cluster_idx: clusters.len(),
                            glyph_idx: glyphs.len(),
                            pen_x,
                        });
                    }
                }

                let Some(metric) = font.data.get_glyph(ch).or(fallback) else {
                    clusters.push(ShapedCluster {
                        source: (source_offset + char_byte_offset) as u32
                            ..(source_offset + char_byte_offset + ch.len_utf8()) as u32,
                        glyphs: glyphs.len() as u32..glyphs.len() as u32,
                    });
                    prev_char = Some(ch);
                    continue;
                };

                let advance = metric.advance;
                let wrapped = max_width.is_some_and(|max_w| pen_x > 0.0 && pen_x + advance > max_w);
                if wrapped {
                    if ch.is_whitespace() {
                        width = width.max(pen_x);
                        pen_x = 0.0;
                        line_count += 1;
                        baseline_y += line_height;
                        line_start_cluster = clusters.len() + 1;
                        last_break = None;

                        let glyph_start = glyphs.len();
                        clusters.push(ShapedCluster {
                            source: (source_offset + char_byte_offset) as u32
                                ..(source_offset + char_byte_offset + ch.len_utf8()) as u32,
                            glyphs: glyph_start as u32..glyph_start as u32,
                        });
                        prev_char = Some(ch);
                        continue;
                    } else if let Some(candidate) = last_break {
                        if candidate.cluster_idx > line_start_cluster {
                            width = width.max(candidate.pen_x);
                            line_count += 1;
                            baseline_y += line_height;
                            let shift_x = candidate.pen_x;

                            for glyph in &mut glyphs[candidate.glyph_idx..] {
                                glyph.x -= shift_x;
                                glyph.y = baseline_y;
                            }

                            pen_x -= shift_x;
                            line_start_cluster = candidate.cluster_idx;
                            last_break = None;
                        } else {
                            width = width.max(pen_x);
                            pen_x = 0.0;
                            line_count += 1;
                            baseline_y += line_height;
                            line_start_cluster = clusters.len();
                            last_break = None;
                        }
                    } else {
                        width = width.max(pen_x);
                        pen_x = 0.0;
                        line_count += 1;
                        baseline_y += line_height;
                        line_start_cluster = clusters.len();
                        last_break = None;
                    }
                }

                let glyph_start = glyphs.len();
                if let Some(rect) = metric.rect {
                    glyphs.push(PositionedGlyph {
                        rect,
                        left: metric.left,
                        top: metric.top,
                        x: pen_x,
                        y: baseline_y,
                    });
                }
                pen_x += advance;

                clusters.push(ShapedCluster {
                    source: (source_offset + char_byte_offset) as u32
                        ..(source_offset + char_byte_offset + ch.len_utf8()) as u32,
                    glyphs: glyph_start as u32..glyphs.len() as u32,
                });
                prev_char = Some(ch);
            }

            width = width.max(pen_x);
            source_offset += line.len();
            if line_index + 1 < explicit_line_count {
                clusters.push(ShapedCluster {
                    source: source_offset as u32..source_offset as u32 + 1,
                    glyphs: glyphs.len() as u32..glyphs.len() as u32,
                });
                source_offset += 1;
            }
        }

        Arc::new(ShapedText {
            glyphs,
            clusters,
            dimensions: TextDimensions {
                width,
                height: line_height * line_count as f32,
                offset_y: font.data.ascent,
            },
        })
    }

    pub(crate) fn measure(
        &mut self,
        font: &Font,
        text: &str,
        requested_pixel_size: f32,
        scale_x: f32,
        scale_y: f32,
    ) -> TextDimensions {
        if text.is_empty() {
            return TextDimensions::default();
        }
        let line_height = font.data.ascent + font.data.descent + font.data.line_gap;
        let fallback = font.data.get_glyph('?');
        let mut width = 0.0_f32;
        let mut line_count = 0usize;

        for line in text.split('\n') {
            line_count += 1;
            let mut pen_x = 0.0;
            for ch in line.chars() {
                if let Some(metric) = font.data.get_glyph(ch).or(fallback) {
                    pen_x += metric.advance;
                }
            }
            width = width.max(pen_x);
        }

        let base_scale = requested_pixel_size / font.data.base_size;
        TextDimensions {
            width: width * base_scale * scale_x,
            height: (line_height * line_count as f32) * base_scale * scale_y,
            offset_y: font.data.ascent * base_scale * scale_y,
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
        _backend: &mut dyn miniquad::RenderingBackend,
    ) -> TextDimensions {
        if text.is_empty() {
            return TextDimensions::default();
        }

        let atlas = font.atlas();
        let texture = atlas.texture();
        let (atlas_width, atlas_height) = atlas.size();
        let base_scale = requested_pixel_size / font.data.base_size;
        let layout_scale_x = base_scale * scale_x;
        let layout_scale_y = base_scale * scale_y;
        let rotation = (rotation != 0.0).then(|| rotation.sin_cos());
        let inv_atlas_width = 1.0 / atlas_width;
        let inv_atlas_height = 1.0 / atlas_height;

        let line_height = font.data.ascent + font.data.descent + font.data.line_gap;
        let fallback = font.data.get_glyph('?');
        let mut width = 0.0_f32;
        let mut line_count = 0usize;

        gl.texture(Some(&texture));
        gl.draw_mode(DrawMode::Triangles);

        self.batch_vertices.clear();
        self.batch_indices.clear();

        for line in text.split('\n') {
            line_count += 1;
            let mut pen_x = 0.0;
            let baseline_y = (line_count - 1) as f32 * line_height;

            for ch in line.chars() {
                let Some(metric) = font.data.get_glyph(ch).or(fallback) else {
                    continue;
                };

                if let Some(rect) = metric.rect {
                    let logical_x = pen_x * layout_scale_x + metric.left as f32 * layout_scale_x;
                    let logical_y =
                        baseline_y * layout_scale_y - metric.top as f32 * layout_scale_y;
                    let (dest_x, dest_y) = if let Some((sin, cos)) = rotation {
                        (
                            x + logical_x * cos - logical_y * sin,
                            y + logical_x * sin + logical_y * cos,
                        )
                    } else {
                        (x + logical_x, y + logical_y)
                    };
                    let w = rect.w * layout_scale_x;
                    let h = rect.h * layout_scale_y;

                    let p = if let Some((sin, cos)) = rotation {
                        [
                            vec2(dest_x, dest_y),
                            vec2(dest_x + w * cos, dest_y + w * sin),
                            vec2(dest_x + w * cos - h * sin, dest_y + w * sin + h * cos),
                            vec2(dest_x - h * sin, dest_y + h * cos),
                        ]
                    } else {
                        [
                            vec2(dest_x, dest_y),
                            vec2(dest_x + w, dest_y),
                            vec2(dest_x + w, dest_y + h),
                            vec2(dest_x, dest_y + h),
                        ]
                    };
                    let sx = rect.x * inv_atlas_width;
                    let sy = rect.y * inv_atlas_height;
                    let sw = rect.w * inv_atlas_width;
                    let sh = rect.h * inv_atlas_height;

                    let base_idx = self.batch_vertices.len() as u16;
                    self.batch_vertices.extend([
                        Vertex::new(p[0].x, p[0].y, 0.0, sx, sy, color),
                        Vertex::new(p[1].x, p[1].y, 0.0, sx + sw, sy, color),
                        Vertex::new(p[2].x, p[2].y, 0.0, sx + sw, sy + sh, color),
                        Vertex::new(p[3].x, p[3].y, 0.0, sx, sy + sh, color),
                    ]);
                    self.batch_indices.extend([
                        base_idx,
                        base_idx + 1,
                        base_idx + 2,
                        base_idx,
                        base_idx + 2,
                        base_idx + 3,
                    ]);
                }

                pen_x += metric.advance;
            }

            width = width.max(pen_x);
        }

        if !self.batch_vertices.is_empty() {
            gl.geometry(&self.batch_vertices, &self.batch_indices);
        }

        TextDimensions {
            width: width * layout_scale_x,
            height: (line_height * line_count as f32) * layout_scale_y,
            offset_y: font.data.ascent * layout_scale_y,
        }
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
        _backend: &mut dyn miniquad::RenderingBackend,
    ) -> TextDimensions {
        let visible_clusters = visible_clusters.min(shaped.clusters.len());
        let atlas = font.atlas();
        let texture = atlas.texture();
        let (atlas_width, atlas_height) = atlas.size();
        let base_scale = requested_pixel_size / font.data.base_size;
        let layout_scale_x = base_scale * scale_x;
        let layout_scale_y = base_scale * scale_y;
        let rotation = (rotation != 0.0).then(|| rotation.sin_cos());
        let inv_atlas_width = 1.0 / atlas_width;
        let inv_atlas_height = 1.0 / atlas_height;

        gl.texture(Some(&texture));
        gl.draw_mode(DrawMode::Triangles);

        self.batch_vertices.clear();
        self.batch_indices.clear();

        for (cluster_index, cluster) in shaped.clusters[..visible_clusters].iter().enumerate() {
            let cluster_color = cluster_colors
                .and_then(|colors| colors.get(cluster_index))
                .copied()
                .flatten()
                .unwrap_or(color);
            let glyph_range = cluster.glyphs.start as usize..cluster.glyphs.end as usize;
            for glyph in &shaped.glyphs[glyph_range] {
                let rect = glyph.rect;
                let logical_x = glyph.x * layout_scale_x + glyph.left as f32 * layout_scale_x;
                let logical_y = glyph.y * layout_scale_y - glyph.top as f32 * layout_scale_y;
                let (dest_x, dest_y) = if let Some((sin, cos)) = rotation {
                    (
                        x + logical_x * cos - logical_y * sin,
                        y + logical_x * sin + logical_y * cos,
                    )
                } else {
                    (x + logical_x, y + logical_y)
                };
                let width = rect.w * layout_scale_x;
                let height = rect.h * layout_scale_y;
                let p = if let Some((sin, cos)) = rotation {
                    [
                        vec2(dest_x, dest_y),
                        vec2(dest_x + width * cos, dest_y + width * sin),
                        vec2(
                            dest_x + width * cos - height * sin,
                            dest_y + width * sin + height * cos,
                        ),
                        vec2(dest_x - height * sin, dest_y + height * cos),
                    ]
                } else {
                    [
                        vec2(dest_x, dest_y),
                        vec2(dest_x + width, dest_y),
                        vec2(dest_x + width, dest_y + height),
                        vec2(dest_x, dest_y + height),
                    ]
                };
                let sx = rect.x * inv_atlas_width;
                let sy = rect.y * inv_atlas_height;
                let sw = rect.w * inv_atlas_width;
                let sh = rect.h * inv_atlas_height;

                let base_idx = self.batch_vertices.len() as u16;
                self.batch_vertices.extend([
                    Vertex::new(p[0].x, p[0].y, 0.0, sx, sy, cluster_color),
                    Vertex::new(p[1].x, p[1].y, 0.0, sx + sw, sy, cluster_color),
                    Vertex::new(p[2].x, p[2].y, 0.0, sx + sw, sy + sh, cluster_color),
                    Vertex::new(p[3].x, p[3].y, 0.0, sx, sy + sh, cluster_color),
                ]);
                self.batch_indices.extend([
                    base_idx,
                    base_idx + 1,
                    base_idx + 2,
                    base_idx,
                    base_idx + 2,
                    base_idx + 3,
                ]);
            }
        }

        if !self.batch_vertices.is_empty() {
            gl.geometry(&self.batch_vertices, &self.batch_indices);
        }

        TextDimensions {
            width: shaped.dimensions.width * layout_scale_x,
            height: shaped.dimensions.height * layout_scale_y,
            offset_y: shaped.dimensions.offset_y * layout_scale_y,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "font-rasterize")]
    fn test_font() -> Font {
        Font::load_for_test(include_bytes!("../../assets/fonts/ProggyClean.ttf")).unwrap()
    }

    #[test]
    #[cfg(feature = "font-rasterize")]
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
    #[cfg(feature = "font-rasterize")]
    fn maximum_width_wraps_without_reshaping_each_character() {
        let mut renderer = TextRenderer::new();
        let font = test_font();
        let unwrapped = renderer.shape(&font, "abcdef", None);
        let wrapped = renderer.shape(&font, "abcdef", Some(unwrapped.dimensions.width / 2.0));

        assert!(wrapped.dimensions.height > unwrapped.dimensions.height);
        assert!(wrapped.dimensions.width < unwrapped.dimensions.width);
        assert_eq!(wrapped.clusters.len(), unwrapped.clusters.len());
    }

    #[test]
    #[cfg(feature = "font-rasterize")]
    fn closing_punctuation_does_not_start_a_line() {
        let mut renderer = TextRenderer::new();
        let font = test_font();
        // ProggyClean uses fallback '?' for CJK chars, so each CJK char has identical advance.
        // "这是一句话" (5 chars). Each char advance = A.
        // Width for 5.5 chars: fits 5 chars, but cannot fit the 6th char '。'.
        let char_advance = font.data.get_glyph('?').unwrap().advance;
        let max_width = char_advance * 5.5;

        let shaped = renderer.shape(&font, "这是一句话。然后继续。", Some(max_width));

        // The 5th char ('话', glyph 4) and 6th char ('。', glyph 5) should be pushed down to line 2 together.
        // Glyphs 0..4 ("这是一句") should be on line 1 (y == 0).
        // Glyphs 4 ("话") and 5 ("。") should be on line 2 (y > 0).
        let line_height = font.data.ascent + font.data.descent + font.data.line_gap;
        assert_eq!(shaped.glyphs[0].y, 0.0);
        assert_eq!(shaped.glyphs[1].y, 0.0);
        assert_eq!(shaped.glyphs[2].y, 0.0);
        assert_eq!(shaped.glyphs[3].y, 0.0);
        assert_eq!(shaped.glyphs[4].y, line_height);
        assert_eq!(shaped.glyphs[5].y, line_height);
        // '话' is at the start of line 2 (x == 0).
        assert_eq!(shaped.glyphs[4].x, 0.0);
        // '。' follows '话' on line 2.
        assert_eq!(shaped.glyphs[5].x, char_advance);
    }

    #[test]
    #[cfg(feature = "font-rasterize")]
    fn opening_punctuation_does_not_end_a_line() {
        let mut renderer = TextRenderer::new();
        let font = test_font();
        let char_advance = font.data.get_glyph('?').unwrap().advance;
        // Max width fits 5 chars.
        // "这是一句（话）": 4 chars + '（' = 5 chars on line 1, but '话' overflows.
        // Since '（' cannot end a line, '（' must be pushed down to line 2 along with '话'.
        let max_width = char_advance * 5.5;

        let shaped = renderer.shape(&font, "这是一句（话）", Some(max_width));
        let line_height = font.data.ascent + font.data.descent + font.data.line_gap;

        // Line 1: "这是一句" (glyphs 0..4 at y == 0)
        assert_eq!(shaped.glyphs[0].y, 0.0);
        assert_eq!(shaped.glyphs[1].y, 0.0);
        assert_eq!(shaped.glyphs[2].y, 0.0);
        assert_eq!(shaped.glyphs[3].y, 0.0);
        // Line 2: "（话）" (glyphs 4..7 at y == line_height)
        assert_eq!(shaped.glyphs[4].y, line_height);
        assert_eq!(shaped.glyphs[4].x, 0.0); // '（' pushed to start of line 2
    }

    #[test]
    #[cfg(feature = "font-rasterize")]
    fn english_word_wrapping_prefers_space() {
        let mut renderer = TextRenderer::new();
        let font = test_font();
        let char_advance = font.data.get_glyph('a').unwrap().advance;
        // "Hello world."
        // "Hello " = 6 chars. Width fits 8 chars.
        // Without word wrap, "Hello wo" would be on line 1 and "rld." on line 2.
        // With word wrap, "Hello " is on line 1 and "world." is on line 2.
        let max_width = char_advance * 8.0;

        let shaped = renderer.shape(&font, "Hello world.", Some(max_width));
        let line_height = font.data.ascent + font.data.descent + font.data.line_gap;

        // 'H','e','l','l','o' are on line 1 (y == 0)
        for i in 0..5 {
            assert_eq!(shaped.glyphs[i].y, 0.0);
        }
        // 'w' (glyph 5, since space has no glyph) is on line 2 (y == line_height) at x == 0
        assert_eq!(shaped.glyphs[5].y, line_height);
        assert_eq!(shaped.glyphs[5].x, 0.0);
    }
}
