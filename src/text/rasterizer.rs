use swash::{
    scale::{image::Content, Render, ScaleContext, Source, StrikeWith},
    zeno::Format,
    FontRef,
};

use crate::texture::Image;

pub(crate) struct RasterizedGlyph {
    pub(crate) image: Image,
    pub(crate) left: i32,
    pub(crate) top: i32,
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
            left: rendered.placement.left,
            top: rendered.placement.top,
        })
    }
}
