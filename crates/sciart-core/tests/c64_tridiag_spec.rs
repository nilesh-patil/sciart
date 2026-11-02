//! SPEC-001 CORE-R100: complex arithmetic and the complex tridiagonal solvers.

use proptest::prelude::*;
use sciart_core::ode::{C64, tridiag_apply_c, tridiag_factor_c, tridiag_solve_c};

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
