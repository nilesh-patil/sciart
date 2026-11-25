//! One scene, stepped and drawn without a terminal (LIB-R5, LIB-R6).

use std::panic::{AssertUnwindSafe, catch_unwind};

use sciart_core::{Canvas, GlyphMode, Meta, SIM_DT, Size};
use sciart_render::{AnsiColor, AnsiFraming, write_ansi};

use crate::{Error, SceneInfo, Theme};

/// Fixed steps per simulated second (ADR-0003).
const STEPS_PER_SECOND: f64 = 60.0;
/// The largest side a stage takes, in cells (LIB-R5); larger sizes are clamped to it.
const MAX_SIDE: u16 = 1024;
/// The most simulated seconds one `advance` call takes (LIB-R5).
const MAX_ADVANCE: f64 = 3600.0;

/// How a [`Stage`] starts: the seed, the glyph set and whether the science title is drawn.
///
/// `Options::default()` is seed 42 (the seed of `sciart record` and the snapshot tests),
/// Unicode glyphs and the title on.
#[derive(Debug, Clone)]
pub struct Options {
    seed: u64,
    ascii: bool,
    title: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            seed: 42,
            ascii: false,
            title: true,
        }
    }
}

impl Options {
    /// The seed of the scene's random numbers.
    #[must_use]
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Pure-ASCII glyphs instead of braille and half blocks.
    #[must_use]
    pub fn ascii(mut self, on: bool) -> Self {
        self.ascii = on;
        self
    }

    /// Whether the science title is drawn at the bottom left.
    #[must_use]
    pub fn title(mut self, on: bool) -> Self {
        self.title = on;
        self
    }
}

/// One running scene: a fixed-step simulation and the frame it draws.
///
/// Time only moves through [`advance`](Stage::advance), in whole steps of 1/60 s, so a
/// frame depends on the scene, the seed, the size and the simulated time, never on how
/// often it is drawn. Each `render_*` method draws the current frame again.
///
/// Sizes are in cells and clamped to 1..=1024 on each side.
///
/// # Panics
///
/// No method panics. If the scene itself panics, the stage reports an error of kind
/// [`ErrorKind::ScenePanicked`](crate::ErrorKind::ScenePanicked) and stays failed,
/// provided the crate is built with unwinding; built with `panic = "abort"`, the process
/// aborts.
#[derive(Debug)]
pub struct Stage {
    inner: sciart_core::Stage,
    meta: &'static Meta,
    /// Simulated seconds asked for so far, and the whole steps taken for them.
    total: f64,
    steps: u64,
    failed: bool,
}

impl Stage {
    /// Start `scene` (a name or a retired name) at `width` × `height` cells.
    pub fn new(scene: &str, width: u16, height: u16, options: Options) -> Result<Stage, Error> {
        let entry = sciart_generators::find(scene).ok_or_else(|| Error::unknown_scene(scene))?;
        Self::start(entry.meta, entry.build, width, height, options)
    }

    fn start(
        meta: &'static Meta,
        build: impl FnOnce() -> Box<dyn sciart_core::Generator>,
        width: u16,
        height: u16,
        options: Options,
    ) -> Result<Stage, Error> {
        let mode = if options.ascii {
            GlyphMode::Ascii
        } else {
            GlyphMode::Unicode
        };
        let size = clamp(width, height);
        // The constructor already sizes the scene, which runs scene code.
        let mut inner = catch_unwind(AssertUnwindSafe(|| {
            sciart_core::Stage::new(build(), options.seed, size, mode)
        }))
        .map_err(|_| Error::panicked(meta.name))?;
        inner.set_title_visible(options.title);
        Ok(Stage {
            inner,
            meta,
            total: 0.0,
            steps: 0,
            failed: false,
        })
    }

    /// The scene this stage runs.
    pub fn scene(&self) -> SceneInfo {
        SceneInfo::of(self.meta)
    }

    /// The size in cells, `(width, height)`.
    pub fn size(&self) -> (u16, u16) {
        let size = self.inner.ctx().size();
        (size.width(), size.height())
    }

    /// Simulated seconds so far: the steps taken divided by 60.
    pub fn time(&self) -> f64 {
        self.steps as f64 / STEPS_PER_SECOND
    }

    /// Whether the scene has panicked; a failed stage only returns errors of kind
    /// [`ErrorKind::ScenePanicked`](crate::ErrorKind::ScenePanicked).
    pub fn is_failed(&self) -> bool {
        self.failed
    }

