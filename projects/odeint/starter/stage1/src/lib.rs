//! odeint: explicit Runge-Kutta integration with error control, from scratch.
//!
//! The right-hand side of `y' = f(t, y)` is any `FnMut(f64, &[f64], &mut [f64])`:
//! it reads the state `y` and writes the derivative into the output slice. No
//! function in this crate allocates once a [`Workspace`] exists; `tests/alloc.rs`
//! measures that with a counting global allocator.
//!
//! Every function marked `// HOLE:` is a hole in the starter stages. Its doc
//! comment is the specification.

use std::fmt;

/// Dormand-Prince 5(4) nodes `c`, matrix `a` (row i holds a[i][0..i]),
/// fifth-order weights `b5` and error weights `e = b5 - b4`.
/// Source: Dormand & Prince (1980), Table 2; Hairer, Norsett & Wanner,
/// *Solving ODEs I*, Table 5.2.
pub mod tableau {
    pub const C: [f64; 7] = [0.0, 1.0 / 5.0, 3.0 / 10.0, 4.0 / 5.0, 8.0 / 9.0, 1.0, 1.0];
    pub const A: [[f64; 6]; 7] = [
        [0.0; 6],
        [1.0 / 5.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [3.0 / 40.0, 9.0 / 40.0, 0.0, 0.0, 0.0, 0.0],
        [44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0, 0.0, 0.0, 0.0],
        [19372.0 / 6561.0, -25360.0 / 2187.0, 64448.0 / 6561.0, -212.0 / 729.0, 0.0, 0.0],
        [9017.0 / 3168.0, -355.0 / 33.0, 46732.0 / 5247.0, 49.0 / 176.0, -5103.0 / 18656.0, 0.0],
        [35.0 / 384.0, 0.0, 500.0 / 1113.0, 125.0 / 192.0, -2187.0 / 6784.0, 11.0 / 84.0],
    ];
    pub const B5: [f64; 7] =
        [35.0 / 384.0, 0.0, 500.0 / 1113.0, 125.0 / 192.0, -2187.0 / 6784.0, 11.0 / 84.0, 0.0];
    pub const E: [f64; 7] = [
        71.0 / 57600.0,
        0.0,
        -71.0 / 16695.0,
        71.0 / 1920.0,
        -17253.0 / 339200.0,
        22.0 / 525.0,
        -1.0 / 40.0,
    ];
}

/// Step-size controller constants (Hairer, Norsett & Wanner, II.4, eq. 4.13).
pub const SAFETY: f64 = 0.9;
pub const FAC_MIN: f64 = 0.2;
pub const FAC_MAX: f64 = 5.0;

/// Scratch buffers for one system dimension, allocated once.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Stage derivatives k1..k7.
    pub k: [Vec<f64>; 7],
    /// The argument of the next stage evaluation.
    pub ytmp: Vec<f64>,
    /// The fifth-order solution of the last `dopri_step`.
    pub ynew: Vec<f64>,
    /// The error estimate of the last `dopri_step` (fifth minus fourth order).
    pub err: Vec<f64>,
}

impl Workspace {
    pub fn new(n: usize) -> Self {
        Workspace {
            k: std::array::from_fn(|_| vec![0.0; n]),
            ytmp: vec![0.0; n],
            ynew: vec![0.0; n],
            err: vec![0.0; n],
        }
    }

    pub fn dim(&self) -> usize {
        self.ytmp.len()
    }
}

/// Tolerances and limits for [`integrate`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    pub atol: f64,
    pub rtol: f64,
    /// First step size; `None` picks one with [`initial_step`].
    pub h0: Option<f64>,
    /// Attempted steps (accepted + rejected) before giving up.
    pub max_steps: usize,
    /// A step size below this (relative to `|t|`, plus `f64::MIN_POSITIVE`) is an error.
    pub h_min_rel: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options { atol: 1e-8, rtol: 1e-8, h0: None, max_steps: 100_000, h_min_rel: 4.0 * f64::EPSILON }
    }
}

/// What an integration did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub accepted: usize,
    pub rejected: usize,
    /// Calls of the right-hand side.
    pub evals: usize,
    pub t_final: f64,
}

