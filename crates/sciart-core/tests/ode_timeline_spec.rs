//! SPEC-001 CORE-R37..R39, R44: RK4 and leapfrog integrators, looping timeline, easing curves.

use std::f64::consts::TAU;

use proptest::prelude::*;
use sciart_core::ease;
use sciart_core::ode::{self, MAX_STEPS};
use sciart_core::{Beat, Timeline};

// ---------------------------------------------------------------- CORE-R37

/// Global error of `y' = -y`, `y(0) = 1` integrated to `t = 1` with fixed step `h`.
fn decay_error(h: f64) -> f64 {
    let mut y = [1.0];
    let n = (1.0 / h).round() as usize;
    for i in 0..n {
        ode::rk4_step(&mut y, i as f64 * h, h, |_, y| [-y[0]]);
    }
    (y[0] - (-1.0f64).exp()).abs()
}

/// Harmonic oscillator `x'' = -x` over one period with step `h`:
/// returns (position error, energy error) against the exact solution.
fn oscillator_errors(h_max: f64) -> (f64, f64) {
    let mut y = [1.0, 0.0];
    ode::integrate(&mut y, 0.0, TAU, h_max, |_, s| [s[1], -s[0]]);
    let pos = ((y[0] - 1.0).powi(2) + y[1].powi(2)).sqrt();
    let energy = (0.5 * (y[0] * y[0] + y[1] * y[1]) - 0.5).abs();
    (pos, energy)
}

#[test]
fn core_r37_rk4_exact_for_solutions_up_to_degree_4() {
    // y' = f(t) reduces RK4 to Simpson's rule, which is exact for cubic f, i.e. for
    // solutions y(t) = t^k with k <= 4. Start off-origin and use a coarse step.
    for k in 0..=4i32 {
        let kf = f64::from(k);
        let (t0, h): (f64, f64) = (0.7, 0.9);
        let mut y = [t0.powi(k)];
        ode::rk4_step(&mut y, t0, h, |t, _| [kf * t.powi((k - 1).max(0))]);
        let exact = (t0 + h).powi(k);
        assert!(
            (y[0] - exact).abs() <= 1e-12 * exact.abs().max(1.0),
            "k={k}: {} vs {exact}",
            y[0]
        );
    }
    // Negative control: a quintic solution is not integrated exactly.
    let mut y = [0.0];
    ode::rk4_step(&mut y, 0.0, 1.0, |t, _| [5.0 * t.powi(4)]);
    assert!((y[0] - 1.0).abs() > 1e-3);
}

#[test]
fn core_r37_order4_convergence_on_decay() {
    let (e1, e2, e3) = (decay_error(0.1), decay_error(0.05), decay_error(0.025));
    assert!(e1 > 0.0 && e1 < 1e-5, "e1={e1}");
    for (a, b) in [(e1, e2), (e2, e3)] {
        let ratio = a / b;
        // Spec floor is 12x; a 4th-order method gives ~16x.
        assert!((12.0..20.0).contains(&ratio), "ratio={ratio}");
    }
}

#[test]
fn core_r37_order4_convergence_on_oscillator() {
    let (p1, en1) = oscillator_errors(0.2);
    let (p2, en2) = oscillator_errors(0.1);
    let (p3, en3) = oscillator_errors(0.05);
    for (a, b) in [(p1, p2), (p2, p3)] {
        let r = a / b;
        assert!((12.0..20.0).contains(&r), "position ratio={r}");
    }
    // RK4 damps the oscillator by ~h^6/72 per step, so energy error over a fixed span
    // shrinks at least as fast as h^4 (it is actually ~h^5).
    for (a, b) in [(en1, en2), (en2, en3)] {
        assert!(a / b >= 12.0, "energy ratio={}", a / b);
    }
    assert!(en3 < 1e-6, "en3={en3}");
}

