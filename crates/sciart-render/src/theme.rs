use sciart_core::{Ink, Role, ink};

use crate::fields;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// Maps semantic inks to colors. Pure: no RNG, no time, so frames never strobe (RND-R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Astro,
    Rainbow,
    Mono,
    /// Auroral emission lines on a polar night (RND-R16, ADR-0022).
    Aurora,
}

impl Theme {
    pub const ALL: [Theme; 4] = [Theme::Astro, Theme::Rainbow, Theme::Mono, Theme::Aurora];

    pub const fn name(self) -> &'static str {
        match self {
            Theme::Astro => "astro",
            Theme::Rainbow => "rainbow",
            Theme::Mono => "mono",
            Theme::Aurora => "aurora",
        }
    }

    /// Case-insensitive parse (RND-R2).
    pub fn from_name(name: &str) -> Option<Theme> {
        Self::ALL
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(name))
    }

    /// Next theme in `ALL`, wrapping (used by the `t` key in M2).
    pub fn next(self) -> Theme {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn color(self, ink: Ink, x: u16, y: u16) -> Rgb {
        let level = f64::from(ink.level) / 255.0;
        match (self, ink.role) {
            (Theme::Aurora, Role::Seq) => gradient(&CURTAIN, ink::seq_value(ink)),
            (Theme::Aurora, Role::Div) => gradient(&AURORA_DIV, div_pos(ink)),
            // Roles scenes may not use (GEN-R23) borrow astro's tables (RND-R16).
            (Theme::Aurora, Role::Phase | Role::Blackbody | Role::Ramp(_)) => {
                Theme::Astro.color(ink, x, y)
            }
            (Theme::Aurora, role) => scale(aurora_base(role), 0.3 + 0.7 * level),
            // Colormap roles ignore the per-cell hash and the role-hue scaling: a field
            // needs one smooth, perceptually ordered ramp (RND-R6, ADR-0011).
            (Theme::Astro, Role::Seq) => gradient(&INFERNO, ink::seq_value(ink)),
            (Theme::Astro, Role::Div) => gradient(&ASTRO_DIV, div_pos(ink)),
            (Theme::Rainbow, Role::Seq) => gradient(&TURBO, ink::seq_value(ink)),
            (Theme::Rainbow, Role::Div) => gradient(&BLUE_RED, div_pos(ink)),
            // A scene's physical or art-directed colour, not the theme's (ADR-0015).
            (Theme::Astro | Theme::Rainbow, Role::Blackbody) => {
                fields::tables().blackbody[usize::from(ink.level)]
            }
            (Theme::Astro | Theme::Rainbow, Role::Ramp(r)) => {
                fields::tables().ramps[r as usize][usize::from(ink.level)]
            }
            (Theme::Astro, Role::Phase) => fields::tables().phase[usize::from(ink.level)],
            (Theme::Rainbow, Role::Phase) => hsv(f64::from(ink.level) / 256.0, 1.0, 1.0),
            // One grey channel: higher is brighter, like `Seq` (RND-R12).
            (Theme::Mono, Role::Seq | Role::Ramp(_)) => mono_table().seq[usize::from(ink.level)],
            // A glow keeps its own lightness in grey, so a star outshines the planets round
            // it in `mono` as it does in colour (RND-R11).
            (Theme::Mono, Role::Blackbody) => mono_table().blackbody[usize::from(ink.level)],
            (Theme::Mono, Role::Div) => mono_table().div[usize::from(ink.level)],
            (Theme::Mono, Role::Phase) => mono_table().phase[usize::from(ink.level)],
            (Theme::Mono, _) => grey(ink.level),
            (Theme::Astro, role) => scale(astro_base(role), 0.3 + 0.7 * level),
            // Per-cell hues would turn titles and labels into confetti; text reads as one
            // neutral grey, the same as astro's (RND-R8).
            (Theme::Rainbow, Role::Text) => scale(astro_base(Role::Text), 0.3 + 0.7 * level),
            (Theme::Rainbow, _) => hsv(cell_hue(x, y), 1.0, 0.35 + 0.65 * level),
        }
    }
}

/// `mono`'s ramp for object roles: level 0 stays visible (40) on a black terminal.
fn grey(level: u8) -> Rgb {
    let g = 40 + (u16::from(level) * 215 / 255) as u8;
    Rgb(g, g, g)
}

// `mono` colormaps (RND-R6). Fields paint whole regions, often as cell backgrounds, so
// unlike `grey` their zero must read as "nothing": a floor just above black rather than
// grey 40, which washed out every dark region. Both ramps are linear in CIE L*, so equal
// level steps look like equal brightness steps.
//
// Grey has one channel, so `Div` cannot be both symmetric in |v| and signed. It keeps
// zero near black on both sides (zero crossings stay dark, luminance stays monotone in
// |v|) and encodes the sign in the slope: positive climbs to white (L* 100), negative
// only to a dim grey (L* 45, just below mid-grey). At equal |v| the positive side is
// always brighter (ΔL* = 55·|v|), and the extremes sit 55 L* apart, so alternating lobes
// read as bright/dim. The cost: a strong negative matches a weak positive (|v| ≈ 0.43);
// scenes that need an exact sign read should also pick glyphs by sign.

/// L* of level 0 (`Seq`) and of zero (`Div`): grey 13 on the sRGB curve.
const MONO_FLOOR_L: f64 = 3.5;
/// L* the negative side of `Div` reaches at −1 (grey ≈ 106).
const MONO_NEG_MAX_L: f64 = 45.0;
/// L* of `Phase` half a turn from 0: a triangle in L* is the one grey loop without a seam
/// (RND-R10).
const MONO_PHASE_PEAK_L: f64 = 85.0;

struct MonoTable {
    seq: [Rgb; 256],
    div: [Rgb; 256],
    phase: [Rgb; 256],
    /// The lightness of each `astro` blackbody colour, in grey (RND-R11).
    blackbody: [Rgb; 256],
}