    /// Change the size; the scene rebuilds what depends on it. Each side is clamped to
    /// 1..=1024.
    pub fn resize(&mut self, width: u16, height: u16) -> Result<(), Error> {
        self.check()?;
        let size = clamp(width, height);
        let inner = &mut self.inner;
        let result = catch_unwind(AssertUnwindSafe(|| inner.resize(size)));
        self.settle(result)
    }

    /// Move simulated time on by `seconds`, at most 3600 per call. The stage takes fixed
    /// steps of 1/60 s until `floor(60 · total + 1e-6)` steps have been taken in all, where
    /// `total` is every second asked for so far; so `advance(1.0)` equals sixty calls of
    /// `advance(1.0 / 60.0)`. A negative, non-finite or larger `seconds` is an error of kind
    /// [`ErrorKind::InvalidInput`](crate::ErrorKind::InvalidInput) and steps nothing. The
    /// cost is linear in the simulated time.
    pub fn advance(&mut self, seconds: f64) -> Result<(), Error> {
        self.check()?;
        if !(seconds.is_finite() && (0.0..=MAX_ADVANCE).contains(&seconds)) {
            return Err(Error::invalid(
                "advance takes a finite number of seconds in 0..=3600",
            ));
        }
        self.total += seconds;
        let target = (self.total * STEPS_PER_SECOND + 1e-6).floor() as u64;
        let (inner, steps) = (&mut self.inner, &mut self.steps);
        let result = catch_unwind(AssertUnwindSafe(|| {
            while *steps < target {
                inner.step(SIM_DT);
                *steps += 1;
            }
        }));
        self.settle(result)
    }

    /// Draw the current frame as plain text: one line per row, each ending in `\n`.
    pub fn render_text(&mut self) -> Result<String, Error> {
        let canvas = self.draw()?;
        let mut out =
            String::with_capacity(canvas.cells().len() + usize::from(canvas.size().height()));
        for line in canvas.to_lines() {
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }

    /// Draw the current frame as ANSI text in `theme`: one line per row with truecolor SGR
    /// sequences, a reset at the end of every line and a final newline. Cells without a
    /// background of their own stay on the terminal's background. These are the bytes
    /// `sciart frame` prints, and the rows `sciart record --format asciicast` embeds.
    pub fn render_ansi(&mut self, theme: Theme) -> Result<String, Error> {
        let canvas = self.draw()?;
        let mut out = String::with_capacity(canvas.cells().len() * 4);
        write_ansi(
            canvas,
            theme.render(),
            AnsiColor::Truecolor,
            AnsiFraming::Lines,
            &mut out,
        );
        Ok(out)
    }

    /// Draw the current frame in `theme` and write it to `out` as an 8-bit RGB PNG, 8 × 16
    /// pixels per cell: the renderer of `sciart record`. Needs the `png` feature.
    ///
    /// ```
    /// use sciart::{Options, Stage, Theme};
    ///
    /// let mut stage = Stage::new("pulsar", 20, 6, Options::default())?;
    /// let mut png = Vec::new();
    /// stage.render_png(Theme::Mono, &mut png)?;
    /// assert!(png.starts_with(b"\x89PNG"));
    /// # Ok::<(), sciart::Error>(())
    /// ```
    #[cfg(feature = "png")]
    pub fn render_png<W: std::io::Write>(&mut self, theme: Theme, out: W) -> Result<(), Error> {
        self.check()?;
        let options = sciart_capture::RenderOptions {
            theme: theme.render(),
            ascii: self.inner.glyph_mode() == GlyphMode::Ascii,
        };
        let inner = &mut self.inner;
        let image = catch_unwind(AssertUnwindSafe(|| {
            sciart_capture::render_frame(inner, &options)
        }));
        let Ok(image) = image else {
            self.failed = true;
            return Err(Error::panicked(self.meta.name));
        };
        sciart_capture::encode_png(&image, out).map_err(Error::io)
    }

    /// Start a scene written against the internal crates (`unstable-generator`, no semver
    /// promise). Each side is clamped to 1..=1024.
    #[cfg(feature = "unstable-generator")]
    pub fn from_generator(
        generator: Box<dyn sciart_core::Generator>,
        width: u16,
        height: u16,
        options: Options,
    ) -> Result<Stage, Error> {
        let meta = generator.meta();
        Self::start(meta, move || generator, width, height, options)
    }

    /// Draw a new frame (LIB-R6: a scene panic fails the stage).
    pub(crate) fn draw(&mut self) -> Result<&Canvas, Error> {
        self.check()?;
        let inner = &mut self.inner;
        let result = catch_unwind(AssertUnwindSafe(|| {
            inner.render();
        }));
        self.settle(result)?;
        Ok(self.inner.frame())
    }

    fn check(&self) -> Result<(), Error> {
        if self.failed {
            Err(Error::panicked(self.meta.name))
        } else {
            Ok(())
        }
    }

    fn settle(&mut self, result: std::thread::Result<()>) -> Result<(), Error> {
        if result.is_err() {
            self.failed = true;
        }
        self.check()
    }
}

/// LIB-R5: each side in 1..=1024 cells.
fn clamp(width: u16, height: u16) -> Size {
    Size::new(width.clamp(1, MAX_SIDE), height.clamp(1, MAX_SIDE))
}

#[cfg(test)]
pub(crate) mod tests {
    use sciart_core::{Category, Ctx, Generator, Glyph, Ink, Role};