#[test]
fn core_r37_integrate_uses_ceil_equal_steps_and_absolute_time() {
    let mut calls = 0usize;
    let mut last_t = f64::NAN;
    let mut y = [0.0];
    let steps = ode::integrate(&mut y, 1.0, 1.0, 0.3, |t, _| {
        calls += 1;
        last_t = t;
        [t.cos()]
    });
    assert_eq!(steps, 4, "ceil(1.0 / 0.3) = 4");
    assert_eq!(calls, 4 * 4, "four evaluations per RK4 step");
    assert!(
        (last_t - 2.0).abs() < 1e-12,
        "last stage evaluates at t + dt"
    );
    assert!((y[0] - (2.0f64.sin() - 1.0f64.sin())).abs() < 1e-5);

    // Exact multiples must not gain a spurious extra step from rounding.
    assert_eq!(ode::integrate(&mut [0.0], 0.0, 0.9, 0.3, |_, _| [0.0]), 3);
    let sim_dt = 1.0 / 60.0;
    assert_eq!(
        ode::integrate(&mut [0.0], 0.0, sim_dt, sim_dt / 4.0, |_, _| [0.0]),
        4
    );
    // dt shorter than h_max: exactly one step.
    assert_eq!(ode::integrate(&mut [0.0], 0.0, 0.01, 1.0, |_, _| [0.0]), 1);
}

#[test]
fn core_r37_integrate_matches_manual_steps() {
    let f = |_: f64, s: &[f64; 2]| [s[1], -s[0]];
    let mut a = [1.0, 0.5];
    let mut b = a;
    let n = ode::integrate(&mut a, 0.0, 1.0, 0.25, f);
    assert_eq!(n, 4);
    for i in 0..4 {
        ode::rk4_step(&mut b, f64::from(i) * 0.25, 0.25, f);
    }
    assert_eq!(a, b);
}

#[test]
fn core_r37_integrate_does_nothing_for_non_positive_or_non_finite_dt() {
    for dt in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut y = [3.0, 4.0];
        let mut called = false;
        let n = ode::integrate(&mut y, 0.0, dt, 0.1, |_, s| {
            called = true;
            *s
        });
        assert_eq!(n, 0, "dt={dt}");
        assert!(!called, "dt={dt}");
        assert_eq!(y, [3.0, 4.0]);
    }
}

#[test]
fn core_r37_degenerate_h_max_is_clamped_and_bounded() {
    for h_max in [
        0.0,
        -1.0,
        1e-300,
        f64::MIN_POSITIVE,
        f64::NAN,
        f64::NEG_INFINITY,
    ] {
        let mut y = [1.0];
        let n = ode::integrate(&mut y, 0.0, 0.25, h_max, |_, s| [-s[0]]);
        assert!((1..=MAX_STEPS).contains(&n), "h_max={h_max}: n={n}");
        assert!((y[0] - (-0.25f64).exp()).abs() < 1e-12, "h_max={h_max}");
    }
    // An infinite h_max means "no limit": one step.
    assert_eq!(
        ode::integrate(&mut [1.0], 0.0, 0.25, f64::INFINITY, |_, s| [-s[0]]),
        1
    );
}

#[test]
fn core_r37_non_finite_results_are_left_to_the_caller() {
    let mut y = [1.0, 2.0];
    ode::rk4_step(&mut y, 0.0, 0.1, |_, _| [f64::NAN, 0.0]);
    assert!(y[0].is_nan());
    assert_eq!(y[1], 2.0);
    let mut y = [1.0];
    ode::integrate(&mut y, 0.0, 1.0, 0.1, |_, s| [s[0] * 1e300]);
    assert!(!y[0].is_finite());
}

#[test]
fn core_r37_zero_dimensional_state_is_fine() {
    let mut y: [f64; 0] = [];
    ode::rk4_step(&mut y, 0.0, 0.1, |_, _| []);
    assert_eq!(ode::integrate(&mut y, 0.0, 1.0, 0.5, |_, _| []), 2);
}

// ---------------------------------------------------------------- CORE-R44

/// Harmonic oscillator `x'' = −x` in any number of independent dimensions.
fn spring(pos: &[f64], acc: &mut [f64]) {
    for (a, x) in acc.iter_mut().zip(pos) {
        *a = -x;
    }
}

/// Nonlinear pendulum `θ'' = −sin θ`, to check reversibility beyond linear forces.
fn pendulum(pos: &[f64], acc: &mut [f64]) {
    for (a, x) in acc.iter_mut().zip(pos) {
        *a = -x.sin();
    }
}

