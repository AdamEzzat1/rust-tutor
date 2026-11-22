# Design

## The four questions (as in julia-tutor)

1. **What is correct?** The behavioural tests in
   `projects/odeint/reference/tests/`. Never the code's shape.
2. **What is wrong, and where?** Failing tests, plus rustc's errors, plus a
   *measured* map from tests to holes (`holemap.toml`: each hole is opened alone
   and the failing tests recorded). Ochiai ranks the holes.
3. **What is likely misunderstood?** Checker rules tied to knowledge components,
   compile-error notes tied to lessons, misconception-tied distractors.
4. **What should happen next?** Fading hints, BKT per knowledge component
   (updated only on unambiguous evidence), SM-2 reviews of lessons.

## Does Rust do scientific computing?

Rust does scientific computing, though the ecosystem is younger than Julia's or
Python's. The crates exist (ndarray, nalgebra, faer, and ODE crates), but no
single stack dominates. That decided the course shape. The core and scicomp
units build numerical kernels **std-only**, where Rust's guarantees are the
lesson:

* `&mut` is exclusive, so in-place kernels cannot alias (the noalias property C
  gets from `restrict`);
* preallocated workspaces with split borrows make allocation-free inner loops
  provable, which a counting allocator then checks;
* `Result` turns numerical failure (step too small, non-finite state) into a
  typed value the caller must handle;
* overflow panics in debug builds and wraps in release builds, which
  lessons L04 and unit 01 measure.

The ecosystem gets a survey unit at the end (39, planned). Its claims stay
labelled unverified until they are measured here.

## Why odeint

An ODE integrator teaches the Rust that numerical code needs: slices and
borrows (`lincomb`), split borrows of a workspace (`rk4_step`, `dopri_step` via
destructuring and `split_at_mut`), `FnMut` right-hand sides, iterator adaptors
(`error_norm`), and `Result` with a typed error (`integrate`). Its correctness
is unusually *measurable*:

* convergence order is a fitted slope (RK4 4.03, DP5 5.005);
* the controller's global error is proportional to the tolerance (slope 1.02);
* step and evaluation counts are exact integers (`Stats`);
* allocations per call are exact integers (0).

Each of these turns a vague failure ("the answer is a bit off") into one that
names a hole. A buggy `next_step_size` exponent shows up as a wrong
proportionality slope, and the explain fixture for it is golden-tested.

## What rustc and clippy already give, and what this adds

rustc is already an excellent tutor for ownership, and its diagnostics are part
of the feedback here, not something to replace. `explain` shows the error code,
the hole containing the first error, and a one-line meaning that links to a
lesson (`explain::diagnostic_note`). Clippy has hundreds of lints. The rules
here are few and numerical:

| Rule | What | Clippy overlap |
|---|---|---|
| RT001 | float `==` in assertions | `float_cmp` (pedantic, typed): RT001 is narrower, syntax-only, and on in tests |
| RT002 | `for i in 0..x.len()` used only to index | `needless_range_loop`: RT002 exempts 3+ indexed slices, where indices are clearer |
| RT003 | allocation inside a loop | none |
| RT004 | `partial_cmp(..).unwrap()` | none (points to `total_cmp`) |
| RT005 | `f64::EPSILON` as an absolute tolerance | none |
| RT006 | strided inner loop | none |
| RT007 | `unwrap` in a function returning `Result` | `unwrap_used` (restriction, everywhere) |

What this tutor adds that neither tool has: tests as the specification, the
measured hole map, mutation-tested holes, explain's ranking, allocation and
convergence measurements, and the learner model.

## Dependencies

* **odeint: none.** Learners should be able to read everything they link.
* **The tutor:**
  * `syn` (full, visit), `proc-macro2` (span-locations) and `quote`: Rust's
    de-facto parser, the only robust way to find holes and match rules on trees
    rather than text (the julia-tutor rule "match on tree kinds" needs a tree);
  * `toml` and `serde`: every data file is TOML.

  All are pinned by `Cargo.lock`.
* Not used:
  * rust-analyzer and rustc's own HIR, because neither has a stable API to link
    against;
  * `cargo test --format json`, which is nightly-only, so libtest's stable
    human output is parsed instead and thread ids are stripped.

## Measurements, and what is not stable

* **Allocation counts are deterministic.** They come from a counting
  `#[global_allocator]` in the odeint tests and in the practice harness, so
  they are verdicts.
* **Timings are not.**
  * They are only ratios to the reference in the same process, using the
    median of 7 runs, with two re-measurements on a borderline result.
  * They are never asserted in tests, and are labelled machine-dependent.
  * `fast` means ≤ 1.75× the reference and `slow` means > 5×.
* **Pinned, not stable:**
  * rustc's error codes and messages;
  * `{:?}` float formatting;
  * which code LLVM vectorises.

  Lesson keys and recorded outputs depend on them, so a toolchain upgrade is
  its own milestone (CLAUDE.md, invariant 4).
* **Release vs debug.** Practice grading checks correctness in the debug
  profile (opt-level 0, overflow checks and debug assertions on, as
  `cargo test` does), so a `usize` underflow panics and is graded incorrect.
  It times in the release profile (opt-level 3, no checks, as
  `cargo build --release` does). Lessons and examples that teach overflow
  choose their profile explicitly.

## Known differences from julia-tutor

* **No sandbox, website, guided projects, programs or case studies yet.**
  These are milestones M4 and M5. Until the sandbox lands, nothing accepts code
  from anyone but the local user.
* **No type-stability material.** In Rust, monomorphisation makes type
  stability a compile-time property rather than a measurement, so the JET and
  `@inferred` material has no equivalent. The counterpart lessons are about
  borrows, `Copy` versus move, and overflow.
