use crate::{Canvas, Ctx, Generator, GlyphMode, Meta, Size};

/// Owns one generator plus its context and a double-buffered canvas, and is the single
/// place that implements the call protocol (CORE-R15, CORE-R19..R21).
pub struct Stage {
    generator: Box<dyn Generator>,
    ctx: Ctx,
    front: Canvas,
    back: Canvas,
}

impl std::fmt::Debug for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stage")
            .field("generator", &self.generator.meta().name)
            .field("ctx", &self.ctx)
            .finish_non_exhaustive()
    }
}

impl Stage {
    pub fn new(mut generator: Box<dyn Generator>, seed: u64, size: Size, mode: GlyphMode) -> Self {
        let mut ctx = Ctx::new(seed, size);
        generator.resize(size, &mut ctx);
        Self {
            generator,
            ctx,
            front: Canvas::new(size, mode),
            back: Canvas::new(size, mode),
        }
    }

    pub fn meta(&self) -> &'static Meta {
        self.generator.meta()
    }

    pub fn ctx(&self) -> &Ctx {
        &self.ctx
    }

    pub fn resize(&mut self, size: Size) {
        if size != self.ctx.size() {
            self.ctx.set_size(size);
            self.generator.resize(size, &mut self.ctx);
            self.front.resize(size);
            self.back.resize(size);
        }
    }

    pub fn set_glyph_mode(&mut self, mode: GlyphMode) {
        self.front.set_mode(mode);
        self.back.set_mode(mode);
    }

    pub fn step(&mut self, dt: f64) {
        self.generator.update(dt, &mut self.ctx);
        self.ctx.advance(dt);
    }

    /// Draw a new frame; on a generator panic the previous front frame is untouched.
    pub fn render(&mut self) -> &Canvas {
        self.back.clear();
        self.generator.render(&mut self.back);
        std::mem::swap(&mut self.front, &mut self.back);
        &self.front
    }

    /// Last completely rendered frame.
    pub fn frame(&self) -> &Canvas {
        &self.front
    }
}