#[test]
fn core_r44_exact_for_constant_acceleration() {
    let g = [-9.81, 0.5];
    let (x0, v0) = ([0.0, 1.0], [2.0, -1.0]);
    let (mut pos, mut vel, mut acc) = (x0, v0, g);
    let t = 1.3;
    let n = ode::verlet_integrate(&mut pos, &mut vel, &mut acc, t, 0.1, |_, a| {
        a.copy_from_slice(&g);
    });
    assert_eq!(n, 13);
    for i in 0..2 {
        let x = x0[i] + v0[i] * t + 0.5 * g[i] * t * t;
        assert!((pos[i] - x).abs() < 1e-12, "x[{i}] = {} vs {x}", pos[i]);
        assert!((vel[i] - (v0[i] + g[i] * t)).abs() < 1e-12);
    }
}

#[test]
fn core_r44_step_is_kick_drift_kick_and_leaves_acc_at_the_new_position() {
    let (mut pos, mut vel, mut acc) = ([1.0], [0.5], [-1.0]);
    let h = 0.1;
    let mut calls = 0;
    ode::verlet_step(&mut pos, &mut vel, &mut acc, h, |p, a| {
        calls += 1;
        spring(p, a);
    });
    let v_half = 0.5 - 0.5 * h; // a(1) = -1
    let x1 = 1.0 + h * v_half;
    assert_eq!(calls, 1, "one force evaluation per step");
    assert!((pos[0] - x1).abs() < 1e-15);
    assert!((acc[0] + x1).abs() < 1e-15, "acc holds a(pos') on exit");
    assert!((vel[0] - (v_half + 0.5 * h * -x1)).abs() < 1e-15);
}

#[test]
fn core_r44_time_reversible() {
    for (name, force) in [
        ("spring", spring as fn(&[f64], &mut [f64])),
        ("pendulum", pendulum),
    ] {
        let start = ([2.5, -0.3, 1.0], [0.1, 1.7, -0.4]);
        let (mut pos, mut vel) = start;
        let mut acc = [0.0; 3];
        force(&pos, &mut acc);
        // One step, and a long run, forward then back.
        for steps in [1, 500] {
            for _ in 0..steps {
                ode::verlet_step(&mut pos, &mut vel, &mut acc, 0.05, force);
            }
            vel.iter_mut().for_each(|v| *v = -*v);
            for _ in 0..steps {
                ode::verlet_step(&mut pos, &mut vel, &mut acc, 0.05, force);
            }
            vel.iter_mut().for_each(|v| *v = -*v);
            let tol = if steps == 1 { 1e-12 } else { 1e-9 };
            for i in 0..3 {
                let (x, v) = (start.0[i], start.1[i]);
                assert!(
                    (pos[i] - x).abs() <= tol * x.abs().max(1.0),
                    "{name}/{steps}: x"
                );
                assert!(
                    (vel[i] - v).abs() <= tol * v.abs().max(1.0),
                    "{name}/{steps}: v"
                );
            }
        }
    }
}

#[test]
fn core_r44_harmonic_energy_error_is_bounded_without_secular_drift() {
    // 10⁴ periods at h = T/50: leapfrog's energy error oscillates with amplitude
    // ~(ωh)²/8 ≈ 2e-3 relative, but does not grow (symplectic).
    let h = TAU / 50.0;
    let (mut pos, mut vel, mut acc) = ([1.0], [0.0], [-1.0]);
    let energy = |x: f64, v: f64| 0.5 * (x * x + v * v);
    let e0 = energy(1.0, 0.0);
    let mut worst_first = 0.0f64;
    let mut worst_last = 0.0f64;
    let periods = 10_000;
    for p in 0..periods {
        for _ in 0..50 {
            ode::verlet_step(&mut pos, &mut vel, &mut acc, h, spring);
            let err = (energy(pos[0], vel[0]) - e0).abs() / e0;
            if p < 10 {
                worst_first = worst_first.max(err);
            } else if p >= periods - 10 {
                worst_last = worst_last.max(err);
            }
        }
    }
    assert!(worst_first < 5e-3, "{worst_first}");
    assert!(
        worst_last <= worst_first * 1.05,
        "drift: {worst_first} -> {worst_last}"
    );
    // The phase drifts (leapfrog's frequency error is O(h²)), the amplitude does not.
    let amplitude = pos[0].hypot(vel[0]);
    assert!((amplitude - 1.0).abs() < 5e-3, "amplitude {amplitude}");
}

