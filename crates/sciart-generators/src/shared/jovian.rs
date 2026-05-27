//! Jupiter's cloud colours and its belt/zone latitude profile, shared by `jupiter_storms`
//! (the close-up) and `comet_breakup` (the whole disc) so the planet reads the same in
//! both (M13 review).
//!
//! Everything but the hot spots is one ramp, `Ramp::Sienna`, and features are blended in
//! *level* space before a single lookup, so no edge is a switch between ramps: dark rust
//! belts, cream zones, the Great Red Spot as a textured brick-to-rust oval inside a pale
//! cream collar, feathered into the flow round it. The hot spots are the one blue
//! (`Ramp::Ice`, low levels): named ramps share their lightness at equal level (RND-R12),
//! so a belt darkened to the hot spot's level and the Ice beyond it meet at the same
//! lightness and the change of hue reads as a soft edge. All false colour (enhanced, as
//! JunoCam frames are).

use sciart_core::{Ink, Ramp, ease, ink};

/// Sienna level of the darkest belt (`a = 0`) and the palest zone (`a = 1`).
const BELT: f64 = 0.3;
const ZONE: f64 = 0.92;
/// The Great Red Spot's pale collar, and its interior's brightest and darkest levels.
const COLLAR: f64 = 0.9;
/// How far the collar pulls the clouds towards its cream (a hint, not a ring).
const COLLAR_W: f64 = 0.65;
const SPOT: (f64, f64) = (0.27, 0.5);
/// Hot spots: the level where a belt darkening towards one meets its Ice core, and how
/// far the core itself darkens.
const HOT_EDGE: f64 = 0.2;
const HOT_DEEP: f64 = 0.07;

/// Unshaded Sienna level of belt/zone dye `a` (0 belt … 1 zone) with an embossing
/// `relief` (positive on sunlit cloud edges).
pub(crate) fn cloud_level(a: f64, relief: f64) -> f64 {
    BELT + (ZONE - BELT) * a.clamp(0.0, 1.0) + relief
}

/// Unshaded Sienna level inside the Great Red Spot: brick where the wound-up chromophore
/// `r` (0 … 1) is dense, rust between its arms, textured by the zone dye `a` and relief.
pub(crate) fn spot_level(r: f64, a: f64, relief: f64) -> f64 {
    let r = ease::smoothstep(((r - 0.05) / 0.45).clamp(0.0, 1.0));
    let v = SPOT.1 - (SPOT.1 - SPOT.0) * r + 0.22 * (a - 0.5) + 0.8 * relief;
    v.clamp(SPOT.0 - 0.04, SPOT.1 + 0.04)
}

/// Weights `(inside, collar)` of the Great Red Spot at normalised elliptical radius `rho`
/// (1 on its rim) for a spot whose smaller semi-axis spans `semi_px` pixels: the rim is
/// feathered over 2.5 pixels, and the collar is a ring out to 0.45 semi-axes beyond it,
/// feathered on both sides.
pub(crate) fn spot_weights(rho: f64, semi_px: f64) -> (f64, f64) {
    const FEATHER: f64 = 2.5;
    let d = (rho - 1.0) * semi_px.max(1.0);
    let inside = ease::smoothstep((0.5 - d / FEATHER).clamp(0.0, 1.0));
    let width = (0.3 * semi_px).max(FEATHER);
    let outer = ease::smoothstep(((width - d) / FEATHER + 0.5).clamp(0.0, 1.0));
    (inside, COLLAR_W * (1.0 - inside) * outer)
}

/// Blend a cloud level with the Great Red Spot's: towards the collar's cream by `collar`,
/// then towards the spot's own level by `inside`.
pub(crate) fn spot_blend(cloud: f64, spot: f64, inside: f64, collar: f64) -> f64 {
    let v = cloud + (COLLAR - cloud) * collar;
    v + (spot - v) * inside
}

/// The one Sienna lookup: `level` (unshaded) times `shade` (limb darkening, shadows).
pub(crate) fn sienna(level: f64, shade: f64) -> Ink {
    ink::ramp(Ramp::Sienna, (level * shade).clamp(0.02, 0.97))
}

/// Cloud colour for dye `a`, `relief` and `shade` in one call (used by the tests).
#[cfg(test)]
pub(crate) fn cloud(a: f64, relief: f64, shade: f64) -> Ink {
    sienna(cloud_level(a, relief), shade)
}

/// Clearing dye `h` below `HOT_ON` leaves the clouds alone; from there to `HOT_CORE` the
/// cloud darkens towards the hot spot's edge level; above it is the Ice core.
const HOT_ON: f64 = 0.12;
const HOT_CORE: f64 = 0.3;

/// A cloud of unshaded level `level` under clearing dye `h` (0 … 1), shaded: untouched
/// for faint dye, darkening smoothly to the hot spot's edge level, then the dark
/// blue-grey core of the hot spot (a clear gap down to warmer, deeper cloud).
pub(crate) fn hot_spot(level: f64, h: f64, shade: f64) -> Ink {
    if h <= HOT_ON {
        return sienna(level, shade);
    }
    if h < HOT_CORE {
        let t = ease::smoothstep((h - HOT_ON) / (HOT_CORE - HOT_ON));
        return sienna(level + (HOT_EDGE - level) * t, shade);
    }
    let depth = ((h - HOT_CORE) / (1.0 - HOT_CORE)).clamp(0.0, 1.0);
    ink::ramp(
        Ramp::Ice,
        ((HOT_EDGE - HOT_DEEP * depth) * shade).clamp(0.02, 0.5),
    )
}

