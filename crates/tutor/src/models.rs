//! Learner and diagnosis models: pure functions, tested on hand-computed values.

/// Bayesian Knowledge Tracing parameters (Corbett & Anderson, 1994).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bkt {
    pub p_init: f64,
    pub p_learn: f64,
    pub p_slip: f64,
    pub p_guess: f64,
}

/// Priors for evidence from code (one observation per distinct attempt at a
/// hole). Not fitted: fitting needs logged data (milestone M6).
pub const CODE_BKT: Bkt = Bkt { p_init: 0.2, p_learn: 0.15, p_slip: 0.1, p_guess: 0.2 };

/// Priors for lesson answers (multiple choice: guessing is easier).
pub const LESSON_BKT: Bkt = Bkt { p_init: 0.2, p_learn: 0.1, p_slip: 0.1, p_guess: 0.25 };

/// One BKT update: posterior given the observation, then the learning transition.
pub fn bkt_update(p: Bkt, p_known: f64, correct: bool) -> f64 {
    let posterior = if correct {
        let a = p_known * (1.0 - p.p_slip);
        a / (a + (1.0 - p_known) * p.p_guess)
    } else {
        let a = p_known * p.p_slip;
        a / (a + (1.0 - p_known) * (1.0 - p.p_guess))
    };
    posterior + (1.0 - posterior) * p.p_learn
}

/// SM-2 state (Wozniak, 1990).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sm2 {
    pub repetitions: u32,
    pub interval_days: u32,
    pub easiness: f64,
}

impl Default for Sm2 {
    fn default() -> Self {
        Sm2 { repetitions: 0, interval_days: 0, easiness: 2.5 }
    }
}

/// One SM-2 review with quality `q` in 0..=5.
pub fn sm2_review(s: Sm2, q: u8) -> Sm2 {
    let q = q.min(5) as f64;
    let ef = (s.easiness + (0.1 - (5.0 - q) * (0.08 + (5.0 - q) * 0.02))).max(1.3);
    if q < 3.0 {
        return Sm2 { repetitions: 0, interval_days: 1, easiness: ef };
    }
    let interval = match s.repetitions {
        0 => 1,
        1 => 6,
        _ => (s.interval_days as f64 * s.easiness).round() as u32,
    };
    Sm2 { repetitions: s.repetitions + 1, interval_days: interval, easiness: ef }
}

/// Ochiai suspiciousness of a component: `ef / sqrt(F * (ef + ep))`, where
/// `ef`/`ep` are failing/passing tests that depend on it and `F` is the
/// number of failing tests. 0 when undefined.
pub fn ochiai(ef: usize, ep: usize, total_failing: usize) -> f64 {
    let d = (total_failing as f64 * (ef + ep) as f64).sqrt();
    if d == 0.0 {
        0.0
    } else {
        ef as f64 / d
    }
}

/// Two-decimal rendering that never prints `-0.00` (determinism in goldens).
pub fn fmt2(x: f64) -> String {
    let s = format!("{:.2}", x);
    if s == "-0.00" {
        "0.00".into()
    } else {
        s
    }
}