/// Built once: the sRGB transfer needs `powf`, too costly to evaluate per painted cell.
fn mono_table() -> &'static MonoTable {
    static TABLE: std::sync::LazyLock<MonoTable> = std::sync::LazyLock::new(|| {
        let mut seq = [Rgb(0, 0, 0); 256];
        let mut div = [Rgb(0, 0, 0); 256];
        let mut phase = [Rgb(0, 0, 0); 256];
        for level in 0..=255u8 {
            let ink = Ink::new(Role::Seq, level);
            let s = MONO_FLOOR_L + (100.0 - MONO_FLOOR_L) * ink::seq_value(ink);
            seq[usize::from(level)] = lightness_grey(s);
            let v = ink::div_value(ink);
            let top = if v >= 0.0 { 100.0 } else { MONO_NEG_MAX_L };
            div[usize::from(level)] = lightness_grey(MONO_FLOOR_L + (top - MONO_FLOOR_L) * v.abs());
            // Distance from phase 0 in half-turns: 0 at level 0, 1 at level 128.
            let half_turns = f64::from(level.min(level.wrapping_neg())) / 128.0;
            phase[usize::from(level)] =
                lightness_grey(MONO_FLOOR_L + (MONO_PHASE_PEAK_L - MONO_FLOOR_L) * half_turns);
        }
        // `i < 256`: the cast is exact.
        let blackbody = std::array::from_fn(|i| {
            lightness_grey(fields::blackbody_lightness(Ink::new(
                Role::Blackbody,
                i as u8,
            )))
        });
        MonoTable {
            seq,
            div,
            phase,
            blackbody,
        }
    });
    &TABLE
}

/// The sRGB grey of CIE lightness `l ∈ [0, 100]` (D65; CIE 15 / IEC 61966-2-1).
/// Monotone in `l`, so a monotone L* ramp stays monotone after rounding.
fn lightness_grey(l: f64) -> Rgb {
    const KAPPA: f64 = 24389.0 / 27.0;
    let l = l.clamp(0.0, 100.0);
    let y = if l > 8.0 {
        ((l + 16.0) / 116.0).powi(3)
    } else {
        l / KAPPA
    };
    let c = if y <= 0.003_130_8 {
        12.92 * y
    } else {
        1.055 * y.powf(1.0 / 2.4) - 0.055
    };
    let g = (c * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb(g, g, g)
}

// Colormap stops, evenly spaced over levels 0..=255 (RND-R6). Each is non-decreasing in
// luminance away from its low end (Seq) or its centre (Div); the RND-R1/R6 tests check
// every level.

/// Inferno-like: deep violet → purple → red → orange → pale yellow.
const INFERNO: [Rgb; 6] = [
    Rgb(8, 5, 30),
    Rgb(72, 12, 104),
    Rgb(160, 42, 96),
    Rgb(228, 90, 40),
    Rgb(250, 175, 20),
    Rgb(252, 250, 160),
];

/// Cyan-blue for negative, a centre just above the background, orange for positive.
const ASTRO_DIV: [Rgb; 5] = [
    Rgb(120, 215, 255),
    Rgb(40, 115, 205),
    Rgb(22, 24, 36),
    Rgb(205, 95, 30),
    Rgb(255, 195, 95),
];

/// Turbo-like hue sweep (indigo → blue → cyan → green → yellow → red). Unlike real
/// turbo, its ends are lifted so luminance never falls (RND-R6).
const TURBO: [Rgb; 6] = [
    Rgb(48, 18, 90),
    Rgb(45, 100, 235),
    Rgb(30, 195, 205),
    Rgb(140, 230, 70),
    Rgb(250, 175, 30),
    Rgb(255, 100, 110),
];

/// Blue for negative, near-black at zero, red for positive.
const BLUE_RED: [Rgb; 5] = [
    Rgb(80, 140, 255),
    Rgb(40, 70, 200),
    Rgb(24, 22, 34),
    Rgb(195, 40, 40),
    Rgb(255, 100, 85),
];

/// Aurora curtain (RND-R16): night → violet (N₂⁺ top) → teal → oxygen green → mint, the way
/// a curtain brightens toward its base.
const CURTAIN: [Rgb; 5] = [
    Rgb(6, 8, 24),
    Rgb(70, 30, 120),
    Rgb(20, 130, 140),
    Rgb(60, 220, 120),
    Rgb(210, 255, 220),
];

/// Violet for negative, a centre just above the background, oxygen green for positive.
const AURORA_DIV: [Rgb; 5] = [
    Rgb(190, 130, 255),
    Rgb(80, 50, 170),
    Rgb(18, 22, 34),
    Rgb(30, 140, 90),
    Rgb(120, 255, 170),
];

/// Position of a `Div` ink along its colormap, `[−1, 1] → [0, 1]`: level 128 (zero) lands
/// exactly on the centre stop, through core's inverse of `ink::div` (CORE-R46, CORE-R56).
fn div_pos(ink: Ink) -> f64 {
    0.5 * (ink::div_value(ink) + 1.0)
}

/// Piecewise-linear interpolation of evenly spaced `stops` (at least two) at
/// `pos ∈ [0, 1]`.
///
/// Rounding each channel on its own lets `r + g + b` dip by one between neighbouring
/// levels wherever channels move in opposite directions (every hue sweep does), which
/// would break RND-R6's monotone luminance. So the channel *sum* is rounded as a whole,
/// and the ±1 of slack goes to the channel whose own rounding was furthest off.
fn gradient(stops: &[Rgb], pos: f64) -> Rgb {
    let segments = stops.len().saturating_sub(1).max(1);
    let x = pos.clamp(0.0, 1.0) * segments as f64;
    let i = (x as usize).min(segments - 1);
    let t = x - i as f64;
    let (a, b) = (stops[i], stops.get(i + 1).copied().unwrap_or(stops[i]));
    let lerp = |p: u8, q: u8| f64::from(p) + (f64::from(q) - f64::from(p)) * t;
    let exact = [lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2)];
    let mut out = exact.map(f64::round);
    // Each channel is off by at most ½, so the slack is −1, 0 or +1.
    let slack = exact.iter().sum::<f64>().round() - out.iter().sum::<f64>();
    let err = |k: usize, out: &[f64; 3]| out[k] - exact[k];
    if slack > 0.0 {
        // Rounded down the most ⇒ strictly below its exact value ≤ 255: room for +1.
        let k = (0..3).min_by(|&p, &q| err(p, &out).total_cmp(&err(q, &out)));
        out[k.unwrap_or(0)] += 1.0;
    } else if slack < 0.0 {
        let k = (0..3).max_by(|&p, &q| err(p, &out).total_cmp(&err(q, &out)));
        out[k.unwrap_or(0)] -= 1.0;
    }
    let channel = |v: f64| v.clamp(0.0, 255.0) as u8;
    Rgb(channel(out[0]), channel(out[1]), channel(out[2]))
}

