//! Executable form of docs/specs/SPEC-001-core.md. Test names cite requirement IDs.

use std::f64::consts::FRAC_PI_2;

use proptest::prelude::*;
use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::title::{self, Form, Title};
use sciart_core::{draw, geom, ink};

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
    category: Category::Physics,
    description: "a moving dot",
    title: "Moving dot · steady drift",
    captions: &[],
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
fn core_r77_categories_have_unique_labels_and_ids_spelled_from_them() {
    use Category::*;
    let table = [
        (DeepCosmos, "Deep Cosmos"),
        (SunPlanets, "Sun & Planets"),
        (EarthSky, "Earth & Sky"),
        (Voyages, "Voyages"),
        (Physics, "Forces & Waves"),
        (Chemistry, "Reactions"),
        (Biology, "Living Systems"),
        (PhysicalChemistry, "States of Matter"),
        (Biophysics, "Physics of Life"),
        (Biochemistry, "Molecules of Life"),
        (LivingMatter, "Life from Matter"),
    ];
    assert_eq!(Category::ALL, table.map(|(c, _)| c));
    for (c, label) in table {
        assert_eq!(c.label(), label);
        let kebab = label
            .split([' ', '&'])
            .filter(|w| !w.is_empty())
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>()
            .join("-");
        assert_eq!(c.id(), kebab, "the id is the label in kebab case");
    }
    let labels: std::collections::BTreeSet<_> = Category::ALL.map(Category::label).into();
    assert_eq!(labels.len(), 11, "labels, and so ids, are unique");
}

// ---------- colormap inks ----------

#[test]
fn core_r46_seq_maps_unit_interval_onto_levels() {
    assert_eq!(ink::seq(0.0), Ink::new(Role::Seq, 0));
    assert_eq!(ink::seq(1.0), Ink::new(Role::Seq, 255));
    assert_eq!(ink::seq(0.5).level, 128);
    for (v, level) in [(-3.0, 0), (7.0, 255), (f64::NAN, 0), (f64::INFINITY, 255)] {
        assert_eq!(ink::seq(v), Ink::new(Role::Seq, level), "{v}");
    }
}

#[test]
fn core_r46_div_maps_signed_interval_with_zero_at_128() {
    assert_eq!(ink::div(0.0), Ink::new(Role::Div, 128));
    assert_eq!(ink::div(-0.0), Ink::new(Role::Div, 128));
    assert_eq!(ink::div(1.0), Ink::new(Role::Div, 255));
    assert_eq!(ink::div(-1.0), Ink::new(Role::Div, 0));
    for (v, level) in [
        (-9.0, 0),
        (9.0, 255),
        (f64::NAN, 128),
        (f64::NEG_INFINITY, 0),
    ] {
        assert_eq!(ink::div(v), Ink::new(Role::Div, level), "{v}");
    }
    assert!(
        ink::div(0.01).level > 128 && ink::div(-0.01).level < 128,
        "sign survives"
    );
}

proptest! {
    #[test]
    fn core_r46_colormap_inks_are_monotone(a in -2.0f64..2.0, b in -2.0f64..2.0) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        prop_assert!(ink::seq(lo).level <= ink::seq(hi).level);
        prop_assert!(ink::div(lo).level <= ink::div(hi).level);
    }
}

// ---------- 3-D camera ----------

const CENTRE: (f64, f64) = (40.0, 12.0);

fn close(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 && (a.2 - b.2).abs() < 1e-9
}

