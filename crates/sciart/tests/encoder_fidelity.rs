//! RND-R17 (M22, ADR-0023): the tighter frame encoder changes the bytes, not the picture;
//! RT-R15: each frame is still one `write`.
//!
//! Every registered scene is painted by the app's `Painter` (plus the HUD where a run has
//! it) and sent through the app's `FrameBackend` and, as the reference, through ratatui's
//! own `CrosstermBackend` (the path before M14), with the buffer-diff protocol of
//! `ratatui::Terminal::draw` (and its resize protocol: clear, then a full redraw). Both byte
//! streams are decoded by the vt100 emulator; at checkpoints the two screens must be
//! identical and must show exactly the painted buffer: every cell's symbol, foreground,
//! background and attributes. Runs cover 200×50 with a theme change (a near-full repaint),
//! 80×24 with the HUD and a resize to 79×23 mid-run, ASCII glyphs, and the degenerate
//! 1×1, 1×7 and 7×1. The app's backend writes behind a `LineWriter` like `stdout()`'s, and
//! every non-empty frame must reach it as exactly one `write`.

use std::cell::RefCell;
use std::io::{self, LineWriter, Write};
use std::rc::Rc;

use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use sciart_core::prelude::*;
use sciart_generators::{Entry, registry};
use sciart_render::{FrameBackend, Hud, Painter, Theme};

const SEED: u64 = 42;

/// One scripted run of a scene.
struct Run {
    size: (u16, u16),
    /// Resize to this size before frame `.0`.
    resize: Option<(u32, (u16, u16))>,
    glyphs: GlyphMode,
    hud: bool,
    /// Switch to `rainbow` before this frame.
    theme_at: Option<u32>,
    frames: u32,
    /// Compare the screens after these frames (and after the frame of a resize).
    check_at: &'static [u32],
}

const RUNS: [Run; 6] = [
    Run {
        size: (200, 50),
        resize: None,
        glyphs: GlyphMode::Unicode,
        hud: false,
        theme_at: Some(30),
        frames: 45,
        check_at: &[1, 2, 15, 30, 31, 45],
    },
    Run {
        size: (80, 24),
        resize: Some((12, (79, 23))),
        glyphs: GlyphMode::Unicode,
        hud: true,
        theme_at: None,
        frames: 24,
        check_at: &[1, 6, 11, 13, 24],
    },
    Run {
        size: (80, 24),
        resize: None,
        glyphs: GlyphMode::Ascii,
        hud: false,
        theme_at: None,
        frames: 12,
        check_at: &[1, 12],
    },
    Run {
        size: (1, 1),
        resize: None,
        glyphs: GlyphMode::Unicode,
        hud: true,
        theme_at: None,
        frames: 6,
        check_at: &[1, 6],
    },
    Run {
        size: (1, 7),
        resize: None,
        glyphs: GlyphMode::Unicode,
        hud: false,
        theme_at: None,
        frames: 6,
        check_at: &[1, 6],
    },
    Run {
        size: (7, 1),
        resize: Some((3, (1, 1))),
        glyphs: GlyphMode::Unicode,
        hud: true,
        theme_at: None,
        frames: 6,
        check_at: &[1, 3, 6],
    },
];

/// Keeps the bytes and counts the `write` calls; cloned handles share them.
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<(Vec<u8>, usize)>>);

