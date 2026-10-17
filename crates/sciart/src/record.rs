//! `sciart record` (CLI-R8, ADR-0025): a headless render of one scene to GIF, a PNG frame
//! sequence, or an asciicast v2 stream.
//!
//! Timing is exact: frame *k* is the scene after exactly *k* / fps simulated seconds, and
//! every frame lands on whole 1/60 s steps (`--fps` must divide 60), so a recording is
//! deterministic and frame 0 is `rs-preview` at `t = 0` through the same renderer.

use std::fmt;
use std::path::{Path, PathBuf};

use sciart_capture::{
    Image, RenderOptions, render_ansi, render_frame, write_asciicast, write_gif, write_png,
};
use sciart_core::prelude::{GlyphMode, SIM_DT, Stage};

use crate::cli::{FormatArg, RecordSpec};

/// Fixed steps per simulated second (ADR-0003); `fps` divides it by contract (CLI-R8).
const STEPS_PER_SECOND: u64 = 60;

/// An I/O failure while writing a recording (exit code 1).
#[derive(Debug)]
pub struct RecordError {
    path: PathBuf,
    source: std::io::Error,
}

impl RecordError {
    fn new(path: &Path, source: std::io::Error) -> Self {
        Self {
            path: path.to_owned(),
            source,
        }
    }

    /// CLI-R8: an I/O failure exits 1 (usage errors exit 2 before this point).
    pub fn code(&self) -> u8 {
        1
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "record: {}: {}", self.path.display(), self.source)
    }
}

impl std::error::Error for RecordError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Record `spec` to its output path (CLI-R8). Frames are produced lazily and written one at
/// a time, so a long recording holds only one frame in memory.
pub fn run(spec: &RecordSpec) -> Result<(), RecordError> {
    let options = RenderOptions {
        theme: spec.theme,
        ascii: spec.ascii,
    };
    match spec.format {
        FormatArg::Gif => write_gif(images(spec, options), &spec.out, spec.fps)
            .map_err(|e| RecordError::new(&spec.out, e)),
        FormatArg::Png => {
            // CLI-R8: PNG writes `frame-000.png`, … into the directory PATH.
            std::fs::create_dir_all(&spec.out).map_err(|e| RecordError::new(&spec.out, e))?;
            for (frame, image) in images(spec, options).enumerate() {
                let path = spec.out.join(format!("frame-{frame:03}.png"));
                write_png(&image, &path).map_err(|e| RecordError::new(&path, e))?;
            }
            Ok(())
        }
        FormatArg::Asciicast => {
            write_asciicast(&spec.out, spec.size, ansi_frames(spec, options), spec.fps)
                .map_err(|e| RecordError::new(&spec.out, e))
        }
    }
}

fn new_stage(spec: &RecordSpec) -> Stage {
    let mode = if spec.ascii {
        GlyphMode::Ascii
    } else {
        GlyphMode::Unicode
    };
    Stage::new((spec.entry.build)(), spec.seed, spec.size, mode)
}

/// Advance to frame `k`, taken after exactly `k * 60 / fps` whole 1/60 s steps (CLI-R8).
fn step_to(stage: &mut Stage, steps: &mut u64, frame: usize, fps: u16) {
    debug_assert_eq!(STEPS_PER_SECOND % u64::from(fps), 0, "fps must divide 60");
    let target = frame as u64 * (STEPS_PER_SECOND / u64::from(fps));
    while *steps < target {
        stage.step(SIM_DT);
        *steps += 1;
    }
}

/// The recording's frames, rendered lazily: each `next` steps to the frame's exact time and
/// draws it, then the caller writes it and drops it.
fn images(spec: &RecordSpec, options: RenderOptions) -> impl Iterator<Item = Image> {
    let mut stage = new_stage(spec);
    let mut steps = 0;
    let mut frame = 0;
    let total = spec.frames;
    let fps = spec.fps;
    std::iter::from_fn(move || {
        if frame >= total {
            return None;
        }
        step_to(&mut stage, &mut steps, frame, fps);
        frame += 1;
        Some(render_frame(&mut stage, &options))
    })
}

fn ansi_frames(spec: &RecordSpec, options: RenderOptions) -> impl Iterator<Item = String> {
    let mut stage = new_stage(spec);
    let mut steps = 0;
    let mut frame = 0;
    let total = spec.frames;
    let fps = spec.fps;
    std::iter::from_fn(move || {
        if frame >= total {
            return None;
        }
        step_to(&mut stage, &mut steps, frame, fps);
        frame += 1;
        Some(render_ansi(&mut stage, &options))
    })
}

#[cfg(test)]
mod tests {
    use sciart_core::prelude::Size;
    use sciart_render::Theme;

    use super::*;

    fn spec(frames: usize) -> RecordSpec {
        RecordSpec {
            entry: sciart_generators::find("starfield").expect("starfield is registered"),
            out: std::env::temp_dir().join("sciart-record-unit.gif"),
            format: FormatArg::Gif,
            frames,
            fps: 10,
            size: Size::new(20, 8),
            theme: Theme::Astro,
            ascii: false,
            seed: 42,
        }
    }

    #[test]
    fn cli_r8_frame_zero_is_t_zero_and_later_frames_step_exactly() {
        let spec = spec(2);
        let options = RenderOptions {
            theme: spec.theme,
            ascii: spec.ascii,
        };
        // Collect only the small prefix the comparison needs; the writer streams the rest.
        let recorded: Vec<Image> = images(&spec, options).take(2).collect();
        assert_eq!(recorded.len(), 2);

        // Frame 0 is a fresh stage rendered without stepping, i.e. `rs-preview` at t = 0.
        let mut fresh = new_stage(&spec);
        assert_eq!(recorded[0].rgb(), render_frame(&mut fresh, &options).rgb());

        // At 10 fps frame 1 is exactly 6 whole 1/60 s steps after frame 0.
        let mut stepped = new_stage(&spec);
        for _ in 0..6 {
            stepped.step(SIM_DT);
        }
        assert_eq!(
            recorded[1].rgb(),
            render_frame(&mut stepped, &options).rgb()
        );

        // The same seed and size reproduce the same frames.
        assert_eq!(images(&spec, options).take(2).collect::<Vec<_>>(), recorded);
    }

    #[test]
    fn cli_r8_asciicast_frames_follow_the_same_timing() {
        let spec = spec(3);
        let options = RenderOptions {
            theme: spec.theme,
            ascii: spec.ascii,
        };
        let frames: Vec<String> = ansi_frames(&spec, options).take(3).collect();
        assert_eq!(frames.len(), 3);
        // Each event is one complete screen: home first, reset last.
        assert!(frames.iter().all(|f| f.starts_with("\x1b[0m\x1b[H")));
        assert!(frames.iter().all(|f| f.ends_with("\x1b[0m")));
    }
}
