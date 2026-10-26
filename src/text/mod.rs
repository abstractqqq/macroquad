//! Functions to construct immutable atlas-backed fonts and draw shapeless text.

use std::{ops::Range, sync::Arc};

#[cfg(feature = "font-rasterize")]
use swash::FontRef;

use crate::{
    color::{Color, WHITE},
    file::load_file,
    get_context,
    math::vec3,
    Error,
};

pub mod font_atlas;
#[cfg(feature = "font-rasterize")]
pub(crate) mod rasterizer;
pub mod renderer;

use renderer::TextRenderer;

use self::font_atlas::FontAtlas;
#[cfg(feature = "font-rasterize")]
use self::{font_atlas::FontAtlasBuilder, rasterizer::Rasterizer};

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct TextDimensions {
    pub width: f32,
    pub height: f32,
    pub offset_y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphMetric {
    pub rect: Option<crate::math::Rect>,
    pub left: i32,
    pub top: i32,
    pub advance: f32,
}

pub struct FontData {
    pub atlas: Option<FontAtlas>,
    pub atlas_image: Option<Arc<crate::texture::Image>>,
    pub base_size: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    pub ascii_glyphs: [Option<GlyphMetric>; 96],
    pub extended_glyphs: Box<[(char, GlyphMetric)]>,
}

impl FontData {
    #[inline(always)]
    pub fn get_glyph(&self, c: char) -> Option<&GlyphMetric> {
        let code = c as u32;
        if (32..128).contains(&code) {
            self.ascii_glyphs[(code - 32) as usize].as_ref()
        } else {
            self.extended_glyphs
                .binary_search_by_key(&c, |(ch, _)| *ch)
                .ok()
                .map(|idx| &self.extended_glyphs[idx].1)
        }
    }

    pub fn serialize_baked(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        // Magic
        bytes.extend_from_slice(b"RQFN");
        // Version
        bytes.extend_from_slice(&1u16.to_le_bytes());
        // Global metrics
        bytes.extend_from_slice(&self.base_size.to_le_bytes());
        bytes.extend_from_slice(&self.ascent.to_le_bytes());
        bytes.extend_from_slice(&self.descent.to_le_bytes());
        bytes.extend_from_slice(&self.line_gap.to_le_bytes());

        // Count total glyphs (ASCII + extended)
        let ascii_count = self.ascii_glyphs.iter().filter(|g| g.is_some()).count();
        let total_glyphs = (ascii_count + self.extended_glyphs.len()) as u32;
        bytes.extend_from_slice(&total_glyphs.to_le_bytes());

        // Write ASCII glyphs
        for (i, metric_opt) in self.ascii_glyphs.iter().enumerate() {
            if let Some(metric) = metric_opt {
                let code = (32 + i) as u32;
                Self::write_glyph_metric(&mut bytes, code, metric);
            }
        }

        // Write Extended glyphs
        for (ch, metric) in self.extended_glyphs.iter() {
            Self::write_glyph_metric(&mut bytes, *ch as u32, metric);
        }

        bytes
    }

    fn write_glyph_metric(bytes: &mut Vec<u8>, code: u32, metric: &GlyphMetric) {
        bytes.extend_from_slice(&code.to_le_bytes());
        if let Some(rect) = metric.rect {
            bytes.push(1);
            bytes.extend_from_slice(&rect.x.to_le_bytes());
            bytes.extend_from_slice(&rect.y.to_le_bytes());
            bytes.extend_from_slice(&rect.w.to_le_bytes());
            bytes.extend_from_slice(&rect.h.to_le_bytes());
        } else {
            bytes.push(0);
        }
        bytes.extend_from_slice(&metric.left.to_le_bytes());
        bytes.extend_from_slice(&metric.top.to_le_bytes());
        bytes.extend_from_slice(&metric.advance.to_le_bytes());
    }

    pub fn deserialize_baked(data: &[u8], atlas: Option<FontAtlas>) -> Result<Self, Error> {
        if data.len() < 26 || &data[0..4] != b"RQFN" {
            return Err(Error::FontError("Invalid font binary format"));
        }
        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != 1 {
            return Err(Error::FontError("Unsupported font binary version"));
        }
        let base_size = f32::from_le_bytes(data[6..10].try_into().unwrap());
        let ascent = f32::from_le_bytes(data[10..14].try_into().unwrap());
        let descent = f32::from_le_bytes(data[14..18].try_into().unwrap());
        let line_gap = f32::from_le_bytes(data[18..22].try_into().unwrap());
        let total_glyphs = u32::from_le_bytes(data[22..26].try_into().unwrap()) as usize;

        let mut offset = 26;
        let mut ascii_glyphs = [None; 96];
        let mut extended_glyphs = Vec::new();

        for _ in 0..total_glyphs {
            if offset + 4 > data.len() {
                return Err(Error::FontError("Truncated font binary data"));
            }
            let code = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            offset += 4;
            if offset >= data.len() {
                return Err(Error::FontError("Truncated font binary data"));
            }
            let has_rect = data[offset];
            offset += 1;
            let rect = if has_rect == 1 {
                if offset + 16 > data.len() {
                    return Err(Error::FontError("Truncated font binary data"));
                }
                let rx = f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
                let ry = f32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
                let rw = f32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap());
                let rh = f32::from_le_bytes(data[offset + 12..offset + 16].try_into().unwrap());
                offset += 16;
                Some(crate::math::Rect::new(rx, ry, rw, rh))
            } else {
                None
            };
            if offset + 12 > data.len() {
                return Err(Error::FontError("Truncated font binary data"));
            }
            let left = i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            let top = i32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
            let advance = f32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap());
            offset += 12;

            let metric = GlyphMetric {
                rect,
                left,
                top,
                advance,
            };

            if (32..128).contains(&code) {
                ascii_glyphs[(code - 32) as usize] = Some(metric);
            } else if let Some(ch) = char::from_u32(code) {
                extended_glyphs.push((ch, metric));
            }
        }

        extended_glyphs.sort_unstable_by_key(|(ch, _)| *ch);

        Ok(Self {
            atlas,
            atlas_image: None,
            base_size,
            ascent,
            descent,
            line_gap,
            ascii_glyphs,
            extended_glyphs: extended_glyphs.into_boxed_slice(),
        })
    }
}

