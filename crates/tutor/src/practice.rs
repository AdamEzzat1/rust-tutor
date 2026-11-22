//! Practice problems: a submission is graded against the reference for
//! correctness (fixed and seeded random cases), compile errors (with the
//! error code), allocations (counted exactly by a global allocator) and
//! speed (a ratio to the reference in the same process; wide thresholds).

use crate::course;
use crate::snippet::{self, Outcome, Profile};
use crate::util::Res;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct Perf {
    /// Statements run once before timing; may use `rng: &mut Rng` and `n`.
    pub setup: String,
    /// The argument list for the timed call.
    pub call: String,
    pub n: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Random {
    pub count: usize,
    /// Statements that build one random case from `rng`.
    pub setup: String,
    pub call: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Variant {
    pub label: String,
    pub code: String,
    pub expect: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Problem {
    pub id: String,
    pub title: String,
    pub difficulty: String,
    pub kcs: Vec<String>,
    pub signature: String,
    pub entry: String,
    pub prompt: String,
    pub starter: String,
    pub reference: String,
    /// Argument lists, e.g. `"&[1.0, 2.0]"`.
    pub cases: Vec<String>,
    #[serde(default)]
    pub random: Option<Random>,
    /// "approx" (default) or "exact".
    #[serde(default)]
    pub compare: Option<String>,
    #[serde(default)]
    pub rtol: Option<f64>,
    #[serde(default)]
    pub atol: Option<f64>,
    #[serde(default)]
    pub hints: Vec<String>,
    #[serde(default)]
    pub perf: Option<Perf>,
    #[serde(default)]
    pub variant: Vec<Variant>,
    /// What the starter must get (default: ["incorrect"]).
    #[serde(default)]
    pub starter_expect: Option<Vec<String>>,
}

pub const SLOW_RATIO: f64 = 5.0;
pub const FAST_RATIO: f64 = 1.75;
/// `verify` re-measures a missing timed verdict this many times.
pub const PERF_RETRIES: usize = 2;

const HARNESS_PRELUDE: &str = r#"
#![allow(unused, dead_code)]
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { ALLOCS.fetch_add(1, Ordering::SeqCst); unsafe { System.alloc(l) } }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 { ALLOCS.fetch_add(1, Ordering::SeqCst); unsafe { System.realloc(p, l, n) } }
}
#[global_allocator]
static A: Counting = Counting;
fn allocs() -> usize { ALLOCS.load(Ordering::SeqCst) }

/// SplitMix64 (Steele, Lea & Flood, 2014): version-stable by construction.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self { Rng(seed) }
    pub fn u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1) with 53 random bits.
    pub fn f64(&mut self) -> f64 { (self.u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64) }
    pub fn range(&mut self, lo: usize, hi: usize) -> usize { lo + (self.u64() % (hi - lo) as u64) as usize }
    pub fn normal(&mut self) -> f64 {
        let u = 1.0 - self.f64();
        let v = self.f64();
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

pub trait Close { fn close(&self, o: &Self, rtol: f64, atol: f64) -> bool; fn show(&self) -> String; }
impl Close for f64 {
    fn close(&self, o: &Self, rtol: f64, atol: f64) -> bool {
        if self.is_nan() || o.is_nan() { return self.is_nan() && o.is_nan(); }
        if self.is_infinite() || o.is_infinite() { return self == o; }
        (self - o).abs() <= atol + rtol * self.abs().max(o.abs())
    }
    fn show(&self) -> String { format!("{self:?}") }
}
impl Close for f32 {
    fn close(&self, o: &Self, rtol: f64, atol: f64) -> bool { (*self as f64).close(&(*o as f64), rtol, atol) }
    fn show(&self) -> String { format!("{self:?}") }
}
macro_rules! exact { ($($t:ty),*) => { $(impl Close for $t {
    fn close(&self, o: &Self, _: f64, _: f64) -> bool { self == o }
    fn show(&self) -> String { format!("{self:?}") }
})* } }
exact!(usize, u8, u16, u32, u64, i8, i16, i32, i64, isize, bool, char, String, &str, (), std::cmp::Ordering);
impl<T: Close> Close for Vec<T> {
    fn close(&self, o: &Self, r: f64, a: f64) -> bool { self.len() == o.len() && self.iter().zip(o).all(|(x, y)| x.close(y, r, a)) }
    fn show(&self) -> String { let v: Vec<String> = self.iter().map(|x| x.show()).collect(); format!("[{}]", v.join(", ")) }
}
impl<T: Close, const N: usize> Close for [T; N] {
    fn close(&self, o: &Self, r: f64, a: f64) -> bool { self.iter().zip(o).all(|(x, y)| x.close(y, r, a)) }
    fn show(&self) -> String { let v: Vec<String> = self.iter().map(|x| x.show()).collect(); format!("[{}]", v.join(", ")) }
}
impl<T: Close> Close for Option<T> {
    fn close(&self, o: &Self, r: f64, a: f64) -> bool { match (self, o) { (Some(x), Some(y)) => x.close(y, r, a), (None, None) => true, _ => false } }
    fn show(&self) -> String { match self { Some(x) => format!("Some({})", x.show()), None => "None".into() } }
}
impl<T: Close, E: std::fmt::Debug> Close for Result<T, E> {
    fn close(&self, o: &Self, r: f64, a: f64) -> bool { match (self, o) { (Ok(x), Ok(y)) => x.close(y, r, a), (Err(x), Err(y)) => format!("{x:?}") == format!("{y:?}"), _ => false } }
    fn show(&self) -> String { match self { Ok(x) => format!("Ok({})", x.show()), Err(e) => format!("Err({e:?})") } }
}
impl<A: Close, B: Close> Close for (A, B) {
    fn close(&self, o: &Self, r: f64, a: f64) -> bool { self.0.close(&o.0, r, a) && self.1.close(&o.1, r, a) }
    fn show(&self) -> String { format!("({}, {})", self.0.show(), self.1.show()) }
}
impl<A: Close, B: Close, C: Close> Close for (A, B, C) {
    fn close(&self, o: &Self, r: f64, a: f64) -> bool { self.0.close(&o.0, r, a) && self.1.close(&o.1, r, a) && self.2.close(&o.2, r, a) }
    fn show(&self) -> String { format!("({}, {}, {})", self.0.show(), self.1.show(), self.2.show()) }
}

fn quiet<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|e| {
        e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into())
    })
}
"#;