#[test]
fn core_r45_orthographic_axes_map_with_aspect_and_y_down() {
    let cam = geom::Camera::default();
    assert_eq!(cam.distance, f64::INFINITY, "default is orthographic");
    let p = |v| cam.project(v, CENTRE, 3.0).expect("finite");
    assert!(
        close(p([0.0, 0.0, 0.0]), (40.0, 12.0, 0.0)),
        "origin maps to centre"
    );
    assert!(close(
        p([1.0, 0.0, 0.0]),
        (40.0 + 3.0 * geom::ASPECT, 12.0, 0.0)
    ));
    assert!(
        close(p([0.0, 1.0, 0.0]), (40.0, 9.0, 0.0)),
        "+y is up on screen"
    );
    assert!(
        close(p([0.0, 0.0, 2.0]), (40.0, 12.0, 2.0)),
        "z is depth only"
    );
}

#[test]
fn core_r45_yaw_and_pitch_rotate_onto_the_depth_axis() {
    let yawed = geom::Camera {
        yaw: FRAC_PI_2,
        ..geom::Camera::default()
    };
    let (x, y, z) = yawed.project([1.0, 0.0, 0.0], CENTRE, 3.0).unwrap();
    assert!(
        (x - 40.0).abs() < 1e-9 && (y - 12.0).abs() < 1e-9,
        "+x lands on the view axis"
    );
    assert!((z.abs() - 1.0).abs() < 1e-9, "depth {z}");
    // Yaw is about y: it never moves y.
    let (_, y, _) = yawed.project([0.3, 1.0, -0.7], CENTRE, 3.0).unwrap();
    assert!((y - 9.0).abs() < 1e-9);

    let pitched = geom::Camera {
        pitch: FRAC_PI_2,
        ..geom::Camera::default()
    };
    let (x, y, z) = pitched.project([0.0, 1.0, 0.0], CENTRE, 3.0).unwrap();
    assert!((x - 40.0).abs() < 1e-9 && (y - 12.0).abs() < 1e-9 && (z.abs() - 1.0).abs() < 1e-9);
}

#[test]
fn core_r45_perspective_scales_by_d_over_d_minus_z() {
    let cam = geom::Camera {
        distance: 10.0,
        ..geom::Camera::default()
    };
    let (x, y, z) = cam.project([1.0, 1.0, 5.0], CENTRE, 2.0).unwrap();
    let k = 10.0 / (10.0 - 5.0);
    assert!((x - (40.0 + 2.0 * geom::ASPECT * k)).abs() < 1e-9);
    assert!((y - (12.0 - 2.0 * k)).abs() < 1e-9);
    assert_eq!(z, 5.0);
    // Nearer (larger z) is drawn larger.
    let (far_x, ..) = cam.project([1.0, 0.0, -5.0], CENTRE, 2.0).unwrap();
    assert!(far_x - 40.0 < x - 40.0);
}

#[test]
fn core_r45_points_behind_the_eye_or_non_finite_are_none() {
    let cam = geom::Camera {
        distance: 10.0,
        ..geom::Camera::default()
    };
    assert_eq!(cam.project([0.0, 0.0, 10.0], CENTRE, 1.0), None);
    assert_eq!(cam.project([0.0, 0.0, 10.0 - 1e-12], CENTRE, 1.0), None);
    assert_eq!(cam.project([0.0, 0.0, 50.0], CENTRE, 1.0), None);
    assert!(cam.project([0.0, 0.0, 9.9], CENTRE, 1.0).is_some());
    for p in [
        [f64::NAN, 0.0, 0.0],
        [0.0, f64::INFINITY, 0.0],
        [0.0, 0.0, f64::NEG_INFINITY],
    ] {
        assert_eq!(cam.project(p, CENTRE, 1.0), None, "{p:?}");
        assert_eq!(
            geom::Camera::default().project(p, CENTRE, 1.0),
            None,
            "{p:?}"
        );
    }
    assert_eq!(cam.project([0.0; 3], (f64::NAN, 0.0), 1.0), None);
    assert_eq!(cam.project([0.0; 3], CENTRE, f64::INFINITY), None);
    let broken = geom::Camera {
        yaw: f64::NAN,
        ..geom::Camera::default()
    };
    assert_eq!(broken.project([1.0, 0.0, 0.0], CENTRE, 1.0), None);
}

