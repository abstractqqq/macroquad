use std::{ops::Range, sync::Arc};

use crate::{
    color::Color,
    math::{vec2, Rect},
    models::Vertex,
    quad_gl::{DrawMode, QuadGl},
};

use super::{Font, TextDimensions};

pub(crate) const BASE_FONT_SIZE: f32 = 32.0;

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
        let mut glyphs = Vec::with_capacity(character_count);
        let mut clusters = Vec::with_capacity(character_count);
        let mut width = 0.0_f32;
        let mut line_count = 0usize;
        let mut source_offset = 0usize;
        let explicit_line_count = text.split('\n').count();

        for (line_index, line) in text.split('\n').enumerate() {
            line_count += 1;
            let mut pen_x = 0.0;
            let mut baseline_y = (line_count - 1) as f32 * line_height;

            for (char_byte_offset, ch) in line.char_indices() {
                let Some(metric) = font.data.get_glyph(ch).or(fallback) else {
                    clusters.push(ShapedCluster {
                        source: (source_offset + char_byte_offset) as u32
                            ..(source_offset + char_byte_offset + ch.len_utf8()) as u32,
                        glyphs: glyphs.len() as u32..glyphs.len() as u32,
                    });
                    continue;
                };

                let advance = metric.advance;
                let wrapped = max_width.is_some_and(|max_w| pen_x > 0.0 && pen_x + advance > max_w);
                if wrapped {
                    width = width.max(pen_x);
                    pen_x = 0.0;
                    line_count += 1;
                    baseline_y += line_height;
                }

                let glyph_start = glyphs.len();
                if wrapped && ch.is_whitespace() {
                    clusters.push(ShapedCluster {
                        source: (source_offset + char_byte_offset) as u32
                            ..(source_offset + char_byte_offset + ch.len_utf8()) as u32,
                        glyphs: glyph_start as u32..glyph_start as u32,
                    });
                    continue;
                }

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
}