/// A case is an argument list (`"&[1.0, 2.0]"`), or, if it mentions `F(`,
/// an expression template in which `F` is the function under test (for
/// in-place functions: `"{ let mut v = vec![1.0]; F(&mut v); v }"`).
pub fn call(module: &str, entry: &str, case: &str) -> String {
    if case.contains("F(") {
        case.replace("F(", &format!("{module}::{entry}("))
    } else {
        format!("{module}::{entry}({case})")
    }
}

// Harness templates: `@SUB@` / `@REF@` are the calls, `@I@` the case index.
const CASE_TEMPLATE: &str = r#"        { let want = @REF@; match quiet(|| @SUB@) {
            Ok(got) => println!("case @I@ {} got={} want={}", if got.close(&want, rtol, atol) { "ok" } else { "fail" }, got.show(), want.show()),
            Err(m) => println!("case @I@ panic {}", m.lines().next().unwrap_or("")),
        } }
"#;

const RANDOM_TEMPLATE: &str = r#"        for k in 0..@COUNT@ {
            let mut rng = Rng::new(0x5EED_0000 + k as u64);
            @SETUP@
            let want = @REF@;
            match quiet(|| @SUB@) {
                Ok(got) => if got.close(&want, rtol, atol) { println!("random {k} ok"); } else { println!("random {k} fail got={} want={}", got.show(), want.show()); },
                Err(m) => println!("random {k} panic {}", m.lines().next().unwrap_or("")),
            }
        }
