//! SPEC-001 M13: the lattice Boltzmann kernel (CORE-R98, CORE-R99) and the complex
//! tridiagonal solvers (CORE-R100).

use proptest::prelude::*;
use sciart_core::Size;
use sciart_core::lbm::{self, CX, CY, Lattice, W};
use sciart_core::ode::{C64, tridiag_apply_c, tridiag_factor_c, tridiag_solve_c};

const TAU: f64 = 0.53;

fn omegas(tau: f64) -> (f32, f32) {
    ((1.0 / tau) as f32, (1.0 / lbm::tau_minus(tau, 0.25)) as f32)
}

/// `(Σh, Σh·cx, Σh·cy)` in f64.
fn moments(h: &[f32; 9]) -> (f64, f64, f64) {
    let mut m = (0.0, 0.0, 0.0);
    for q in 0..9 {
        let v = f64::from(h[q]);
        m.0 += v;
        m.1 += v * f64::from(CX[q]);
        m.2 += v * f64::from(CY[q]);
    }
    m
}

#[test]
fn core_r98_weights_velocities_and_opposites_are_consistent() {
    let sum: f64 = W.iter().map(|&w| f64::from(w)).sum();
    assert!((sum - 1.0).abs() < 1e-7);
    for q in 0..9 {
        let o = lbm::OPP[q];
        assert_eq!((CX[o], CY[o]), (-CX[q], -CY[q]));
        assert_eq!(W[o], W[q]);
    }
    assert!((lbm::tau_minus(0.53, 0.25) - (0.5 + 0.25 / 0.03)).abs() < 1e-9);
}

proptest! {
    #[test]
    fn core_r98_equilibrium_has_the_stated_moments(
        drho in -0.1f32..0.1, ux in -0.15f32..0.15, uy in -0.15f32..0.15,
    ) {
        let eq = lbm::heq(drho, ux, uy);
        let (m0, jx, jy) = moments(&eq);
        let rho = 1.0 + f64::from(drho);
        prop_assert!((m0 - f64::from(drho)).abs() < 1e-6);
        prop_assert!((jx - rho * f64::from(ux)).abs() < 1e-6);
        prop_assert!((jy - rho * f64::from(uy)).abs() < 1e-6);
    }

    #[test]
    fn core_r98_collision_conserves_mass_and_momentum(
        h in prop::array::uniform9(-0.02f32..0.02), tau in 0.51f64..1.5,
    ) {
        let (wp, wm) = omegas(tau);
        let mut post = h;
        let (rho, ux, uy) = lbm::collide(&mut post, wp, wm);
        let (a, b) = (moments(&h), moments(&post));
        prop_assert!((a.0 - b.0).abs() < 1e-6, "mass {a:?} → {b:?}");
        prop_assert!((a.1 - b.1).abs() < 1e-6 && (a.2 - b.2).abs() < 1e-6);
        prop_assert!((f64::from(rho) - (1.0 + a.0)).abs() < 1e-6);
        prop_assert!((f64::from(ux) * f64::from(rho) - a.1).abs() < 1e-6);
        prop_assert!((f64::from(uy) * f64::from(rho) - a.2).abs() < 1e-6);
    }
}

#[test]
fn core_r98_equilibrium_is_a_fixed_point_of_the_collision() {
    let (wp, wm) = omegas(TAU);
    let eq = lbm::heq(0.02, 0.05, -0.03);
    let mut h = eq;
    lbm::collide(&mut h, wp, wm);
    for q in 0..9 {
        assert!((h[q] - eq[q]).abs() < 1e-7, "{q}: {} vs {}", h[q], eq[q]);
    }
}

fn shear_lattice(n: u16, amp: f32) -> Lattice {
    let mut lat = Lattice::new(Size::new(n, n), |_, _| false);
    let k = std::f64::consts::TAU / f64::from(n);
    let (_, ux, _) = lat.moments_mut();
    for y in 0..n {
        let v = amp * sciart_core::trig::sin(k * f64::from(y)) as f32;
        for x in 0..n {
            ux.set(i32::from(x), i32::from(y), v);
        }
    }
    lat.equilibrate();
    lat
}

/// Amplitude of the sin(ky) mode of uₓ.
fn shear_amplitude(lat: &Lattice) -> f64 {
    let n = lat.height();
    let k = std::f64::consts::TAU / n as f64;
    let ux = lat.ux().as_slice();
    let mut s = 0.0;
    for y in 0..n {
        let row_mean: f64 = (0..lat.width())
            .map(|x| f64::from(ux[y * lat.width() + x]))
            .sum::<f64>()
            / lat.width() as f64;
        s += row_mean * sciart_core::trig::sin(k * y as f64);
    }
    2.0 * s / n as f64
}

