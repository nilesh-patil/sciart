//! SPEC-001 CORE-R28..R32: particle helpers.

use proptest::prelude::*;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use sciart_core::particles::{self, ParticlePool, Trail};
use sciart_core::prelude::*;

const G: Glyph = Glyph::ascii('*');
const INK: Ink = Ink::new(Role::Particle, 200);

fn sorted(pool: &ParticlePool<u32>) -> Vec<u32> {
    let mut v: Vec<u32> = pool.iter().copied().collect();
    v.sort_unstable();
    v
}

fn drawn(c: &Canvas) -> Vec<(i32, i32)> {
    let w = i32::from(c.size().width());
    c.cells()
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.ch != ' ')
        .map(|(i, _)| (i as i32 % w, i as i32 / w))
        .collect()
}

// ---------------------------------------------------------------- CORE-R28

#[test]
fn core_r28_spawn_until_full_then_reject() {
    let mut pool = ParticlePool::with_capacity(3);
    assert!(pool.is_empty());
    assert_eq!(pool.capacity(), 3);
    for i in 0..3u32 {
        assert!(pool.spawn(i));
    }
    assert!(!pool.spawn(99), "full pool must reject");
    assert_eq!(pool.len(), 3);
    assert_eq!(sorted(&pool), vec![0, 1, 2]);
}

#[test]
fn core_r28_capacity_zero_holds_nothing() {
    let mut pool = ParticlePool::<u32>::with_capacity(0);
    assert!(!pool.spawn(1));
    assert!(pool.is_empty());
    assert_eq!(pool.capacity(), 0);
    pool.retain(|_| true);
    assert_eq!(pool.iter().count(), 0);
}

#[test]
fn core_r28_retain_updates_in_place_and_removes_rejected() {
    let mut pool = ParticlePool::with_capacity(10);
    for i in 0..10u32 {
        pool.spawn(i);
    }
    // Every particle is visited exactly once, and mutations stick.
    let mut visits = 0;
    pool.retain(|p| {
        visits += 1;
        *p *= 10;
        *p % 20 == 0
    });
    assert_eq!(visits, 10);
    assert_eq!(sorted(&pool), vec![0, 20, 40, 60, 80]);
    // Freed slots are reusable.
    assert!(pool.spawn(7));
}

#[test]
fn core_r28_retain_all_and_none() {
    let mut pool = ParticlePool::with_capacity(5);
    for i in 0..5u32 {
        pool.spawn(i);
    }
    pool.retain(|_| true);
    assert_eq!(sorted(&pool), vec![0, 1, 2, 3, 4]);
    pool.retain(|_| false);
    assert!(pool.is_empty());
    assert_eq!(pool.capacity(), 5);
}

#[test]
fn core_r28_iter_mut_and_clear() {
    let mut pool = ParticlePool::with_capacity(4);
    pool.spawn(1u32);
    pool.spawn(2);
    for p in pool.iter_mut() {
        *p += 100;
    }
    assert_eq!(sorted(&pool), vec![101, 102]);
    pool.clear();
    assert!(pool.is_empty());
    assert_eq!(pool.capacity(), 4);
}

#[test]
fn core_r28_set_capacity_shrink_truncates_and_grow_accepts_more() {
    let mut pool = ParticlePool::with_capacity(6);
    for i in 0..6u32 {
        pool.spawn(i);
    }
    pool.set_capacity(2);
    assert_eq!(pool.capacity(), 2);
    assert_eq!(pool.len(), 2);
    assert!(!pool.spawn(9));

    pool.set_capacity(5);
    assert_eq!(pool.capacity(), 5);
    assert_eq!(pool.len(), 2, "growing keeps existing particles");
    for i in 0..3u32 {
        assert!(pool.spawn(i));
    }
    assert!(!pool.spawn(9));

    pool.set_capacity(0);
    assert!(pool.is_empty());
    assert!(!pool.spawn(1));
}

// ---------------------------------------------------------------- CORE-R29