/// Band edges in sine of planetocentric latitude (north positive), south to north: the
/// polar hood, STZ, STB, STrZ, SEB, EZ, NEB, NTrZ, NTB, NTZ, NNTB and the north polar hood.
const EDGES: [f64; 11] = [
    -0.669, -0.574, -0.454, -0.342, -0.122, 0.122, 0.309, 0.407, 0.515, 0.6, 0.707,
];
/// Dye of each band between those edges (0 = belt, 1 = zone).
const VALUES: [f64; 12] = [
    0.42, 0.78, 0.32, 0.92, 0.14, 0.86, 0.08, 0.9, 0.3, 0.74, 0.4, 0.46,
];
/// Width of a band edge, in sine of latitude.
const EDGE_W: f64 = 0.05;

/// Belt/zone dye (0 belt … 1 zone) of Jupiter at sine latitude `s`, smooth across edges.
pub(crate) fn band_profile(s: f64) -> f64 {
    let mut v = VALUES[0];
    for (k, e) in EDGES.iter().enumerate() {
        let u = ease::smoothstep(((s - e) / EDGE_W + 0.5).clamp(0.0, 1.0));
        v += (VALUES[k + 1] - VALUES[k]) * u;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::Role;
    use sciart_core::ink::seq_value;

    #[test]
    fn jup_a6_band_profile_has_belts_and_zones_in_place() {
        // Equatorial zone pale, the belts either side of it dark, tropical zones pale.
        assert!(band_profile(0.0) > 0.8);
        assert!(band_profile(0.2) < 0.2 && band_profile(-0.22) < 0.25);
        assert!(band_profile(0.36) > 0.8 && band_profile(-0.4) > 0.8);
        // Smooth: no jump between neighbouring samples.
        let mut last = band_profile(-1.0);
        for k in 1..=2000 {
            let v = band_profile(-1.0 + f64::from(k) / 1000.0);
            assert!((v - last).abs() < 0.04, "jump at {k}: {last} -> {v}");
            last = v;
        }
    }

    #[test]
    fn jup_a6_cloud_colour_is_one_continuous_sienna_map() {
        let mut last = 0.0;
        for k in 0..=100 {
            let c = cloud(f64::from(k) / 100.0, 0.0, 1.0);
            assert_eq!(c.role, Role::Ramp(Ramp::Sienna));
            let v = seq_value(c);
            assert!(v >= last, "not monotone at {k}");
            last = v;
        }
    }

    #[test]
    fn jup_a8_red_spot_is_sienna_feathered_into_a_cream_collar() {
        // Inside: rust to brick, never as pale as a zone.
        for r in [0.0, 0.5, 1.0] {
            let v = spot_level(r, 0.5, 0.0);
            assert!((0.22..=0.55).contains(&v), "spot level {v} at r = {r}");
        }
        assert!(spot_level(1.0, 0.5, 0.0) < spot_level(0.0, 0.5, 0.0));
        // The rim is feathered over pixels, not a step: neighbouring pixels (1 px apart
        // at a 10 px semi-axis) differ by at most 0.6 of the full blend (a smoothstep
        // over 2.5 px), never a full step.
        let semi = 10.0;
        let mut last = spot_weights(0.0, semi);
        assert_eq!(last, (1.0, 0.0));
        for k in 1..=30 {
            let w = spot_weights(f64::from(k) / semi, semi);
            assert!((w.0 - last.0).abs() <= 0.61, "inside step at {k}");
            assert!((w.1 - last.1).abs() <= 0.61, "collar step at {k}");
            last = w;
        }
        // A pale collar just outside the rim, gone further out.
        let (_, collar) = spot_weights(1.15, semi);
        assert!(collar > 0.6, "collar {collar}");
        assert_eq!(spot_weights(3.0, semi), (0.0, 0.0));
        let belt = cloud_level(0.0, 0.0);
        assert!(spot_blend(belt, 0.5, 0.0, COLLAR_W) > 0.65);
    }

    #[test]
    fn jup_a6_hot_spots_meet_the_belt_at_equal_lightness() {
        // Faint dye leaves the cloud; the darkened edge and the Ice core share a level,
        // so the hue change is not a lightness step.
        let belt = cloud_level(0.1, 0.0);
        assert_eq!(hot_spot(belt, 0.05, 1.0), sienna(belt, 1.0));
        let edge = hot_spot(belt, HOT_CORE - 1e-9, 1.0);
        let core = hot_spot(belt, HOT_CORE + 1e-9, 1.0);
        assert_eq!(edge.role, Role::Ramp(Ramp::Sienna));
        assert_eq!(core.role, Role::Ramp(Ramp::Ice));
        assert!(edge.level.abs_diff(core.level) <= 1, "{edge:?} vs {core:?}");
    }
}