"#;

const PERF_TEMPLATE: &str = r#"    if mode == "perf" {
        let n: usize = @N@;
        let mut rng = Rng::new(0x9E3779B9);
        @SETUP@
        let _ = std::hint::black_box(@SUB@);
        let _ = std::hint::black_box(@REF@);
        let a0 = allocs(); let _ = std::hint::black_box(@SUB@); let alloc_sub = allocs() - a0;
        let a0 = allocs(); let _ = std::hint::black_box(@REF@); let alloc_ref = allocs() - a0;
        // Calibrate repetitions so one sample of the reference takes >= 5 ms.
        let mut reps = 1usize;
        loop {
            let t = std::time::Instant::now();
            for _ in 0..reps { std::hint::black_box(@REF@); }
            if t.elapsed().as_millis() >= 5 || reps > 1 << 24 { break; }
            reps *= 2;
        }
        // Alternate submission and reference; compare medians of 7.
        let mut ts = vec![]; let mut tr = vec![];
        for _ in 0..7 {
            let t = std::time::Instant::now(); for _ in 0..reps { std::hint::black_box(@SUB@); } ts.push(t.elapsed().as_secs_f64());
            let t = std::time::Instant::now(); for _ in 0..reps { std::hint::black_box(@REF@); } tr.push(t.elapsed().as_secs_f64());
        }
        ts.sort_by(|a, b| a.total_cmp(b)); tr.sort_by(|a, b| a.total_cmp(b));
        println!("alloc_sub {alloc_sub}"); println!("alloc_ref {alloc_ref}"); println!("ratio {:.3}", ts[3] / tr[3]);
    }
"#;

/// Builds the harness: the submission and the reference in sibling modules.
/// Returns the source and the line range of the submission module.
pub fn harness(p: &Problem, submission: &str) -> (String, (usize, usize)) {
    let rtol = p.rtol.unwrap_or(if p.compare.as_deref() == Some("exact") { 0.0 } else { 1e-12 });
    let atol = p.atol.unwrap_or(0.0);
    let e = &p.entry;
    let mut s = String::from(HARNESS_PRELUDE);
    s.push_str("\nmod submission {\n    use super::Rng;\n");
    let start = s.lines().count() + 1;
    s.push_str(submission);
    let end = s.lines().count();
    s.push_str("\n}\n\nmod reference {\n    use super::Rng;\n");
    s.push_str(&p.reference);
    s.push_str("\n}\n\nfn main() {\n    std::panic::set_hook(Box::new(|_| {}));\n");
    s.push_str("    let mode = std::env::args().nth(1).unwrap_or_default();\n");
    s.push_str(&format!("    let (rtol, atol) = ({rtol:e}_f64, {atol:e}_f64);\n"));
    s.push_str("    if mode == \"cases\" {\n");
    for (i, c) in p.cases.iter().enumerate() {
        s.push_str(
            &CASE_TEMPLATE
                .replace("@REF@", &call("reference", e, c))
                .replace("@SUB@", &call("submission", e, c))
                .replace("@I@", &i.to_string()),
        );
    }
    if let Some(r) = &p.random {
        s.push_str(
            &RANDOM_TEMPLATE
                .replace("@COUNT@", &r.count.to_string())
                .replace("@SETUP@", &r.setup)
                .replace("@REF@", &call("reference", e, &r.call))
                .replace("@SUB@", &call("submission", e, &r.call)),
        );
    }
    s.push_str("    }\n");
    if let Some(pf) = &p.perf {
        s.push_str(
            &PERF_TEMPLATE
                .replace("@N@", &pf.n.to_string())
                .replace("@SETUP@", &pf.setup)
                .replace("@REF@", &call("reference", e, &pf.call))
                .replace("@SUB@", &call("submission", e, &pf.call)),
        );
    }
    s.push_str("}\n");
    (s, (start, end))
}

