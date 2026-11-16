use crate::{
    get_context,
    texture::{Image, Texture2D},
};

/// A glyph's pixel rectangle inside a font atlas.
///
/// Stored as `u16` because atlas dimensions are `u16`, which keeps per-glyph data small.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasRect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl AtlasRect {
    pub const fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self { x, y, w, h }
    }
}

pub struct FontAtlas {
    pub texture: miniquad::TextureId,
    pub width: u16,
    pub height: u16,
}

impl FontAtlas {
    pub(crate) fn texture(&self) -> Texture2D {
        Texture2D::unmanaged(self.texture)
    }

    pub(crate) const fn size(&self) -> (f32, f32) {
        (self.width as f32, self.height as f32)
    }

    pub(crate) fn from_rgba8(
        width: u16,
        height: u16,
        bytes: &[u8],
        filter: miniquad::FilterMode,
        ctx: &mut miniquad::Context,
    ) -> Self {
        use miniquad::RenderingBackend;
        let texture = ctx.new_texture_from_rgba8(width, height, bytes);
        ctx.texture_set_filter(texture, filter, miniquad::MipmapFilterMode::None);
        Self {
            texture,
            width,
            height,
        }
    }
}

impl Drop for FontAtlas {
    fn drop(&mut self) {
        use miniquad::RenderingBackend;
        get_context().quad_context.delete_texture(self.texture);
    }
}

/// A horizontal strip of the atlas that glyphs are packed into left to right.
#[derive(Debug, Clone, Copy)]
struct Shelf {
    y: u16,
    height: u16,
    /// End of the last glyph on this shelf, plus `GAP`.
    cursor_x: u16,
}

pub(crate) struct FontAtlasBuilder {
    image: Image,
    shelves: Vec<Shelf>,
}

impl FontAtlasBuilder {
    // Transparent texels kept on each side of a glyph (so 2 between neighbours).
    // A fragment is drawn only if its center is inside the glyph quad, so its UV is
    // inside the glyph rect, and bilinear filtering reads at most 0.5 texel past it:
    // 1 transparent texel is enough. This holds only without mipmaps; each mip level
    // would need the gap doubled.
    const GAP: u16 = 1;
    const WIDTH: u16 = 1024;
    const HEIGHT: u16 = 1024;

    pub(crate) fn new() -> Self {
        Self::with_size(Self::WIDTH, Self::HEIGHT)
    }

    fn with_size(width: u16, height: u16) -> Self {
        Self {
            image: Image::gen_image_color(width, height, crate::Color::new(0.0, 0.0, 0.0, 0.0)),
            shelves: Vec::new(),
        }
    }

    pub(crate) fn insert(&mut self, glyph: &Image) -> Option<AtlasRect> {
        let (width, height) = (glyph.width, glyph.height);
        let (x, y) = loop {
            if let Some(position) = self.place_on_existing_shelf(width, height)? {
                break position;
            }

            // Open a new shelf below the last one.
            let x = Self::GAP;
            let y = match self.shelves.last() {
                Some(shelf) => shelf.y.checked_add(shelf.height)?.checked_add(Self::GAP * 2)?,
                None => 0,
            };
            if x.checked_add(width)? > self.image.width {
                self.grow(true)?;
                continue;
            }
            if y.checked_add(height)? > self.image.height {
                self.grow_shorter_side()?;
                continue;
            }
            self.shelves.push(Shelf {
                y,
                height,
                cursor_x: x.checked_add(width)?.checked_add(Self::GAP)?,
            });
            break (x, y);
        };

        let atlas_stride = self.image.width as usize * 4;
        let glyph_stride = width as usize * 4;
        for row in 0..height as usize {
            let source = row * glyph_stride;
            let destination = (y as usize + row) * atlas_stride + x as usize * 4;
            self.image.bytes[destination..destination + glyph_stride]
                .copy_from_slice(&glyph.bytes[source..source + glyph_stride]);
        }

        Some(AtlasRect::new(x, y, width, height))
    }