#[test]
fn core_r29_trail_keeps_last_n_oldest_first() {
    let mut t = Trail::new(3);
    assert!(t.is_empty());
    t.push(1);
    t.push(2);
    assert_eq!(t.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
    for v in 3..=7 {
        t.push(v);
        assert!(t.len() <= 3);
    }
    assert_eq!(t.len(), 3);
    assert_eq!(t.capacity(), 3);
    assert_eq!(t.iter().copied().collect::<Vec<_>>(), vec![5, 6, 7]);
    assert_eq!(t.newest(), Some(7));
}

#[test]
fn core_r29_trail_zero_keeps_nothing() {
    let mut t = Trail::new(0);
    for v in 0..10 {
        t.push(v);
    }
    assert_eq!(t.len(), 0);
    assert_eq!(t.iter().count(), 0);
    assert_eq!(t.newest(), None);
}

#[test]
fn core_r29_trail_clear_then_reuse() {
    let mut t = Trail::new(2);
    for v in 0..5 {
        t.push(v);
    }
    t.clear();
    assert!(t.is_empty());
    assert_eq!(t.newest(), None);
    t.push(42);
    t.push(43);
    t.push(44);
    assert_eq!(t.iter().copied().collect::<Vec<_>>(), vec![43, 44]);
}

// ---------------------------------------------------------------- CORE-R30

#[test]
fn core_r30_zero_sd_returns_mean_exactly() {
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    for &mean in &[0.0, 1.5, -3.25, 1e12] {
        for _ in 0..100 {
            assert_eq!(particles::gaussian(&mut rng, mean, 0.0), mean);
        }
    }
    for _ in 0..100 {
        assert_eq!(particles::gaussian2(&mut rng, 0.0), (0.0, 0.0));
    }
}

fn mean_sd(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0);
    (m, var.sqrt())
}

#[test]
fn core_r30_seeded_sample_matches_parameters() {
    let mut rng = ChaCha8Rng::seed_from_u64(0x5eed);
    let (mu, sigma) = (10.0, 2.0);
    let xs: Vec<f64> = (0..20_000)
        .map(|_| particles::gaussian(&mut rng, mu, sigma))
        .collect();
    assert!(xs.iter().all(|x| x.is_finite()));
    let (m, s) = mean_sd(&xs);
    assert!((m - mu).abs() <= 0.03 * mu, "mean {m}");
    assert!((s - sigma).abs() <= 0.03 * sigma, "sd {s}");
}

#[test]
fn core_r30_gaussian2_is_isotropic_with_given_sd() {
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let sigma = 3.0;
    let (xs, ys): (Vec<f64>, Vec<f64>) = (0..20_000)
        .map(|_| particles::gaussian2(&mut rng, sigma))
        .unzip();
    for axis in [&xs, &ys] {
        let (m, s) = mean_sd(axis);
        // Mean is 0, so 3 % of sd is the meaningful tolerance for it.
        assert!(m.abs() <= 0.03 * sigma, "mean {m}");
        assert!((s - sigma).abs() <= 0.03 * sigma, "sd {s}");
    }
    // Components are uncorrelated.
    let cov = xs.iter().zip(&ys).map(|(x, y)| x * y).sum::<f64>() / xs.len() as f64;
    assert!(cov.abs() / (sigma * sigma) < 0.03, "corr {cov}");
}

#[test]
fn core_r30_same_seed_same_stream() {
    let mut a = ChaCha8Rng::seed_from_u64(3);
    let mut b = ChaCha8Rng::seed_from_u64(3);
    for _ in 0..50 {
        assert_eq!(
            particles::gaussian(&mut a, 0.0, 1.0).to_bits(),
            particles::gaussian(&mut b, 0.0, 1.0).to_bits()
        );
    }
}

#[test]
fn core_r30_degenerate_parameters_stay_finite() {
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    for &(mean, sd) in &[
        (f64::NAN, 1.0),
        (0.0, f64::NAN),
        (f64::INFINITY, 1.0),
        (0.0, f64::INFINITY),
        (f64::MAX, f64::MAX),
        (0.0, -2.0),
    ] {
        for _ in 0..100 {
            assert!(particles::gaussian(&mut rng, mean, sd).is_finite());
            let (x, y) = particles::gaussian2(&mut rng, sd);
            assert!(x.is_finite() && y.is_finite());
        }
    }
}

// ---------------------------------------------------------------- CORE-R31

#[test]
fn core_r31_kepler_values() {
    assert!((particles::kepler_speed(4.0, 1.0) - 2.0).abs() < 1e-12);
    assert!((particles::kepler_speed(8.0, 2.0) - 2.0).abs() < 1e-12);
    assert!((particles::kepler_omega(8.0, 2.0) - 1.0).abs() < 1e-12);
    assert!((particles::kepler_omega(1.0, 4.0) - 0.125).abs() < 1e-12);
    assert_eq!(particles::kepler_speed(0.0, 3.0), 0.0);
    // v = ω r for circular orbits.
    let (gm, r) = (3.7, 5.3);
    assert!((particles::kepler_speed(gm, r) - particles::kepler_omega(gm, r) * r).abs() < 1e-12);
}

