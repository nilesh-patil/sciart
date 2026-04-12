use sciart_core::{Ink, Role};

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
        match self {
            Theme::Astro => scale(astro_base(ink.role), 0.3 + 0.7 * level),
            Theme::Rainbow => hsv(cell_hue(x, y), 1.0, 0.35 + 0.65 * level),
            Theme::Mono => {
                let g = 40 + (u16::from(ink.level) * 215 / 255) as u8;
                Rgb(g, g, g)
            }
        }
    }
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

    const ROLES: [Role; 12] = [
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
    ];

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
    fn rnd_r1_brighter_level_is_not_darker() {
        let luma = |Rgb(r, g, b): Rgb| u32::from(r) + u32::from(g) + u32::from(b);
        for theme in Theme::ALL {
            for role in ROLES {
                let lo = theme.color(Ink::new(role, 10), 5, 5);
                let hi = theme.color(Ink::new(role, 250), 5, 5);
                assert!(luma(hi) >= luma(lo), "{theme:?}/{role:?}");
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
