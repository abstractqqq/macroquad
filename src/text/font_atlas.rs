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
    // Keep transparent texels between glyphs so linear filtering at a glyph's
    // edges cannot sample pixels from its neighbours.
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
        let (width, height) = (glyph.width, glyph.height);
        let (x, y, fits_current_row) = loop {
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

            if x.checked_add(width)? > self.image.width {
                self.grow(true)?;
                continue;
            }
            if y.checked_add(height)? > self.image.height {
                self.grow(false)?;
                continue;
            }
            break (x, y, fits_current_row);
        };

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

    fn grow(&mut self, width: bool) -> Option<()> {
        let new_width = if width {
            self.image.width.checked_mul(2)?
        } else {
            self.image.width
        };
        let new_height = if width {
            self.image.height
        } else {
            self.image.height.checked_mul(2)?
        };
        let mut grown =
            Image::gen_image_color(new_width, new_height, crate::Color::new(0.0, 0.0, 0.0, 0.0));

        let old_stride = self.image.width as usize * 4;
        let new_stride = new_width as usize * 4;
        for row in 0..self.image.height as usize {
            let old_start = row * old_stride;
            let new_start = row * new_stride;
            grown.bytes[new_start..new_start + old_stride]
                .copy_from_slice(&self.image.bytes[old_start..old_start + old_stride]);
        }
        self.image = grown;
        Some(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_grows_the_backing_image_and_preserves_pixels() {
        let mut atlas = FontAtlasBuilder {
            image: Image::gen_image_color(4, 4, crate::Color::new(0.0, 0.0, 0.0, 0.0)),
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
            filter: miniquad::FilterMode::Linear,
        };
        let glyph = Image::gen_image_color(5, 5, crate::Color::new(1.0, 1.0, 1.0, 1.0));

        let rect = atlas.insert(&glyph).unwrap();

        assert_eq!(rect, Rect::new(2.0, 0.0, 5.0, 5.0));
        assert_eq!((atlas.image.width, atlas.image.height), (8, 8));
        let first_glyph_pixel = rect.x as usize * 4;
        assert_eq!(
            &atlas.image.bytes[first_glyph_pixel..first_glyph_pixel + 4],
            &[255, 255, 255, 255]
        );
    }
}