fn astro_base(role: Role) -> Rgb {
    match role {
        Role::Star => Rgb(255, 248, 230),
        Role::Sun => Rgb(255, 196, 60),
        Role::Planet => Rgb(110, 175, 255),
        Role::Particle => Rgb(170, 255, 200),
        Role::Wave => Rgb(80, 205, 255),
        Role::Hot => Rgb(255, 105, 55),
        Role::Cold => Rgb(95, 140, 255),
        Role::Structure => Rgb(150, 150, 175),
        Role::Accent => Rgb(255, 90, 200),
        Role::Text => Rgb(230, 230, 230),
        Role::Dim | Role::Background => Rgb(90, 90, 115),
        _ => Rgb(200, 200, 200),
    }
}

/// Aurora's object roles (RND-R16): the two oxygen lines carry the `Hot`/`Cold` pair
/// (630 nm red, 557.7 nm green) and N₂⁺ (427.8 nm) the violet `Accent`; the rest stay cool
/// on night blue, except a pale-gold `Sun` so stars do not all turn green.
fn aurora_base(role: Role) -> Rgb {
    match role {
        Role::Star => Rgb(225, 245, 255),
        Role::Sun => Rgb(255, 215, 120),
        Role::Planet => Rgb(90, 170, 255),
        Role::Particle => Rgb(150, 255, 200),
        Role::Wave => Rgb(40, 220, 210),
        Role::Hot => Rgb(255, 70, 110),
        Role::Cold => Rgb(60, 255, 150),
        Role::Structure => Rgb(130, 140, 175),
        Role::Accent => Rgb(170, 90, 255),
        Role::Text => Rgb(220, 235, 235),
        Role::Dim | Role::Background => Rgb(60, 80, 110),
        _ => Rgb(200, 200, 200),
    }
}

fn scale(Rgb(r, g, b): Rgb, k: f64) -> Rgb {
    let f = |c: u8| (f64::from(c) * k).round().clamp(0.0, 255.0) as u8;
    Rgb(f(r), f(g), f(b))
}

/// Stable per-cell hue in `[0, 1)` from a small integer hash of the position.
fn cell_hue(x: u16, y: u16) -> f64 {
    let mut h = u32::from(x).wrapping_mul(0x9E37_79B1) ^ u32::from(y).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    f64::from(h) / (f64::from(u32::MAX) + 1.0)
}

fn hsv(h: f64, s: f64, v: f64) -> Rgb {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as u8 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let c = |x: f64| (x * 255.0).round() as u8;
    Rgb(c(r), c(g), c(b))
}

// ----- Paint tables (RND-R15) -----
//
// `Theme::color` is the definition; these tables are built from it once, so a painted
// colour is byte-identical to it by construction and costs one index per cell.

/// Roles: the 16 fixed ones, then one per `Ramp`.
pub(crate) const ROLE_COUNT: usize = 16 + sciart_core::Ramp::ALL.len();

/// Dense index of a role into [`Lut`]; `None` for a role without a table.
pub(crate) fn role_index(role: Role) -> Option<usize> {
    Some(match role {
        Role::Background => 0,
        Role::Dim => 1,
        Role::Star => 2,
        Role::Sun => 3,
        Role::Planet => 4,
        Role::Particle => 5,
        Role::Wave => 6,
        Role::Hot => 7,
        Role::Cold => 8,
        Role::Structure => 9,
        Role::Accent => 10,
        Role::Text => 11,
        Role::Seq => 12,
        Role::Div => 13,
        Role::Phase => 14,
        Role::Blackbody => 15,
        Role::Ramp(r) => 16 + r as usize,
        // `Role` is non-exhaustive: a role added later is coloured by `Theme::color`.
        _ => return None,
    })
}

/// The inverse of [`role_index`].
fn role_at(i: usize) -> Role {
    const FIXED: [Role; 16] = [
        Role::Background,
        Role::Dim,
        Role::Star,
        Role::Sun,
        Role::Planet,
        Role::Particle,
        Role::Wave,
        Role::Hot,
        Role::Cold,
        Role::Structure,
        Role::Accent,
        Role::Text,
        Role::Seq,
        Role::Div,
        Role::Phase,
        Role::Blackbody,
    ];
    FIXED
        .get(i)
        .copied()
        .unwrap_or_else(|| Role::Ramp(sciart_core::Ramp::ALL[i - FIXED.len()]))
}

/// Whether `theme` colours `role` per cell position (rainbow's hashed hues), so it has no
/// 256-entry table.
pub(crate) fn per_cell(theme: Theme, role: Role) -> bool {
    theme == Theme::Rainbow
        && !matches!(
            role,
            Role::Text | Role::Seq | Role::Div | Role::Phase | Role::Blackbody | Role::Ramp(_)
        )
}

/// Every (role, level) colour of one theme; per-cell roles hold a placeholder.
pub(crate) struct Lut {
    pub(crate) colors: [[Rgb; 256]; ROLE_COUNT],
}

/// The table of `theme`, built on first use.
pub(crate) fn lut(theme: Theme) -> &'static Lut {
    fn build(theme: Theme) -> Lut {
        let mut colors = [[Rgb(0, 0, 0); 256]; ROLE_COUNT];
        for (i, table) in colors.iter_mut().enumerate() {
            let role = role_at(i);
            if per_cell(theme, role) {
                continue;
            }
            for (level, c) in table.iter_mut().enumerate() {
                // `level < 256`: the cast is exact.
                *c = theme.color(Ink::new(role, level as u8), 0, 0);
            }
        }
        Lut { colors }
    }
    static ASTRO: std::sync::LazyLock<Lut> = std::sync::LazyLock::new(|| build(Theme::Astro));
    static RAINBOW: std::sync::LazyLock<Lut> = std::sync::LazyLock::new(|| build(Theme::Rainbow));
    static MONO: std::sync::LazyLock<Lut> = std::sync::LazyLock::new(|| build(Theme::Mono));
    static AURORA: std::sync::LazyLock<Lut> = std::sync::LazyLock::new(|| build(Theme::Aurora));
    match theme {
        Theme::Astro => &ASTRO,
        Theme::Rainbow => &RAINBOW,
        Theme::Mono => &MONO,
        Theme::Aurora => &AURORA,
    }
}

