//! Ballistic volcanic and impact plumes (M13 shared model: `io_volcanoes`,
//! `comet_breakup`).
//!
//! Grains leave a vent with speed `v0` in a direction drawn uniformly (in solid angle)
//! between two zenith angles and fly in a uniform gravity `g` with no air: every path is
//! an exact parabola, advanced in closed form, so the apex is exactly `(v0·cos θ)²/(2g)`
//! and a grain launched at zenith angle θ lands `v0²·sin 2θ / g` away on flat ground
//! (IO-A1, IO-A3). A landing is reported at its exact point inside the step, then the
//! grain is removed. Coordinates are local to the vent: `x`, `y` along the ground, `z` up.
//!
//! The pool has a fixed capacity (GEN-R11): a launch into a full pool is dropped.

use rand::Rng;
use sciart_core::ParticlePool;
use sciart_core::Trig;

/// One plume grain.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Grain {
    /// Position relative to the vent (`z` up), in the scene's length unit.
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    /// Seconds since launch.
    pub age: f32,
    /// Which vent (or impact) launched it.
    pub src: u8,
}

#[cfg(test)]
/// Apex height of a grain launched at `v0` at zenith angle `zenith` (radians).
pub(crate) fn apex(v0: f64, g: f64, zenith: f64) -> f64 {
    let vz = v0 * zenith.cos_det();
    vz * vz / (2.0 * g)
}

/// Flat-ground range of a grain launched at `v0` at zenith angle `zenith`.
pub(crate) fn range(v0: f64, g: f64, zenith: f64) -> f64 {
    v0 * v0 * (2.0 * zenith).sin_det() / g
}

#[derive(Debug, Clone)]
pub(crate) struct Plume {
    pool: ParticlePool<Grain>,
}