/// Why an integration stopped early. `t` is the time reached.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OdeError {
    /// `t_end < t0`, or a bound is not finite.
    InvalidSpan,
    /// `y` and the workspace have different lengths.
    DimensionMismatch { y: usize, workspace: usize },
    MaxSteps { t: f64 },
    StepTooSmall { t: f64, h: f64 },
    /// The error estimate was NaN or infinite: the solution blew up or `f` returned NaN.
    NonFinite { t: f64 },
}

impl fmt::Display for OdeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OdeError::InvalidSpan => write!(f, "invalid time span"),
            OdeError::DimensionMismatch { y, workspace } => {
                write!(f, "state has length {y} but the workspace has length {workspace}")
            }
            OdeError::MaxSteps { t } => write!(f, "too many steps; stopped at t = {t}"),
            OdeError::StepTooSmall { t, h } => write!(f, "step size {h:e} too small at t = {t}"),
            OdeError::NonFinite { t } => write!(f, "non-finite error estimate at t = {t}"),
        }
    }
}

impl std::error::Error for OdeError {}

// HOLE: lincomb
/// Writes `out[i] = y[i] + h * (c1 * k1[i] + c2 * k2[i] + ...)` for every `i`,
/// where `terms = [(c1, k1), (c2, k2), ...]`.
///
/// * With no terms, `out` becomes a copy of `y`.
/// * Terms whose coefficient is exactly `0.0` may be skipped, but the result
///   must not depend on it (a zero term still multiplies a finite value).
/// * Panics if any slice has a different length from `out`.
/// * Allocates nothing.
///
/// `out` and `y` cannot be the same slice: the borrow checker rejects
/// `lincomb(&mut v, &v, ...)`, which is why stages go through `Workspace::ytmp`.
pub fn lincomb(out: &mut [f64], y: &[f64], h: f64, terms: &[(f64, &[f64])]) {
    todo!("TODO(hole:lincomb)")
}

// HOLE: rk4_step
/// One step of the classical fourth-order Runge-Kutta method, in place:
/// on return `y` holds the approximation at `t + h`.
///
/// ```text
/// k1 = f(t,       y)
/// k2 = f(t + h/2, y + h/2 k1)
/// k3 = f(t + h/2, y + h/2 k2)
/// k4 = f(t + h,   y + h   k3)
/// y  = y + h (k1 + 2 k2 + 2 k3 + k4) / 6
/// ```
///
/// Calls `f` exactly four times and allocates nothing: use `ws.k[0..4]` for
/// the stages and `ws.ytmp` for each stage's argument. Its global error is
/// O(h^4) (`tests/behaviour.rs` fits the order).
pub fn rk4_step<F>(f: &mut F, t: f64, y: &mut [f64], h: f64, ws: &mut Workspace)
where
    F: FnMut(f64, &[f64], &mut [f64]),
{
    todo!("TODO(hole:rk4_step)")
}

// HOLE: error_norm
/// The scaled root-mean-square norm used for step acceptance
/// (Hairer, Norsett & Wanner, II.4, eq. 4.11):
///
/// ```text
/// sc_i = atol + rtol * max(|y0_i|, |y1_i|)
/// norm = sqrt( (1/n) * sum_i (err_i / sc_i)^2 )
/// ```
///
/// A step is accepted when the norm is at most 1. Returns `0.0` when `n == 0`.
/// A NaN anywhere gives NaN (never hide it: `integrate` turns it into
/// `OdeError::NonFinite`). Allocates nothing.
pub fn error_norm(err: &[f64], y0: &[f64], y1: &[f64], atol: f64, rtol: f64) -> f64 {
    todo!("TODO(hole:error_norm)")
}

// HOLE: fit_order
/// The observed order of convergence: the least-squares slope of
/// `ln(errs[i])` against `ln(hs[i])`.
///
/// ```text
/// x_i = ln h_i,  z_i = ln e_i
/// slope = sum (x_i - mean x)(z_i - mean z) / sum (x_i - mean x)^2
/// ```
///
/// For `errs[i] = C * hs[i]^p` it returns `p` up to rounding. Returns `None`
/// if there are fewer than two points, the lengths differ, any value is not
/// finite and positive, or every `h` is the same.
pub fn fit_order(hs: &[f64], errs: &[f64]) -> Option<f64> {
    todo!("TODO(hole:fit_order)")
}