/// Parameters used to construct an immutable font atlas from TTF.
#[derive(Debug, Clone)]
pub struct FontLoadParams {
    /// Characters whose nominal glyphs will be stored in the atlas.
    pub characters: Vec<char>,
    /// Canonical raster size. Drawing scales these cached glyphs.
    pub raster_size: u16,
    /// Texture filtering used by the finished atlas.
    pub filter: miniquad::FilterMode,
}

impl Default for FontLoadParams {
    fn default() -> Self {
        Self {
            characters: Font::ascii_character_list(),
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
            .field("id", &self.id())
            .field("base_size", &self.data.base_size)
            .finish()
    }
}

impl FontData {
    #[cfg(feature = "font-rasterize")]
    pub fn bake_from_ttf_bytes(
        bytes: &[u8],
        params: FontLoadParams,
    ) -> Result<(Self, crate::texture::Image), Error> {
        let font = FontRef::from_index(bytes, 0).ok_or(Error::FontError(
            "Font parser could not parse the supplied font data",
        ))?;
        if font.variations().len() != 0 {
            return Err(Error::FontError("Variable fonts are not supported"));
        }
        if params.raster_size == 0 {
            return Err(Error::FontError(
                "Font raster size must be greater than zero",
            ));
        }

        let mut atlas = FontAtlasBuilder::new();
        let mut rasterizer = Rasterizer::new();
        let base_size = params.raster_size as f32;
        let font_metrics = font.metrics(&[]).scale(base_size);
        let glyph_metrics = font.glyph_metrics(&[]).scale(base_size);

        let mut ascii_glyphs = [None; 96];
        let mut extended_glyphs = Vec::new();

        let mut chars = params.characters;
        if !chars.contains(&'?') {
            chars.push('?');
        }
        if !chars.contains(&' ') {
            chars.push(' ');
        }

        let mut rasterized_cache: foldhash::HashMap<u16, (Option<crate::math::Rect>, i32, i32)> =
            foldhash::HashMap::default();

        for ch in chars {
            let glyph_id = font.charmap().map(ch);
            let advance = glyph_metrics.advance_width(glyph_id);

            let (rect, left, top) = if ch.is_whitespace() || glyph_id == 0 {
                (None, 0, 0)
            } else if let Some(cached) = rasterized_cache.get(&glyph_id) {
                *cached
            } else if let Some(rendered) = rasterizer.rasterize(bytes, 0, glyph_id, base_size) {
                let rect = if rendered.image.width == 0 || rendered.image.height == 0 {
                    None
                } else {
                    Some(atlas.insert(&rendered.image).ok_or(Error::FontError(
                        "The font atlas is too small for the requested characters",
                    ))?)
                };
                let info = (rect, rendered.left, rendered.top);
                rasterized_cache.insert(glyph_id, info);
                info
            } else {
                (None, 0, 0)
            };

            let metric = GlyphMetric {
                rect,
                left,
                top,
                advance,
            };

            let code = ch as u32;
            if (32..128).contains(&code) {
                ascii_glyphs[(code - 32) as usize] = Some(metric);
            } else {
                extended_glyphs.push((ch, metric));
            }
        }

        extended_glyphs.sort_unstable_by_key(|(ch, _)| *ch);
        let image = atlas.into_image();

        Ok((
            Self {
                atlas: None,
                atlas_image: Some(Arc::new(image.clone())),
                base_size,
                ascent: font_metrics.ascent,
                descent: font_metrics.descent,
                line_gap: font_metrics.leading,
                ascii_glyphs,
                extended_glyphs: extended_glyphs.into_boxed_slice(),
            },
            image,
        ))
    }
}

impl Font {
    #[cfg(feature = "font-rasterize")]
    pub(crate) fn load_from_bytes(
        bytes: &[u8],
        params: FontLoadParams,
        ctx: &mut dyn miniquad::RenderingBackend,
    ) -> Result<Self, Error> {
        let filter = params.filter;
        let (mut font_data, image) = FontData::bake_from_ttf_bytes(bytes, params)?;
        let atlas = FontAtlas::from_rgba8(image.width, image.height, &image.bytes, filter, ctx);
        font_data.atlas = Some(atlas);
        Ok(Self {
            data: Arc::new(font_data),
        })
    }