/// The position-dependent part of rainbow's per-cell hue (`hsv(cell_hue(x, y), 1, v)`),
/// precomputed per cell on resize: the sextant and the two factors `hsv` multiplies `v`
/// by, as the same floating-point values, so the result is byte-identical.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HuePos {
    sextant: u8,
    q: f64,
    t: f64,
}

pub(crate) fn hue_pos(x: u16, y: u16) -> HuePos {
    let h = cell_hue(x, y);
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    // `hsv` with s = 1: `f * s` and `(1 - f) * s` are exact, `p = v * 0 = 0`.
    HuePos {
        sextant: i as u8 % 6,
        q: 1.0 - f,
        t: 1.0 - (1.0 - f),
    }
}

/// Rainbow's object-role colour at a precomputed position (RND-R2, RND-R15).
pub(crate) fn rainbow_at(pos: HuePos, level: u8) -> Rgb {
    let v = 0.35 + 0.65 * (f64::from(level) / 255.0);
    let (q, t) = (v * pos.q, v * pos.t);
    let c = |x: f64| (x * 255.0).round() as u8;
    let (v, q, t, p) = (c(v), c(q), c(t), 0);
    match pos.sextant {
        0 => Rgb(v, t, p),
        1 => Rgb(q, v, p),
        2 => Rgb(p, v, t),
        3 => Rgb(p, q, v),
        4 => Rgb(t, p, v),
        _ => Rgb(v, p, q),
    }
}

#[cfg(test)]
mod lut_tests {
    use super::*;

