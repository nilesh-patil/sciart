use crate::title::{self, Title};
use crate::{Canvas, Ctx, Generator, GlyphMode, Meta, Size};

/// Owns one generator plus its context and a double-buffered canvas, and is the single
/// place that implements the call protocol (CORE-R15, CORE-R19..R21) and draws the
/// science title (CORE-R75).
pub struct Stage {
    generator: Box<dyn Generator>,
    ctx: Ctx,
    front: Canvas,
    back: Canvas,
    title_visible: bool,
    /// The caption after the last step and the scene time it appeared: the title fades by
    /// its age, so every new act's caption shows at full strength (CORE-R75).
    caption: Option<&'static str>,
    caption_since: f64,
}

impl std::fmt::Debug for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stage")
            .field("generator", &self.generator.meta().name)
            .field("ctx", &self.ctx)
            .field("title_visible", &self.title_visible)
            .finish_non_exhaustive()
    }
}

impl Stage {
    /// The title starts visible (CORE-R75), so every consumer sees the frame the runtime
    /// shows unless it opts out.
    pub fn new(mut generator: Box<dyn Generator>, seed: u64, size: Size, mode: GlyphMode) -> Self {
        let mut ctx = Ctx::new(seed, size);
        generator.resize(size, &mut ctx);
        let caption = generator.caption();
        Self {
            generator,
            ctx,
            front: Canvas::new(size, mode),
            back: Canvas::new(size, mode),
            title_visible: true,
            caption,
            caption_since: 0.0,
        }
    }

    pub fn meta(&self) -> &'static Meta {
        self.generator.meta()
    }

    pub fn ctx(&self) -> &Ctx {
        &self.ctx
    }

    /// The live title: `meta().title` with the current caption (CORE-R71). Also what the
    /// HUD shows while the canvas title is hidden under it (RT-R12, RND-R9).
    pub fn title(&self) -> Title {
        Title::of(self.generator.as_ref())
    }

    /// Whether `render` draws the title (CORE-R75); kept across `resize`.
    pub fn set_title_visible(&mut self, visible: bool) {
        self.title_visible = visible;
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
        let caption = self.generator.caption();
        if caption != self.caption {
            self.caption = caption;
            self.caption_since = self.ctx.time();
        }
    }

    /// Draw a new frame, the title over the scene; on a generator panic the previous front
    /// frame is untouched.
    pub fn render(&mut self) -> &Canvas {
        self.back.clear();
        self.generator.render(&mut self.back);
        if self.title_visible {
            let (title, age) = (self.title(), self.ctx.time() - self.caption_since);
            title::draw(&mut self.back, &title, age);
        }
        std::mem::swap(&mut self.front, &mut self.back);
        &self.front
    }

    /// Last completely rendered frame.
    pub fn frame(&self) -> &Canvas {
        &self.front
    }
}