// HOLE: dopri_step
/// One Dormand-Prince 5(4) step from `(t, y)` with step `h`. Does not change
/// `y`. On return:
///
/// * `ws.ynew = y + h * sum_j B5[j] * k_j`   (fifth order),
/// * `ws.err  = h * sum_j E[j] * k_j`        (fifth minus fourth order),
///
/// with `k_1 = f(t, y)` and, for stage `i = 2..7`,
/// `k_i = f(t + C[i-1] h, y + h * sum_{j<i} A[i-1][j-1] k_j)` (tableau in
/// [`tableau`], rows and columns from 0). Stage 7 is evaluated at `ynew`
/// (row 7 of `A` equals `B5`), so `f` is called exactly seven times.
/// Allocates nothing: stages go in `ws.k`, stage arguments in `ws.ytmp`.
pub fn dopri_step<F>(f: &mut F, t: f64, y: &[f64], h: f64, ws: &mut Workspace)
where
    F: FnMut(f64, &[f64], &mut [f64]),
{
    use tableau::{A, B5, C, E};
    let Workspace { k, ytmp, ynew, err } = ws;
    f(t, y, &mut k[0]);
    for s in 1..7 {
        // Stages 0..s are read while stage s is written: split the array.
        let (done, rest) = k.split_at_mut(s);
        ytmp.copy_from_slice(y);
        for (j, kj) in done.iter().enumerate() {
            let a = h * A[s][j];
            for (t, kv) in ytmp.iter_mut().zip(kj) {
                *t += a * kv;
            }
        }
        f(t + C[s] * h, ytmp, &mut rest[0]);
    }
    // Stage-major accumulation: the inner loop walks each k_j with unit stride.
    ynew.copy_from_slice(y);
    err.fill(0.0);
    for (j, kj) in k.iter().enumerate() {
        let (b, e) = (h * B5[j], h * E[j]);
        for ((yn, er), kv) in ynew.iter_mut().zip(err.iter_mut()).zip(kj) {
            *yn += b * kv;
            *er += e * kv;
        }
    }
}

// HOLE: next_step_size
/// The step size to try next, given the step `h` just tried and its error
/// norm `err` (from [`error_norm`]):
///
/// ```text
/// fac = SAFETY * err^(-1/5)    (5 = 1 + the lower order, 4)
/// h_new = h * clamp(fac, FAC_MIN, fac_max)
/// ```
///
/// `err == 0.0` (an exact step) gives `h * fac_max`. `fac_max` is a
/// parameter because after a rejected step [`integrate`] passes `1.0`: the
/// step must not grow right after it failed. `err` is finite and `>= 0`.
pub fn next_step_size(h: f64, err: f64, fac_max: f64) -> f64 {
    if err == 0.0 {
        return h * fac_max;
    }
    let fac = SAFETY * err.powf(-1.0 / 5.0);
    h * fac.clamp(FAC_MIN, fac_max)
}

/// A first step size (Hairer, Norsett & Wanner, II.4, simplified "starting
/// step size" algorithm): `0.01 * |y0|_sc / |f0|_sc`, or `1e-6` if either
/// norm is tiny, never more than the span. Uses `ws.k[0]`; one call of `f`.
pub fn initial_step<F>(f: &mut F, t0: f64, t_end: f64, y: &[f64], opts: &Options, ws: &mut Workspace) -> f64
where
    F: FnMut(f64, &[f64], &mut [f64]),
{
    f(t0, y, &mut ws.k[0]);
    let d0 = error_norm(y, y, y, opts.atol, opts.rtol);
    let d1 = error_norm(&ws.k[0], y, y, opts.atol, opts.rtol);
    let h = if d0 < 1e-5 || d1 < 1e-5 { 1e-6 } else { 0.01 * d0 / d1 };
    h.min(t_end - t0)
}