#[test]
fn core_r44_mismatched_lengths_are_a_no_op() {
    let (mut pos, mut vel, mut acc) = ([1.0, 2.0], [3.0, 4.0, 5.0], [0.0, 0.0]);
    let mut called = false;
    ode::verlet_step(&mut pos, &mut vel, &mut acc, 0.1, |_, _| called = true);
    let n = ode::verlet_integrate(&mut pos, &mut vel, &mut acc, 1.0, 0.1, |_, _| called = true);
    assert_eq!(n, 0);
    assert!(!called);
    assert_eq!((pos, vel), ([1.0, 2.0], [3.0, 4.0, 5.0]));

    let (mut pos, mut vel, mut acc) = ([1.0], [3.0], [0.0, 0.0]);
    ode::verlet_step(&mut pos, &mut vel, &mut acc, 0.1, |_, _| called = true);
    assert!(!called && pos == [1.0] && vel == [3.0]);
}

#[test]
fn core_r44_integrate_follows_the_integrate_step_count_rules() {
    let run = |dt: f64, h_max: f64| {
        let (mut pos, mut vel, mut acc) = ([1.0], [0.0], [-1.0]);
        let mut calls = 0usize;
        let n = ode::verlet_integrate(&mut pos, &mut vel, &mut acc, dt, h_max, |p, a| {
            calls += 1;
            spring(p, a);
        });
        assert_eq!(n, calls, "one force evaluation per substep");
        n
    };
    for dt in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(run(dt, 0.1), 0, "dt={dt}");
    }
    assert_eq!(
        run(0.9, 0.3),
        3,
        "exact multiples take no spurious extra step"
    );
    assert_eq!(run(1.0, 0.3), 4);
    assert_eq!(run(0.01, 1.0), 1);
    assert_eq!(run(0.25, f64::INFINITY), 1);
    for h_max in [0.0, -1.0, f64::NAN, 1e-300] {
        assert_eq!(run(0.25, h_max), MAX_STEPS, "h_max={h_max}");
    }
    // The substeps are equal and cover dt exactly: same result as manual steps.
    let (mut a, mut va, mut aa) = ([1.0], [0.0], [-1.0]);
    let (mut b, mut vb, mut ab) = (a, va, aa);
    ode::verlet_integrate(&mut a, &mut va, &mut aa, 1.0, 0.25, spring);
    for _ in 0..4 {
        ode::verlet_step(&mut b, &mut vb, &mut ab, 0.25, spring);
    }
    assert_eq!((a, va, aa), (b, vb, ab));
}

// ---------------------------------------------------------------- CORE-R38

const PHASES: &[(&str, f64)] = &[("a", 1.0), ("b", 2.0), ("c", 0.5)];

fn beat(tl: &Timeline, t: f64) -> (usize, &'static str, f64, f64) {
    let Beat {
        index,
        label,
        local,
        progress,
    } = tl.at(t);
    (index, label, local, progress)
}

#[test]
fn core_r38_phase_starts_are_exact() {
    let tl = Timeline::new(PHASES);
    assert_eq!(tl.total(), 3.5);
    assert_eq!(beat(&tl, 0.0), (0, "a", 0.0, 0.0));
    assert_eq!(beat(&tl, 0.5), (0, "a", 0.5, 0.5));
    assert_eq!(beat(&tl, 1.0), (1, "b", 0.0, 0.0));
    assert_eq!(beat(&tl, 2.0), (1, "b", 1.0, 0.5));
    assert_eq!(beat(&tl, 3.0), (2, "c", 0.0, 0.0));
    assert_eq!(beat(&tl, 3.25), (2, "c", 0.25, 0.5));
}

#[test]
fn core_r38_wraps_at_total() {
    let tl = Timeline::new(PHASES);
    assert_eq!(beat(&tl, 3.5), (0, "a", 0.0, 0.0));
    assert_eq!(beat(&tl, 7.0 + 1.0), (1, "b", 0.0, 0.0));
    let b = tl.at(3.5 - 1e-9);
    assert_eq!((b.index, b.label), (2, "c"));
    assert!(b.progress < 1.0 && b.progress > 0.99);
}