    use super::*;
    use crate::ErrorKind;

    static META: Meta = Meta {
        name: "fragile",
        category: Category::Physics,
        description: "Panics on demand",
        title: "Fragile · test scene",
        captions: &[],
    };

    /// Where a test scene panics.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum PanicIn {
        Build,
        Resize,
        Update,
        Render,
    }

    #[derive(Debug)]
    struct Fragile {
        at: PanicIn,
        sized: bool,
    }

    impl Generator for Fragile {
        fn meta(&self) -> &'static Meta {
            &META
        }
        fn resize(&mut self, _size: Size, _ctx: &mut Ctx) {
            // The constructor sizes the scene once; `Resize` panics on the next one.
            assert!(!(self.sized && self.at == PanicIn::Resize), "resize panic");
            assert!(self.at != PanicIn::Build, "build panic");
            self.sized = true;
        }
        fn update(&mut self, _dt: f64, _ctx: &mut Ctx) {
            assert!(self.at != PanicIn::Update, "update panic");
        }
        fn render(&self, canvas: &mut Canvas) {
            assert!(self.at != PanicIn::Render, "render panic");
            canvas.put(0, 0, Glyph::new('x', 'x'), Ink::new(Role::Accent, 255));
        }
    }

    /// A stage over a scene that panics in `at`.
    pub(crate) fn fragile(at: PanicIn) -> Result<Stage, Error> {
        let build = move || Box::new(Fragile { at, sized: false }) as Box<dyn Generator>;
        Stage::start(&META, build, 4, 2, Options::default())
    }

    fn failed(result: Result<impl std::fmt::Debug, Error>) -> bool {
        result.is_err_and(|e| e.kind() == ErrorKind::ScenePanicked && e.scene() == Some("fragile"))
    }

    #[test]
    fn lib_r6_a_scene_panic_fails_the_stage_for_good() {
        assert!(failed(fragile(PanicIn::Build)));

        let mut resize = fragile(PanicIn::Resize).unwrap();
        assert!(failed(resize.resize(5, 5)));
        assert!(resize.is_failed());
        assert!(failed(resize.advance(1.0)));

        let mut update = fragile(PanicIn::Update).unwrap();
        assert!(
            update.render_text().is_ok(),
            "drawing before any step works"
        );
        assert!(failed(update.advance(0.5)));
        assert!(update.is_failed());
        assert!(failed(update.render_text()));
        assert!(failed(update.render_ansi(Theme::Astro)));
        assert!(failed(update.resize(9, 9)));
        assert!(failed(update.advance(0.5)));
        assert!(
            failed(update.advance(-1.0)),
            "a failed stage reports the panic first"
        );

        let mut render = fragile(PanicIn::Render).unwrap();
        assert!(render.advance(1.0).is_ok());
        assert!(failed(render.render_ansi(Theme::Astro)));
        assert!(failed(render.render_text()));
        assert_eq!(
            render.scene().name(),
            "fragile",
            "a failed stage still names its scene"
        );
    }

    #[cfg(feature = "png")]
    #[test]
    fn lib_r6_a_panic_while_drawing_a_png_fails_the_stage() {
        let mut render = fragile(PanicIn::Render).unwrap();
        assert!(failed(render.render_png(Theme::Astro, Vec::new())));
        assert!(failed(render.advance(1.0)));
    }

    #[test]
    fn lib_r5_sides_clamp_to_1_through_1024() {
        assert_eq!(clamp(0, 0), Size::new(1, 1));
        assert_eq!(clamp(1024, 1025), Size::new(1024, 1024));
        assert_eq!(clamp(u16::MAX, 7), Size::new(1024, 7));
        let mut stage = fragile(PanicIn::Update).unwrap();
        stage.resize(u16::MAX, 2).unwrap();
        assert_eq!(stage.size(), (1024, 2));
        stage.resize(2, u16::MAX).unwrap();
        assert_eq!(stage.size(), (2, 1024));
    }
}