// HOLE: integrate
/// Integrates `y' = f(t, y)` from `t0` to `t_end` with adaptive
/// Dormand-Prince 5(4), overwriting `y` with the solution at `t_end`.
///
/// Loop until `t == t_end`:
/// 1. Clip the step so it does not pass `t_end`. On the step that reaches
///    `t_end`, set `t = t_end` exactly (`t + h` may round to a different float).
/// 2. `dopri_step`, then `error_norm(&ws.err, y, &ws.ynew, atol, rtol)`.
/// 3. A non-finite norm is `Err(OdeError::NonFinite { t })`.
/// 4. `norm <= 1`: accept (copy `ws.ynew` into `y`, advance `t`) and grow with
///    `next_step_size(h, norm, FAC_MAX)`; otherwise reject (keep `y` and `t`)
///    and shrink with `next_step_size(h, norm, 1.0)`. Use `fac_max = 1.0` also
///    for the step right after a rejection.
/// 5. More than `opts.max_steps` attempts is `Err(MaxSteps { t })`; a step
///    below `opts.h_min_rel * |t| + f64::MIN_POSITIVE` is `Err(StepTooSmall { t, h })`.
///
/// Errors before any step: `InvalidSpan` if `t_end < t0` or a bound is not
/// finite, `DimensionMismatch` if `y.len() != ws.dim()`. `t0 == t_end` returns
/// at once with zero steps and zero evaluations. `Stats::evals` counts every
/// call of `f`, including the one in [`initial_step`]. On error `y` holds the
/// last accepted state. Allocates nothing.
pub fn integrate<F>(
    f: &mut F,
    t0: f64,
    t_end: f64,
    y: &mut [f64],
    opts: &Options,
    ws: &mut Workspace,
) -> Result<Stats, OdeError>
where
    F: FnMut(f64, &[f64], &mut [f64]),
{
    if !(t0.is_finite() && t_end.is_finite()) || t_end < t0 {
        return Err(OdeError::InvalidSpan);
    }
    if y.len() != ws.dim() {
        return Err(OdeError::DimensionMismatch { y: y.len(), workspace: ws.dim() });
    }
    let mut evals = 0usize;
    let mut g = |t: f64, y: &[f64], dy: &mut [f64]| {
        evals += 1;
        f(t, y, dy)
    };
    let mut stats = Stats { accepted: 0, rejected: 0, evals: 0, t_final: t0 };
    let mut t = t0;
    if t0 == t_end {
        return Ok(stats);
    }
    let mut h = match opts.h0 {
        Some(h0) => h0,
        None => initial_step(&mut g, t0, t_end, y, opts, ws),
    };
    let mut last_rejected = false;
    let mut result = Ok(());
    while t < t_end {
        if stats.accepted + stats.rejected >= opts.max_steps {
            result = Err(OdeError::MaxSteps { t });
            break;
        }
        let h_min = opts.h_min_rel * t.abs() + f64::MIN_POSITIVE;
        if h < h_min {
            result = Err(OdeError::StepTooSmall { t, h });
            break;
        }
        let last = h >= t_end - t;
        if last {
            h = t_end - t;
        }
        dopri_step(&mut g, t, y, h, ws);
        let norm = error_norm(&ws.err, y, &ws.ynew, opts.atol, opts.rtol);
        if !norm.is_finite() {
            result = Err(OdeError::NonFinite { t });
            break;
        }
        if norm <= 1.0 {
            y.copy_from_slice(&ws.ynew);
            t = if last { t_end } else { t + h };
            stats.accepted += 1;
            h = next_step_size(h, norm, if last_rejected { 1.0 } else { FAC_MAX });
            last_rejected = false;
        } else {
            stats.rejected += 1;
            h = next_step_size(h, norm, 1.0);
            last_rejected = true;
        }
    }
    stats.t_final = t;
    stats.evals = evals;
    result.map(|()| stats)
}

/// Fixed-step integration with `n` equal steps of [`rk4_step`] (n >= 1).
pub fn rk4_fixed<F>(f: &mut F, t0: f64, t_end: f64, y: &mut [f64], n: usize, ws: &mut Workspace)
where
    F: FnMut(f64, &[f64], &mut [f64]),
{
    let h = (t_end - t0) / n as f64;
    for i in 0..n {
        rk4_step(f, t0 + i as f64 * h, y, h, ws);
    }
}

/// Fixed-step integration with `n` equal steps of [`dopri_step`]'s fifth-order solution.
pub fn dopri_fixed<F>(f: &mut F, t0: f64, t_end: f64, y: &mut [f64], n: usize, ws: &mut Workspace)
where
    F: FnMut(f64, &[f64], &mut [f64]),
{
    let h = (t_end - t0) / n as f64;
    for i in 0..n {
        dopri_step(f, t0 + i as f64 * h, y, h, ws);
        y.copy_from_slice(&ws.ynew);
    }
}
