# Authoring a unit

A unit is `course/units/<id>.toml` and belongs to exactly one course manifest
(`course/courses/<course>.toml`). Planned units hold only `status = "planned"`,
a summary and an outline. A unit is **ready** when `tutor course verify <id>`
passes and every check below has been done.

## Workflow

1. Write the unit (sections, examples, analogies, questions, problems,
   references). The ready units are worked examples.
2. Run `tutor course record <id>`. It compiles and runs every example on the
   pinned rustc and writes `course/outputs/<id>.toml`. Never edit that file.
3. **Read every recorded output against its `notice`.** The recorder cannot
   check prose: a notice that says "the naive formula loses digits" must be
   true of the output actually recorded. While the four ready units were
   written, this caught two claims:
   * integer data near 1e6 made the naive variance *exact*;
   * a quadratic made a central difference exact.

   Both examples were changed, rather than the prose kept.
4. Run `tutor course verify <id>`. It re-runs every example and fails if any
   key changed. It also checks:
   * that every `output` question's answer matches the measured output (this
     caught a wrong key in unit 30);
   * that the correct option is not the longest one in most questions (length
     bias);
   * that the reference is `correct`;
   * that the starter, and every variant, gets the verdicts it declares (this
     caught an E0503 expected as E0502).
5. Check every reference: fetch it, and confirm that it says what the citing
   sentence claims. This is not yet done for any unit (CLAUDE.md, Known gaps).

## Rules for content

* **Examples** are programs or bare snippets (wrapped in `fn main`). They set
  `profile = "debug"` when overflow checks matter. Their output must be
  deterministic: no timings, addresses or HashMap order. Pointer differences
  are fine; absolute addresses are not.
* **Every analogy has a `breaks`** that names where it fails.
* **Questions:**
  * `output` questions are measured; `choice` questions are argued in the
    `explanation`;
  * every wrong option has a misconception id and feedback that says what is
    wrong, not just that it is wrong.
* **Problems:**
  * `pub fn` with the declared `signature`.
  * A problem whose return type the learner should not define (an error
    enum, say) puts it in `provided`. It is compiled once and shared by the
    submission and the reference, so their results have the same type and
    can be compared, and it is shown with the problem. Errors are compared
    by their `Debug` text, so give error variants integer or unit fields,
    not floats that two correct versions may round differently (unit 03).
  * Cases are argument lists, or templates containing `F(` that the harness
    calls, e.g. `"{ let mut v = vec![1.0]; F(&mut v); v }"`.
  * To grade how a closure argument is used, a template can return the
    result together with a count the closure keeps:
    `"{ let mut calls = 0usize; let r = F(|x: f64| { calls += 1; x * x }, 0.0, 1.0, 4); (r, calls) }"`.
    Counts compare exactly (unit 04, `clo-trapezoid`).
  * `random.setup` may use `rng.f64()`, `rng.range(lo, hi)` and `rng.normal()`
    (SplitMix64, stable by construction).
  * `perf` declares `setup`, `call` and `n`; the harness times the call.
* **Tolerances are derived in the prompt.** Give the error bound with the
  numbers that apply to the test data, and show that each wrong variant
  exceeds it.
* **Variants must fail the way their label says.** If a label promises a
  panic, a case must trigger it. Unit 31 had to add a zero-column case for
  this.
* **Timed verdicts:**
  * declare `slow` or `fast` only after measuring several runs, and record
    the numbers in a comment next to the claim;
  * if a claim flips on another machine, drop it rather than widen the
    threshold;
  * allocation verdicts (`allocates`) are exact and need no such care.
* **Compile-error verdicts** use the code rustc actually emits
  (`compile_error:E0503`). The code is measured, not predicted.
