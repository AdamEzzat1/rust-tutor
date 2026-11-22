//! Measurements for `tutor explain` and `tutor baseline`: allocation counts
//! from a counting global allocator and step statistics on fixed problems.
//! Deterministic: no clocks, no randomness. Output is `key = value` lines.

use odeint::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static A: Counting = Counting;

fn count(f: impl FnOnce()) -> usize {
    let b = ALLOCS.load(Ordering::SeqCst);
    f();
    ALLOCS.load(Ordering::SeqCst) - b
}

fn oscillator(_t: f64, y: &[f64], dy: &mut [f64]) {
    dy[0] = y[1];
    dy[1] = -y[0];
}

/// Runs `f`, turning a panic (e.g. an open hole's `todo!`) into "unavailable".
fn guarded<T: std::fmt::Display>(key: &str, f: impl FnOnce() -> T + std::panic::UnwindSafe) {
    match std::panic::catch_unwind(f) {
        Ok(v) => println!("{key} = {v}"),
        Err(_) => println!("{key} = unavailable"),
    }
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    guarded("allocations rk4_step x100", || {
        let mut ws = Workspace::new(2);
        let mut y = [1.0, 0.0];
        count(|| for i in 0..100 { rk4_step(&mut oscillator, i as f64 * 0.01, &mut y, 0.01, &mut ws) })
    });
    guarded("allocations dopri_step x100", || {
        let mut ws = Workspace::new(2);
        let y = [1.0, 0.0];
        count(|| for _ in 0..100 { dopri_step(&mut oscillator, 0.0, &y, 0.01, &mut ws) })
    });
    guarded("allocations integrate(oscillator, 0..10)", || {
        let mut ws = Workspace::new(2);
        let mut y = [1.0, 0.0];
        let o = Options { atol: 1e-8, rtol: 1e-8, ..Options::default() };
        count(|| { let _ = integrate(&mut oscillator, 0.0, 10.0, &mut y, &o, &mut ws); })
    });
    guarded("steps integrate(oscillator, 0..10, tol 1e-8)", || {
        let mut ws = Workspace::new(2);
        let mut y = [1.0, 0.0];
        let o = Options { atol: 1e-8, rtol: 1e-8, ..Options::default() };
        match integrate(&mut oscillator, 0.0, 10.0, &mut y, &o, &mut ws) {
            Ok(s) => format!("accepted {} rejected {} evals {}", s.accepted, s.rejected, s.evals),
            Err(e) => format!("error: {e}"),
        }
    });
}
