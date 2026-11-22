//! Allocation tests: measured with a counting global allocator, not inferred.
//!
//! The counter is per thread. libtest runs tests on several threads by
//! default, and a process-wide counter would also count the harness's and the
//! other tests' allocations: a plain `cargo test` measured 27 "allocations" in
//! `integrate` that way. Each test runs its measured closure on its own
//! thread, so the count is exact for any `--test-threads`.

use odeint::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    // const-initialised Cell<usize>: no lazy init and no destructor, so
    // touching it from inside the allocator never allocates or recurses.
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

fn bump() {
    // try_with: the slot may be gone while a thread is being torn down.
    let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        bump();
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn allocations(f: impl FnOnce()) -> usize {
    let before = ALLOCS.with(Cell::get);
    f();
    ALLOCS.with(Cell::get) - before
}

/// The instrument is tested too: a counter that always read 0 would make
/// every test below pass.
#[test]
fn counter_sees_allocations() {
    let n = allocations(|| {
        let v = vec![1.0f64; 16];
        std::hint::black_box(&v);
        let mut w: Vec<f64> = Vec::with_capacity(1);
        w.extend_from_slice(&v); // forces a realloc
        std::hint::black_box(&w);
    });
    assert!(n >= 2, "the counting allocator saw {n} allocations, expected at least 2");
}

fn oscillator(_t: f64, y: &[f64], dy: &mut [f64]) {
    dy[0] = y[1];
    dy[1] = -y[0];
}

#[test]
fn rk4_step_allocates_nothing() {
    let mut ws = Workspace::new(2);
    let mut y = [1.0, 0.0];
    let n = allocations(|| {
        for i in 0..10 {
            rk4_step(&mut oscillator, i as f64 * 0.1, &mut y, 0.1, &mut ws);
        }
    });
    assert_eq!(n, 0, "rk4_step allocated {n} times in 10 steps");
}

#[test]
fn dopri_step_allocates_nothing() {
    let mut ws = Workspace::new(2);
    let y = [1.0, 0.0];
    let n = allocations(|| {
        for _ in 0..10 {
            dopri_step(&mut oscillator, 0.0, &y, 0.1, &mut ws);
        }
    });
    assert_eq!(n, 0, "dopri_step allocated {n} times in 10 steps");
}

#[test]
fn integrate_allocates_nothing() {
    let mut ws = Workspace::new(2);
    let mut y = [1.0, 0.0];
    let opts = Options { atol: 1e-8, rtol: 1e-8, ..Options::default() };
    let n = allocations(|| {
        integrate(&mut oscillator, 0.0, 10.0, &mut y, &opts, &mut ws).unwrap();
    });
    assert_eq!(n, 0, "integrate allocated {n} times");
}

#[test]
fn helpers_allocate_nothing() {
    let mut out = [0.0; 3];
    let y = [1.0, 2.0, 3.0];
    let k = [1.0, 1.0, 1.0];
    let n = allocations(|| {
        lincomb(&mut out, &y, 0.5, &[(1.0, &k), (2.0, &k)]);
        std::hint::black_box(error_norm(&out, &y, &k, 1e-6, 1e-6));
        std::hint::black_box(next_step_size(0.1, 0.5, FAC_MAX));
    });
    assert_eq!(n, 0, "helpers allocated {n} times");
}