proptest! {
    #[test]
    fn core_r45_project_is_finite_or_none(
        p in prop::array::uniform3(any::<f64>()),
        yaw in any::<f64>(), pitch in any::<f64>(), distance in any::<f64>(),
        scale in any::<f64>(),
    ) {
        let cam = geom::Camera { yaw, pitch, distance };
        if let Some((x, y, z)) = cam.project(p, CENTRE, scale) {
            prop_assert!(x.is_finite() && y.is_finite() && z.is_finite());
        }
    }

    /// Rotation preserves distance from the origin (orthographic, scale 1, aspect undone).
    #[test]
    fn core_r45_rotation_preserves_length(
        p in prop::array::uniform3(-100.0f64..100.0), yaw in -7.0f64..7.0, pitch in -7.0f64..7.0,
    ) {
        let cam = geom::Camera { yaw, pitch, ..geom::Camera::default() };
        let (x, y, z) = cam.project(p, (0.0, 0.0), 1.0).unwrap();
        let (x, y) = (x / geom::ASPECT, -y);
        let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        prop_assert!((len([x, y, z]) - len(p)).abs() < 1e-9 * len(p).max(1.0));
    }
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

/// Logs every call as a letter and rasterises in `prepare`: `render` draws the dot where
/// the last `prepare` put it, so a missed `prepare` shows as a stale frame.
#[derive(Debug, Default)]
struct Lazy {
    x: f64,
    drawn_x: i32,
    log: std::sync::Arc<std::sync::Mutex<String>>,
}

impl Lazy {
    fn note(&self, c: char) {
        self.log.lock().unwrap().push(c);
    }
}

impl Generator for Lazy {
    fn meta(&self) -> &'static Meta {
        &DOT_META
    }
    fn resize(&mut self, _size: Size, _ctx: &mut Ctx) {
        self.note('z');
        self.x = 0.0;
    }
    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.note('u');
        self.x += dt;
    }
    fn prepare(&mut self) {
        self.note('p');
        self.drawn_x = self.x as i32;
    }
    fn render(&self, canvas: &mut Canvas) {
        self.note('r');
        canvas.put(self.drawn_x, 0, STAR, INK);
    }
}

#[test]
fn core_r101_stage_render_prepares_once_then_renders_every_call() {
    let lazy = Lazy::default();
    let log = lazy.log.clone();
    let mut stage = Stage::new(Box::new(lazy), 7, Size::new(4, 1), GlyphMode::Ascii);
    stage.step(1.0);
    stage.step(1.0);
    assert_eq!(stage.render().to_lines(), vec!["  * "]);
    assert_eq!(stage.render().to_lines(), vec!["  * "]);
    stage.resize(Size::new(5, 1));
    assert_eq!(stage.render().to_lines(), vec!["*    "]);
    // build resizes (z), two steps (uu), then each render is one prepare + one render.
    assert_eq!(*log.lock().unwrap(), "zuuprprzpr");
}

#[test]
fn core_r101_default_prepare_is_a_no_op() {
    // `Dot` does not override `prepare`: rendering twice without a step is identical.
    let mut stage = Stage::new(
        Box::new(Dot::default()),
        7,
        Size::new(4, 1),
        GlyphMode::Ascii,
    );
    stage.step(2.0);
    let first = stage.render().to_lines();
    assert_eq!(first, vec!["  * "]);
    assert_eq!(stage.render().to_lines(), first);
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

// ---------- Science titles (M11) ----------

/// Paints every cell `#` over a background ink, so tests see exactly which cells the
/// title overwrote; reports a live caption.
#[derive(Debug, Default)]
struct Fill;

static FILL_META: Meta = Meta {
    name: "fill",
    category: Category::Physics,
    description: "every cell painted",
    title: "Solid fill · every cell",
    captions: &["act two"],
};

const FILL_BG: Ink = Ink::new(Role::Seq, 9);

fn fill(canvas: &mut Canvas) {
    let (w, h) = (canvas.size().width(), canvas.size().height());
    for y in 0..i32::from(h) {
        for x in 0..i32::from(w) {
            canvas.put_bg(x, y, Glyph::ascii('#'), INK, FILL_BG);
        }
    }
}

impl Generator for Fill {
    fn meta(&self) -> &'static Meta {
        &FILL_META
    }
    fn resize(&mut self, _size: Size, _ctx: &mut Ctx) {}
    fn update(&mut self, _dt: f64, _ctx: &mut Ctx) {}
    fn render(&self, canvas: &mut Canvas) {
        fill(canvas);
    }
    fn caption(&self) -> Option<&'static str> {
        Some(FILL_META.captions[0])
    }
}

