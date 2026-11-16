#![cfg(feature = "font-rasterize")]

use swash::{
    scale::{image::Content, Render, ScaleContext, Source, StrikeWith},
    zeno::Format,
    FontRef,
};

use super::sdf;
use crate::texture::Image;

pub(crate) struct RasterizedGlyph {
    pub(crate) image: Image,
    pub(crate) left: i16,
    pub(crate) top: i16,
}

pub(crate) struct Rasterizer {
    context: ScaleContext,
}

impl Rasterizer {
    pub(crate) fn new() -> Self {
        Self {
            context: ScaleContext::new(),
        }
    }

    pub(crate) fn rasterize(
        &mut self,
        font_data: &[u8],
        font_index: usize,
        glyph_id: u16,
        size: f32,
    ) -> Option<RasterizedGlyph> {
        let font_ref = FontRef::from_index(font_data, font_index)?;
        let mut scaler = self.context.builder(font_ref).size(size).hint(true).build();
        let rendered = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ])
        .format(Format::Alpha)
        .render(&mut scaler, glyph_id)?;

        let width = rendered.placement.width.try_into().ok()?;
        let height = rendered.placement.height.try_into().ok()?;
        let bytes = match rendered.content {
            Content::Mask => rendered
                .data
                .into_iter()
                .flat_map(|alpha| [255, 255, 255, alpha])
                .collect(),
            Content::Color | Content::SubpixelMask => rendered.data,
        };
        Some(RasterizedGlyph {
            image: Image {
                bytes,
                width,
                height,
            },
            left: rendered.placement.left as i16,
            top: rendered.placement.top as i16,
        })
    }

    /// Renders a glyph as a signed distance field with `spread` px of padding on each side.
    ///
    /// The outline is rendered unhinted at [`sdf::SUPERSAMPLE`] times `size`; hinting snaps
    /// to the supersampled grid, which would distort the downsampled shape. Color glyphs are
    /// rendered from their outline only. The returned image is RGBA8 `[255, 255, 255, value]`.
    pub(crate) fn rasterize_sdf(
        &mut self,
        font_data: &[u8],
        font_index: usize,
        glyph_id: u16,
        size: f32,
        spread: u8,
    ) -> Option<RasterizedGlyph> {
        let font_ref = FontRef::from_index(font_data, font_index)?;
        let supersample = sdf::SUPERSAMPLE;
        let mut scaler = self
            .context
            .builder(font_ref)
            .size(size * supersample as f32)
            .hint(false)
            .build();
        let rendered = Render::new(&[Source::Outline])
            .format(Format::Alpha)
            .render(&mut scaler, glyph_id)?;
        let placement = rendered.placement;
        if placement.width == 0 || placement.height == 0 {
            return Some(RasterizedGlyph {
                image: Image::empty(),
                left: 0,
                top: 0,
            });
        }

        let field = sdf::glyph_sdf(
            &rendered.data,
            placement.width,
            placement.height,
            placement.left,
            placement.top,
            supersample,
            spread.into(),
        );
        Some(RasterizedGlyph {
            image: Image {
                bytes: field
                    .values
                    .into_iter()
                    .flat_map(|value| [255, 255, 255, value])
                    .collect(),
                width: field.width.try_into().ok()?,
                height: field.height.try_into().ok()?,
            },
            left: field.left.try_into().ok()?,
            top: field.top.try_into().ok()?,
        })
    }
}
