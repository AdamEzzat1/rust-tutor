//! Rule cases. Invariant 1 (style tolerance): for every rule, at least two
//! different correct styles produce zero findings from *any* rule.

use tutor::checker::{self, check_source, Severity};

fn ids(path: &str, src: &str) -> Vec<&'static str> {
    match check_source(path, src) {
        Ok(f) => f.into_iter().map(|f| f.rule).collect(),
        Err(_) => panic!("case does not parse:\n{src}"),
    }
}

fn fires(rule: &str, src: &str) {
    let got = ids("src/lib.rs", src);
    assert!(got.contains(&rule), "{rule} should fire on:\n{src}\ngot {got:?}");
}

fn silent(rule: &str, src: &str) {
    let got = ids("src/lib.rs", src);
    assert!(!got.contains(&rule), "{rule} should not fire on:\n{src}\ngot {got:?}");
}

fn clean(src: &str) {
    let got = ids("src/lib.rs", src);
    assert!(got.is_empty(), "a correct style should give no findings:\n{src}\ngot {got:?}");
}

// RT001 ------------------------------------------------------------------

#[test]
fn rt001_positive() {
    fires("RT001", "#[test] fn t() { let x = 0.1 + 0.2; assert_eq!(x, 0.3); }");
    fires("RT001", "#[test] fn t() { let x = f(); assert!(x == 1.5); }");
    fires("RT001", "#[test] fn t() { assert_ne!(g(), f64::NAN); }");
    fires("RT001", "fn t() { debug_assert_eq!(n as f64, y); }");
}

#[test]
fn rt001_negative() {
    silent("RT001", "#[test] fn t() { assert_eq!(count(), 3); }");
    silent("RT001", "#[test] fn t() { assert!(x.to_bits() == 2.0f64.to_bits()); }");
    silent("RT001", "fn f(e: f64) -> f64 { if e == 0.0 { 1.0 } else { e } }"); // not an assertion
}

#[test]
fn rt001_styles() {
    clean("#[test] fn t() { let x = 0.1 + 0.2; assert!((x - 0.3).abs() <= 4.0 * f64::EPSILON * 0.3); }");
    clean("#[test] fn t() { let x: f64 = 0.5 * 4.0; assert!(x.to_bits() == 2.0f64.to_bits(), \"exact by construction\"); }");
}

// RT002 ------------------------------------------------------------------

#[test]
fn rt002_positive() {
    fires("RT002", "fn s(x: &[f64]) -> f64 { let mut t = 0.0; for i in 0..x.len() { t += x[i]; } t }");
    fires("RT002", "fn a(o: &mut [f64], x: &[f64]) { for i in 0..o.len() { o[i] = 2.0 * x[i]; } }");
}

#[test]
fn rt002_negative() {
    // i used in arithmetic: the index itself matters.
    silent("RT002", "fn d(o: &mut [f64], x: &[f64]) { for i in 0..o.len() { o[i] = x[i + 1] - x[i]; } }");
    // three slices: exempt (a three-way zip is not clearer).
    silent("RT002", "fn s(o: &mut [f64], a: &[f64], b: &[f64]) { for i in 0..o.len() { o[i] = a[i] * b[i]; } }");
    // i printed: used outside indexing.
    silent("RT002", "fn p(x: &[f64]) { for i in 0..x.len() { println!(\"{} {}\", i, x[i]); } }");
    // test code is skipped.
    silent("RT002", "#[test] fn t() { let x = [1.0]; for i in 0..x.len() { let _ = x[i]; } }");
}

#[test]
fn rt002_styles() {
    clean("fn s(x: &[f64]) -> f64 { x.iter().sum() }");
    clean("fn a(o: &mut [f64], x: &[f64]) { for (o, v) in o.iter_mut().zip(x) { *o = 2.0 * v; } }");
}

// RT003 ------------------------------------------------------------------

