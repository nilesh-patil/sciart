//! xterm-256 quantization for `--color 256` (RND-R18, ADR-0026).
//!
//! Every painted RGB is mapped to the nearest entry of the standard xterm-256 palette:
//! the 6×6×6 cube (levels 0/95/135/175/215/255, indices 16..=231) and the 24-step grey
//! ramp (8 + 10i, indices 232..=255). The mapping is a precomputed 32³ table keyed by the
//! top five bits of each channel, so a frame costs one lookup per colour; it is built
//! once, deterministically, and never changes at runtime.
//!
//! The key loses the low three bits, so the table stores the nearest palette entry to the
//! bucket's centre (8k + 4), not to its lower bound: that keeps the worst-case per-channel
//! error at the bucket half-width plus half the widest cube step, and makes every colour
//! whose bucket centre is a palette entry round-trip exactly.

use std::sync::LazyLock;

#[cfg(feature = "terminal")]
use ratatui::style::Color;

use crate::Rgb;

/// Cube levels in index order.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// `Rgb` for an xterm-256 index in the cube+greys range (16..=255).
fn xterm_rgb(index: u8) -> Rgb {
    match index {
        16..=231 => {
            let i = usize::from(index - 16);
            Rgb(CUBE[i / 36], CUBE[(i / 6) % 6], CUBE[i % 6])
        }
        232..=255 => {
            let v = 8 + 10 * (index - 232);
            Rgb(v, v, v)
        }
        _ => panic!("xterm_rgb: index {index} is outside the cube and grey ramp"),
    }
}

/// Squared RGB distance between two colours.
fn distance2(a: Rgb, b: Rgb) -> u32 {
    let d = |x: u8, y: u8| {
        let d = i32::from(x) - i32::from(y);
        (d * d) as u32
    };
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

/// The nearest cube/grey palette index to `c` (ties go to the lower index, so the result
/// is deterministic).
fn nearest(c: Rgb) -> u8 {
    (16..=255)
        .min_by_key(|&i| distance2(c, xterm_rgb(i)))
        .expect("16..=255 is not empty")
}

/// Build the 32³ lookup table: index `(r5 << 10) | (g5 << 5) | b5`.
fn build() -> [u8; 32 * 32 * 32] {
    let mut lut = [0u8; 32 * 32 * 32];
    for r in 0..32u8 {
        for g in 0..32u8 {
            for b in 0..32u8 {
                let centre = Rgb((r << 3) | 4, (g << 3) | 4, (b << 3) | 4);
                let key = (usize::from(r) << 10) | (usize::from(g) << 5) | usize::from(b);
                lut[key] = nearest(centre);
            }
        }
    }
    lut
}

/// The table, built on first use (32768 bytes, 240 palette probes per entry).
static LUT: LazyLock<[u8; 32 * 32 * 32]> = LazyLock::new(build);

/// Nearest xterm-256 index for `c` (RND-R18).
pub fn quantize(c: Rgb) -> u8 {
    let key = (usize::from(c.0 >> 3) << 10) | (usize::from(c.1 >> 3) << 5) | usize::from(c.2 >> 3);
    LUT[key]
}

/// `c` as an indexed ratatui colour; the encoder writes `38;5;n`/`48;5;n` (RND-R18).
#[cfg(feature = "terminal")]
pub fn indexed(c: Rgb) -> Color {
    Color::Indexed(quantize(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rnd_r18_quantizer_known_colours() {
        // Pure primaries and corners of the cube map to their cube entries...
        assert_eq!(quantize(Rgb(255, 0, 0)), 196, "red");
        assert_eq!(quantize(Rgb(0, 255, 0)), 46, "green");
        assert_eq!(quantize(Rgb(0, 0, 255)), 21, "blue");
        assert_eq!(quantize(Rgb(255, 255, 0)), 226, "yellow");
        assert_eq!(quantize(Rgb(255, 255, 255)), 231, "white");
        assert_eq!(quantize(Rgb(0, 0, 0)), 16, "black");
        // ...and the near-black first cube step is why the lookup uses bucket centres.
        assert_eq!(quantize(Rgb(95, 0, 0)), 52, "dark red");
        assert_eq!(quantize(Rgb(255, 0, 255)), 201, "magenta");
    }

    #[test]
    fn rnd_r18_quantizer_greys_use_the_24_step_ramp() {
        for (v, index) in [(8u8, 232u8), (18, 233), (28, 234), (238, 255)] {
            assert_eq!(quantize(Rgb(v, v, v)), index, "grey {v}");
            assert_eq!(xterm_rgb(index), Rgb(v, v, v));
        }
        assert_eq!(quantize(Rgb(128, 128, 128)), 102, "128 is nearer cube 135");
    }

    #[test]
    fn rnd_r18_quantizer_cube_corners_decode_back() {
        for (i, c) in (16..=231u8)
            .map(|i| (i, xterm_rgb(i)))
            .filter(|(_, c)| c.0 == c.1 && c.1 == c.2)
            .collect::<Vec<_>>()
        {
            assert_eq!(quantize(c), i, "{c:?}");
        }
    }

    #[test]
    fn rnd_r18_quantizer_round_trip_is_within_the_bucket_bound() {
        // The table quantizes the bucket centre: the worst channel error is half a bucket
        // (4) plus the widest cube gap's half (48); 52 leaves room for the joint nearest.
        let mut worst = 0u8;
        let mut worst_at = None;
        for r in 0..32u8 {
            for g in 0..32u8 {
                for b in 0..32u8 {
                    let c = Rgb((r << 3) | 4, (g << 3) | 4, (b << 3) | 4);
                    let got = xterm_rgb(quantize(c));
                    let d = [
                        c.0.abs_diff(got.0),
                        c.1.abs_diff(got.1),
                        c.2.abs_diff(got.2),
                    ];
                    let max = d.into_iter().max().unwrap_or(0);
                    if max > worst {
                        worst = max;
                        worst_at = Some(c);
                    }
                }
            }
        }
        assert!(
            worst <= 52,
            "worst per-channel error {worst} at {worst_at:?}"
        );
    }

    #[test]
    fn rnd_r18_quantizer_is_deterministic_and_always_in_palette() {
        for c in [
            Rgb(1, 2, 3),
            Rgb(200, 100, 50),
            Rgb(17, 240, 9),
            Rgb(255, 255, 254),
        ] {
            let i = quantize(c);
            assert!((16..=255).contains(&i), "{c:?} -> {i}");
            assert_eq!(i, quantize(c), "{c:?}");
            #[cfg(feature = "terminal")]
            assert_eq!(indexed(c), Color::Indexed(i));
        }
    }
}