#[derive(Debug, Clone, Default)]
pub struct Grade {
    pub verdicts: Vec<String>,
    pub lines: Vec<String>,
    pub ratio: Option<f64>,
}

fn grade_once(p: &Problem, submission: &str, with_perf: bool) -> Res<Grade> {
    let mut g = Grade::default();
    let (src, (s0, s1)) = harness(p, submission);
    let exe = match snippet::compile(&src, Profile::Debug, &[])? {
        Ok(exe) => exe,
        Err(diags) => {
            let mine: Vec<_> = diags.iter().filter(|d| d.line >= s0 && d.line <= s1).collect();
            match mine.first() {
                Some(d) => {
                    let code = d.code.clone().unwrap_or_else(|| "error".into());
                    g.verdicts.push(format!("compile_error:{code}"));
                    g.lines.push(format!("does not compile: line {}: error[{code}]: {}", d.line + 1 - s0, d.message));
                    if let Some(n) = crate::explain::diagnostic_note(&code) {
                        g.lines.push(format!("  {code}: {n}"));
                    }
                }
                None => {
                    g.verdicts.push("compile_error:signature".into());
                    let d = &diags[0];
                    g.lines.push(format!(
                        "your code compiles on its own but does not fit the problem's signature `{}` ({}{})",
                        p.signature,
                        d.code.as_deref().map(|c| format!("[{c}] ")).unwrap_or_default(),
                        d.message
                    ));
                }
            }
            return Ok(g);
        }
    };
    let o = snippet::run_exe(&exe, &["cases"], Duration::from_secs(60))?;
    let Outcome::Ran { stdout, timed_out, .. } = o else { unreachable!() };
    let mut fails = vec![];
    let mut total = 0;
    for l in stdout.lines() {
        total += 1;
        if !l.contains(" ok") {
            fails.push(l.to_string());
        }
    }
    if timed_out {
        fails.push("timed out after 60 s".into());
    }
    if total == 0 && !timed_out {
        fails.push("the harness printed nothing (crashed?)".into());
    }
    if !fails.is_empty() {
        g.verdicts.push("incorrect".into());
        g.lines.push(format!("incorrect: {} of {} cases fail", fails.len(), total.max(fails.len())));
        for (i, f) in fails.iter().take(3).enumerate() {
            let shown = describe_case(p, f);
            g.lines.push(format!("  {}. {shown}", i + 1));
        }
        return Ok(g);
    }
    g.verdicts.push("correct".into());
    g.lines.push(format!("correct on {total} cases ({} fixed, {} seeded random)", p.cases.len(), p.random.as_ref().map_or(0, |r| r.count)));
    if with_perf && p.perf.is_some() {
        let exe = match snippet::compile(&src, Profile::Release, &[])? {
            Ok(e) => e,
            Err(_) => return Ok(g),
        };
        let o = snippet::run_exe(&exe, &["perf"], Duration::from_secs(120))?;
        if let Outcome::Ran { stdout, .. } = o {
            let kv: BTreeMap<&str, &str> = stdout.lines().filter_map(|l| l.split_once(' ')).collect();
            let a_sub: usize = kv.get("alloc_sub").and_then(|v| v.parse().ok()).unwrap_or(0);
            let a_ref: usize = kv.get("alloc_ref").and_then(|v| v.parse().ok()).unwrap_or(0);
            let ratio: f64 = kv.get("ratio").and_then(|v| v.parse().ok()).unwrap_or(f64::NAN);
            g.ratio = Some(ratio);
            g.lines.push(format!("allocations per call: {a_sub} (reference {a_ref})  [counted, exact]"));
            if a_sub > a_ref && (a_ref == 0 || a_sub >= 2 * a_ref) {
                g.verdicts.push("allocates".into());
                g.lines.push("  allocates: more heap allocations than the reference".into());
            }
            g.lines.push(format!("time per call: {ratio:.2}x the reference  [timed, median of 7; varies by machine]"));
            if ratio > SLOW_RATIO {
                g.verdicts.push("slow".into());
                g.lines.push(format!("  slow: more than {SLOW_RATIO}x the reference"));
            } else if ratio <= FAST_RATIO {
                g.verdicts.push("fast".into());
            }
        }
    }
    Ok(g)
}

