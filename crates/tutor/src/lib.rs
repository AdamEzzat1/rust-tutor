//! rust-tutor: a symbolic, deterministic tutor for Rust in scientific
//! computing. Behavioural tests define correctness, a static checker on `syn`
//! trees flags misconceptions, measurements (allocation counts, convergence
//! fits, compiler error codes) are preferred to inference, and the explain
//! step joins them with a measured test-to-hole map. No LLM in the loop.

pub mod checker;
pub mod course;
pub mod explain;
pub mod fixtures;
pub mod holemap;
pub mod lessons;
pub mod models;
pub mod practice;
pub mod progress;
pub mod rules;
pub mod runner;
pub mod scaffold;
pub mod snippet;
pub mod util;
