//! Headless capture pipeline (ADR-0025, CLI-R8).
//!
//! One renderer for review and recording: a seeded [`Stage`] rendered through the real
//! [`Theme`] into an 8 × 16 px-per-cell RGB image (braille dots and half blocks drawn
//! exactly, other glyphs through `font8x8`, wide or missing glyphs through the scene's ASCII
//! fallback), then written as PNG, as a looping GIF, or as asciicast v2 text.
//!
//! The scene itself is built by the caller (the `preview` example and `sciart record` both
//! resolve it from `sciart-generators`), so this crate depends on neither and stays usable
//! from tests.

mod asciicast;
mod gif;
mod image;

use sciart_core::prelude::{GlyphMode, Stage};
use sciart_render::Theme;

pub use self::asciicast::{render_ansi, write_asciicast};
pub use self::gif::write_gif;
pub use self::image::{CELL_H, CELL_W, Image, paint, write_png};

/// How to render frames: the colour theme and whether to use the ASCII glyph set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    pub theme: Theme,
    pub ascii: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            theme: Theme::Astro,
            ascii: false,
        }
    }
}

/// Render the stage's current frame (no steps are taken) as an RGB image.
///
/// The canvas glyphs come from `options.ascii`; the ASCII frame of the same moment is used
/// only as the glyph fallback, exactly as the preview example did (CORE-R75 title included).
pub fn render_frame(stage: &mut Stage, options: &RenderOptions) -> Image {
    let previous = stage.glyph_mode();
    stage.set_glyph_mode(if options.ascii {
        GlyphMode::Ascii
    } else {
        GlyphMode::Unicode
    });
    let canvas = stage.render().clone();
    // Rendering does not advance the scene, so the ASCII frame is the same moment.
    stage.set_glyph_mode(GlyphMode::Ascii);
    let image = paint(&canvas, stage.render(), options.theme);
    stage.set_glyph_mode(previous);
    image
}
