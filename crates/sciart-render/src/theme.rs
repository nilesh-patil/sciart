use sciart_core::{Ink, Role, ink};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// Maps semantic inks to colors. Pure: no RNG, no time, so frames never strobe (RND-R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Astro,
    Rainbow,
    Mono,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Astro, Theme::Rainbow, Theme::Mono];

    pub const fn name(self) -> &'static str {
        match self {
            Theme::Astro => "astro",
            Theme::Rainbow => "rainbow",
            Theme::Mono => "mono",
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
            // Colormap roles ignore the per-cell hash and the role-hue scaling: a field
            // needs one smooth, perceptually ordered ramp (RND-R6, ADR-0011).
            (Theme::Astro, Role::Seq) => gradient(&INFERNO, ink::seq_value(ink)),
            (Theme::Astro, Role::Div) => gradient(&ASTRO_DIV, div_pos(ink)),
            (Theme::Rainbow, Role::Seq) => gradient(&TURBO, ink::seq_value(ink)),
            (Theme::Rainbow, Role::Div) => gradient(&BLUE_RED, div_pos(ink)),
            // |level − 128| ∈ [0, 128] scaled so both ends reach the top of the grey ramp.
            (Theme::Mono, Role::Div) => {
                let d = u16::from(ink.level.abs_diff(128));
                grey((d * 255 / 127).min(255) as u8)
            }
            (Theme::Mono, _) => grey(ink.level),
            (Theme::Astro, role) => scale(astro_base(role), 0.3 + 0.7 * level),
            (Theme::Rainbow, _) => hsv(cell_hue(x, y), 1.0, 0.35 + 0.65 * level),
        }
    }
}

/// `mono`'s ramp: level 0 stays visible (40) on a black terminal.
fn grey(level: u8) -> Rgb {
    let g = 40 + (u16::from(level) * 215 / 255) as u8;
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

#[cfg(test)]
mod tests {
    use super::*;

    const ROLES: [Role; 14] = [
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
            for role in ROLES {
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

    #[test]
    fn rnd_r6_mono_is_grey_by_level_and_by_distance_from_zero() {
        for level in [0u8, 1, 64, 127, 128, 129, 200, 255] {
            for role in [Role::Seq, Role::Div] {
                let Rgb(r, g, b) = Theme::Mono.color(Ink::new(role, level), 0, 0);
                assert!(r == g && g == b, "{role:?} {level}");
            }
        }
        let seq = |l| Theme::Mono.color(Ink::new(Role::Seq, l), 0, 0);
        assert_eq!(seq(77), Theme::Mono.color(Ink::new(Role::Star, 77), 0, 0));
        let div = |l| luma(Theme::Mono.color(Ink::new(Role::Div, l), 0, 0));
        assert_eq!(div(128 + 50), div(128 - 50), "symmetric about zero");
        assert!(div(128) < div(100) && div(100) < div(0));
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
}