impl Shared {
    /// The bytes and the `write` calls since the last take.
    fn take(&self) -> (Vec<u8>, usize) {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

impl Write for Shared {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let mut s = self.0.borrow_mut();
        s.0.extend_from_slice(b);
        s.1 += 1;
        Ok(b.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `ratatui::Terminal::draw`'s protocol: diff, draw, hide the cursor, flush.
fn send<B: Backend>(backend: &mut B, prev: &Buffer, cur: &Buffer) {
    backend.draw(prev.diff_iter(cur)).expect("in-memory sink");
    backend.hide_cursor().expect("in-memory sink");
    backend.flush().expect("in-memory sink");
}

fn vt_color(c: Color) -> vt100::Color {
    match c {
        Color::Rgb(r, g, b) => vt100::Color::Rgb(r, g, b),
        Color::Indexed(i) => vt100::Color::Idx(i),
        _ => vt100::Color::Default,
    }
}

type Look = (String, vt100::Color, vt100::Color, bool, bool, bool, bool);

fn look(cell: &vt100::Cell) -> Look {
    let s = cell.contents();
    (
        if s.is_empty() {
            " ".to_owned()
        } else {
            s.to_owned()
        },
        cell.fgcolor(),
        cell.bgcolor(),
        cell.bold(),
        cell.italic(),
        cell.underline(),
        cell.inverse(),
    )
}

fn painted(cell: &ratatui::buffer::Cell) -> Look {
    (
        if cell.symbol().is_empty() {
            " "
        } else {
            cell.symbol()
        }
        .to_owned(),
        vt_color(cell.fg),
        vt_color(cell.bg),
        cell.modifier.contains(Modifier::BOLD),
        cell.modifier.contains(Modifier::ITALIC),
        cell.modifier.contains(Modifier::UNDERLINED),
        cell.modifier.contains(Modifier::REVERSED),
    )
}

fn check(entry: &Entry, run: &Run) {
    let name = entry.meta.name;
    let (mut w, mut h) = run.size;
    let mut area = Rect::new(0, 0, w, h);
    let mut stage = Stage::new((entry.build)(), SEED, Size::new(w, h), run.glyphs);
    let mut painter = Painter::new();
    let (new_out, ref_out) = (Shared::default(), Shared::default());
    let mut new_be =
        FrameBackend::with_size(LineWriter::with_capacity(1024, new_out.clone()), w, h);
    let mut ref_be = CrosstermBackend::new(ref_out.clone());
    let (mut new_vt, mut ref_vt) = (vt100::Parser::new(h, w, 0), vt100::Parser::new(h, w, 0));
    let (mut prev, mut cur) = (Buffer::empty(area), Buffer::empty(area));
    let mut theme = Theme::Astro;
    let mut hud_text = String::new();
    for frame in 1..=run.frames {
        let what = format!("{name} {:?} {:?} frame {frame}", run.size, run.glyphs);
        if run.theme_at == Some(frame) {
            theme = Theme::Rainbow;
        }
        if let Some((at, size)) = run.resize
            && at == frame
        {
            // `ratatui::Terminal::resize`: new buffers, then a clear.
            (w, h) = size;
            area = Rect::new(0, 0, w, h);
            stage.resize(Size::new(w, h));
            (prev, cur) = (Buffer::empty(area), Buffer::empty(area));
            new_be.set_fixed_size(w, h);
            new_be.clear().expect("in-memory sink");
            ref_be.clear().expect("in-memory sink");
            new_vt.screen_mut().set_size(h, w);
            ref_vt.screen_mut().set_size(h, w);
        }
        stage.step(SIM_DT);
        stage.step(SIM_DT);
        stage.render();
        painter.paint(stage.frame(), theme, area, &mut cur);
        if run.hud {
            let hud = Hud {
                generator: name,
                title: None,
                fps: 28.0 + f64::from(frame % 3),
                theme,
                seed: SEED,
                keep_awake: "caffeinate:on",
                low_power: false,
                paused: frame % 2 == 0,
                quarantined: 0,
            };
            hud.render_into(area, &mut cur, &mut hud_text);
        }
        send(&mut new_be, &prev, &cur);
        send(&mut ref_be, &prev, &cur);
        let (new_bytes, writes) = new_out.take();
        assert!(writes <= 1, "{what}: RT-R15, {writes} writes");
        assert_eq!(
            writes == 1,
            !new_bytes.is_empty(),
            "{what}: one write per non-empty frame"
        );
        new_vt.process(&new_bytes);
        ref_vt.process(&ref_out.take().0);
        let resized = run.resize.is_some_and(|(at, _)| at == frame);
        if run.check_at.contains(&frame) || resized {
            let (a, b) = (new_vt.screen(), ref_vt.screen());
            for y in 0..h {
                for x in 0..w {
                    let got = look(a.cell(y, x).expect("cell"));
                    let reference = look(b.cell(y, x).expect("cell"));
                    assert_eq!(got, reference, "{what} ({x},{y}): vs crossterm");
                    assert_eq!(got, painted(&cur[(x, y)]), "{what} ({x},{y}): vs painted");
                }
            }
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.reset();
    }
}

#[test]
fn rnd_r17_rt_r15_every_scene_decodes_to_the_painted_frame_and_the_reference_screen() {
    let entries = registry();
    let jobs: Vec<(&Entry, &Run)> = entries
        .iter()
        .flat_map(|e| RUNS.iter().map(move |r| (e, r)))
        .collect();
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&(entry, run)) = jobs.get(i) else {
                        break;
                    };
                    check(entry, run);
                }
            });
        }
    });
}
