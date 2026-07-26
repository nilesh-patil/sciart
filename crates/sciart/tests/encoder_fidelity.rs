//! RND-R17 (M22, ADR-0023): the tighter frame encoder changes the bytes, not the picture.
//! Every registered scene at 200×50 (seed 42) is painted by the app's `Painter` and sent
//! through the app's `FrameBackend` and, as the reference, through ratatui's own
//! `CrosstermBackend` (the path before M14), with the same buffer-diff protocol as
//! `ratatui::Terminal::draw`. Both byte streams are decoded by the vt100 emulator; at
//! checkpoints over 1.5 s of frames, including a theme change (a near-full repaint), the
//! two screens must be identical and must show exactly the painted buffer: every cell's
//! symbol, foreground, background and attributes.

use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;

use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use sciart_core::prelude::*;
use sciart_generators::{Entry, registry};
use sciart_render::{FrameBackend, Painter, Theme};

const SEED: u64 = 42;
const SIZE: (u16, u16) = (200, 50);
const FRAMES: u32 = 45;
/// The theme switches before this frame.
const THEME_AT: u32 = 30;
const CHECK_AT: [u32; 6] = [1, 2, 15, THEME_AT, THEME_AT + 1, FRAMES];

/// Keeps the bytes; cloned handles share them.
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Vec<u8>>>);

impl Shared {
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

impl Write for Shared {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
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

fn check(entry: &Entry) {
    let name = entry.meta.name;
    let (w, h) = SIZE;
    let area = Rect::new(0, 0, w, h);
    let mut stage = Stage::new((entry.build)(), SEED, Size::new(w, h), GlyphMode::Unicode);
    let mut painter = Painter::new();
    let (new_out, ref_out) = (Shared::default(), Shared::default());
    let mut new_be = FrameBackend::with_size(new_out.clone(), w, h);
    let mut ref_be = CrosstermBackend::new(ref_out.clone());
    let (mut new_vt, mut ref_vt) = (vt100::Parser::new(h, w, 0), vt100::Parser::new(h, w, 0));
    let (mut prev, mut cur) = (Buffer::empty(area), Buffer::empty(area));
    let mut theme = Theme::Astro;
    for frame in 1..=FRAMES {
        if frame == THEME_AT {
            theme = Theme::Rainbow;
        }
        stage.step(SIM_DT);
        stage.step(SIM_DT);
        stage.render();
        painter.paint(stage.frame(), theme, area, &mut cur);
        send(&mut new_be, &prev, &cur);
        send(&mut ref_be, &prev, &cur);
        new_vt.process(&new_out.take());
        ref_vt.process(&ref_out.take());
        if CHECK_AT.contains(&frame) {
            let (a, b) = (new_vt.screen(), ref_vt.screen());
            for y in 0..h {
                for x in 0..w {
                    let got = look(a.cell(y, x).expect("cell"));
                    let reference = look(b.cell(y, x).expect("cell"));
                    assert_eq!(
                        got, reference,
                        "{name} frame {frame} ({x},{y}): vs crossterm"
                    );
                    let want = &cur[(x, y)];
                    let painted = (
                        if want.symbol().is_empty() {
                            " "
                        } else {
                            want.symbol()
                        }
                        .to_owned(),
                        vt_color(want.fg),
                        vt_color(want.bg),
                        want.modifier.contains(Modifier::BOLD),
                        want.modifier.contains(Modifier::ITALIC),
                        want.modifier.contains(Modifier::UNDERLINED),
                        want.modifier.contains(Modifier::REVERSED),
                    );
                    assert_eq!(got, painted, "{name} frame {frame} ({x},{y}): vs painted");
                }
            }
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.reset();
    }
}

#[test]
fn rnd_r17_every_scene_decodes_to_the_painted_frame_and_the_reference_screen() {
    let entries = registry();
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(entry) = entries.get(i) else { break };
                    check(entry);
                }
            });
        }
    });
}
