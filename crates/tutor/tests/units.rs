//! Models on hand-computed values; scaffolder and runner invariants.

use tutor::models::*;
use tutor::{runner, scaffold, snippet};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12
}

#[test]
fn bkt_by_hand() {
    // p = 0.2, correct: 0.2*0.9 / (0.18 + 0.8*0.2) = 0.18/0.34 = 0.529411...;
    // then + (1 - 0.529411) * 0.15 = 0.6.
    let p = bkt_update(CODE_BKT, 0.2, true);
    assert!(close(p, 0.18 / 0.34 + (1.0 - 0.18 / 0.34) * 0.15), "{p}");
    assert!(close(p, 0.6), "{p}");
    // incorrect: 0.2*0.1 / (0.02 + 0.8*0.8) = 0.02/0.66; + (1 - that) * 0.15.
    let q = bkt_update(CODE_BKT, 0.2, false);
    let post = 0.02 / 0.66;
    assert!(close(q, post + (1.0 - post) * 0.15), "{q}");
}

#[test]
fn sm2_by_hand() {
    let s = sm2_review(Sm2::default(), 5);
    assert_eq!((s.repetitions, s.interval_days), (1, 1));
    assert!(close(s.easiness, 2.6));
    let s = sm2_review(s, 5);
    assert_eq!((s.repetitions, s.interval_days), (2, 6));
    let s2 = sm2_review(s, 4);
    // interval = round(6 * 2.7) = 16; easiness 2.7 + (0.1 - 1*(0.08+0.02)) = 2.7
    assert_eq!(s2.interval_days, 16);
    assert!(close(s2.easiness, 2.7));
    let s3 = sm2_review(s2, 1);
    assert_eq!((s3.repetitions, s3.interval_days), (0, 1));
    assert!(s3.easiness >= 1.3);
}

#[test]
fn ochiai_by_hand() {
    assert!(close(ochiai(1, 0, 1), 1.0));
    assert!(close(ochiai(1, 8, 1), 1.0 / 3.0));
    assert!(close(ochiai(0, 5, 3), 0.0));
    assert!(close(ochiai(0, 0, 0), 0.0));
    assert_eq!(fmt2(-0.0001), "0.00");
}

const SRC: &str = r#"
// HOLE: add
/// Adds.
/// Second doc line.
pub fn add(a: i32, b: i32) -> i32 {
    let s = a + b;
    s
}

pub fn keep() -> i32 { 1 }

// HOLE: neg
/// Negates.
pub fn neg<T: std::ops::Neg<Output = T>>(x: T) -> T
where
    T: Copy,
{
    -x
}
"#;

#[test]
fn opened_hole_contains_only_todo_and_its_doc() {
    let p = scaffold::Project {
        project: "t".into(),
        crate_name: "t".into(),
        file: "src/lib.rs".into(),
        stages: vec![1],
        hole: vec![
            scaffold::Hole { id: "add".into(), stage: 1, kc: "k".into(), summary: "s".into(), trivial: vec![], measures: vec![] },
            scaffold::Hole { id: "neg".into(), stage: 1, kc: "k".into(), summary: "s".into(), trivial: vec![], measures: vec![] },
        ],
    };
    let out = scaffold::starter_source(&p, SRC, 1).unwrap();
    assert!(!out.contains("a + b"), "answer leaked:\n{out}");
    assert!(!out.contains("-x"), "answer leaked:\n{out}");
    assert!(out.contains("/// Adds.\n/// Second doc line.\npub fn add"));
    assert!(out.contains("pub fn keep() -> i32 { 1 }"), "non-hole code must be untouched");
    assert_eq!(scaffold::hole_body(&out, "add").unwrap(), "todo!(\"TODO(hole:add)\")");
    assert!(scaffold::is_open(&out, "neg"));
    // Replacing is reversible for a body taken from the source.
    let body = scaffold::hole_body(SRC, "add").unwrap();
    let back = scaffold::replace_body(&out, "add", &body).unwrap();
    assert_eq!(scaffold::hole_body(&back, "add").unwrap(), body);
    assert!(syn::parse_file(&out).is_ok());
}

#[test]
fn marker_must_sit_above_a_fn() {
    let bad = "// HOLE: x\n\npub fn x() {}\n";
    assert!(scaffold::find_holes(bad).is_err());
}

#[test]
fn libtest_output_parses_deterministically() {
    let stderr = "   Compiling odeint v0.1.0\n     Running unittests src\\lib.rs (target\\debug\\deps\\odeint-1.exe)\n     Running tests\\behaviour.rs (target\\debug\\deps\\behaviour-2.exe)\n";
    let stdout = "\nrunning 0 tests\n\ntest result: ok. 0 passed\n\nrunning 2 tests\ntest rk4::order_is_four ... FAILED\ntest norm::hand_computed ... ok\n\nfailures:\n\n---- rk4::order_is_four stdout ----\n\nthread 'rk4::order_is_four' (24852) panicked at tests\\behaviour.rs:146:9:\nfitted order 2.0\nnote: run with `RUST_BACKTRACE=1`\n\nfailures:\n    rk4::order_is_four\n";
    let r = runner::parse(stdout, stderr);
    assert_eq!(r.status, runner::Status::Ran);
    let ids: Vec<&str> = r.tests.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["behaviour::norm::hand_computed", "behaviour::rk4::order_is_four"]);
    let f = &r.tests[1];
    assert!(!f.passed);
    assert_eq!(f.location.as_deref(), Some("tests/behaviour.rs:146"));
    assert_eq!(f.message, vec!["fitted order 2.0".to_string()]);
}

#[test]
fn compile_errors_parse_with_codes() {
    let stderr = "src\\lib.rs:181:14: error[E0499]: cannot borrow `ws.k[_]` as mutable more than once at a time\nsrc\\lib.rs:3:1: warning: unused\nerror: could not compile `odeint` (lib) due to 1 previous error\n";
    let r = runner::parse("", stderr);
    match r.status {
        runner::Status::CompileError(d) => {
            assert_eq!(d.len(), 1);
            assert_eq!((d[0].file.as_str(), d[0].line, d[0].col), ("src/lib.rs", 181, 14));
            assert_eq!(d[0].code.as_deref(), Some("E0499"));
        }
        s => panic!("{s:?}"),
    }
}

#[test]
fn outcome_keys() {
    let o = snippet::Outcome::Ran { stdout: "3\n".into(), stderr: String::new(), exit: Some(0), panic: None, timed_out: false };
    assert_eq!(o.key(), "3");
    let o = snippet::Outcome::Ran {
        stdout: String::new(),
        stderr: "thread 'main' (7) panicked at snippet.rs:2:5:\nattempt to add with overflow\n".into(),
        exit: Some(101),
        panic: snippet::panic_message("thread 'main' (7) panicked at snippet.rs:2:5:\nattempt to add with overflow\n"),
        timed_out: false,
    };
    assert_eq!(o.key(), "panic: attempt to add with overflow");
    assert_eq!(snippet::as_program("let x = 1;"), "fn main() {\nlet x = 1;\n}\n");
    assert_eq!(snippet::as_program("fn main() {}"), "fn main() {}");
}

#[test]
fn fnv_is_stable() {
    // Published FNV-1a 64-bit test vectors.
    assert_eq!(tutor::util::fnv1a(""), 0xcbf29ce484222325);
    assert_eq!(tutor::util::fnv1a("a"), 0xaf63dc4c8601ec8c);
}
