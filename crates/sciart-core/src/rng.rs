//! A tiny generator for hot inner loops (SPEC-001 CORE-R57), and stateless hashes on its
//! finaliser (CORE-R70).
//!
//! `ctx.rng` (ChaCha8) is the scene's one source of randomness (GEN-R7, ADR-0008), but
//! it is a 136-byte state with block buffering behind `&mut`. A Monte-Carlo sweep over a
//! lattice (Ising, stochastic automata) draws millions of numbers per second and wants
//! an 8-byte state in a register: seed a [`SplitMix64`] from `ctx.rng` once, then draw
//! from it. Determinism is preserved because the seed itself came from `ctx.rng`.

use rand::{Rng, RngCore};

/// Steele, Lea & Flood's SplitMix64 (the `java.util.SplittableRandom` mixer): a Weyl
/// sequence through a 64-bit avalanche finaliser (CORE-R57). Passes BigCrush; period
/// 2⁶⁴. Not cryptographic, which art does not need.
///
/// Implements [`RngCore`], so `rand`'s [`Rng`] methods and the `particles` samplers
/// accept it too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

    /// A generator seeded with `seed`.
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// A generator seeded from `rng` (normally `ctx.rng`), consuming one `u64` of it.
    pub fn from_rng(rng: &mut impl Rng) -> Self {
        Self::new(rng.next_u64())
    }

    /// The SplitMix64 finaliser: a bijective 64-bit avalanche, so neighbouring keys give
    /// independent-looking outputs (CORE-R70).
    #[inline]
    pub const fn mix(mut z: u64) -> u64 {
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// The next 64 random bits. Seed 0 yields `0xE220A8397B1DCDAF` first.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(Self::GAMMA);
        Self::mix(self.state)
    }

    /// Uniform in `[0, 1)` from the top 53 bits: every value is a multiple of 2⁻⁵³.
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        unit(self.next_u64())
    }

    /// Uniform in `0..n` (`0` for `n = 0`), by Lemire's multiply-shift on the top 32
    /// bits: branch-free, with a bias below `n / 2³²`, invisible at art scale.
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        // (r · n) >> 32 < n for r < 2³², so the narrowing cast is lossless.
        (((self.next_u64() >> 32) * u64::from(n)) >> 32) as u32
    }
}

/// `[0, 1)` from the top 53 bits of `z`: every value is a multiple of 2⁻⁵³.
#[inline]
fn unit(z: u64) -> f64 {
    const SCALE: f64 = 1.0 / (1u64 << 53) as f64;
    (z >> 11) as f64 * SCALE
}

/// A stateless uniform in `[0, 1)` keyed by `key`: the top 53 bits of
/// [`SplitMix64::mix`]`(key)` (CORE-R70). For per-cell dither and speckle that must hold
/// still from frame to frame, e.g. `hash01(salt ^ cell_key(x, y))`.
#[inline]
pub fn hash01(key: u64) -> f64 {
    unit(SplitMix64::mix(key))
}

/// A cell's coordinates packed into one hash key, `x` in the high and `y` in the low 32
/// bits (two's complement), so distinct cells get distinct keys (CORE-R70).
#[inline]
pub const fn cell_key(x: i32, y: i32) -> u64 {
    ((x as u32 as u64) << 32) | y as u32 as u64
}

/// Smooth 2-D value noise in `[0, 1]` (CORE-R97): lattice corners every `cell` units carry
/// `hash01(salt ^ cell_key(i, j))`, blended bilinearly with smoothstep weights. Stateless,
/// so a cloud or clump texture holds still between frames without a stored field.
pub fn value_noise(salt: u64, x: f64, y: f64, cell: f64) -> f64 {
    use crate::timeline::ease;
    let (u, v) = (x / cell, y / cell);
    let (i, j) = (u.floor(), v.floor());
    // Saturating float→int casts: far-off or NaN coordinates stay finite, never panic.
    let corner = |di: i32, dj: i32| {
        hash01(salt ^ cell_key((i as i32).wrapping_add(di), (j as i32).wrapping_add(dj)))
    };
    let (fu, fv) = (ease::smoothstep(u - i), ease::smoothstep(v - j));
    ease::lerp(
        ease::lerp(corner(0, 0), corner(1, 0), fu),
        ease::lerp(corner(0, 1), corner(1, 1), fu),
        fv,
    )
}

impl RngCore for SplitMix64 {
    #[inline]
    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        SplitMix64::next_u64(self)
    }

    fn fill_bytes(&mut self, dst: &mut [u8]) {
        for chunk in dst.chunks_mut(8) {
            let bytes = SplitMix64::next_u64(self).to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }
}