const SPOTS: &str = "Chemical spots · feed and kill";

fn row(canvas: &Canvas, y: usize) -> String {
    canvas.to_lines().swap_remove(y)
}

#[test]
fn core_r71_title_splits_at_first_separator_and_caption_replaces_mechanism() {
    let t = Title::new(SPOTS, None);
    assert_eq!(
        (t.phenomenon, t.mechanism),
        ("Chemical spots", "feed and kill")
    );
    let t = Title::new("Strange attractor · chaos", Some("Rössler"));
    assert_eq!(
        (t.phenomenon, t.mechanism),
        ("Strange attractor", "Rössler")
    );
    let t = Title::new("Aurora", None);
    assert_eq!((t.phenomenon, t.mechanism), ("Aurora", ""));
    let t = Title::new("Aurora", Some("green"));
    assert_eq!((t.phenomenon, t.mechanism), ("Aurora", "green"));
    let t = Title::new("a · b · c", None);
    assert_eq!((t.phenomenon, t.mechanism), ("a", "b · c"));
}

#[test]
fn core_r71_caption_defaults_to_none_and_meta_carries_title_data() {
    let dot = Dot::default();
    assert_eq!(dot.caption(), None);
    assert_eq!(dot.meta().title, "Moving dot · steady drift");
    assert!(dot.meta().captions.is_empty());
    assert_eq!(Fill.caption(), Some("act two"));
    assert_eq!(
        Title::of(&Fill),
        Title::new(FILL_META.title, Some("act two"))
    );
}

#[test]
fn core_r72_ascii_of_defines_the_title_charset() {
    for c in ' '..='~' {
        assert_eq!(title::ascii_of(c), Some(c), "{c:?}");
    }
    for (c, a) in [('·', '-'), ('–', '-'), ('á', 'a'), ('é', 'e'), ('ö', 'o')] {
        assert_eq!(title::ascii_of(c), Some(a), "{c:?}");
    }
    for c in ['\n', '\t', '\u{7f}', '—', 'σ', '★', '世', 'ü', 'Ö'] {
        assert_eq!(title::ascii_of(c), None, "{c:?}");
    }
}

#[test]
fn core_r72_width_is_display_cells_of_each_form() {
    let t = Title::new("Rössler · chaos", None);
    assert_eq!(t.width(Form::Phenomenon), 7);
    assert_eq!(t.width(Form::Full), 15);
    let bare = Title::new("Aurora", None);
    assert_eq!(bare.width(Form::Full), 6);
    assert_eq!(bare.width(Form::Phenomenon), 6);
}

#[test]
fn core_r73_fit_examples() {
    let t = Title::new(SPOTS, None); // full 30 cells, phenomenon 14
    assert_eq!(t.fit(Size::new(80, 24)), Some(Form::Full)); // limit 48
    assert_eq!(t.fit(Size::new(54, 24)), Some(Form::Full)); // limit 32 = 30 + 2
    assert_eq!(t.fit(Size::new(53, 24)), Some(Form::Phenomenon)); // limit 31
    assert_eq!(t.fit(Size::new(27, 24)), Some(Form::Phenomenon)); // limit 16 = 14 + 2
    assert_eq!(t.fit(Size::new(26, 24)), None);
    assert_eq!(t.fit(Size::new(200, 5)), None, "hidden below 6 rows");
    assert_eq!(t.fit(Size::new(200, 6)), Some(Form::Full));
}