#[test]
fn core_r31_kepler_degenerate_inputs_are_zero() {
    let bad = [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
    for &r in &bad {
        assert_eq!(particles::kepler_speed(1.0, r), 0.0, "r={r}");
        assert_eq!(particles::kepler_omega(1.0, r), 0.0, "r={r}");
    }
    for &gm in &[-1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(particles::kepler_speed(gm, 1.0), 0.0, "gm={gm}");
        assert_eq!(particles::kepler_omega(gm, 1.0), 0.0, "gm={gm}");
    }
    // Overflow/underflow extremes must not leak ∞.
    for &(gm, r) in &[(f64::MAX, f64::MIN_POSITIVE), (1.0, 1e-320), (1.0, 1e200)] {
        assert!(particles::kepler_speed(gm, r).is_finite());
        assert!(particles::kepler_omega(gm, r).is_finite());
    }
}

// ---------------------------------------------------------------- CORE-R32

#[test]
fn core_r32_plot_rounds_to_nearest_cell() {
    let mut c = Canvas::new(Size::new(10, 5), GlyphMode::Ascii);
    particles::plot(&mut c, 2.4, 1.6, G, INK);
    particles::plot(&mut c, 6.5, 3.49, G, INK);
    particles::plot(&mut c, -0.4, -0.4, G, INK); // rounds onto (0,0)
    let mut pts = drawn(&c);
    pts.sort_unstable();
    assert_eq!(pts, vec![(0, 0), (2, 2), (7, 3)]);
    assert_eq!(c.get(2, 2).map(|cell| cell.ink), Some(INK));
}

#[test]
fn core_r32_plot_ignores_non_finite_and_off_canvas() {
    let mut c = Canvas::new(Size::new(10, 5), GlyphMode::Ascii);
    for &(x, y) in &[
        (f64::NAN, 1.0),
        (1.0, f64::NAN),
        (f64::INFINITY, 1.0),
        (1.0, f64::NEG_INFINITY),
        (-0.6, 1.0),
        (1.0, -3.0),
        (9.5, 1.0),
        (1.0, 4.5),
        (1e300, -1e300),
        (f64::from(i32::MAX) * 4.0, 0.0),
    ] {
        particles::plot(&mut c, x, y, G, INK);
    }
    assert!(drawn(&c).is_empty());
}

// ---------------------------------------------------------------- properties

#[derive(Debug, Clone)]
enum Op {
    Spawn(u32),
    Retain(u32),
    SetCapacity(usize),
    Clear,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => any::<u32>().prop_map(Op::Spawn),
        2 => (1u32..5).prop_map(Op::Retain),
        1 => (0usize..40).prop_map(Op::SetCapacity),
        1 => Just(Op::Clear),
    ]
}

proptest! {
    /// CORE-R28 against a trivially-correct model: same multiset, len ≤ capacity,
    /// and `spawn` succeeds exactly when there is room.
    #[test]
    fn core_r28_pool_matches_model(cap in 0usize..40, ops in prop::collection::vec(op(), 0..200)) {
        let mut pool = ParticlePool::with_capacity(cap);
        let mut model: Vec<u32> = Vec::new();
        let mut model_cap = cap;
        for op in ops {
            match op {
                Op::Spawn(v) => {
                    let room = model.len() < model_cap;
                    prop_assert_eq!(pool.spawn(v), room);
                    if room { model.push(v); }
                }
                Op::Retain(k) => {
                    pool.retain(|p| *p % k != 0);
                    model.retain(|p| *p % k != 0);
                }
                Op::SetCapacity(n) => {
                    pool.set_capacity(n);
                    model_cap = n;
                    // Which particles survive a shrink is unspecified; resync the model.
                    if model.len() > n {
                        model = pool.iter().copied().collect();
                    }
                }
                Op::Clear => { pool.clear(); model.clear(); }
            }
            prop_assert!(pool.len() <= pool.capacity());
            prop_assert_eq!(pool.capacity(), model_cap);
            let mut m = model.clone();
            m.sort_unstable();
            prop_assert_eq!(sorted(&pool), m);
        }
    }

    /// CORE-R29: a trail always equals the last `min(n, pushes)` values in order.
    #[test]
    fn core_r29_trail_matches_suffix(n in 0usize..16, vals in prop::collection::vec(any::<i32>(), 0..64)) {
        let mut t = Trail::new(n);
        for (i, &v) in vals.iter().enumerate() {
            t.push(v);
            let seen = &vals[..=i];
            let expect = &seen[seen.len().saturating_sub(n)..];
            prop_assert_eq!(t.iter().copied().collect::<Vec<_>>(), expect.to_vec());
            prop_assert!(t.len() <= n);
        }
    }

    /// CORE-R32: plot never panics and draws at most one cell.
    #[test]
    fn core_r32_plot_never_panics(
        w in 1u16..50, h in 1u16..50,
        x in prop::num::f64::ANY, y in prop::num::f64::ANY,
    ) {
        let mut c = Canvas::new(Size::new(w, h), GlyphMode::Ascii);
        particles::plot(&mut c, x, y, G, INK);
        prop_assert!(drawn(&c).len() <= 1);
    }
}
