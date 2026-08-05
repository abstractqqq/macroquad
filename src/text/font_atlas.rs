use crate::{
    get_context,
    math::Rect,
    texture::{Image, Texture2D},
};

pub(crate) struct FontAtlas {
    texture: miniquad::TextureId,
    width: u16,
    height: u16,
}

impl FontAtlas {
    pub(crate) fn texture(&self) -> Texture2D {
        Texture2D::unmanaged(self.texture)
    }

    pub(crate) const fn size(&self) -> (f32, f32) {
        (self.width as f32, self.height as f32)
    }
}

impl Drop for FontAtlas {
    fn drop(&mut self) {
        get_context().quad_context.delete_texture(self.texture);
    }
}

pub(crate) struct FontAtlasBuilder {
    image: Image,
    cursor_x: u16,
    cursor_y: u16,
    row_height: u16,
    filter: miniquad::FilterMode,
}

impl FontAtlasBuilder {
    const GAP: u16 = 2;
    const WIDTH: u16 = 1024;
    const HEIGHT: u16 = 1024;

    pub(crate) fn new(filter: miniquad::FilterMode) -> Self {
        Self {
            image: Image::gen_image_color(
                Self::WIDTH,
                Self::HEIGHT,
                crate::Color::new(0.0, 0.0, 0.0, 0.0),
            ),
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
            filter,
        }
    }

    pub(crate) fn insert(&mut self, glyph: &Image) -> Option<Rect> {
        let width = glyph.width;
        let height = glyph.height;
        let next_x = self.cursor_x.checked_add(Self::GAP)?;
        let fits_current_row = next_x.checked_add(width)? <= self.image.width;
        let (x, y) = if fits_current_row {
            (next_x, self.cursor_y)
        } else {
            (
                Self::GAP,
                self.cursor_y
                    .checked_add(self.row_height)?
                    .checked_add(Self::GAP * 2)?,
            )
        };
        if x.checked_add(width)? > self.image.width || y.checked_add(height)? > self.image.height {
            return None;
        }

        let atlas_stride = self.image.width as usize * 4;
        let glyph_stride = width as usize * 4;
        for row in 0..height as usize {
            let source = row * glyph_stride;
            let destination = (y as usize + row) * atlas_stride + x as usize * 4;
            self.image.bytes[destination..destination + glyph_stride]
                .copy_from_slice(&glyph.bytes[source..source + glyph_stride]);
        }

        if fits_current_row {
            self.cursor_x = x.checked_add(width)?.checked_add(Self::GAP)?;
            self.row_height = self.row_height.max(height);
        } else {
            self.cursor_x = x.checked_add(width)?.checked_add(Self::GAP)?;
            self.cursor_y = y;
            self.row_height = height;
        }

        Some(Rect::new(x as f32, y as f32, width as f32, height as f32))
    }

    pub(crate) fn finish(self, ctx: &mut dyn miniquad::RenderingBackend) -> FontAtlas {
        let texture =
            ctx.new_texture_from_rgba8(self.image.width, self.image.height, &self.image.bytes);
        ctx.texture_set_filter(texture, self.filter, miniquad::MipmapFilterMode::None);
        FontAtlas {
            texture,
            width: self.image.width,
            height: self.image.height,
        }
    }
}