    /// Loads a pre-baked font from metric binary (`.bin`) and PNG atlas texture bytes.
    /// This works in production with 0 font dependencies.
    pub fn load_baked_from_bytes(
        bin_bytes: &[u8],
        png_bytes: &[u8],
        filter: miniquad::FilterMode,
        ctx: &mut dyn miniquad::RenderingBackend,
    ) -> Result<Self, Error> {
        let (width, height, rgba) = crate::texture::decode_png_rgba8(png_bytes)?;
        let atlas = FontAtlas::from_rgba8(width, height, &rgba, filter, ctx);
        let data = FontData::deserialize_baked(bin_bytes, Some(atlas))?;
        Ok(Self {
            data: Arc::new(data),
        })
    }

    /// Exports the loaded font asset as pre-baked `.bin` metrics and `.png` atlas image.
    pub fn export_baked(&self, bin_path: &str, png_path: &str) -> Result<(), Error> {
        let bin_data = self.data.serialize_baked();
        std::fs::write(bin_path, bin_data)
            .map_err(|_| Error::FontError("Failed to write font .bin file"))?;
        if let Some(image) = &self.data.atlas_image {
            let file = std::fs::File::create(png_path)
                .map_err(|_| Error::FontError("Failed to create font .png file"))?;
            let mut encoder = png::Encoder::new(file, image.width.into(), image.height.into());
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .map_err(|_| Error::FontError("Failed to write png header"))?;
            writer
                .write_image_data(&image.bytes)
                .map_err(|_| Error::FontError("Failed to encode png data"))?;
        } else {
            return Err(Error::FontError(
                "Font atlas image is not available for export",
            ));
        }
        Ok(())
    }