#[test]
fn rt003_positive() {
    fires("RT003", "fn f(n: usize) { for _ in 0..n { let t = vec![0.0; 8]; use_it(&t); } }");
    fires("RT003", "fn f(x: &[f64]) { loop { let y = x.to_vec(); if g(&y) { break; } } }");
    fires("RT003", "fn f(x: &[f64]) { while go() { let y: Vec<f64> = x.iter().map(|v| v * 2.0).collect(); g(&y); } }");
}

#[test]
fn rt003_negative() {
    silent("RT003", "fn f(n: usize) -> Vec<f64> { let t = vec![0.0; n]; for _ in 0..n { g(&t); } t }");
    silent("RT003", "fn f(n: usize) { for _ in 0..n { let t: Vec<f64> = Vec::new(); g(&t); } }"); // does not allocate
    silent("RT003", "fn f(x: &[f64]) { for v in x.to_vec() { g(v); } }"); // the iterable runs once
    silent("RT003", "#[test] fn t() { for _ in 0..3 { let v = vec![1, 2]; assert_eq!(v.len(), 2); } }");
}

#[test]
fn rt003_styles() {
    clean("fn f(x: &[f64], n: usize) { let mut t = vec![0.0; x.len()]; for _ in 0..n { t.copy_from_slice(x); g(&t); } }");
    clean("struct W { t: Vec<f64> } fn f(w: &mut W, x: &[f64], n: usize) { for _ in 0..n { w.t.clear(); w.t.extend_from_slice(x); g(&w.t); } }");
}

// RT004 ------------------------------------------------------------------

#[test]
fn rt004_positive() {
    fires("RT004", "fn s(v: &mut [f64]) { v.sort_by(|a, b| a.partial_cmp(b).unwrap()); }");
    fires("RT004", "fn m(v: &[f64]) -> f64 { *v.iter().max_by(|a, b| a.partial_cmp(b).expect(\"no NaN\")).unwrap() }");
}

#[test]
fn rt004_negative() {
    silent("RT004", "fn s(v: &mut [f64]) { v.sort_by(|a, b| a.total_cmp(b)); }");
    silent("RT004", "fn c(a: f64, b: f64) -> bool { a.partial_cmp(&b).is_some() }");
}

#[test]
fn rt004_styles() {
    clean("fn s(v: &mut [f64]) { v.sort_by(|a, b| a.total_cmp(b)); }");
    clean("fn m(v: &[f64]) -> f64 { v.iter().copied().fold(f64::NEG_INFINITY, f64::max) }");
}

// RT005 ------------------------------------------------------------------

#[test]
fn rt005_positive() {
    fires("RT005", "fn eq(a: f64, b: f64) -> bool { (a - b).abs() < f64::EPSILON }");
    fires("RT005", "fn eq(a: f32, b: f32) -> bool { std::f32::EPSILON >= (a - b).abs() }");
}

#[test]
fn rt005_negative() {
    silent("RT005", "fn eq(a: f64, b: f64) -> bool { (a - b).abs() <= 4.0 * f64::EPSILON * a.abs().max(b.abs()) }");
    silent("RT005", "fn small(h: f64) -> bool { h < f64::EPSILON }"); // no abs: a step-size floor
}

#[test]
fn rt005_styles() {
    clean("fn eq(a: f64, b: f64) -> bool { (a - b).abs() <= 1e-12 + 1e-9 * a.abs().max(b.abs()) }");
    clean("fn eq(a: f64, b: f64, tol: f64) -> bool { (a - b).abs() <= tol }");
}

// RT006 ------------------------------------------------------------------

#[test]
fn rt006_positive() {
    fires("RT006", "fn s(a: &[f64], n: usize) -> f64 { let mut t = 0.0; for j in 0..n { for i in 0..n { t += a[i * n + j]; } } t }");
    fires("RT006", "fn s(a: &[Vec<f64>], n: usize) -> f64 { let mut t = 0.0; for j in 0..n { for i in 0..n { t += a[i][j]; } } t }");
}