fn describe_case(p: &Problem, line: &str) -> String {
    // "case 3 fail got=.. want=.." -> show the argument list
    let mut parts = line.splitn(3, ' ');
    let kind = parts.next().unwrap_or("");
    let idx: usize = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let rest = parts.next().unwrap_or("");
    let arg = if kind == "case" { p.cases.get(idx).cloned().unwrap_or_default() } else { format!("random case #{idx} (seed 0x5EED_{idx:04})") };
    let arg = if arg.len() > 80 { format!("{}...", &arg[..77]) } else { arg };
    format!("{}({arg}): {rest}", p.entry)
}

pub fn grade(p: &Problem, submission: &str) -> Res<Grade> {
    grade_once(p, submission, true)
}

pub fn find(id: &str) -> Res<(course::Unit, Problem)> {
    for u in course::load_units()? {
        if let Some(p) = u.problem.iter().find(|p| p.id == id) {
            return Ok((u.clone(), p.clone()));
        }
    }
    Err(format!("no practice problem {id} (see `tutor practice list`)"))
}

pub fn format_list() -> Res<String> {
    let mut s = String::new();
    for u in course::load_units()? {
        for p in &u.problem {
            s.push_str(&format!("{:<28} {:<9} {}  [{}]\n", p.id, p.difficulty, p.title, u.id));
        }
    }
    Ok(s)
}

pub fn format_problem(id: &str) -> Res<String> {
    let (u, p) = find(id)?;
    Ok(format!(
        "== {} ({}, unit {}) ==\n\n{}\n\nSignature: {}\n\nStarter:\n```rust\n{}\n```\nGrade with: tutor practice {} <file.rs>\n",
        p.title,
        p.difficulty,
        u.id,
        p.prompt.trim(),
        p.signature,
        p.starter.trim_end(),
        p.id
    ))
}

pub fn grade_submission(id: &str, code: &str) -> Res<(String, bool)> {
    let (_, p) = find(id)?;
    let g = grade(&p, code)?;
    let mut s = format!("== {} ==\n", p.title);
    for l in &g.lines {
        s.push_str(l);
        s.push('\n');
    }
    let ok = g.verdicts.iter().any(|v| v == "correct");
    if !ok && !p.hints.is_empty() {
        s.push_str(&format!("hint: {}\n", p.hints[0]));
    }
    s.push_str(&format!("verdicts: {}\n", g.verdicts.join(", ")));
    Ok((s, ok))
}

/// Does `g` carry every expected verdict? (A timed verdict may be retried.)
pub fn missing(expect: &[String], g: &Grade) -> Vec<String> {
    expect.iter().filter(|e| !g.verdicts.contains(e)).cloned().collect()
}

/// Grade for `verify`: re-measures up to PERF_RETRIES times when only a
/// timed verdict is missing. Returns the grade and a log of retries.
pub fn grade_for_verify(p: &Problem, code: &str, expect: &[String]) -> Res<(Grade, Vec<String>)> {
    let mut g = grade(p, code)?;
    let mut notes = vec![];
    for k in 0..PERF_RETRIES {
        let m = missing(expect, &g);
        let timed_only = !m.is_empty() && m.iter().all(|v| v == "slow" || v == "fast");
        if !timed_only {
            break;
        }
        notes.push(format!("retry {} for {:?} (ratio {:.2})", k + 1, m, g.ratio.unwrap_or(f64::NAN)));
        g = grade(p, code)?;
    }
    Ok((g, notes))
}
