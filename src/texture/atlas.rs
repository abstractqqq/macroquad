use crate::{get_context, get_quad_context, math::Rect, texture::Image, Color};

use foldhash::{HashMap, HashMapExt};

#[derive(Debug, Clone, Copy)]
pub struct Sprite {
    pub rect: Rect,
}

pub type SpriteKey = miniquad::TextureId;
pub struct Atlas {
    pub texture: miniquad::TextureId,
    pub image: Image,
    pub sprites: HashMap<SpriteKey, Sprite>,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub max_line_height: u16,

    pub dirty: bool,

    pub filter: miniquad::FilterMode,
}

impl Drop for Atlas {
    fn drop(&mut self) {
        let ctx = &mut get_context().quad_context;
        ctx.delete_texture(self.texture);
    }
}

impl Atlas {
    // Pixel gap between packed textures.
    const GAP: u16 = 2;

    pub fn new(ctx: &mut dyn miniquad::RenderingBackend, filter: miniquad::FilterMode) -> Atlas {
        // rayquad's default was 512x512
        // Too small. Use 1024x1024 instead.
        let image = Image::gen_image_color(1024, 1024, Color::new(0.0, 0.0, 0.0, 0.0));
        let texture = ctx.new_texture_from_rgba8(image.width, image.height, &image.bytes);
        ctx.texture_set_filter(texture, filter, miniquad::MipmapFilterMode::None);

        Atlas {
            image,
            texture,
            cursor_x: 0,
            cursor_y: 0,
            dirty: false,
            max_line_height: 0,
            sprites: HashMap::new(),
            filter,
        }
    }

    pub fn set_filter(&mut self, filter_mode: miniquad::FilterMode) {
        let ctx = get_quad_context();
        self.set_filter_with(ctx, filter_mode);
    }

    pub(crate) fn set_filter_with(
        &mut self,
        ctx: &mut dyn miniquad::RenderingBackend,
        filter_mode: miniquad::FilterMode,
    ) {
        self.filter = filter_mode;
        ctx.texture_set_filter(self.texture, filter_mode, miniquad::MipmapFilterMode::None);
    }

    pub fn get(&self, key: SpriteKey) -> Option<Sprite> {
        self.sprites.get(&key).cloned()
    }

    pub fn texture(&mut self) -> miniquad::TextureId {
        let ctx = get_quad_context();
        self.flush(ctx);
        self.texture
    }

    pub(crate) fn flush(&mut self, ctx: &mut dyn miniquad::RenderingBackend) {
        if self.dirty {
            self.dirty = false;
            let (texture_width, texture_height) = ctx.texture_size(self.texture);
            if texture_width != self.image.width as _ || texture_height != self.image.height as _ {
                ctx.delete_texture(self.texture);

                self.texture = ctx.new_texture_from_rgba8(
                    self.image.width,
                    self.image.height,
                    &self.image.bytes[..],
                );
                ctx.texture_set_filter(self.texture, self.filter, miniquad::MipmapFilterMode::None);
            }

            ctx.texture_update(self.texture, &self.image.bytes);
        }
    }

    pub fn get_uv_rect(&self, key: SpriteKey) -> Option<Rect> {
        let ctx = get_quad_context();
        self.get(key).map(|sprite| {
            let (w, h) = ctx.texture_size(self.texture);

            Rect::new(
                sprite.rect.x / w as f32,
                sprite.rect.y / h as f32,
                sprite.rect.w / w as f32,
                sprite.rect.h / h as f32,
            )
        })
    }

    pub fn cache_sprite(&mut self, key: SpriteKey, sprite: Image) {
        let (width, height) = (sprite.width as usize, sprite.height as usize);

        let x = if self.cursor_x + (width as u16) < self.image.width {
            if height as u16 > self.max_line_height {
                self.max_line_height = height as u16;
            }
            let res = self.cursor_x + Self::GAP;
            self.cursor_x += width as u16 + Self::GAP * 2;
            res
        } else {
            self.cursor_y += self.max_line_height + Self::GAP * 2;
            self.cursor_x = width as u16 + Self::GAP;
            self.max_line_height = height as u16;
            Self::GAP
        };
        let y = self.cursor_y;

        // texture bounds exceeded
        if y + sprite.height > self.image.height || x + sprite.width > self.image.width {
            // Reset packing state and rebuild the larger texture atlas.
            let sprites = self.sprites.drain().collect::<Vec<_>>();
            self.cursor_x = 0;
            self.cursor_y = 0;
            self.max_line_height = 0;

            let old_image = self.image.clone();

            // Increase the texture atlas size.
            // note: if we tried to fit gigantic texture into a small atlas,
            // new_width will still be not enough. But its fine, it will
            // be regenerated on the recursion call.
            let new_width = self.image.width * 2;
            let new_height = self.image.height * 2;

            self.image =
                Image::gen_image_color(new_width, new_height, Color::new(0.0, 0.0, 0.0, 0.0));

            // recache all previously cached symbols
            for (key, sprite) in sprites {
                let image = old_image.sub_image(sprite.rect);
                self.cache_sprite(key, image);
            }

            // cache the new sprite
            self.cache_sprite(key, sprite);
        } else {
            self.dirty = true;

            let atlas_stride = self.image.width as usize * 4;
            let sprite_stride = width * 4;
            for row in 0..height {
                let source = row * sprite_stride;
                let destination = (y as usize + row) * atlas_stride + x as usize * 4;
                self.image.bytes[destination..destination + sprite_stride]
                    .copy_from_slice(&sprite.bytes[source..source + sprite_stride]);
            }

            self.sprites.insert(
                key,
                Sprite {
                    rect: Rect::new(x as f32, y as f32, width as f32, height as f32),
                },
            );
        }
    }
}
