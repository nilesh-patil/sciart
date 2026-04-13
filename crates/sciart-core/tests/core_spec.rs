//! Executable form of docs/specs/SPEC-001-core.md. Test names cite requirement IDs.

use proptest::prelude::*;
use rand::Rng;
use sciart_core::geom;
use sciart_core::prelude::*;

const STAR: Glyph = Glyph::new('★', '*');
const INK: Ink = Ink::new(Role::Star, 200);

// ---------- Size ----------

#[test]
fn core_r1_size_clamps_to_one() {
    let s = Size::new(0, 0);
    assert_eq!((s.width(), s.height()), (1, 1));
    assert_eq!(Size::new(7, 3).area(), 21);
}

// ---------- Canvas ----------

#[test]
fn core_r2_new_canvas_is_blank_and_sized() {
    let c = Canvas::new(Size::new(4, 3), GlyphMode::Unicode);
    assert_eq!(c.size(), Size::new(4, 3));
    assert_eq!(c.cells().len(), 12);
    assert!(
        c.cells()
            .iter()
            .all(|cell| cell.ch == ' ' && cell.ink == Ink::BLANK)
    );
}

#[test]
fn core_r3_out_of_bounds_put_is_noop() {
    let mut c = Canvas::new(Size::new(3, 2), GlyphMode::Unicode);
    for (x, y) in [
        (-1, 0),
        (0, -1),
        (3, 0),
        (0, 2),
        (i32::MIN, i32::MAX),
        (i32::MAX, 1),
    ] {
        c.put(x, y, STAR, INK);
    }
    assert!(c.cells().iter().all(|cell| cell.ch == ' '));
    c.put(2, 1, STAR, INK);
    assert_eq!(c.get(2, 1).map(|cell| cell.ch), Some('★'));
}

#[test]
fn core_r4_glyph_mode_selects_char_and_rejects_wide_unicode() {
    let mut ascii = Canvas::new(Size::new(2, 1), GlyphMode::Ascii);
    ascii.put(0, 0, STAR, INK);
    assert_eq!(ascii.get(0, 0).unwrap().ch, '*');

    let mut uni = Canvas::new(Size::new(2, 1), GlyphMode::Unicode);
    uni.put(0, 0, Glyph::new('🚀', '^'), INK); // double-width emoji
    uni.put(1, 0, Glyph::new('\u{0301}', '\''), INK); // zero-width combining mark
    assert_eq!(uni.get(0, 0).unwrap().ch, '^');
    assert_eq!(uni.get(1, 0).unwrap().ch, '\'');
}

#[test]
fn core_r5_resize_and_clear_blank_the_canvas() {
    let mut c = Canvas::new(Size::new(3, 3), GlyphMode::Unicode);
    c.put(1, 1, STAR, INK);
    c.clear();
    assert!(c.cells().iter().all(|cell| cell.ch == ' '));

    c.put(1, 1, STAR, INK);
    c.resize(Size::new(5, 2));
    assert_eq!(c.cells().len(), 10);
    assert!(c.cells().iter().all(|cell| cell.ch == ' '));
}

#[test]
fn core_r6_to_lines_is_exactly_width_by_height() {
    let mut c = Canvas::new(Size::new(4, 2), GlyphMode::Unicode);
    c.put(3, 1, STAR, INK);
    assert_eq!(c.to_lines(), vec!["    ".to_string(), "   ★".to_string()]);
}

#[test]
fn core_r7_get_out_of_bounds_is_none() {
    let c = Canvas::new(Size::new(2, 2), GlyphMode::Unicode);
    assert!(c.get(2, 0).is_none());
    assert!(c.get(0, 2).is_none());
    assert!(c.get(1, 1).is_some());
}

#[test]
fn core_r8_blank_ink_is_background_zero() {
    assert_eq!(Ink::BLANK, Ink::new(Role::Background, 0));
}

// ---------- FixedStep ----------

#[test]
fn core_r9_advance_returns_whole_steps_and_keeps_remainder() {
    let mut fs = FixedStep::new(0.1, 100);
    assert_eq!(fs.advance(0.25), 2);
    assert!((fs.alpha() - 0.5).abs() < 1e-9);
    assert_eq!(fs.advance(0.05), 1);
}