#[test]
fn core_r38_negative_time_maps_into_the_cycle() {
    let tl = Timeline::new(PHASES);
    assert_eq!(beat(&tl, -0.25), (2, "c", 0.25, 0.5));
    assert_eq!(beat(&tl, -3.5), (0, "a", 0.0, 0.0));
    assert_eq!(tl.cycle(-0.25), -1);
    assert_eq!(tl.cycle(-3.5), -1);
    assert_eq!(tl.cycle(-3.6), -2);
    // Tiny negatives round `rem_euclid` up to `total`; must still land in [0, 1).
    for t in [-1e-300, -f64::MIN_POSITIVE, -1e-17] {
        let b = tl.at(t);
        assert!((0.0..1.0).contains(&b.progress), "t={t}: {b:?}");
        assert!(b.local >= 0.0);
    }
}

#[test]
fn core_r38_huge_time_stays_in_range() {
    let tl = Timeline::new(PHASES);
    for t in [1e15 + 0.5, 1e300, -1e300, f64::MAX, f64::MIN] {
        let b = tl.at(t);
        assert!(b.index < PHASES.len(), "t={t}");
        assert!((0.0..1.0).contains(&b.progress), "t={t}: {b:?}");
        assert!(
            b.local >= 0.0 && b.local < PHASES[b.index].1,
            "t={t}: {b:?}"
        );
    }
}

#[test]
fn core_r38_non_finite_time_is_phase_zero() {
    let tl = Timeline::new(PHASES);
    for t in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(beat(&tl, t), (0, "a", 0.0, 0.0), "t={t}");
        assert_eq!(tl.cycle(t), 0, "t={t}");
    }
}

#[test]
fn core_r38_zero_length_phases_are_skipped_but_keep_their_index() {
    let tl = Timeline::new(&[
        ("zero-first", 0.0),
        ("a", 1.0),
        ("zero", 0.0),
        ("neg", -3.0),
        ("nan", f64::NAN),
        ("inf", f64::INFINITY),
        ("b", 1.0),
    ]);
    assert_eq!(tl.total(), 2.0);
    assert_eq!(beat(&tl, 0.0), (1, "a", 0.0, 0.0));
    assert_eq!(beat(&tl, 1.0), (6, "b", 0.0, 0.0));
    assert_eq!(beat(&tl, 1.5), (6, "b", 0.5, 0.5));
}

#[test]
fn core_r38_empty_and_all_zero_never_panic() {
    let empty = Timeline::new(&[]);
    assert_eq!(empty.total(), 0.0);
    assert!(empty.is_empty());
    let zero = Timeline::new(&[("x", 0.0), ("y", -1.0)]);
    assert_eq!(zero.total(), 0.0);
    assert!(zero.is_empty());
    for tl in [&empty, &zero] {
        for t in [0.0, 1.0, -1.0, f64::NAN, f64::INFINITY] {
            let b = tl.at(t);
            assert_eq!((b.index, b.local, b.progress), (0, 0.0, 0.0));
            assert_eq!(tl.cycle(t), 0);
        }
    }
    assert_eq!(empty.at(1.0).label, "");
    assert_eq!(zero.at(1.0).label, "x", "all-zero reports the first phase");
}

#[test]
fn core_r38_cycle_counts_completed_loops() {
    let tl = Timeline::new(PHASES);
    assert_eq!(tl.cycle(0.0), 0);
    assert_eq!(tl.cycle(3.49), 0);
    assert_eq!(tl.cycle(3.5), 1);
    assert_eq!(tl.cycle(7.1), 2);
    assert_eq!(tl.cycle(1e300), i64::MAX, "saturates");
}

#[test]
fn core_r38_single_phase_progress_is_fraction_of_total() {
    let tl = Timeline::new(&[("only", 4.0)]);
    assert_eq!(beat(&tl, 5.0), (0, "only", 1.0, 0.25));
}