#[test]
fn core_r99_periodic_step_conserves_mass_to_rounding() {
    let (wp, wm) = omegas(TAU);
    let mut lat = shear_lattice(64, 0.08);
    let m0 = lat.mass();
    for _ in 0..200 {
        let before = lat.mass();
        lat.step_periodic(wp, wm);
        let rel = ((lat.mass() - before) / before).abs();
        assert!(rel < 1e-8, "{rel}");
    }
    assert!(((lat.mass() - m0) / m0).abs() < 1e-6);
}

#[test]
fn core_r99_uniform_flow_stays_uniform_and_momentum_is_kept() {
    let (wp, wm) = omegas(TAU);
    let mut lat = Lattice::new(Size::new(24, 16), |_, _| false);
    {
        let (_, ux, uy) = lat.moments_mut();
        ux.fill(0.1);
        uy.fill(-0.04);
    }
    lat.equilibrate();
    for _ in 0..50 {
        lat.step_periodic(wp, wm);
    }
    for (&u, &v) in lat.ux().as_slice().iter().zip(lat.uy().as_slice()) {
        assert!((u - 0.1).abs() < 1e-5 && (v + 0.04).abs() < 1e-5, "{u} {v}");
    }
    assert!(lat.healthy((0.99, 1.01)));
}

#[test]
fn core_r99_shear_wave_decays_at_the_trt_viscosity() {
    for tau in [0.53, 0.8] {
        let (wp, wm) = omegas(tau);
        let mut lat = shear_lattice(64, 0.01);
        // One step to fill the moments from the populations.
        lat.step_periodic(wp, wm);
        let a0 = shear_amplitude(&lat);
        let steps = 400;
        for _ in 0..steps {
            lat.step_periodic(wp, wm);
        }
        let a1 = shear_amplitude(&lat);
        let k = std::f64::consts::TAU / 64.0;
        let nu = -(a1 / a0).ln() / (k * k * f64::from(steps));
        let expect = (tau - 0.5) / 3.0;
        assert!(
            (nu / expect - 1.0).abs() < 0.02,
            "τ {tau}: ν {nu} vs {expect}"
        );
    }
}

#[test]
fn core_r99_bounce_back_keeps_mass_and_solids_at_rest() {
    let (wp, wm) = omegas(TAU);
    let solid = |x: usize, y: usize| (10..14).contains(&x) && (6..10).contains(&y);
    let mut lat = Lattice::new(Size::new(32, 16), solid);
    {
        let (_, ux, _) = lat.moments_mut();
        ux.fill(0.08);
    }
    lat.equilibrate();
    let m0 = lat.mass();
    for _ in 0..300 {
        lat.step_periodic(wp, wm);
    }
    assert!(
        ((lat.mass() - m0) / m0).abs() < 1e-6,
        "{m0} → {}",
        lat.mass()
    );
    for (i, &s) in lat.solid().iter().enumerate() {
        if s {
            assert_eq!(lat.ux().as_slice()[i], 0.0);
        }
    }
    assert!(lat.healthy((0.9, 1.1)));
}

#[test]
fn core_r99_without_wrap_the_border_columns_are_left_to_the_caller() {
    let (wp, wm) = omegas(TAU);
    let mut lat = Lattice::new(Size::new(8, 5), |_, _| false);
    lat.spare_mut().fill(7.0);
    lat.stream_collide(wp, wm, false);
    let (next, ..) = lat.next_and_moments();
    for plane in lbm::planes_mut(next, 40) {
        for row in plane.chunks_exact(8) {
            assert_eq!((row[0], row[7]), (7.0, 7.0));
            assert!(row[1..7].iter().all(|&v| v != 7.0));
        }
    }
}

#[test]
fn core_r99_degenerate_lattices_do_not_panic() {
    let (wp, wm) = omegas(TAU);
    for (w, h) in [(1, 1), (1, 5), (5, 1), (2, 2)] {
        let mut lat = Lattice::new(Size::new(w, h), |_, _| false);
        lat.step_periodic(wp, wm);
        lat.stream_collide(wp, wm, false);
        lat.swap();
        assert!(lat.healthy((0.99, 1.01)));
    }
}

// ---- CORE-R100 ------------------------------------------------------------------------

fn c(re: f64, im: f64) -> C64 {
    C64::new(re, im)
}