#[test]
fn core_r10_advance_clamps_to_max_steps_and_drops_excess() {
    let mut fs = FixedStep::new(0.1, 5);
    assert_eq!(fs.advance(100.0), 5);
    assert_eq!(fs.advance(0.0), 0, "excess accumulator must be discarded");
}

#[test]
fn core_r11_negative_or_non_finite_elapsed_is_zero() {
    let mut fs = FixedStep::new(0.1, 5);
    for e in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(fs.advance(e), 0);
    }
}

proptest! {
    #[test]
    fn core_r9_r10_r12_fixed_step_invariants(
        dt in 0.001f64..0.5,
        max in 1u32..10,
        elapsed in proptest::collection::vec(-1.0f64..2.0, 1..50),
    ) {
        let mut fs = FixedStep::new(dt, max);
        for e in elapsed {
            let steps = fs.advance(e);
            prop_assert!(steps <= max);
            let a = fs.alpha();
            prop_assert!((0.0..1.0).contains(&a), "alpha {a} out of range");
        }
    }
}

// ---------- Ctx ----------

#[test]
fn core_r13_same_seed_same_sequence() {
    let mut a = Ctx::new(42, Size::new(10, 10));
    let mut b = Ctx::new(42, Size::new(10, 10));
    let xs: Vec<u32> = (0..16).map(|_| a.rng.random()).collect();
    let ys: Vec<u32> = (0..16).map(|_| b.rng.random()).collect();
    assert_eq!(xs, ys);

    let mut c = Ctx::new(43, Size::new(10, 10));
    let zs: Vec<u32> = (0..16).map(|_| c.rng.random()).collect();
    assert_ne!(xs, zs);
}

#[test]
fn core_r13_time_advances_only_via_ctx_advance() {
    let mut ctx = Ctx::new(1, Size::new(1, 1));
    assert_eq!(ctx.time(), 0.0);
    ctx.advance(0.5);
    ctx.advance(0.25);
    assert!((ctx.time() - 0.75).abs() < 1e-12);
}

// ---------- Generator trait ----------

#[derive(Debug, Default)]
struct Dot {
    x: f64,
}

static DOT_META: Meta = Meta {
    name: "dot",
    category: Category::Basic,
    description: "a moving dot",
};

impl Generator for Dot {
    fn meta(&self) -> &'static Meta {
        &DOT_META
    }
    fn resize(&mut self, _size: Size, _ctx: &mut Ctx) {
        self.x = 0.0;
    }
    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.x += dt;
    }
    fn render(&self, canvas: &mut Canvas) {
        canvas.put(self.x as i32, 0, STAR, INK);
    }
}

#[test]
fn core_r14_generator_is_object_safe_and_send() {
    fn assert_send<T: Send + ?Sized>() {}
    assert_send::<dyn Generator>();

    let mut g: Box<dyn Generator> = Box::new(Dot::default());
    let mut ctx = Ctx::new(0, Size::new(3, 1));
    let mut canvas = Canvas::new(Size::new(3, 1), GlyphMode::Ascii);
    g.resize(ctx.size(), &mut ctx);
    g.update(1.0, &mut ctx);
    g.render(&mut canvas);
    assert_eq!(g.meta().name, "dot");
    assert_eq!(canvas.to_lines(), vec![" * "]);
}

#[test]
fn core_r14_category_labels() {
    assert_eq!(Category::Astronomy.label(), "astronomy");
    assert_eq!(Category::SpaceExploration.label(), "space-exploration");
    assert_eq!(Category::PhysicsChemistry.label(), "physics-chemistry");
    assert_eq!(Category::Basic.label(), "basic");
}

// ---------- geom ----------

#[test]
fn core_r16_polar_applies_aspect_to_x() {
    assert_eq!(geom::polar(10.0, 10.0, 2.0, 0.0), (14, 10));
    assert_eq!(
        geom::polar(10.0, 10.0, 2.0, std::f64::consts::FRAC_PI_2),
        (10, 12)
    );
}