fn rank(f: Option<Form>) -> u8 {
    match f {
        None => 0,
        Some(Form::Phenomenon) => 1,
        Some(Form::Full) => 2,
    }
}

proptest! {
    #[test]
    fn core_r73_fit_is_whole_forms_within_limit_and_monotone_in_width(
        w in 1u16..400,
        h in 1u16..80,
        which in 0usize..3,
    ) {
        let t = Title::new(["Murmuration · copy 7 neighbours", SPOTS, "Aurora"][which], None);
        let fit = t.fit(Size::new(w, h));
        let limit = usize::from(w) * 3 / 5;
        if h < 6 {
            prop_assert_eq!(fit, None);
        } else if t.width(Form::Full) + 2 <= limit {
            prop_assert_eq!(fit, Some(Form::Full));
        } else if t.width(Form::Phenomenon) + 2 <= limit {
            prop_assert_eq!(fit, Some(Form::Phenomenon));
        } else {
            prop_assert_eq!(fit, None);
        }
        prop_assert!(rank(t.fit(Size::new(w.saturating_add(1), h))) >= rank(fit));
    }
}

#[test]
fn core_r74_draw_layout_inks_and_cleared_background() {
    let mut c = Canvas::new(Size::new(80, 24), GlyphMode::Unicode);
    fill(&mut c);
    title::draw(&mut c, &Title::new(SPOTS, None), 0.0);
    // Gutter at 0, 30 text cells from 1, pad at 31.
    assert_eq!(row(&c, 23), format!(" {SPOTS} {}", "#".repeat(48)));
    for y in 0..23 {
        assert_eq!(row(&c, y), "#".repeat(80), "row {y} untouched");
    }
    let cell = |x| *c.get(x, 23).unwrap();
    assert_eq!(cell(0).ink, Ink::BLANK);
    assert_eq!(cell(31).ink, Ink::BLANK);
    for x in 1..31 {
        let level = if x <= 14 { 170 } else { 110 };
        assert_eq!(cell(x).ink, Ink::new(Role::Text, level), "col {x}");
    }
    for x in 0..=31 {
        assert_eq!(cell(x).bg, Ink::BLANK, "col {x} background cleared");
    }
    assert_eq!(cell(32).bg, FILL_BG, "art after the pad is kept");
}

#[test]
fn core_r74_ascii_mode_writes_the_ascii_form() {
    let mut c = Canvas::new(Size::new(80, 8), GlyphMode::Ascii);
    let t = Title::new("Strange attractor · chaos", Some("Rössler"));
    title::draw(&mut c, &t, 0.0);
    assert!(row(&c, 7).starts_with(" Strange attractor - Rossler "));
    assert!(c.cells().iter().all(|cell| cell.ch.is_ascii()));
}

#[test]
fn core_r74_dims_linearly_from_8_to_10_seconds() {
    let t = Title::new(SPOTS, None);
    let levels = |time: f64| {
        let mut c = Canvas::new(Size::new(80, 24), GlyphMode::Unicode);
        title::draw(&mut c, &t, time);
        let level = |x| c.get(x, 23).unwrap().ink.level;
        (level(1), level(20))
    };
    assert_eq!(levels(0.0), (170, 110));
    assert_eq!(levels(8.0), (170, 110));
    assert_eq!(levels(9.0), (130, 100));
    assert_eq!(levels(10.0), (90, 90));
    assert_eq!(levels(1e6), (90, 90));
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(levels(bad), (170, 110), "non-finite time counts as 0");
    }
}