/// A diagonally dominant complex system of size n from a seed-like parameter.
fn system(n: usize, s: f64) -> (Vec<C64>, Vec<C64>, Vec<C64>, Vec<C64>) {
    let f = |i: usize, k: f64| sciart_core::trig::sin(s * (i as f64 + 1.0) * k);
    let sub: Vec<C64> = (0..n).map(|i| c(f(i, 1.3), f(i, 2.1))).collect();
    let sup: Vec<C64> = (0..n).map(|i| c(f(i, 0.7), f(i, 3.3))).collect();
    let diag: Vec<C64> = (0..n)
        .map(|i| c(4.0 + f(i, 0.4), f(i, 1.9) * 2.0))
        .collect();
    let rhs: Vec<C64> = (0..n).map(|i| c(f(i, 5.1), f(i, 0.2))).collect();
    (sub, diag, sup, rhs)
}

fn residual(sub: &[C64], diag: &[C64], sup: &[C64], x: &[C64], rhs: &[C64]) -> f64 {
    let n = x.len();
    (0..n)
        .map(|i| {
            let mut v = diag[i] * x[i];
            if i > 0 {
                v = v + sub[i] * x[i - 1];
            }
            if i + 1 < n {
                v = v + sup[i] * x[i + 1];
            }
            (v - rhs[i]).norm_sqr().sqrt()
        })
        .fold(0.0, f64::max)
}

#[test]
fn core_r100_complex_arithmetic_is_the_textbook_formulas() {
    let (a, b) = (c(1.0, 2.0), c(-3.0, 0.5));
    assert_eq!(a * b, c(-3.0 - 1.0, 0.5 - 6.0));
    let q = a / b;
    let back = q * b;
    assert!((back - a).norm_sqr() < 1e-24);
    assert_eq!(a.conj(), c(1.0, -2.0));
    assert_eq!(a.norm_sqr(), 5.0);
    assert_eq!(-a + a, C64::ZERO);
    assert_eq!(C64::I * C64::I, c(-1.0, 0.0));
}

proptest! {
    #[test]
    fn core_r100_complex_thomas_solves_and_factored_form_agrees(n in 1usize..64, s in 0.1f64..9.0) {
        let (sub, diag, sup, rhs) = system(n, s);
        let mut x = rhs.clone();
        let mut scratch = vec![C64::ZERO; n];
        prop_assert!(tridiag_solve_c(&sub, &diag, &sup, &mut x, &mut scratch));
        prop_assert!(residual(&sub, &diag, &sup, &x, &rhs) < 1e-12);

        let (mut inv, mut cp) = (vec![C64::ZERO; n], vec![C64::ZERO; n]);
        prop_assert!(tridiag_factor_c(&sub, &diag, &sup, &mut inv, &mut cp));
        let mut y = rhs.clone();
        tridiag_apply_c(&sub, &inv, &cp, &mut y);
        for (a, b) in x.iter().zip(&y) {
            prop_assert!((*a - *b).norm_sqr().sqrt() <= 1e-12 * (1.0 + a.norm_sqr().sqrt()));
        }
    }
}

#[test]
fn core_r100_bad_input_returns_false_and_leaves_outputs_untouched() {
    let (sub, mut diag, sup, rhs) = system(5, 1.7);
    let mut x = rhs.clone();
    let mut short = vec![C64::ZERO; 4];
    assert!(!tridiag_solve_c(&sub, &diag, &sup, &mut x, &mut short));
    assert_eq!(x, rhs);
    diag[0] = C64::ZERO;
    let mut scratch = vec![C64::ZERO; 5];
    assert!(!tridiag_solve_c(&sub, &diag, &sup, &mut x, &mut scratch));
    assert_eq!(x, rhs);
    let (mut inv, mut cp) = (vec![c(9.0, 9.0); 5], vec![c(9.0, 9.0); 5]);
    assert!(!tridiag_factor_c(&sub, &diag, &sup, &mut inv, &mut cp));
    assert!(inv.iter().chain(&cp).all(|&v| v == c(9.0, 9.0)));
    diag[0] = c(f64::NAN, 0.0);
    assert!(!tridiag_factor_c(&sub, &diag, &sup, &mut inv, &mut cp));
    let mut y = rhs.clone();
    tridiag_apply_c(&sub, &inv[..4], &cp, &mut y);
    assert_eq!(y, rhs);
    let mut empty: [C64; 0] = [];
    tridiag_apply_c(&[], &[], &[], &mut empty);
}