proptest! {
    /// CORE-R38: any finite t maps to a valid beat, consistent with `cycle`.
    #[test]
    fn core_r38_any_finite_t_gives_valid_beat(
        durs in prop::collection::vec(-1.0f64..5.0, 0..8),
        t in -1e6f64..1e6,
    ) {
        let phases: Vec<(&'static str, f64)> = durs.iter().map(|&d| ("p", d)).collect();
        let tl = Timeline::new(&phases);
        let b = tl.at(t);
        prop_assert!((0.0..1.0).contains(&b.progress), "{:?}", b);
        prop_assert!(b.local >= 0.0);
        if tl.is_empty() {
            prop_assert_eq!(b.index, 0);
        } else {
            let d = phases[b.index].1;
            prop_assert!(d > 0.0, "landed on a skipped phase: {:?}", b);
            prop_assert!(b.local < d);
            let start: f64 = phases[..b.index].iter().map(|p| p.1.max(0.0)).sum();
            let rebuilt = tl.cycle(t) as f64 * tl.total() + start + b.local;
            prop_assert!((rebuilt - t).abs() <= 1e-6 * t.abs().max(1.0), "{} vs {}", rebuilt, t);
        }
    }

    /// CORE-R38: arbitrary bit patterns never panic.
    #[test]
    fn core_r38_any_f64_never_panics(t in any::<f64>(), d in any::<f64>()) {
        let tl = Timeline::new(&[("a", d), ("b", 1.0)]);
        let b = tl.at(t);
        prop_assert!((0.0..1.0).contains(&b.progress));
        let _ = tl.cycle(t);
    }
}

// ---------------------------------------------------------------- CORE-R39

type Curve = fn(f64) -> f64;

const CURVES: [(&str, Curve); 3] = [
    ("smoothstep", ease::smoothstep),
    ("in_out_cubic", ease::in_out_cubic),
    ("out_quad", ease::out_quad),
];

#[test]
fn core_r39_endpoints_and_midpoints() {
    for (name, f) in CURVES {
        assert_eq!(f(0.0), 0.0, "{name}");
        assert_eq!(f(1.0), 1.0, "{name}");
    }
    assert_eq!(ease::smoothstep(0.5), 0.5);
    assert_eq!(ease::in_out_cubic(0.5), 0.5);
    assert_eq!(ease::out_quad(0.5), 0.75);
}

#[test]
fn core_r39_monotone_on_a_fine_grid() {
    for (name, f) in CURVES {
        let mut prev = f(0.0);
        for i in 1..=10_000 {
            let y = f(f64::from(i) / 10_000.0);
            assert!(y >= prev, "{name} decreases at i={i}");
            assert!((0.0..=1.0).contains(&y), "{name}({i}) = {y}");
            prev = y;
        }
    }
}

#[test]
fn core_r39_clamps_out_of_range_and_nan() {
    for (name, f) in CURVES {
        assert_eq!(f(-0.5), 0.0, "{name}");
        assert_eq!(f(f64::NEG_INFINITY), 0.0, "{name}");
        assert_eq!(f(1.5), 1.0, "{name}");
        assert_eq!(f(f64::INFINITY), 1.0, "{name}");
        assert_eq!(f(f64::NAN), 0.0, "{name}");
    }
}

#[test]
fn core_r39_lerp_endpoints_and_extrapolation() {
    assert_eq!(ease::lerp(2.0, 10.0, 0.0), 2.0);
    assert_eq!(ease::lerp(2.0, 10.0, 1.0), 10.0);
    assert_eq!(ease::lerp(2.0, 10.0, 0.5), 6.0);
    assert_eq!(ease::lerp(0.1, 0.7, 1.0), 0.7, "exact at t = 1");
    assert_eq!(ease::lerp(0.0, 10.0, 1.5), 15.0, "t is not clamped");
    assert_eq!(ease::lerp(5.0, 5.0, 0.3), 5.0);
}

proptest! {
    /// CORE-R39: every curve maps any input into [0, 1] and preserves order.
    #[test]
    fn core_r39_curves_bounded_and_monotone(a in any::<f64>(), b in any::<f64>()) {
        for (name, f) in CURVES {
            let (fa, fb) = (f(a), f(b));
            prop_assert!((0.0..=1.0).contains(&fa), "{}({}) = {}", name, a, fa);
            if a <= b {
                prop_assert!(fa <= fb, "{} not monotone: f({})={} > f({})={}", name, a, fa, b, fb);
            }
        }
    }
}
