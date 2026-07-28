use crate::{get_context, get_quad_context, math::Rect, texture::Image, Color};
use foldhash::{HashMap, HashMapExt};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Sprite {
    pub(crate) rect: Rect,
}

pub(crate) struct Atlas {
    texture: miniquad::TextureId,
    image: Image,
    sprites: HashMap<u64, Sprite>,
    cursor_x: u16,
    cursor_y: u16,
    max_line_height: u16,
    dirty: bool,
    filter: miniquad::FilterMode,
    next_id: u64,
}

impl Drop for Atlas {
    fn drop(&mut self) {
        get_context().quad_context.delete_texture(self.texture);
    }
}

impl Atlas {
    const GAP: u16 = 2;

    pub(crate) fn new(
        ctx: &mut dyn miniquad::RenderingBackend,
        filter: miniquad::FilterMode,
    ) -> Self {
        let image = Image::gen_image_color(1024, 1024, Color::new(0.0, 0.0, 0.0, 0.0));
        let texture = ctx.new_texture_from_rgba8(image.width, image.height, &image.bytes);
        ctx.texture_set_filter(texture, filter, miniquad::MipmapFilterMode::None);
        Self {
            texture,
            image,
            sprites: HashMap::new(),
            cursor_x: 0,
            cursor_y: 0,
            max_line_height: 0,
            dirty: false,
            filter,
            next_id: 0,
        }
    }

    pub(crate) fn set_filter(&mut self, filter: miniquad::FilterMode) {
        self.filter = filter;
        get_quad_context().texture_set_filter(
            self.texture,
            filter,
            miniquad::MipmapFilterMode::None,
        );
    }

    pub(crate) fn insert(&mut self, image: Image) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.cache(id, image);
        id
    }

    pub(crate) fn get(&self, id: u64) -> Option<Sprite> {
        self.sprites.get(&id).copied()
    }

    pub(crate) fn texture(&mut self) -> miniquad::TextureId {
        if self.dirty {
            self.dirty = false;
            let ctx = get_quad_context();
            let (width, height) = ctx.texture_size(self.texture);
            if width != self.image.width as _ || height != self.image.height as _ {
                ctx.delete_texture(self.texture);
                self.texture = ctx.new_texture_from_rgba8(
                    self.image.width,
                    self.image.height,
                    &self.image.bytes,
                );
                ctx.texture_set_filter(self.texture, self.filter, miniquad::MipmapFilterMode::None);
            } else {
                ctx.texture_update(self.texture, &self.image.bytes);
            }
        }
        self.texture
    }

    fn cache(&mut self, id: u64, image: Image) {
        let width = image.width;
        let height = image.height;
        let x = if self.cursor_x + width + Self::GAP < self.image.width {
            self.max_line_height = self.max_line_height.max(height);
            let x = self.cursor_x + Self::GAP;
            self.cursor_x += width + Self::GAP * 2;
            x
        } else {
            self.cursor_y += self.max_line_height + Self::GAP * 2;
            self.cursor_x = width + Self::GAP;
            self.max_line_height = height;
            Self::GAP
        };
        let y = self.cursor_y;

        if x + width > self.image.width || y + height > self.image.height {
            let old_image = self.image.clone();
            let old_sprites: Vec<_> = self.sprites.drain().collect();
            self.image = Image::gen_image_color(
                self.image.width.saturating_mul(2),
                self.image.height.saturating_mul(2),
                Color::new(0.0, 0.0, 0.0, 0.0),
            );
            self.cursor_x = 0;
            self.cursor_y = 0;
            self.max_line_height = 0;
            for (old_id, sprite) in old_sprites {
                self.cache(old_id, old_image.sub_image(sprite.rect));
            }
            self.cache(id, image);
            return;
        }

        for row in 0..height {
            for column in 0..width {
                self.image.set_pixel(
                    (x + column) as u32,
                    (y + row) as u32,
                    image.get_pixel(column as u32, row as u32),
                );
            }
        }
        self.sprites.insert(
            id,
            Sprite {
                rect: Rect::new(x as f32, y as f32, width as f32, height as f32),
            },
        );
        self.dirty = true;
    }
}