#[test]
fn rt006_negative() {
    // column-major storage traversed down columns: unit stride.
    silent("RT006", "fn s(a: &[f64], n: usize) -> f64 { let mut t = 0.0; for j in 0..n { for i in 0..n { t += a[i + j * n]; } } t }");
    // single loop
    silent("RT006", "fn s(a: &[f64], n: usize) -> f64 { let mut t = 0.0; for i in 0..n { t += a[i * n]; } t }");
}

#[test]
fn rt006_styles() {
    clean("fn s(a: &[f64], n: usize) -> f64 { let mut t = 0.0; for i in 0..n { for j in 0..n { t += a[i * n + j]; } } t }");
    clean("fn s(a: &[f64], n: usize) -> f64 { a.chunks_exact(n).map(|row| row.iter().sum::<f64>()).sum() }");
}

// RT007 ------------------------------------------------------------------

#[test]
fn rt007_positive() {
    fires("RT007", "fn parse(s: &str) -> Result<f64, String> { let v: f64 = s.parse().unwrap(); Ok(v) }");
    fires("RT007", "impl X { fn go(&self) -> std::io::Result<()> { let f = std::fs::read(\"a\").expect(\"read\"); Ok(()) } }");
}

#[test]
fn rt007_negative() {
    silent("RT007", "fn parse(s: &str) -> f64 { s.parse().unwrap() }"); // no Result to propagate into
    silent("RT007", "#[cfg(test)] mod t { fn p() -> Result<(), String> { let _ = \"1\".parse::<i32>().unwrap(); Ok(()) } }");
}

#[test]
fn rt007_styles() {
    clean("fn parse(s: &str) -> Result<f64, String> { let v: f64 = s.parse().map_err(|e| format!(\"{e}\"))?; Ok(v) }");
    clean("fn parse(s: &str) -> Result<f64, std::num::ParseFloatError> { s.trim().parse() }");
}

// Rule metadata ----------------------------------------------------------

#[test]
fn every_rule_is_well_formed() {
    let rules = checker::all_rules();
    let mut seen = std::collections::BTreeSet::new();
    for r in &rules {
        let m = r.meta();
        assert!(seen.insert(m.id), "duplicate rule id {}", m.id);
        assert!(m.fp_risk.len() > 40, "{}: write an honest false-positive note", m.id);
        assert!(!m.measured_by.is_empty(), "{}: say what measures it, or that nothing does", m.id);
        assert!(!m.answer_tokens.is_empty(), "{}: list the answer tokens", m.id);
        for t in m.answer_tokens {
            assert!(!m.hints[0].contains(t), "{}: level-1 hint contains answer token {t:?}", m.id);
        }
        assert!(m.hints[0].trim_end().ends_with('?'), "{}: level-1 hint must be a question", m.id);
    }
    assert_eq!(rules.len(), 7);
}

#[test]
fn advice_never_changes_the_exit_code() {
    let src = "fn s(x: &[f64]) -> f64 { let mut t = 0.0; for i in 0..x.len() { t += x[i]; } t }";
    let findings = check_source("src/lib.rs", src).ok().unwrap();
    assert!(findings.iter().all(|f| f.severity == Severity::Advice) && !findings.is_empty());
    let r = checker::Report { findings, parse_errors: vec![] };
    assert_eq!(checker::exit_code(&r), 0);
}

#[test]
fn output_is_deterministic_and_sorted() {
    let src = "fn f(v: &mut [f64], n: usize) { for _ in 0..n { let t = v.to_vec(); v.sort_by(|a, b| a.partial_cmp(b).unwrap()); } }";
    let a = check_source("src/lib.rs", src).ok().unwrap();
    let b = check_source("src/lib.rs", src).ok().unwrap();
    assert_eq!(a, b);
    let mut sorted = a.clone();
    sorted.sort();
    assert_eq!(a, sorted);
}

#[test]
fn files_under_tests_are_test_code() {
    let src = "fn helper(x: &[f64]) -> f64 { let mut t = 0.0; for i in 0..x.len() { t += x[i]; } t }";
    assert!(ids("tests/behaviour.rs", src).is_empty());
    assert!(!ids("src/lib.rs", src).is_empty());
}