    #[test]
    fn rnd_r15_tables_are_byte_identical_to_theme_color() {
        for theme in Theme::ALL {
            let table = lut(theme);
            for i in 0..ROLE_COUNT {
                let role = role_at(i);
                assert_eq!(role_index(role), Some(i));
                for level in 0..=255u8 {
                    let ink = Ink::new(role, level);
                    for (x, y) in [(0, 0), (3, 7), (199, 49), (499, 149)] {
                        let want = theme.color(ink, x, y);
                        let got = if per_cell(theme, role) {
                            rainbow_at(hue_pos(x, y), level)
                        } else {
                            table.colors[i][usize::from(level)]
                        };
                        assert_eq!(got, want, "{theme:?} {role:?} {level} at ({x},{y})");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::Ramp;

    const ROLES: [Role; 17] = [
        Role::Background,
        Role::Dim,
        Role::Star,
        Role::Sun,
        Role::Planet,
        Role::Particle,
        Role::Wave,
        Role::Hot,
        Role::Cold,
        Role::Structure,
        Role::Accent,
        Role::Text,
        Role::Seq,
        Role::Div,
        Role::Phase,
        Role::Blackbody,
        Role::Ramp(Ramp::Ember),
    ];

    fn luma(Rgb(r, g, b): Rgb) -> u32 {
        u32::from(r) + u32::from(g) + u32::from(b)
    }

    /// How far a level is from "nothing": the level itself, except for `Div`, whose
    /// zero sits at 128 (RND-R6 amends RND-R1).
    fn magnitude(role: Role, level: u8) -> u8 {
        if role == Role::Div {
            level.abs_diff(128)
        } else {
            level
        }
    }

    #[test]
    fn rnd_r1_color_is_pure() {
        for theme in Theme::ALL {
            for role in ROLES {
                for level in [0, 128, 255] {
                    let ink = Ink::new(role, level);
                    assert_eq!(theme.color(ink, 3, 7), theme.color(ink, 3, 7));
                }
            }
        }
    }

    #[test]
    fn rnd_r1_r6_brighter_level_is_not_darker() {
        for theme in Theme::ALL {
            // A phase level is an angle, not an intensity (RND-R10 amends RND-R1).
            for role in ROLES.into_iter().filter(|&r| r != Role::Phase) {
                // For Div, 128 ± 120 are both "bright" and 128 ± 5 are both "dim".
                let (lo, hi) = if role == Role::Div {
                    ([123, 133], [8, 248])
                } else {
                    ([10, 10], [250, 250])
                };
                for (l, h) in lo.into_iter().zip(hi) {
                    assert!(magnitude(role, h) >= magnitude(role, l));
                    let dim = theme.color(Ink::new(role, l), 5, 5);
                    let bright = theme.color(Ink::new(role, h), 5, 5);
                    assert!(luma(bright) >= luma(dim), "{theme:?}/{role:?} {l} vs {h}");
                }
            }
        }
    }

    #[test]
    fn rnd_r6_colormap_luminance_is_monotone_at_every_level() {
        for theme in Theme::ALL {
            // Seq: non-decreasing in level.
            let seq = |l| luma(theme.color(Ink::new(Role::Seq, l), 9, 4));
            for l in 1..=255u8 {
                assert!(seq(l) >= seq(l - 1), "{theme:?} Seq at {l}");
            }
            // Div: non-decreasing in |level − 128|, on both sides of zero.
            let div = |l| luma(theme.color(Ink::new(Role::Div, l), 9, 4));
            for d in 1..=127u8 {
                assert!(div(128 + d) >= div(128 + d - 1), "{theme:?} Div +{d}");
                assert!(div(128 - d) >= div(128 - d + 1), "{theme:?} Div -{d}");
            }
            assert!(div(0) >= div(1), "{theme:?} Div -128");
        }
    }

    #[test]
    fn rnd_r6_colormaps_ignore_the_cell_position() {
        for theme in Theme::ALL {
            for role in [Role::Seq, Role::Div] {
                for level in [0, 64, 128, 200, 255] {
                    let ink = Ink::new(role, level);
                    let first = theme.color(ink, 0, 0);
                    for (x, y) in [(1, 0), (17, 3), (199, 49), (u16::MAX, 7)] {
                        assert_eq!(theme.color(ink, x, y), first, "{theme:?}/{role:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn rnd_r6_astro_seq_runs_violet_red_orange_pale_yellow() {
        let c = |l| Theme::Astro.color(Ink::new(Role::Seq, l), 0, 0);
        let Rgb(_, g, b) = c(0);
        assert!(b > g && luma(c(0)) < 80, "deep violet: {:?}", c(0));
        let Rgb(r2, g2, b2) = c(150);
        assert!(r2 > g2 && r2 > b2, "red in the middle: {:?}", c(150));
        let Rgb(r3, g3, b3) = c(200);
        assert!(r3 > g3 && g3 > b3, "orange: {:?}", c(200));
        let Rgb(r4, g4, b4) = c(255);
        assert!(r4 > 230 && g4 > 230 && b4 < g4, "pale yellow: {:?}", c(255));
    }

    #[test]
    fn rnd_r6_div_zero_is_dark_and_the_signs_differ_in_hue() {
        for theme in [Theme::Astro, Theme::Rainbow] {
            let c = |l| theme.color(Ink::new(Role::Div, l), 0, 0);
            assert!(
                luma(c(128)) < 120,
                "{theme:?}: zero near background {:?}",
                c(128)
            );
            let (Rgb(nr, _, nb), Rgb(pr, _, pb)) = (c(0), c(255));
            assert!(nb > nr, "{theme:?}: negative is blue {:?}", c(0));
            assert!(pr > pb, "{theme:?}: positive is warm {:?}", c(255));
        }
    }

    #[test]
    fn rnd_r6_rainbow_seq_sweeps_hue_from_blue_to_red() {
        let c = |l| Theme::Rainbow.color(Ink::new(Role::Seq, l), 0, 0);
        let (Rgb(r0, _, b0), Rgb(_, g_mid, _), Rgb(r1, g1, b1)) = (c(0), c(128), c(255));
        assert!(b0 > r0, "starts blue/violet: {:?}", c(0));
        assert!(g_mid > 180, "green/cyan in the middle: {:?}", c(128));
        assert!(r1 > g1 && r1 > b1, "ends red: {:?}", c(255));
    }

    /// CIE L* (0..=100) of an sRGB grey: the test's independent inverse of the ramp.
    fn grey_lightness(Rgb(g, _, _): Rgb) -> f64 {
        let c = f64::from(g) / 255.0;
        let y = if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        };
        if y > 216.0 / 24389.0 {
            116.0 * y.cbrt() - 16.0
        } else {
            y * 24389.0 / 27.0
        }
    }

    #[test]
    fn rnd_r6_mono_colormaps_are_grey() {
        for level in [0u8, 1, 64, 127, 128, 129, 200, 255] {
            for role in [Role::Seq, Role::Div] {
                let Rgb(r, g, b) = Theme::Mono.color(Ink::new(role, level), 0, 0);
                assert!(r == g && g == b, "{role:?} {level}");
            }
        }
    }

    #[test]
    fn rnd_r6_mono_seq_starts_near_black_and_rises_evenly_in_lightness() {
        let seq = |l| Theme::Mono.color(Ink::new(Role::Seq, l), 0, 0);
        // A zero field is (almost) the terminal background, not a grey wash.
        assert!(seq(0).0 <= 20, "near-black at level 0: {:?}", seq(0));
        assert_eq!(seq(255), Rgb(255, 255, 255));
        // Equal level steps are equal lightness steps (perceptual, not linear in sRGB).
        let (l0, l1) = (grey_lightness(seq(0)), grey_lightness(seq(255)));
        for level in (0..=255u8).step_by(15) {
            let want = l0 + (l1 - l0) * f64::from(level) / 255.0;
            let got = grey_lightness(seq(level));
            assert!(
                (got - want).abs() < 1.0,
                "L* at {level}: {got:.1} vs {want:.1}"
            );
        }
    }

    #[test]
    fn rnd_r6_mono_div_keeps_the_sign_legible() {
        let div = |l| Theme::Mono.color(Ink::new(Role::Div, l), 0, 0);
        let lightness = |l| grey_lightness(div(l));
        assert!(div(128).0 <= 20, "zero is near-black: {:?}", div(128));
        assert_eq!(div(128), Theme::Mono.color(Ink::new(Role::Seq, 0), 0, 0));
        assert_eq!(div(255), Rgb(255, 255, 255), "+1 reaches white");
        // The negative side tops out below mid-grey, leaving a clear gap to +1.
        assert!(
            (35.0..=50.0).contains(&lightness(0)),
            "-1 is a dim grey: L* {:.1}",
            lightness(0)
        );
        // At equal |v|, positive is always the brighter, by a visible margin off zero.
        for d in 1..=127u8 {
            let (pos, neg) = (lightness(128 + d), lightness(128 - d));
            assert!(pos > neg, "±{d}: {pos:.1} vs {neg:.1}");
            if d >= 32 {
                assert!(pos - neg >= 12.0, "±{d}: ΔL* {:.1}", pos - neg);
            }
        }
    }

    #[test]
    fn rnd_r2_rainbow_varies_by_cell_mono_is_grey() {
        let ink = Ink::new(Role::Star, 200);
        let hues: std::collections::HashSet<_> =
            (0..20).map(|x| Theme::Rainbow.color(ink, x, 0)).collect();
        assert!(hues.len() > 10, "rainbow should differ across cells");
        let Rgb(r, g, b) = Theme::Mono.color(ink, 1, 1);
        assert!(r == g && g == b);
    }

    #[test]
    fn rnd_r8_rainbow_text_is_one_neutral_grey_rising_with_level() {
        let text = |l, x, y| Theme::Rainbow.color(Ink::new(Role::Text, l), x, y);
        for level in 0..=255u8 {
            let c = text(level, 0, 0);
            let Rgb(r, g, b) = c;
            assert!(r == g && g == b, "neutral at {level}: {c:?}");
            for (x, y) in [(1, 0), (17, 3), (199, 49)] {
                assert_eq!(text(level, x, y), c, "same grey in every cell");
            }
            if level > 0 {
                assert!(luma(c) >= luma(text(level - 1, 0, 0)), "level {level}");
            }
        }
        // Other roles keep their per-cell hues.
        let star = Ink::new(Role::Star, 200);
        assert_ne!(
            Theme::Rainbow.color(star, 0, 0),
            Theme::Rainbow.color(star, 1, 0)
        );
    }

    #[test]
    fn rnd_r2_from_name_is_case_insensitive_and_next_cycles() {
        assert_eq!(Theme::from_name("ASTRO"), Some(Theme::Astro));
        assert_eq!(Theme::from_name("Rainbow"), Some(Theme::Rainbow));
        assert_eq!(Theme::from_name("neon"), None);
        let mut t = Theme::Astro;
        for _ in 0..Theme::ALL.len() {
            t = t.next();
        }
        assert_eq!(t, Theme::Astro);
    }

    /// CIE L* of any sRGB colour: an independent check on the field tables.
    fn rgb_lightness(Rgb(r, g, b): Rgb) -> f64 {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let y = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
        if y > 216.0 / 24389.0 {
            116.0 * y.cbrt() - 16.0
        } else {
            y * 24389.0 / 27.0
        }
    }

    /// HSV hue in degrees.
    fn hue(Rgb(r, g, b): Rgb) -> f64 {
        let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let d = max - min;
        let h = if d == 0.0 {
            0.0
        } else if max == r {
            60.0 * ((g - b) / d)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        h.rem_euclid(360.0)
    }

    fn hue_gap(a: f64, b: f64) -> f64 {
        let d = (a - b).abs();
        d.min(360.0 - d)
    }

    fn distance(Rgb(r, g, b): Rgb, Rgb(p, q, s): Rgb) -> u32 {
        u32::from(r.abs_diff(p)) + u32::from(g.abs_diff(q)) + u32::from(b.abs_diff(s))
    }

    fn field_roles() -> impl Iterator<Item = Role> {
        [Role::Phase, Role::Blackbody]
            .into_iter()
            .chain(Ramp::ALL.map(Role::Ramp))
    }

    #[test]
    fn rnd_r10_r11_r12_field_roles_ignore_the_cell_position() {
        for theme in Theme::ALL {
            for role in field_roles() {
                for level in [0, 64, 128, 200, 255] {
                    let ink = Ink::new(role, level);
                    let first = theme.color(ink, 0, 0);
                    for (x, y) in [(1, 0), (17, 3), (199, 49), (u16::MAX, 7)] {
                        assert_eq!(theme.color(ink, x, y), first, "{theme:?}/{role:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn rnd_r10_phase_is_a_loop_without_a_seam() {
        for theme in Theme::ALL {
            let c = |l: u8| theme.color(Ink::new(Role::Phase, l), 0, 0);
            let widest_step = (1..=255u8).map(|l| distance(c(l - 1), c(l))).max();
            let seam = distance(c(255), c(0));
            assert!(
                Some(seam) <= widest_step,
                "{theme:?}: seam {seam} vs widest step {widest_step:?}"
            );
        }
    }

    #[test]
    fn rnd_r10_astro_phase_is_isoluminant_and_passes_six_hues() {
        let c = |l: u8| Theme::Astro.color(Ink::new(Role::Phase, l), 0, 0);
        let lights: Vec<f64> = (0..=255u8).map(|l| rgb_lightness(c(l))).collect();
        let (lo, hi) = lights
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi - lo <= 5.0, "L* spread {lo:.1}..{hi:.1}");
        // Six phases a sixth of a turn apart are six clearly different hues.
        let six: Vec<f64> = (0..6u8).map(|k| hue(c(k * 43))).collect();
        for (i, a) in six.iter().enumerate() {
            for b in &six[i + 1..] {
                assert!(hue_gap(*a, *b) >= 30.0, "hues {six:?}");
            }
        }
    }

    #[test]
    fn rnd_r10_rainbow_phase_is_a_full_hue_wheel() {
        let c = |l: u8| Theme::Rainbow.color(Ink::new(Role::Phase, l), 0, 0);
        for level in 0..=255u8 {
            assert_eq!(c(level), hsv(f64::from(level) / 256.0, 1.0, 1.0));
        }
        assert_eq!(c(0), Rgb(255, 0, 0));
    }

    #[test]
    fn rnd_r10_mono_phase_is_a_grey_triangle_from_the_floor() {
        let c = |l: u8| Theme::Mono.color(Ink::new(Role::Phase, l), 0, 0);
        assert_eq!(c(0), Theme::Mono.color(Ink::new(Role::Seq, 0), 0, 0));
        assert!((rgb_lightness(c(128)) - 85.0).abs() < 1.0, "{:?}", c(128));
        for level in 1..=255u8 {
            let Rgb(r, g, b) = c(level);
            assert!(r == g && g == b, "grey at {level}");
            assert_eq!(c(level), c(level.wrapping_neg()), "symmetric at {level}");
        }
        for level in 1..=128u8 {
            assert!(luma(c(level)) >= luma(c(level - 1)), "rising at {level}");
        }
    }

    #[test]
    fn rnd_r11_blackbody_follows_the_planck_locus() {
        let c = |l: u8| Theme::Astro.color(Ink::new(Role::Blackbody, l), 0, 0);
        let kelvin = |l: u8| ink::blackbody_kelvin(Ink::new(Role::Blackbody, l));
        for level in 0..=255u8 {
            assert_eq!(
                c(level),
                Theme::Rainbow.color(Ink::new(Role::Blackbody, level), 0, 0)
            );
            let Rgb(r, g, b) = c(level);
            if kelvin(level) < 3000.0 {
                assert!(
                    r > g && g >= b,
                    "red below 3000 K at {level}: {:?}",
                    c(level)
                );
            }
            if kelvin(level) >= 10_000.0 {
                assert!(b > r && b > g, "blue at {level}: {:?}", c(level));
                assert!(rgb_lightness(c(level)) >= 75.0, "bright at {level}");
            }
        }
        let dark = rgb_lightness(c(0));
        assert!(
            (10.0..=14.0).contains(&dark),
            "dark red at 1000 K: L* {dark:.1}"
        );
        // Near white at 6500 K.
        let white = c(ink::blackbody(6500.0).level);
        assert!(white.0.min(white.1).min(white.2) >= 235, "{white:?}");
        for level in 1..=BLACKBODY_MONOTONE_TO {
            assert!(luma(c(level)) >= luma(c(level - 1)), "rising at {level}");
        }
    }

    /// Blackbody luminance rises up to the white point, 6500 K (RND-R11).
    const BLACKBODY_MONOTONE_TO: u8 = 140;

    #[test]
    fn rnd_r11_mono_blackbody_keeps_the_colour_lightness() {
        // A 5000–6000 K star is near white in colour, so it must be near white in grey too,
        // not the mid-grey its level would give as `Seq`.
        let l = |theme: Theme, level: u8| {
            let Rgb(r, g, b) = theme.color(Ink::new(Role::Blackbody, level), 0, 0);
            rgb_lightness(Rgb(r, g, b))
        };
        for level in 0..=255u8 {
            let Rgb(r, g, b) = Theme::Mono.color(Ink::new(Role::Blackbody, level), 0, 0);
            assert!(r == g && g == b, "grey at {level}");
            assert!(
                (l(Theme::Mono, level) - l(Theme::Astro, level)).abs() < 1.5,
                "{level}"
            );
        }
        for level in 1..=BLACKBODY_MONOTONE_TO {
            assert!(
                l(Theme::Mono, level) >= l(Theme::Mono, level - 1) - 0.3,
                "{level}"
            );
        }
        let sun = ink::blackbody(5772.0).level;
        assert!(l(Theme::Mono, sun) > l(Theme::Mono, 0) + 60.0 && l(Theme::Mono, sun) > 85.0);
    }

    #[test]
    fn rnd_r12_mono_maps_ramps_like_seq() {
        for role in field_roles().filter(|&r| r != Role::Phase && r != Role::Blackbody) {
            for level in 0..=255u8 {
                assert_eq!(
                    Theme::Mono.color(Ink::new(role, level), 0, 0),
                    Theme::Mono.color(Ink::new(Role::Seq, level), 0, 0),
                    "{role:?} {level}"
                );
            }
        }
    }

    #[test]
    fn rnd_r12_ramps_rise_from_near_black_to_pale() {
        for ramp in Ramp::ALL {
            let c = |l: u8| Theme::Astro.color(Ink::new(Role::Ramp(ramp), l), 0, 0);
            assert!(rgb_lightness(c(0)) <= 6.0, "{ramp:?} dark: {:?}", c(0));
            assert!(rgb_lightness(c(255)) >= 90.0, "{ramp:?} pale: {:?}", c(255));
            for level in 1..=255u8 {
                assert_eq!(
                    c(level),
                    Theme::Rainbow.color(Ink::new(Role::Ramp(ramp), level), 0, 0)
                );
                let (a, b) = (rgb_lightness(c(level - 1)), rgb_lightness(c(level)));
                // 8-bit rounding may shave a hair off; a level step is ~0.35 L*.
                assert!(b >= a - 0.25, "{ramp:?} L* at {level}: {a:.2} -> {b:.2}");
            }
        }
    }

    #[test]
    fn rnd_r12_starlight_is_grey_while_faint_and_warm_when_bright() {
        let c = |l: u8| Theme::Astro.color(Ink::new(Role::Ramp(Ramp::Starlight), l), 0, 0);
        let spread = |Rgb(r, g, b): Rgb| r.max(g).max(b) - r.min(g).min(b);
        // The faint fringe levels a halo or the Milky Way ends on: neutral grey, not sepia.
        for level in 0..=24u8 {
            assert!(spread(c(level)) <= 3, "level {level}: {:?}", c(level));
        }
        // The bright end is pale yellow-white (B − V ≈ 0.7): red above blue.
        let Rgb(r, _, b) = c(200);
        assert!(r > b + 8, "{:?}", c(200));
    }

    #[test]
    fn rnd_r12_sulfur_is_chrome_yellow_not_amber() {
        // Physarum's own colour: yellow through the middle (red and green both well above
        // blue), clearly yellower than Ember's orange at the same level.
        let c = |r: Ramp, l: u8| Theme::Astro.color(Ink::new(Role::Ramp(r), l), 0, 0);
        for level in [128u8, 170, 210] {
            let Rgb(r, g, b) = c(Ramp::Sulfur, level);
            let (h, ember) = (hue(c(Ramp::Sulfur, level)), hue(c(Ramp::Ember, level)));
            assert!((48.0..=66.0).contains(&h), "level {level}: hue {h:.0}");
            assert!(r > b + 60 && g > b + 60, "level {level}: {r} {g} {b}");
            assert!(
                h >= ember + 12.0,
                "level {level}: sulfur {h:.0}, ember {ember:.0}"
            );
        }
    }

    #[test]
    fn rnd_r12_cobalt_is_blue_and_eosin_is_rose() {
        // Cobalt is a blue, not Ice's cyan; Eosin a rose, neither plum nor crimson.
        let c = |r: Ramp, l: u8| Theme::Astro.color(Ink::new(Role::Ramp(r), l), 0, 0);
        for level in [96u8, 128, 170] {
            let Rgb(r, g, b) = c(Ramp::Cobalt, level);
            assert!(b > g + 60 && g >= r, "cobalt {level}: {r} {g} {b}");
            // Eosin is rose-pink: red leads blue (away from Ferroin's plum and Plasma's
            // violet, where blue leads or ties), but blue stays well above green and
            // within reach of red (not HAlpha's crimson, where red leads by ≈ 100).
            let Rgb(r, g, b) = c(Ramp::Eosin, level);
            assert!(r > g + 50 && b > g + 25, "eosin {level}: {r} {g} {b}");
            assert!(
                (15..=80).contains(&(i32::from(r) - i32::from(b))),
                "eosin {level}: {r} {g} {b}"
            );
        }
    }

    #[test]
    fn rnd_r12_sienna_is_a_muted_earth_tone() {
        // Jupiter's belts are rust and tan, not fire: at every mid level Sienna is warm (red
        // over green over blue) but well below Ember's saturation.
        let c = |r: Ramp, l: u8| Theme::Astro.color(Ink::new(Role::Ramp(r), l), 0, 0);
        let sat = |Rgb(r, g, b): Rgb| {
            let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
            f64::from(mx - mn) / f64::from(mx.max(1))
        };
        for level in [96u8, 128, 170, 210] {
            let s = c(Ramp::Sienna, level);
            let Rgb(r, g, b) = s;
            assert!(r > g && g > b, "sienna {level}: {r} {g} {b}");
            let (ss, se) = (sat(s), sat(c(Ramp::Ember, level)));
            assert!(
                ss + 0.2 <= se,
                "sienna {level}: sat {ss:.2} vs ember {se:.2}"
            );
        }
    }

    #[test]
    fn rnd_r12_infrared_is_an_enhancement_curve() {
        // Grey while warm (levels ≤ 76 within 3 of grey), then cyan-teal, green, yellow,
        // orange: the hue falls as the cloud top gets colder.
        let c = |l: u8| Theme::Astro.color(Ink::new(Role::Ramp(Ramp::Infrared), l), 0, 0);
        for level in 0..=76u8 {
            let Rgb(r, g, b) = c(level);
            assert!(r.max(g).max(b) - r.min(g).min(b) <= 3, "level {level}");
        }
        let steps = [(112u8, 150.0, 185.0), (140, 80.0, 130.0), (200, 10.0, 40.0)];
        for (level, lo, hi) in steps {
            let h = hue(c(level));
            assert!((lo..=hi).contains(&h), "level {level}: hue {h:.0}");
        }
    }

    #[test]
    fn rnd_r12_ramps_end_on_distinct_hues() {
        // Sienna is an earth tone in Ember's and Starlight's warm family, told apart by
        // chroma instead (rnd_r12_sienna_is_a_muted_earth_tone).
        let tops: Vec<(Ramp, f64)> = Ramp::ALL
            .iter()
            .filter(|&&r| r != Ramp::Sienna)
            .map(|&r| {
                (
                    r,
                    hue(Theme::Astro.color(Ink::new(Role::Ramp(r), 255), 0, 0)),
                )
            })
            .collect();
        for (i, (ra, a)) in tops.iter().enumerate() {
            for (rb, b) in &tops[i + 1..] {
                assert!(hue_gap(*a, *b) >= 15.0, "{ra:?} {a:.0} vs {rb:?} {b:.0}");
            }
        }
    }

    // ----- aurora (RND-R16) -----

    fn aurora(role: Role, level: u8) -> Rgb {
        Theme::Aurora.color(Ink::new(role, level), 0, 0)
    }

    /// Index of the strictly largest channel (0 red, 1 green, 2 blue).
    fn dominant(Rgb(r, g, b): Rgb) -> Option<usize> {
        let c = [r, g, b];
        let max = c.into_iter().max()?;
        let mut at = (0..3).filter(|&i| c[i] == max);
        let i = at.next()?;
        at.next().is_none().then_some(i)
    }

    #[test]
    fn rnd_r16_aurora_is_named_and_cycles_after_mono() {
        assert_eq!(Theme::from_name("Aurora"), Some(Theme::Aurora));
        assert_eq!(Theme::Aurora.name(), "aurora");
        assert_eq!(Theme::Mono.next(), Theme::Aurora);
        assert_eq!(Theme::Aurora.next(), Theme::Astro);
        assert_eq!(Theme::default(), Theme::Astro, "astro stays the default");
    }

    #[test]
    fn rnd_r16_object_roles_follow_the_emission_lines() {
        let bases = [
            (Role::Hot, Rgb(255, 70, 110)),
            (Role::Cold, Rgb(60, 255, 150)),
            (Role::Accent, Rgb(170, 90, 255)),
            (Role::Wave, Rgb(40, 220, 210)),
            (Role::Particle, Rgb(150, 255, 200)),
            (Role::Planet, Rgb(90, 170, 255)),
            (Role::Sun, Rgb(255, 215, 120)),
            (Role::Star, Rgb(225, 245, 255)),
            (Role::Structure, Rgb(130, 140, 175)),
            (Role::Dim, Rgb(60, 80, 110)),
            (Role::Background, Rgb(60, 80, 110)),
            (Role::Text, Rgb(220, 235, 235)),
        ];
        for (role, base) in bases {
            assert_eq!(aurora(role, 255), base, "{role:?} at full level");
            assert_eq!(aurora(role, 0), scale(base, 0.3), "{role:?} at level 0");
            for (x, y) in [(1, 0), (17, 3), (199, 49)] {
                let ink = Ink::new(role, 200);
                assert_eq!(
                    Theme::Aurora.color(ink, x, y),
                    aurora(role, 200),
                    "{role:?}"
                );
            }
        }
        // Hot/Cold are the two oxygen lines, a complementary red/green pair; N2+ is violet.
        assert_eq!(dominant(aurora(Role::Hot, 200)), Some(0), "Hot is red");
        assert_eq!(dominant(aurora(Role::Cold, 200)), Some(1), "Cold is green");
        assert_eq!(
            dominant(aurora(Role::Accent, 200)),
            Some(2),
            "Accent is violet"
        );
    }

    #[test]
    fn rnd_r16_seq_is_the_curtain_ramp() {
        let c = |l| aurora(Role::Seq, l);
        assert!(luma(c(0)) < 60, "level 0 is near black: {:?}", c(0));
        for l in [16, 48] {
            assert_eq!(dominant(c(l)), Some(2), "blue leads low: {l} {:?}", c(l));
        }
        for l in [192, 230, 255] {
            assert_eq!(dominant(c(l)), Some(1), "green leads high: {l} {:?}", c(l));
        }
        assert_eq!(c(255), Rgb(210, 255, 220), "ends on mint");
    }

    #[test]
    fn rnd_r16_div_is_violet_dark_green() {
        let c = |l| aurora(Role::Div, l);
        assert!(luma(c(128)) < 120, "zero near background: {:?}", c(128));
        for l in [0, 32] {
            assert_eq!(
                dominant(c(l)),
                Some(2),
                "negative is violet: {l} {:?}",
                c(l)
            );
        }
        for l in [224, 255] {
            assert_eq!(dominant(c(l)), Some(1), "positive is green: {l} {:?}", c(l));
        }
    }

    #[test]
    fn rnd_r16_unused_field_roles_reuse_astro() {
        let roles = [
            Role::Phase,
            Role::Blackbody,
            Role::Ramp(Ramp::Ember),
            Role::Ramp(Ramp::Ice),
        ];
        for role in roles {
            for level in [0, 64, 140, 255] {
                let ink = Ink::new(role, level);
                assert_eq!(
                    Theme::Aurora.color(ink, 3, 5),
                    Theme::Astro.color(ink, 3, 5),
                    "{role:?} {level}"
                );
            }
        }
    }
}