#[test]
fn core_r74_r76_readouts_never_outshine_the_dimmed_title() {
    let t = Title::new(SPOTS, None);
    let mut c = Canvas::new(Size::new(80, 24), GlyphMode::Unicode);
    title::draw(&mut c, &t, 1e6);
    let dimmest = (1..30)
        .filter_map(|x| c.get(x, 23).filter(|cell| cell.ch != ' '))
        .map(|cell| cell.ink.level)
        .min();
    assert_eq!(dimmest, Some(title::READOUT_LEVEL));
}

proptest! {
    #[test]
    fn core_r74_span_is_exactly_the_cells_draw_writes(w in 1u16..160, h in 1u16..40) {
        let size = Size::new(w, h);
        let t = Title::new(SPOTS, None);
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        fill(&mut c);
        title::draw(&mut c, &t, 3.0);
        let span = title::span(size, &t);
        prop_assert_eq!(span.is_some(), t.fit(size).is_some());
        for y in 0..i32::from(h) {
            for x in 0..i32::from(w) {
                let touched = c.get(x, y).unwrap().ch != '#';
                let inside = span.as_ref().is_some_and(|(r, cols)| *r == y && cols.contains(&x));
                prop_assert_eq!(touched, inside, "cell ({}, {})", x, y);
            }
        }
        if let Some((r, cols)) = span {
            prop_assert_eq!(r, i32::from(h) - 1);
            prop_assert_eq!(cols.start, 0);
        }
    }

    #[test]
    fn core_r76_gen_r13_readout_room_keeps_four_blanks_and_half_the_row(
        w in 1u16..160,
        h in 1u16..40,
    ) {
        let size = Size::new(w, h);
        let t = Title::new(SPOTS, None);
        // The leading blank may sit at column `free` at the leftmost: the title text ends
        // at `cols.end - 2`, so its pad, two more blanks and the lead make four.
        let free = title::span(size, &t).map_or(0, |(_, cols)| cols.end + 2);
        let room = title::readout_room(size, &t);
        let half = usize::from(w) / 2;
        let lead = |n: usize| {
            let mut c = Canvas::new(size, GlyphMode::Unicode);
            draw::label_right(&mut c, i32::from(h) - 1, &"x".repeat(n), Ink::new(Role::Text, 150))
        };
        prop_assert!(room <= half, "room {} is over half of {}", room, w);
        if w >= 2 {
            prop_assert!(lead(room) >= free, "room {} leaves the lead at {}", room, lead(room));
            // Maximal: one more char would reach the gap, unless the half-row cap bound it.
            prop_assert!(room == half || lead(room + 1) < free);
        } else {
            prop_assert_eq!(room, 0);
        }
    }

    #[test]
    fn core_r83_stable_readout_room_fits_every_caption_and_grows_with_width(
        w in 1u16..240,
        h in 1u16..40,
    ) {
        // Caption lengths span the real scenes' (up to ≈ 30 chars) and beyond.
        static CAPS: [&str; 4] = [
            "a",
            "a much longer caption",
            "mid caption",
            "a caption well beyond any real scene's length",
        ];
        static M: Meta = Meta {
            name: "m",
            category: Category::Physics,
            description: "d",
            title: "Phenomenon · mechanism",
            captions: &CAPS,
        };
        let room = title::stable_readout_room(Size::new(w, h), &M);
        for c in std::iter::once(None).chain(CAPS.iter().copied().map(Some)) {
            let t = Title::new(M.title, c);
            prop_assert!(room <= title::readout_room(Size::new(w, h), &t), "{:?}", c);
        }
        let wider = title::stable_readout_room(Size::new(w + 1, h), &M);
        prop_assert!(wider >= room, "w {}: {} then {}", w, room, wider);
    }

    #[test]
    fn core_r73_full_fits_from_is_the_narrowest_full_width(
        w in 1u16..240,
        len in 0usize..60,
    ) {
        const LONG: &str = "a caption that runs on and on well past any real one";
        let caption = &LONG[..len.min(LONG.len())];
        let t = Title::new("Phenomenon · mechanism", Some(caption));
        let size = Size::new(w, 40);
        let full = t.fit(size) == Some(Form::Full);
        prop_assert_eq!(full, usize::from(w) >= t.full_fits_from(), "w {} len {}", w, len);
    }
}

