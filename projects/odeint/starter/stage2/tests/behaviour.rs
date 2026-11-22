//! Behavioural tests: the specification of odeint.
//!
//! Every tolerance below says where it comes from. Floats are never compared
//! with `==` except through `to_bits()`, which states that a value must be
//! bit-for-bit exact by construction (and says why in a comment).

use odeint::*;

const EPS: f64 = f64::EPSILON;

/// Least-squares slope of ln(e) against ln(x). Local to the tests so that a
/// convergence test does not also depend on the `fit_order` hole.
fn slope(xs: &[f64], es: &[f64]) -> f64 {
    let n = xs.len() as f64;
    let mx = xs.iter().map(|x| x.ln()).sum::<f64>() / n;
    let me = es.iter().map(|e| e.ln()).sum::<f64>() / n;
    let (mut sxe, mut sxx) = (0.0, 0.0);
    for (x, e) in xs.iter().zip(es) {
        sxe += (x.ln() - mx) * (e.ln() - me);
        sxx += (x.ln() - mx).powi(2);
    }
    sxe / sxx
}

fn decay(_t: f64, y: &[f64], dy: &mut [f64]) {
    dy[0] = -y[0];
}

fn oscillator(_t: f64, y: &[f64], dy: &mut [f64]) {
    dy[0] = y[1];
    dy[1] = -y[0];
}