    /// First-fit search over existing shelves. Earlier shelves are revisited because
    /// widening the atlas gives every shelf new room on the right. The last shelf may
    /// grow taller, since nothing is packed below it.
    ///
    /// Returns `Some(None)` when no shelf fits, and `None` on arithmetic overflow.
    fn place_on_existing_shelf(&mut self, width: u16, height: u16) -> Option<Option<(u16, u16)>> {
        let atlas_width = self.image.width;
        let atlas_height = self.image.height;
        let last_index = self.shelves.len().checked_sub(1);
        for (index, shelf) in self.shelves.iter_mut().enumerate() {
            let x = shelf.cursor_x.checked_add(Self::GAP)?;
            if x.checked_add(width)? > atlas_width {
                continue;
            }
            let fits_height = height <= shelf.height
                || (Some(index) == last_index && shelf.y.checked_add(height)? <= atlas_height);
            if !fits_height {
                continue;
            }
            shelf.cursor_x = x.checked_add(width)?.checked_add(Self::GAP)?;
            shelf.height = shelf.height.max(height);
            return Some(Some((x, shelf.y)));
        }
        Some(None)
    }

    /// Doubles the shorter side (width on ties) so the atlas stays close to square.
    /// A tall, narrow atlas would exceed the 4096 px texture limit common on WebGL
    /// and mobile GPUs much sooner than a square one.
    fn grow_shorter_side(&mut self) -> Option<()> {
        self.grow(self.image.width <= self.image.height)
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

    pub(crate) fn into_image(self) -> Image {
        self.image
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(width: u16, height: u16) -> Image {
        Image::gen_image_color(width, height, crate::Color::new(1.0, 1.0, 1.0, 1.0))
    }

    #[test]
    fn insertion_grows_the_backing_image_and_preserves_pixels() {
        let mut atlas = FontAtlasBuilder::with_size(4, 4);

        let rect = atlas.insert(&glyph(5, 5)).unwrap();

        assert_eq!(rect, AtlasRect::new(1, 0, 5, 5));
        assert_eq!((atlas.image.width, atlas.image.height), (8, 8));
        let first_glyph_pixel = rect.x as usize * 4;
        assert_eq!(
            &atlas.image.bytes[first_glyph_pixel..first_glyph_pixel + 4],
            &[255, 255, 255, 255]
        );
    }

    #[test]
    fn growth_alternates_sides_to_stay_square() {
        let mut atlas = FontAtlasBuilder::with_size(64, 64);
        let mut sizes = vec![(64, 64)];
        for _ in 0..200 {
            atlas.insert(&glyph(12, 12)).unwrap();
            let size = (atlas.image.width, atlas.image.height);
            if sizes.last() != Some(&size) {
                sizes.push(size);
            }
        }
        assert_eq!(
            sizes,
            vec![(64, 64), (128, 64), (128, 128), (256, 128), (256, 256)]
        );
    }

    #[test]
    fn widening_reuses_room_on_earlier_shelves() {
        let mut atlas = FontAtlasBuilder::with_size(16, 16);
        let first = atlas.insert(&glyph(12, 12)).unwrap();
        assert_eq!((first.x, first.y), (1, 0));
        // A new shelf at y = 14 does not fit (14 + 12 > 16). Width and height tie, so the
        // width doubles to 32 and the glyph lands beside the first one on the same shelf
        // (2 px apart: GAP after the first glyph plus GAP before the second).
        let second = atlas.insert(&glyph(12, 12)).unwrap();
        assert_eq!((atlas.image.width, atlas.image.height), (32, 16));
        assert_eq!((second.x, second.y), (15, 0));
    }

    #[test]
    fn glyphs_never_overlap() {
        let mut atlas = FontAtlasBuilder::with_size(32, 32);
        let mut rects = Vec::new();
        for index in 0..300u16 {
            let size = (3 + index % 7, 4 + index % 5);
            rects.push(atlas.insert(&glyph(size.0, size.1)).unwrap());
        }
        for rect in &rects {
            assert!(rect.x + rect.w <= atlas.image.width);
            assert!(rect.y + rect.h <= atlas.image.height);
        }
        for (index, a) in rects.iter().enumerate() {
            for b in &rects[index + 1..] {
                let apart = a.x + a.w + FontAtlasBuilder::GAP <= b.x
                    || b.x + b.w + FontAtlasBuilder::GAP <= a.x
                    || a.y + a.h + FontAtlasBuilder::GAP <= b.y
                    || b.y + b.h + FontAtlasBuilder::GAP <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        let (w, h) = (atlas.image.width, atlas.image.height);
        assert!(w.max(h) <= 2 * w.min(h), "atlas not near square: {w}x{h}");
    }
}
