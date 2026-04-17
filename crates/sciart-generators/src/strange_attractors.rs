//! A turning gallery of chaotic attractors traced by swarms of glowing threads (SPEC-002 `strange_attractors`).
//!
//! M10 stub so the crate compiles; the scene replaces it.

use sciart_core::geom;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "strange_attractors",
    category: Category::PhysicsChemistry,
    description: "A turning gallery of chaotic attractors traced by swarms of glowing threads",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Stub::default())
}

/// Horizontal drift in cells per second (visible motion within 1 s, GEN-R9).
const SPEED: f64 = 8.0;

#[derive(Debug, Default)]
struct Stub {
    x: f64,
    width: f64,
    row: i32,
}

impl Generator for Stub {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.width = f64::from(size.width());
        self.row = i32::from(size.height()) / 2;
        self.x = geom::wrap(self.x, self.width);
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.x = geom::wrap(self.x + SPEED * dt, self.width);
    }

    fn render(&self, canvas: &mut Canvas) {
        canvas.put(self.x as i32, self.row, Glyph::new('•', '*'), Ink::new(Role::Particle, 255));
    }
}