#[test]
fn core_r75_stage_draws_the_live_title_after_the_scene() {
    let mut stage = Stage::new(Box::new(Fill), 1, Size::new(60, 8), GlyphMode::Unicode);
    assert_eq!(stage.title(), Title::new(FILL_META.title, Some("act two")));

    // Visible by default.
    let want = " Solid fill · act two ";
    assert!(row(stage.render(), 7).starts_with(want));
    assert_eq!(row(stage.frame(), 6), "#".repeat(60));

    stage.set_title_visible(false);
    assert!(
        stage.render().cells().iter().all(|c| c.ch == '#'),
        "hidden: exactly the scene's frame"
    );

    stage.set_title_visible(true);
    stage.resize(Size::new(50, 10));
    assert!(
        row(stage.render(), 9).starts_with(want),
        "visibility survives resize"
    );
}

#[test]
fn core_r75_stage_dims_the_title_by_scene_time() {
    let mut stage = Stage::new(Box::new(Fill), 1, Size::new(60, 8), GlyphMode::Unicode);
    assert_eq!(stage.render().get(1, 7).unwrap().ink.level, 170);
    for _ in 0..10 {
        stage.step(1.0);
    }
    assert_eq!(stage.render().get(1, 7).unwrap().ink.level, 90);
}

/// Two acts: the caption turns from `one` to `two` at 12 s of scene time.
#[derive(Debug, Default)]
struct Acts {
    t: f64,
}

static ACTS_META: Meta = Meta {
    name: "acts",
    category: Category::Physics,
    description: "two captioned acts",
    title: "Two acts · one",
    captions: &["one", "two"],
};

impl Generator for Acts {
    fn meta(&self) -> &'static Meta {
        &ACTS_META
    }
    fn resize(&mut self, _size: Size, _ctx: &mut Ctx) {}
    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
    }
    fn render(&self, _canvas: &mut Canvas) {}
    fn caption(&self) -> Option<&'static str> {
        Some(ACTS_META.captions[usize::from(self.t >= 12.0)])
    }
}

#[test]
fn core_r74_r75_the_fade_restarts_when_the_caption_changes() {
    let mut stage = Stage::new(
        Box::<Acts>::default(),
        1,
        Size::new(60, 8),
        GlyphMode::Unicode,
    );
    let level = |stage: &mut Stage| stage.render().get(1, 7).unwrap().ink.level;
    for _ in 0..11 {
        stage.step(1.0);
    }
    assert_eq!(level(&mut stage), 90, "act one has faded by 11 s");
    stage.step(1.0);
    assert_eq!(stage.title().mechanism, "two");
    assert_eq!(
        level(&mut stage),
        170,
        "a new caption shows at full strength"
    );
    for _ in 0..9 {
        stage.step(1.0);
    }
    assert_eq!(level(&mut stage), 130, "then fades 8 s after it appeared");
    stage.step(1.0);
    assert_eq!(level(&mut stage), 90);
}

#[test]
fn core_r21_r75_panicking_render_keeps_previous_titled_frame() {
    let mut stage = Stage::new(
        Box::new(Probe::new(1)),
        7,
        Size::new(20, 6),
        GlyphMode::Ascii,
    );
    let first = stage.render().to_lines();
    assert!(first[5].starts_with(" Moving dot "), "{first:?}");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stage.render();
    }));
    assert!(result.is_err());
    assert_eq!(stage.frame().to_lines(), first);
}
