# rust-tutor

A symbolic, deterministic tutor for scientific computing in Rust, with no LLM
in the feedback loop. Behavioural tests define correctness. A static checker
(on `syn` syntax trees) flags numerical-code misconceptions, and the compiler's
own errors are first-class feedback. An `explain` step joins four sources to say
where to start: test failures, compile errors, measurements (exact allocation
counts, step counts) and a measured test-to-hole map.

It is the Rust counterpart of [julia-tutor](https://github.com/AdamEzzat1/julia-tutor),
with the same contracts: holes checked by mutation testing, Ochiai ranking on
a measured hole map, honest false-positive notes for every rule, lesson keys
verified on a pinned compiler, practice problems graded against a reference
whose own grader passes its own tests.

| Course | Status |
|---|---|
| Core Rust for scientific computing | 2 of 8 units ready (ownership and borrowing; traits and generics) |
| Scientific computing in Rust | 2 of 10 units ready (floating point; memory layout) |

"Ready" means the unit has recorded outputs and graders that pass their own
self-tests. It says nothing yet about how well the units teach, because no
learner has taken one. The remaining units are planned stubs with outlines
(`tutor courses`).

The project is **odeint**: an allocation-free explicit ODE integrator (RK4
and Dormand–Prince 5(4) with an adaptive step controller and typed errors),
with seven holes over two stages. The tests check its convergence order
(fitted: RK4 4.03, DP5 5.005) and that the step controller is proportional to
the tolerance. They also check that every kernel performs zero heap
allocations, counted by a global allocator.

```bash
rustup toolchain install 1.97.1          # rust-toolchain.toml pins it; tools refuse any other
cargo build --release
target/release/tutor start odeint 1 work/s1
target/release/tutor explain work/s1 --hints=1
target/release/tutor lesson L02
target/release/tutor practice own-prefix-sum my_answer.rs
scripts/verify_all.sh                    # everything below, as CI runs it
```

* `CLAUDE.md`: roles, invariants, milestones, known gaps, how to continue.
* `docs/DESIGN.md`: why odeint, why `syn`, what rustc and clippy already give
  and what this adds, and what is not stable.
* `course/AUTHORING.md`: how to write a unit whose every claim is measured.

**Safety: there is no sandbox yet.** `tutor test`, `explain`, `practice` and
`run` compile and run code as plain processes with your permissions. Only run
code you wrote yourself. A container sandbox like julia-tutor's is milestone
M4 (`CLAUDE.md`).