impl Plume {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            pool: ParticlePool::with_capacity(capacity),
        }
    }

    /// Launch one grain from `src` at speed `v0`, zenith angle uniform in solid angle over
    /// `[zenith.0, zenith.1]` (radians) and azimuth uniform. Returns `false` when the
    /// pool is full.
    pub fn launch(&mut self, rng: &mut impl Rng, src: u8, v0: f32, zenith: (f32, f32)) -> bool {
        let (c0, c1) = (zenith.0.cos_det(), zenith.1.cos_det());
        let cz = c1 + (c0 - c1) * rng.random::<f32>();
        let sz = (1.0 - cz * cz).max(0.0).sqrt();
        let (sa, ca) = (std::f32::consts::TAU * rng.random::<f32>()).sin_cos_det();
        self.pool.spawn(Grain {
            pos: [0.0; 3],
            vel: [v0 * sz * ca, v0 * sz * sa, v0 * cz],
            age: 0.0,
            src,
        })
    }

    /// [`launch`](Self::launch), but with the azimuth drawn from `rays` evenly spaced
    /// directions (a little jittered), so the plume shows radial filaments; `rays = 0`
    /// is a uniform azimuth.
    pub fn launch_rays(
        &mut self,
        rng: &mut impl Rng,
        src: u8,
        v0: f32,
        zenith: (f32, f32),
        rays: u32,
    ) -> bool {
        let (c0, c1) = (zenith.0.cos_det(), zenith.1.cos_det());
        let cz = c1 + (c0 - c1) * rng.random::<f32>();
        let sz = (1.0 - cz * cz).max(0.0).sqrt();
        let turn = if rays == 0 {
            rng.random::<f32>()
        } else {
            let k = rng.random_range(0..rays) as f32;
            (k + 0.4 * (rng.random::<f32>() - 0.5)) / rays as f32
        };
        let (sa, ca) = (std::f32::consts::TAU * turn).sin_cos_det();
        self.pool.spawn(Grain {
            pos: [0.0; 3],
            vel: [v0 * sz * ca, v0 * sz * sa, v0 * cz],
            age: 0.0,
            src,
        })
    }

    /// Fly every grain for `dt` seconds under gravity `g` (down `z`); a grain that
    /// reaches the ground (`z ≤ 0` after lift-off) is reported to `landed` at its exact
    /// landing point and removed.
    pub fn step(&mut self, dt: f32, g: f32, mut landed: impl FnMut(&Grain)) {
        if !(dt.is_finite() && dt > 0.0 && g.is_finite() && g > 0.0) {
            return;
        }
        self.pool.retain(|p| {
            let z_new = p.pos[2] + p.vel[2] * dt - 0.5 * g * dt * dt;
            if z_new <= 0.0 {
                // The later root of z0 + vz·τ − ½gτ² = 0, clamped into the step.
                let disc = (p.vel[2] * p.vel[2] + 2.0 * g * p.pos[2].max(0.0)).sqrt();
                let tau = ((p.vel[2] + disc) / g).clamp(0.0, dt);
                let mut at = *p;
                at.pos[0] += p.vel[0] * tau;
                at.pos[1] += p.vel[1] * tau;
                at.pos[2] = 0.0;
                at.vel[2] -= g * tau;
                at.age += tau;
                landed(&at);
                return false;
            }
            p.pos[0] += p.vel[0] * dt;
            p.pos[1] += p.vel[1] * dt;
            p.pos[2] = z_new;
            p.vel[2] -= g * dt;
            p.age += dt;
            true
        });
    }

    pub fn iter(&self) -> impl Iterator<Item = &Grain> {
        self.pool.iter()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pool.len()
    }

    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.pool.capacity()
    }

    pub fn clear(&mut self) {
        self.pool.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::SplitMix64;

    #[test]
    fn io_a1_grains_peak_at_the_closed_form_apex() {
        let mut rng = SplitMix64::new(1);
        let mut plume = Plume::with_capacity(64);
        let (v0, g) = (1.0f32, 1.8e-3f32); // km/s and km/s²: Pele
        for _ in 0..32 {
            plume.launch(&mut rng, 0, v0, (0.0, 0.9));
        }
        let mut peak = [0.0f32; 64];
        let zen: Vec<f32> = plume.iter().map(|p| (p.vel[2] / v0).acos()).collect();
        let dt = 0.5;
        for _ in 0..4000 {
            for (k, p) in plume.iter().enumerate() {
                peak[k] = peak[k].max(p.pos[2]);
            }
            plume.step(dt, g, |_| {});
            if plume.len() < zen.len() {
                break;
            }
        }
        for (k, &z) in zen.iter().enumerate() {
            let want = apex(f64::from(v0), f64::from(g), f64::from(z)) as f32;
            assert!(
                (peak[k] - want).abs() <= 0.01 * want,
                "{} vs {want}",
                peak[k]
            );
        }
    }

    #[test]
    fn io_a3_landings_match_the_ballistic_range() {
        let mut rng = SplitMix64::new(2);
        let mut plume = Plume::with_capacity(8);
        let (v0, g) = (0.5f32, 1.8e-3f32);
        let z = std::f32::consts::FRAC_PI_4;
        plume.launch(&mut rng, 0, v0, (z, z));
        let mut got = None;
        for _ in 0..10_000 {
            plume.step(0.7, g, |p| got = Some(p.pos[0].hypot(p.pos[1])));
            if got.is_some() {
                break;
            }
        }
        let want = range(f64::from(v0), f64::from(g), f64::from(z)) as f32;
        let got = got.expect("the grain lands");
        assert!((got - want).abs() < 1e-3 * want, "{got} vs {want}");
    }

    #[test]
    fn io_a4_the_pool_never_exceeds_capacity() {
        let mut rng = SplitMix64::new(3);
        let mut plume = Plume::with_capacity(10);
        let launched = (0..25)
            .filter(|_| plume.launch(&mut rng, 0, 1.0, (0.0, 1.0)))
            .count();
        assert_eq!(launched, 10);
        assert_eq!(plume.len(), plume.capacity());
        plume.step(f32::NAN, 1.0, |_| {});
        assert_eq!(plume.len(), 10);
    }
}