fn osc_error(y: &[f64], t: f64) -> f64 {
    ((y[0] - t.cos()).powi(2) + (y[1] + t.sin()).powi(2)).sqrt()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

mod lincomb {
    use super::*;

    #[test]
    fn exact_on_small_integers() {
        // Every value is a small integer or a power of two: the arithmetic is
        // exact, so bitwise equality is the specification, not a tolerance.
        let y = [1.0, 2.0, 3.0];
        let k1: [f64; 3] = [1.0, 0.0, -1.0];
        let k2 = [4.0, 4.0, 4.0];
        let mut out = [0.0; 3];
        lincomb(&mut out, &y, 0.5, &[(2.0, &k1), (0.25, &k2)]);
        let want: [f64; 3] = [2.5, 2.5, 2.5];
        assert!(out.iter().zip(&want).all(|(a, b)| a.to_bits() == b.to_bits()), "got {out:?}");
    }

    #[test]
    fn no_terms_copies_y() {
        let y = [1.5, -2.25];
        let mut out = [9.0, 9.0];
        lincomb(&mut out, &y, 0.1, &[]);
        assert!(out[0].to_bits() == y[0].to_bits() && out[1].to_bits() == y[1].to_bits(), "got {out:?}");
    }

    #[test]
    fn zero_coefficient_term_is_harmless() {
        // A zero coefficient contributes nothing for finite k, whether it is
        // skipped or multiplied: 1 + 1 * (0 * 3 + 1 * 3) = 4 exactly.
        let y = [1.0];
        let k = [3.0];
        let mut out = [0.0];
        lincomb(&mut out, &y, 1.0, &[(0.0, &k), (1.0, &k)]);
        assert!(out[0].to_bits() == 4.0f64.to_bits(), "got {out:?}");
    }

    #[test]
    fn length_mismatch_panics() {
        // Not `#[should_panic]`: an unimplemented body (`todo!`) panics too.
        // Matching lengths must work and a mismatch must panic.
        let fine = std::panic::catch_unwind(|| {
            let mut out = [0.0; 2];
            lincomb(&mut out, &[1.0, 2.0], 1.0, &[(1.0, &[1.0, 1.0])]);
        });
        let bad = std::panic::catch_unwind(|| {
            let mut out = [0.0; 2];
            lincomb(&mut out, &[1.0, 2.0], 1.0, &[(1.0, &[1.0])]);
        });
        assert!(fine.is_ok(), "matching lengths panicked");
        assert!(bad.is_err(), "a term of the wrong length was accepted");
    }
}

mod rk4 {
    use super::*;

    #[test]
    fn exact_for_cubic_quadrature() {
        // y' = 4 t^3: RK4 reduces to Simpson's rule, exact for cubics. Only
        // rounding remains: each step adds at most ~10 roundings of size
        // EPS * |y|, so 7 steps stay below 16 * 7 * EPS * 16.
        let mut ws = Workspace::new(1);
        let mut y = [0.0];
        rk4_fixed(&mut |t, _y: &[f64], dy: &mut [f64]| dy[0] = 4.0 * t * t * t, 0.0, 2.0, &mut y, 7, &mut ws);
        assert!(close(y[0], 16.0, 16.0 * 7.0 * EPS * 16.0), "y = {}", y[0]);
    }

    #[test]
    fn one_step_matches_taylor() {
        // For y' = -y one RK4 step multiplies by 1 - h + h^2/2 - h^3/6 + h^4/24
        // exactly (in exact arithmetic). Rounding: a few EPS.
        let h: f64 = 0.1;
        let mut ws = Workspace::new(1);
        let mut y = [1.0];
        rk4_step(&mut decay, 0.0, &mut y, h, &mut ws);
        let want = 1.0 - h + h * h / 2.0 - h.powi(3) / 6.0 + h.powi(4) / 24.0;
        assert!(close(y[0], want, 8.0 * EPS), "y = {}, want {want}", y[0]);
    }

    #[test]
    fn calls_f_four_times() {
        let mut ws = Workspace::new(1);
        let mut calls = 0;
        let mut y = [1.0];
        rk4_step(&mut |_t, y: &[f64], dy: &mut [f64]| { calls += 1; dy[0] = -y[0]; }, 0.0, &mut y, 0.1, &mut ws);
        assert_eq!(calls, 4);
    }

    #[test]
    fn order_is_four() {
        // Global error e(h) = C h^4 (1 + D h + ...): the fitted slope deviates
        // from 4 by about D h. From the data |D| < 1 and h <= 0.1, so the
        // deviation is below 0.1 (measured: 4.03). Errors stay above 4e-12,
        // far above the rounding floor (~n EPS = 4e-14 at n = 160).
        let mut ws = Workspace::new(1);
        let ns = [10usize, 20, 40, 80, 160];
        let mut hs = vec![];
        let mut es = vec![];
        for n in ns {
            let mut y = [1.0];
            rk4_fixed(&mut decay, 0.0, 1.0, &mut y, n, &mut ws);
            hs.push(1.0 / n as f64);
            es.push((y[0] - (-1.0f64).exp()).abs());
        }
        let p = slope(&hs, &es);
        assert!((p - 4.0).abs() < 0.1, "fitted order {p}, errors {es:?}");
    }

    #[test]
    fn oscillator_keeps_phase() {
        // 2-D system: stages must be per-component. Error bound from the
        // order-4 constant measured on this problem (2.7e-5 at n = 10, h = 0.2),
        // times 2 for margin; a scalar-only implementation is off by O(1).
        let mut ws = Workspace::new(2);
        let mut y = [1.0, 0.0];
        rk4_fixed(&mut oscillator, 0.0, 2.0, &mut y, 10, &mut ws);
        assert!(osc_error(&y, 2.0) < 5.4e-5, "error {}", osc_error(&y, 2.0));
    }
}

mod norm {
    use super::*;

    #[test]
    fn hand_computed() {
        // sc = [1 + 1*max(1,3), 1 + 1*max(2,0)] = [4, 3]; ratios [2/4, 3/3];
        // sqrt((0.25 + 1)/2) = sqrt(0.625).
        let v = error_norm(&[2.0, 3.0], &[1.0, 2.0], &[-3.0, 0.0], 1.0, 1.0);
        assert!(close(v, 0.625f64.sqrt(), 2.0 * EPS), "got {v}");
    }

    #[test]
    fn uses_the_larger_of_old_and_new() {
        // Scale uses max(|y0|, |y1|): with y0 = 0, y1 = 10, rtol = 0.1,
        // atol = 0: sc = 1 and the norm is |err| = 0.5.
        let v = error_norm(&[0.5], &[0.0], &[10.0], 0.0, 0.1);
        assert!(close(v, 0.5, 2.0 * EPS), "got {v}");
    }

    #[test]
    fn is_rms_not_max_or_sum() {
        // Four equal ratios of 1: RMS is 1, a sum would be 4, a 2-norm 2.
        let v = error_norm(&[1.0; 4], &[0.0; 4], &[0.0; 4], 1.0, 0.0);
        assert!(close(v, 1.0, 2.0 * EPS), "got {v}");
    }

    #[test]
    fn empty_is_zero_and_nan_propagates() {
        assert!(error_norm(&[], &[], &[], 1.0, 1.0).to_bits() == 0.0f64.to_bits());
        assert!(error_norm(&[f64::NAN, 0.0], &[1.0, 1.0], &[1.0, 1.0], 1.0, 1.0).is_nan());
    }
}

mod order {
    use super::*;

    #[test]
    fn recovers_power_laws() {
        // e = 3 h^p exactly (up to the rounding of powf and ln): the slope is p
        // to within ~100 EPS relative.
        for p in [1.0, 2.0, 4.0, 5.0] {
            let hs = [0.1, 0.05, 0.025, 0.0125];
            let es: Vec<f64> = hs.iter().map(|h: &f64| 3.0 * h.powf(p)).collect();
            let got = fit_order(&hs, &es).expect("four valid points");
            assert!(close(got, p, 1e-12), "p = {p}, got {got}");
        }
    }

    #[test]
    fn is_least_squares_not_endpoints() {
        // ln h = [0, 1, 2], ln e = [0, 0, 3]: dx = [-1, 0, 1], dz = [-1, -1, 2],
        // slope = 3 / 2. (The first-pair slope would be 0.)
        let hs = [1.0, 1f64.exp(), 2f64.exp()];
        let es = [1.0, 1.0, 3f64.exp()];
        let got = fit_order(&hs, &es).unwrap();
        assert!(close(got, 1.5, 1e-12), "got {got}");
        // Unequal spacing separates least squares from the endpoint slope:
        // ln h = [0, 1, 3], ln e = [0, 2, 3]: dx = [-4, -1, 5]/3, dz = [-5, 1, 4]/3,
        // slope = (39/9) / (42/9) = 13/14; the endpoint slope would be 1.
        let hs = [1.0, 1f64.exp(), 3f64.exp()];
        let es = [1.0, 2f64.exp(), 3f64.exp()];
        let got = fit_order(&hs, &es).unwrap();
        assert!(close(got, 13.0 / 14.0, 1e-12), "got {got}");
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(fit_order(&[0.1], &[0.2]), None);
        assert_eq!(fit_order(&[0.1, 0.2], &[0.2]), None);
        assert_eq!(fit_order(&[0.1, 0.1], &[0.2, 0.3]), None);
        assert_eq!(fit_order(&[0.1, 0.2], &[0.0, 0.3]), None);
        assert_eq!(fit_order(&[0.1, f64::NAN], &[0.2, 0.3]), None);
        assert_eq!(fit_order(&[-0.1, 0.2], &[0.2, 0.3]), None);
    }
}

mod dopri {
    use super::*;

    #[test]
    fn exact_for_quartic_quadrature() {
        // y' = 5 t^4: the fifth-order weights integrate quartics exactly, so
        // only rounding remains (bound as in rk4::exact_for_cubic_quadrature,
        // scaled to |y| = 32).
        let mut ws = Workspace::new(1);
        let mut y = [0.0];
        dopri_fixed(&mut |t, _y: &[f64], dy: &mut [f64]| dy[0] = 5.0 * t.powi(4), 0.0, 2.0, &mut y, 7, &mut ws);
        assert!(close(y[0], 32.0, 16.0 * 7.0 * EPS * 32.0), "y = {}", y[0]);
    }

    #[test]
    fn does_not_touch_y_and_calls_f_seven_times() {
        let mut ws = Workspace::new(1);
        let y = [1.0];
        let mut calls = 0;
        dopri_step(&mut |_t, y: &[f64], dy: &mut [f64]| { calls += 1; dy[0] = -y[0]; }, 0.0, &y, 0.1, &mut ws);
        assert_eq!(calls, 7);
        assert!(y[0].to_bits() == 1.0f64.to_bits());
    }

    #[test]
    fn order_is_five() {
        // As rk4::order_is_four, for p = 5 on the oscillator with h = 2/n <= 0.2:
        // deviation ~ D h with |D| < 0.75 from the data, so < 0.15 (measured 5.005).
        // Smallest error 1.7e-13, above the rounding floor ~1e-14.
        let mut ws = Workspace::new(2);
        let mut hs = vec![];
        let mut es = vec![];
        for n in [10usize, 20, 40, 80, 160] {
            let mut y = [1.0, 0.0];
            dopri_fixed(&mut oscillator, 0.0, 2.0, &mut y, n, &mut ws);
            hs.push(2.0 / n as f64);
            es.push(osc_error(&y, 2.0));
        }
        let p = slope(&hs, &es);
        assert!((p - 5.0).abs() < 0.15, "fitted order {p}, errors {es:?}");
    }

    #[test]
    fn error_estimate_is_fourth_order_local() {
        // The embedded estimate is O(h^5) per step (local error of the
        // fourth-order solution). Halving h divides it by ~2^5 = 32; the
        // band [24, 40] allows the O(h) correction at h <= 0.2.
        let mut ws = Workspace::new(1);
        let y = [1.0];
        dopri_step(&mut decay, 0.0, &y, 0.2, &mut ws);
        let e1 = ws.err[0].abs();
        dopri_step(&mut decay, 0.0, &y, 0.1, &mut ws);
        let e2 = ws.err[0].abs();
        let r = e1 / e2;
        assert!((24.0..=40.0).contains(&r), "ratio {r} ({e1:e} / {e2:e})");
    }

    #[test]
    fn error_estimate_vanishes_on_constant_rhs() {
        // f constant: every k is 1, sum(E) = 0 exactly in exact arithmetic;
        // in floats the sum of the seven rounded E's is below 8 EPS.
        let mut ws = Workspace::new(1);
        dopri_step(&mut |_t, _y: &[f64], dy: &mut [f64]| dy[0] = 1.0, 0.0, &[0.0], 1.0, &mut ws);
        assert!(ws.err[0].abs() < 8.0 * EPS, "err {}", ws.err[0]);
        assert!(close(ws.ynew[0], 1.0, 8.0 * EPS), "ynew {}", ws.ynew[0]);
    }
}

mod control {
    use super::*;

    #[test]
    fn hand_computed_factor() {
        // err = 2^-5 * 0.9^5 / 1.5^5 ... simpler: err = 1 gives fac = SAFETY.
        let h = next_step_size(0.1, 1.0, FAC_MAX);
        assert!(close(h, 0.09, 4.0 * EPS), "got {h}");
        // err = 1/32: fac = 0.9 * 2 = 1.8.
        let h = next_step_size(1.0, 1.0 / 32.0, FAC_MAX);
        assert!(close(h, 1.8, 4.0 * EPS), "got {h}");
    }

    #[test]
    fn clamps_both_ways() {
        assert!(close(next_step_size(1.0, 1e-20, FAC_MAX), FAC_MAX, 4.0 * EPS));
        assert!(close(next_step_size(1.0, 1e20, FAC_MAX), FAC_MIN, 4.0 * EPS));
        // After a rejection the step never grows.
        assert!(close(next_step_size(1.0, 1e-20, 1.0), 1.0, 4.0 * EPS));
    }

    #[test]
    fn exact_step_grows_by_fac_max() {
        assert!(close(next_step_size(0.5, 0.0, FAC_MAX), 0.5 * FAC_MAX, 4.0 * EPS));
        assert!(next_step_size(0.5, 0.0, FAC_MAX).is_finite());
    }

    #[test]
    fn uses_exponent_one_fifth() {
        // err = 0.5: fac = 0.9 * 2^(1/5) = 1.0338...; exponent 1/4 would give
        // 1.0703 and 1/6 gives 1.0102.
        let h = next_step_size(1.0, 0.5, FAC_MAX);
        assert!(close(h, 0.9 * 2f64.powf(0.2), 4.0 * EPS), "got {h}");
    }
}

mod integrate {
    use super::*;

    fn opts(tol: f64) -> Options {
        Options { atol: tol, rtol: tol, ..Options::default() }
    }

    #[test]
    fn decay_to_tolerance() {
        // Global error is not bounded by tol (tol is a local criterion); on
        // smooth problems it is a small multiple. Measured ~4-6x on these
        // problems; 20x is the stated band.
        let mut ws = Workspace::new(1);
        let mut y = [1.0];
        let s = integrate(&mut decay, 0.0, 5.0, &mut y, &opts(1e-8), &mut ws).unwrap();
        assert!(close(y[0], (-5.0f64).exp(), 20.0 * 1e-8), "y = {}", y[0]);
        assert!(s.accepted > 0);
    }

    #[test]
    fn lands_exactly_on_t_end() {
        // The spec requires t = t_end on the last step: t + h may round to a
        // neighbouring float, so this is a bitwise contract.
        let mut ws = Workspace::new(2);
        for t_end in [0.3, 1.0 / 3.0, 10.0, 7.1] {
            let mut y = [1.0, 0.0];
            let s = integrate(&mut oscillator, 0.1, t_end, &mut y, &opts(1e-6), &mut ws).unwrap();
            assert!(s.t_final.to_bits() == t_end.to_bits(), "t_final {} vs {t_end}", s.t_final);
        }
    }

    #[test]
    fn tolerance_proportionality() {
        // For error-per-step control with local extrapolation, the global
        // error scales like tol^(5/5) = tol (Hairer, Norsett & Wanner II.4).
        // Measured slope 1.02 on this problem; the +-0.1 band is calibrated
        // on it, not derived.
        let mut ws = Workspace::new(2);
        let tols = [1e-4, 1e-5, 1e-6, 1e-7, 1e-8, 1e-9, 1e-10];
        let mut es = vec![];
        for tol in tols {
            let mut y = [1.0, 0.0];
            integrate(&mut oscillator, 0.0, 10.0, &mut y, &opts(tol), &mut ws).unwrap();
            es.push(osc_error(&y, 10.0));
        }
        let p = slope(&tols, &es);
        assert!((p - 1.0).abs() < 0.1, "slope {p}, errors {es:?}");
    }

    #[test]
    fn steps_grow_with_accuracy_like_tol_to_minus_one_fifth() {
        // Accepted steps ~ tol^(-1/5): 1e-4 -> 1e-9 multiplies them by
        // 10^(5/5) = 10. Measured 17 -> 149 (8.8x); band [6, 14].
        let mut ws = Workspace::new(2);
        let count = |tol: f64, ws: &mut Workspace| {
            let mut y = [1.0, 0.0];
            integrate(&mut oscillator, 0.0, 10.0, &mut y, &opts(tol), ws).unwrap().accepted as f64
        };
        let r = count(1e-9, &mut ws) / count(1e-4, &mut ws);
        assert!((6.0..=14.0).contains(&r), "ratio {r}");
    }

    #[test]
    fn kepler_energy_drift_is_linear() {
        // Dormand-Prince is not symplectic: energy error accumulates roughly
        // linearly in time (each orbit adds a similar local error), not
        // exponentially and not bounded. Slope of dE against periods
        // measured 1.01 over 1..16 periods at tol 1e-10; band +-0.15.
        let ecc = 0.5f64;
        let mut kep = |_t: f64, y: &[f64], dy: &mut [f64]| {
            let r3 = (y[0] * y[0] + y[1] * y[1]).powf(1.5);
            dy[0] = y[2];
            dy[1] = y[3];
            dy[2] = -y[0] / r3;
            dy[3] = -y[1] / r3;
        };
        let energy = |y: &[f64]| 0.5 * (y[2] * y[2] + y[3] * y[3]) - 1.0 / (y[0] * y[0] + y[1] * y[1]).sqrt();
        let y0 = [1.0 - ecc, 0.0, 0.0, ((1.0 + ecc) / (1.0 - ecc)).sqrt()];
        let mut ws = Workspace::new(4);
        let periods = [1.0, 2.0, 4.0, 8.0, 16.0];
        let mut de = vec![];
        for p in periods {
            let mut y = y0;
            integrate(&mut kep, 0.0, 2.0 * std::f64::consts::PI * p, &mut y, &opts(1e-10), &mut ws).unwrap();
            de.push((energy(&y) - energy(&y0)).abs());
        }
        let s = slope(&periods, &de);
        assert!((s - 1.0).abs() < 0.15, "drift slope {s}, dE {de:?}");
    }

    #[test]
    fn counts_every_evaluation() {
        // Without FSAL every attempted step costs 7 calls, plus 1 for the
        // initial step size.
        let mut ws = Workspace::new(2);
        let mut calls = 0usize;
        let mut y = [1.0, 0.0];
        let s = integrate(
            &mut |t, y: &[f64], dy: &mut [f64]| { calls += 1; oscillator(t, y, dy) },
            0.0, 3.0, &mut y, &opts(1e-6), &mut ws,
        ).unwrap();
        assert_eq!(s.evals, calls);
        assert_eq!(s.evals, 7 * (s.accepted + s.rejected) + 1);
    }

    #[test]
    fn rejects_then_recovers() {
        // A deliberately huge first step must be rejected, and the
        // integration must still meet the accuracy of `decay_to_tolerance`.
        let mut ws = Workspace::new(1);
        let mut y = [1.0];
        let o = Options { h0: Some(4.0), ..opts(1e-8) };
        let s = integrate(&mut decay, 0.0, 5.0, &mut y, &o, &mut ws).unwrap();
        assert!(s.rejected >= 1, "{s:?}");
        assert!(close(y[0], (-5.0f64).exp(), 20.0 * 1e-8), "y = {}", y[0]);
    }

    #[test]
    fn empty_span_does_nothing() {
        let mut ws = Workspace::new(1);
        let mut y = [2.0];
        let s = integrate(&mut decay, 1.0, 1.0, &mut y, &opts(1e-6), &mut ws).unwrap();
        assert_eq!((s.accepted, s.rejected, s.evals), (0, 0, 0));
        assert!(y[0].to_bits() == 2.0f64.to_bits());
    }

    #[test]
    fn reports_errors_as_values() {
        let mut ws = Workspace::new(1);
        let mut y = [1.0];
        assert_eq!(integrate(&mut decay, 1.0, 0.0, &mut y, &opts(1e-6), &mut ws), Err(OdeError::InvalidSpan));
        assert_eq!(integrate(&mut decay, 0.0, f64::INFINITY, &mut y, &opts(1e-6), &mut ws), Err(OdeError::InvalidSpan));
        let mut y2 = [1.0, 2.0];
        assert_eq!(
            integrate(&mut decay, 0.0, 1.0, &mut y2, &opts(1e-6), &mut ws),
            Err(OdeError::DimensionMismatch { y: 2, workspace: 1 })
        );
        let o = Options { max_steps: 3, ..opts(1e-10) };
        assert!(matches!(integrate(&mut decay, 0.0, 100.0, &mut y, &o, &mut ws), Err(OdeError::MaxSteps { .. })));
    }

    #[test]
    fn nan_is_an_error_not_a_hang() {
        let mut ws = Workspace::new(1);
        let mut y = [1.0];
        let r = integrate(
            &mut |t, _y: &[f64], dy: &mut [f64]| dy[0] = if t > 0.5 { f64::NAN } else { 1.0 },
            0.0, 1.0, &mut y, &opts(1e-6), &mut ws,
        );
        assert!(matches!(r, Err(OdeError::NonFinite { .. })), "{r:?}");
        // y keeps the last accepted state, which is finite.
        assert!(y[0].is_finite());
    }

    #[test]
    fn finite_time_blowup_stops() {
        // y' = y^2, y(0) = 1 blows up at t = 1. The step size collapses:
        // StepTooSmall, NonFinite or MaxSteps are all honest outcomes, Ok is not.
        let mut ws = Workspace::new(1);
        let mut y = [1.0];
        let r = integrate(&mut |_t, y: &[f64], dy: &mut [f64]| dy[0] = y[0] * y[0], 0.0, 2.0, &mut y, &opts(1e-8), &mut ws);
        assert!(r.is_err(), "{r:?}");
    }
}