    pub(crate) fn id(&self) -> FontId {
        FontId(Arc::as_ptr(&self.data) as usize)
    }

    pub(crate) fn atlas(&self) -> &FontAtlas {
        self.data
            .atlas
            .as_ref()
            .expect("renderable fonts always have an atlas")
    }

    pub fn raster_size(&self) -> f32 {
        self.data.base_size
    }

    #[cfg(all(test, feature = "font-rasterize"))]
    pub(crate) fn load_for_test(bytes: &[u8]) -> Result<Self, Error> {
        let font = FontRef::from_index(bytes, 0).ok_or(Error::FontError(
            "Font parser could not parse the supplied font data",
        ))?;
        let base_size = renderer::BASE_FONT_SIZE;
        let font_metrics = font.metrics(&[]).scale(base_size);
        let glyph_metrics = font.glyph_metrics(&[]).scale(base_size);

        let mut ascii_glyphs = [None; 96];
        for ch in ' '..='~' {
            let glyph_id = font.charmap().map(ch);
            let advance = glyph_metrics.advance_width(glyph_id);
            let code = ch as u32;
            ascii_glyphs[(code - 32) as usize] = Some(GlyphMetric {
                rect: None,
                left: 0,
                top: 0,
                advance,
            });
        }

        Ok(Self {
            data: Arc::new(FontData {
                atlas: None,
                atlas_image: None,
                base_size,
                ascent: font_metrics.ascent,
                descent: font_metrics.descent,
                line_gap: font_metrics.leading,
                ascii_glyphs,
                extended_glyphs: Box::default(),
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

    #[test]
    fn binary_serialization_roundtrip() {
        let mut ascii_glyphs = [None; 96];
        ascii_glyphs[0] = Some(super::GlyphMetric {
            rect: Some(crate::math::Rect::new(2.0, 4.0, 8.0, 12.0)),
            left: 1,
            top: 10,
            advance: 8.5,
        });

        let extended = vec![(
            '中',
            super::GlyphMetric {
                rect: Some(crate::math::Rect::new(10.0, 4.0, 16.0, 16.0)),
                left: 0,
                top: 14,
                advance: 16.0,
            },
        )];

        let font_data = super::FontData {
            atlas: None,
            atlas_image: None,
            base_size: 32.0,
            ascent: 26.0,
            descent: 6.0,
            line_gap: 2.0,
            ascii_glyphs,
            extended_glyphs: extended.into_boxed_slice(),
        };

        let bin = font_data.serialize_baked();
        let loaded = super::FontData::deserialize_baked(&bin, None).unwrap();

        assert_eq!(loaded.base_size, 32.0);
        assert_eq!(loaded.ascent, 26.0);
        assert_eq!(loaded.descent, 6.0);
        assert_eq!(loaded.line_gap, 2.0);
        assert_eq!(loaded.get_glyph(' '), font_data.get_glyph(' '));
        assert_eq!(loaded.get_glyph('中'), font_data.get_glyph('中'));
    }

    #[test]
    #[cfg(feature = "font-rasterize")]
    fn bake_proggy_clean_assets() {
        let ttf_bytes = include_bytes!("../../assets/fonts/ProggyClean.ttf");
        let (font_data, image) =
            super::FontData::bake_from_ttf_bytes(ttf_bytes, super::FontLoadParams::default())
                .unwrap();
        let bin = font_data.serialize_baked();
        std::fs::write("assets/fonts/ProggyClean.bin", bin).unwrap();

        let file = std::fs::File::create("assets/fonts/ProggyClean.png").unwrap();
        let mut encoder = png::Encoder::new(file, image.width.into(), image.height.into());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&image.bytes).unwrap();
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

/// Parameters used to prepare a reusable [`TextLayout`].
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
    /// Number of complete reveal clusters to reveal.
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

/// Positioned text that can be drawn repeatedly without layout work.
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
            .map(|cluster| cluster.source.start as usize..cluster.source.end as usize)
    }
}

/// Prepares text once for repeated measurement and drawing.
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
    let base_scale = params.font_size as f32 / font.data.base_size;
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
                    span.range.start < cluster.source.end as usize
                        && (cluster.source.start as usize) < span.range.end
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

#[cfg(feature = "font-rasterize")]
pub async fn load_ttf_font(path: &str) -> Result<Font, Error> {
    load_ttf_font_ex(path, FontLoadParams::default()).await
}

#[cfg(feature = "font-rasterize")]
pub async fn load_ttf_font_ex(path: &str, params: FontLoadParams) -> Result<Font, Error> {
    let bytes = load_file(path)
        .await
        .map_err(|_| Error::FontError("The Font file couldn't be loaded"))?;
    load_ttf_font_from_bytes_ex(&bytes, params)
}

#[cfg(feature = "font-rasterize")]
pub fn load_ttf_font_from_bytes(bytes: &[u8]) -> Result<Font, Error> {
    load_ttf_font_from_bytes_ex(bytes, FontLoadParams::default())
}

#[cfg(feature = "font-rasterize")]
pub fn load_ttf_font_from_bytes_ex(bytes: &[u8], params: FontLoadParams) -> Result<Font, Error> {
    let context = get_context();
    Font::load_from_bytes(bytes, params, &mut *context.quad_context)
}

/// Loads a pre-baked font asset (bin metrics + png atlas) asynchronously from file paths.
pub async fn load_baked_font(
    bin_path: &str,
    png_path: &str,
    filter: miniquad::FilterMode,
) -> Result<Font, Error> {
    let bin_bytes = load_file(bin_path)
        .await
        .map_err(|_| Error::FontError("Could not load font .bin file"))?;
    let png_bytes = load_file(png_path)
        .await
        .map_err(|_| Error::FontError("Could not load font .png file"))?;
    let context = get_context();
    Font::load_baked_from_bytes(&bin_bytes, &png_bytes, filter, &mut *context.quad_context)
}

/// Loads a pre-baked font asset (bin metrics + png atlas) synchronously from byte slices.
pub fn load_baked_font_from_bytes(
    bin_bytes: &[u8],
    png_bytes: &[u8],
    filter: miniquad::FilterMode,
) -> Result<Font, Error> {
    let context = get_context();
    Font::load_baked_from_bytes(bin_bytes, png_bytes, filter, &mut *context.quad_context)
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
    let context = get_context();
    let font = font
        .cloned()
        .unwrap_or_else(|| context.fonts_storage.default_font.clone());
    let fallback = font.data.get_glyph('?');
    let scale = font_size as f32 / font.data.base_size * font_scale;
    let mut output = String::with_capacity(text.len());
    let mut line = String::new();
    let mut line_width = 0.0;

    for character in text.chars() {
        if character == '\n' {
            output.push_str(&line);
            output.push('\n');
            line.clear();
            line_width = 0.0;
            continue;
        }

        let advance = font
            .data
            .get_glyph(character)
            .or(fallback)
            .map_or(0.0, |metric| metric.advance * scale);
        if !line.is_empty() && line_width + advance > maximum_line_length {
            output.push_str(&line);
            output.push('\n');
            line.clear();
            line_width = 0.0;
        }
        line.push(character);
        line_width += advance;
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
    pub(crate) default_font: Font,
}

impl FontsStorage {
    pub(crate) fn new(
        ctx: &mut dyn miniquad::RenderingBackend,
        _text_renderer: &mut TextRenderer,
        filter: miniquad::FilterMode,
    ) -> Self {
        #[cfg(feature = "font-rasterize")]
        let default_font = Font::load_from_bytes(
            include_bytes!("../../assets/fonts/ProggyClean.ttf"),
            FontLoadParams {
                filter,
                ..Default::default()
            },
            ctx,
        )
        .unwrap();

        #[cfg(not(feature = "font-rasterize"))]
        let default_font = Font::load_baked_from_bytes(
            include_bytes!("../../assets/fonts/ProggyClean.bin"),
            include_bytes!("../../assets/fonts/ProggyClean.png"),
            filter,
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
