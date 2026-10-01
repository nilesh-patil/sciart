//! Jupiter's cloud colours and its belt/zone latitude profile, used by `comet_breakup`
//! (the whole disc). Until M16 `jupiter_storms` shared them, so the planet read the same
//! in both (M13 review); it now draws its clouds as threads (GEN-R25).
//!
//! Features are blended in one *level* space (0 dark … 1 pale) and then drawn in M11's
//! flat, stepped semantic colours (GEN-R23/R24, ADR-0018): dark rust belts in `Hot`, ochre
//! in `Sun`, cream zones in `Star`, so the Great Red Spot (a brick-to-rust oval inside a
//! pale collar) lands on the `Hot` steps. Five steps, like M11's stepped star in
//! exoplanet_transit; few inks also churn fewer cells than a smooth ramp. False colour.

use sciart_core::{Ink, Role, ease};

/// Cloud level of the darkest belt (`a = 0`) and the palest zone (`a = 1`).
const BELT: f64 = 0.3;
const ZONE: f64 = 0.92;
/// The Great Red Spot's pale collar, and its interior's brightest and darkest levels.
const COLLAR: f64 = 0.9;
const SPOT: (f64, f64) = (0.27, 0.5);
/// Unshaded cloud level of belt/zone dye `a` (0 belt … 1 zone) with an embossing
/// `relief` (positive on sunlit cloud edges).
pub(crate) fn cloud_level(a: f64, relief: f64) -> f64 {
    BELT + (ZONE - BELT) * a.clamp(0.0, 1.0) + relief
}

/// Unshaded cloud level inside the Great Red Spot: brick where the wound-up chromophore
/// `r` (0 … 1) is dense, rust between its arms, textured by the zone dye `a` and relief.
pub(crate) fn spot_level(r: f64, a: f64, relief: f64) -> f64 {
    let r = ease::smoothstep(((r - 0.05) / 0.45).clamp(0.0, 1.0));
    let v = SPOT.1 - (SPOT.1 - SPOT.0) * r + 0.22 * (a - 0.5) + 0.8 * relief;
    v.clamp(SPOT.0 - 0.04, SPOT.1 + 0.04)
}

/// Blend a cloud level with the Great Red Spot's: towards the collar's cream by `collar`,
/// then towards the spot's own level by `inside`.
pub(crate) fn spot_blend(cloud: f64, spot: f64, inside: f64, collar: f64) -> f64 {
    let v = cloud + (COLLAR - cloud) * collar;
    v + (spot - v) * inside
}

/// The stepped cloud palette, darkest first: `(upper bound of level·shade, ink)`.
const STEPS: [(f64, Ink); 5] = [
    (0.24, Ink::new(Role::Hot, 25)),
    (0.4, Ink::new(Role::Hot, 105)),
    (0.56, Ink::new(Role::Sun, 70)),
    (0.72, Ink::new(Role::Sun, 150)),
    (f64::INFINITY, Ink::new(Role::Star, 170)),
];

/// The one cloud lookup: `level` (unshaded) times `shade` (limb darkening, shadows),
/// snapped to one of five flat steps.
pub(crate) fn cloud_ink(level: f64, shade: f64) -> Ink {
    let v = level * shade;
    STEPS
        .iter()
        .find(|(top, _)| v < *top)
        .map_or(STEPS[4].1, |s| s.1)
}

/// Rank of a cloud ink in [`STEPS`] (0 darkest); `None` for anything else.
pub(crate) fn step_of(ink: Ink) -> Option<usize> {
    STEPS.iter().position(|s| s.1 == ink)
}

/// Whether `ink` is one of the cloud steps (or a scar on them).
pub(crate) fn is_cloud(ink: Ink) -> bool {
    ink == SCAR || step_of(ink).is_some()
}

/// Impact scars: darker than the darkest belt.
const SCAR: Ink = Ink::new(Role::Dim, 0);

/// `ink` stepped `n` steps darker, into a dark scar below the darkest belt.
pub(crate) fn darker(ink: Ink, n: usize) -> Ink {
    match step_of(ink) {
        Some(s) if s >= n => STEPS[s - n].1,
        _ => SCAR,
    }
}

/// `ink` one step paler (a scar lifts to the darkest belt).
pub(crate) fn lighter(ink: Ink) -> Ink {
    step_of(ink).map_or(STEPS[0].1, |s| STEPS[(s + 1).min(STEPS.len() - 1)].1)
}

/// Cloud colour for dye `a`, `relief` and `shade` in one call (used by the tests).
#[cfg(test)]
pub(crate) fn cloud(a: f64, relief: f64, shade: f64) -> Ink {
    cloud_ink(cloud_level(a, relief), shade)
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
    fn jup_a6_cloud_colour_is_a_stepped_m11_palette() {
        // Belts in Hot, zones in Sun/Star (GEN-R23), monotone in the dye, flat steps.
        let mut last = 0;
        for k in 0..=100 {
            let c = cloud(f64::from(k) / 100.0, 0.0, 1.0);
            assert!(
                matches!(c.role, Role::Hot | Role::Sun | Role::Star),
                "{c:?}"
            );
            let s = step_of(c).expect("a palette step");
            assert!(s >= last, "not monotone at {k}");
            last = s;
        }
        assert_eq!(cloud(0.0, 0.0, 1.0).role, Role::Hot, "belts are rust");
        assert_eq!(cloud(1.0, 0.0, 1.0).role, Role::Star, "zones are cream");
    }

    #[test]
    fn jup_a8_red_spot_is_sienna_in_a_cream_collar() {
        // Inside: rust to brick, never as pale as a zone.
        for r in [0.0, 0.5, 1.0] {
            let v = spot_level(r, 0.5, 0.0);
            assert!((0.22..=0.55).contains(&v), "spot level {v} at r = {r}");
        }
        assert!(spot_level(1.0, 0.5, 0.0) < spot_level(0.0, 0.5, 0.0));
        let belt = cloud_level(0.0, 0.0);
        // The collar pulls a dark belt towards its cream.
        assert!(spot_blend(belt, 0.5, 0.0, 0.65) > 0.65);
    }
}