#[test]
fn core_r17_line_includes_both_endpoints() {
    let pts: Vec<_> = geom::line(0, 0, 3, 0).collect();
    assert_eq!(pts, vec![(0, 0), (1, 0), (2, 0), (3, 0)]);
    let diag: Vec<_> = geom::line(2, 2, 0, 0).collect();
    assert_eq!(diag, vec![(2, 2), (1, 1), (0, 0)]);
    assert_eq!(geom::line(5, 5, 5, 5).collect::<Vec<_>>(), vec![(5, 5)]);
}

proptest! {
    #[test]
    fn core_r17_line_is_connected(x0 in -50i32..50, y0 in -50i32..50, x1 in -50i32..50, y1 in -50i32..50) {
        let pts: Vec<_> = geom::line(x0, y0, x1, y1).collect();
        prop_assert_eq!(pts.first().copied(), Some((x0, y0)));
        prop_assert_eq!(pts.last().copied(), Some((x1, y1)));
        for w in pts.windows(2) {
            prop_assert!((w[0].0 - w[1].0).abs() <= 1 && (w[0].1 - w[1].1).abs() <= 1);
        }
    }

    #[test]
    fn core_r18_wrap_stays_in_range(v in -1e9f64..1e9, max in 0.001f64..1e4) {
        let w = geom::wrap(v, max);
        prop_assert!((0.0..max).contains(&w), "wrap({v}, {max}) = {w}");
    }
}

// ---------- Stage ----------

/// Draws its render count at (0,0); panics mid-render once `renders > boom_after`.
#[derive(Debug)]
struct Probe {
    resizes: std::sync::Arc<std::sync::atomic::AtomicU32>,
    renders: std::cell::Cell<u32>,
    boom_after: u32,
}

impl Probe {
    fn new(boom_after: u32) -> Self {
        Self {
            resizes: Default::default(),
            renders: Default::default(),
            boom_after,
        }
    }
}

impl Generator for Probe {
    fn meta(&self) -> &'static Meta {
        &DOT_META
    }
    fn resize(&mut self, _size: Size, _ctx: &mut Ctx) {
        self.resizes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn update(&mut self, _dt: f64, _ctx: &mut Ctx) {}
    fn render(&self, canvas: &mut Canvas) {
        let n = self.renders.get() + 1;
        self.renders.set(n);
        canvas.text(0, 0, &n.to_string(), INK);
        assert!(n <= self.boom_after, "probe exploded mid-render");
    }
}

#[test]
fn core_r15_r19_stage_resizes_once_on_build_and_only_on_change() {
    let probe = Probe::new(u32::MAX);
    let resizes = probe.resizes.clone();
    let mut stage = Stage::new(Box::new(probe), 7, Size::new(4, 2), GlyphMode::Ascii);
    stage.resize(Size::new(4, 2));
    stage.resize(Size::new(5, 2));
    stage.resize(Size::new(5, 2));
    assert_eq!(resizes.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(stage.render().size(), Size::new(5, 2));
    assert_eq!(stage.ctx().size(), Size::new(5, 2));
}

#[test]
fn core_r20_stage_step_advances_ctx_time() {
    let mut stage = Stage::new(
        Box::new(Dot::default()),
        7,
        Size::new(4, 1),
        GlyphMode::Ascii,
    );
    for _ in 0..3 {
        stage.step(1.0);
    }
    assert!((stage.ctx().time() - 3.0).abs() < 1e-12);
    assert_eq!(stage.render().to_lines(), vec!["   *"]);
}

#[test]
fn core_r21_panicking_render_keeps_previous_frame() {
    let mut stage = Stage::new(
        Box::new(Probe::new(1)),
        7,
        Size::new(3, 1),
        GlyphMode::Ascii,
    );
    stage.render();
    assert_eq!(stage.frame().to_lines(), vec!["1  "]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stage.render();
    }));
    assert!(result.is_err());
    assert_eq!(
        stage.frame().to_lines(),
        vec!["1  "],
        "front buffer must survive"
    );
}
