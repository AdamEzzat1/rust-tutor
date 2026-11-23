//! The explain step: tests, compiler errors, static findings, measurements and
//! the measured test-to-hole map, joined into deterministic text that says
//! where to start. No clocks in the text: timings never reach a golden.

use crate::checker;
use crate::holemap::HoleMap;
use crate::models::{fmt2, ochiai};
use crate::runner::{self, Diagnostic, Status, TestRun};
use crate::scaffold::{self, Project};
use crate::util::{self, Res};
use std::collections::BTreeMap;
use std::path::Path;

/// One line per common borrow-checker / type error: what the code means in
/// this project's terms, and which lesson covers it. Never the fix itself.
pub fn diagnostic_note(code: &str) -> Option<&'static str> {
    Some(match code {
        "E0499" => "two `&mut` borrows of the same value are alive at once. Fields of a struct can be borrowed separately (destructure it); elements of an array or Vec cannot, unless you split it (lesson L02).",
        "E0502" => "a value is borrowed mutably while a shared borrow of it is still in use. Reading stage k_j while writing stage k_i needs two disjoint borrows (lesson L02).",
        "E0503" => "a value is read while a mutable borrow of it is still in use (for example indexing `v` inside a loop over `v.iter_mut()`). Keep what you need in a local first (course unit 01).",
        "E0382" => "a value was used after it was moved. A `Vec` passed by value is given away (lesson L01); an `FnOnce` closure is moved by its first call, so it cannot be called again (course unit 04).",
        "E0505" => "a value was moved while a borrow of it is still alive (lesson L01).",
        "E0596" => "a mutable borrow of something not declared `mut`, or behind a `&` reference. Calling an `FnMut` closure borrows it mutably, so its binding must be mutable (course unit 04).",
        "E0594" => "an assignment through a shared reference (`iter()` yields `&T`), or to a variable captured by an `Fn` closure, which may only read its captures (course unit 04).",
        "E0384" => "a variable assigned twice without `mut`.",
        "E0308" => "a type mismatch: check the declared return type and what each branch produces (`f64` vs `Option<f64>`, `&[f64]` vs `Vec<f64>`).",
        "E0277" => "a trait bound is not met, e.g. adding `&f64` to `f64` in an iterator chain, or calling `f` without `FnMut`.",
        "E0425" => "a name that is not in scope: a helper or constant may need its module path (`tableau::A`).",
        "E0603" => "the function is private: the grader calls it from outside your module, so declare it `pub fn`.",
        "E0369" =>"an operator that is not defined for these operand types (often `&f64` vs `f64`).",
        _ => return None,
    })
}

pub struct Stage {
    pub project: String,
    pub stage: u32,
}

pub fn read_stage(dir: &Path) -> Res<Stage> {
    let text = util::read(&dir.join(".tutor-stage"))
        .map_err(|_| format!("{} was not created by `tutor start` (no .tutor-stage file)", dir.display()))?;
    let mut project = None;
    let mut stage = None;
    for l in text.lines() {
        if let Some((k, v)) = l.split_once('=') {
            match k.trim() {
                "project" => project = Some(v.trim().trim_matches('"').to_string()),
                "stage" => stage = v.trim().parse().ok(),
                _ => {}
            }
        }
    }
    match (project, stage) {
        (Some(project), Some(stage)) => Ok(Stage { project, stage }),
        _ => Err(format!("malformed {}/.tutor-stage", dir.display())),
    }
}

pub fn write_stage(dir: &Path, project: &str, stage: u32) -> Res<()> {
    util::write(&dir.join(".tutor-stage"), &format!("project = \"{project}\"\nstage = {stage}\n"))
}

pub fn load_baseline(project: &str) -> BTreeMap<String, String> {
    let path = scaffold::project_dir(project).join("measure_baseline.toml");
    let Ok(text) = std::fs::read_to_string(path) else { return BTreeMap::new() };
    let Ok(v) = text.parse::<toml::Table>() else { return BTreeMap::new() };
    v.get("measurement")
        .and_then(|m| m.as_table())
        .map(|t| t.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default()
}

pub struct Evidence {
    pub stage: u32,
    pub run: TestRun,
    pub open: BTreeMap<String, bool>,
}

struct HoleRow<'a> {
    id: &'a str,
    score: f64,
    /// Failing tests whose only dependency among this stage's holes is this hole.
    sole: usize,
    ef: usize,
    ep: usize,
    open: bool,
    line: usize,
    order: usize,
    kc: &'a str,
    measured: Vec<String>,
}

pub struct Explained {
    pub text: String,
    pub evidence: Evidence,
}

pub fn explain(dir: &Path, hints: usize) -> Res<Explained> {
    let st = read_stage(dir)?;
    let p = Project::load(&st.project)?;
    let map = HoleMap::load(&st.project)?;
    let src_path = dir.join(&p.file);
    let src = util::read(&src_path)?;
    let holes = p.holes_at(st.stage);
    let mut out = String::new();
    out.push_str(&format!("== tutor explain: {}, stage {} ==\n\n", p.project, st.stage));

    let locs = scaffold::find_holes(&src);
    let open: BTreeMap<String, bool> = holes.iter().map(|h| (h.id.clone(), scaffold::is_open(&src, &h.id))).collect();
    let run = runner::run_tests(dir, &p.project)?;

    let measurements = if matches!(run.status, Status::CompileError(_)) {
        None
    } else {
        Some(runner::run_example(dir, &p.project, "measure"))
    };
    let notes = match &measurements {
        Some(Ok(ms)) => measurement_notes(&p, ms),
        _ => BTreeMap::new(),
    };

    // Static findings, on the source the learner edits.
    let report = checker::check_paths(&checker::collect_files(&[dir.join("src").to_string_lossy().to_string()]), dir);

    match &run.status {
        Status::CompileError(diags) => {
            explain_compile_error(&mut out, &p, &src, diags, locs.as_ref().ok());
        }
        Status::Timeout => {
            out.push_str(&format!(
                "Tests: TIMEOUT after {} s. A loop that never ends is the usual cause: in an adaptive driver, a step that never advances `t`.\n\n",
                runner::TEST_TIMEOUT.as_secs()
            ));
        }
        Status::Crashed(e) => {
            out.push_str(&format!("Tests: the test run crashed:\n{e}\n\n"));
        }
        Status::Ran => {
            explain_tests(&mut out, &p, &map, &run, &src, &holes, &open, &notes, st.stage);
        }
    }

    out.push_str(&format!("Static findings ({}):\n", report.findings.len() + report.parse_errors.len()));
    if report.findings.is_empty() && report.parse_errors.is_empty() {
        out.push_str("  none\n");
    } else {
        for e in &report.parse_errors {
            out.push_str(&format!("  {e}\n"));
        }
        for f in &report.findings {
            for l in checker::format_finding(f, hints).lines() {
                out.push_str(&format!("  {l}\n"));
            }
        }
    }
    out.push('\n');

    out.push_str("Measurements (release build, counting global allocator, deterministic):\n");
    let base = load_baseline(&p.project);
    match &measurements {
        None => out.push_str("  not run: the crate does not compile\n"),
        Some(Ok(ms)) => {
            for (k, v) in ms {
                let r = base.get(k).map(|b| format!("  (reference {b})")).unwrap_or_default();
                out.push_str(&format!("  {k}: {v}{r}\n"));
            }
        }
        Some(Err(e)) => out.push_str(&format!("  unavailable: {e}\n")),
    }
    Ok(Explained { text: out, evidence: Evidence { stage: st.stage, run, open } })
}

fn explain_compile_error(out: &mut String, p: &Project, src: &str, diags: &[Diagnostic], locs: Option<&Vec<scaffold::HoleLoc>>) {
    let first = diags.iter().find(|d| d.file.ends_with(&p.file)).or(diags.first());
    let hole = first.and_then(|d| {
        if !d.file.ends_with(&p.file) {
            return None;
        }
        let off = line_offset(src, d.line);
        locs?.iter().find(|h| h.body_start <= off && off <= h.body_end + 1).map(|h| (h.id.clone(), h.line))
    });
    if let Some((id, line)) = &hole {
        let kc = p.hole(id).map(|h| h.kc.as_str()).unwrap_or("");
        out.push_str(&format!("Start here: hole `{id}` at {}:{line}\n", p.file));
        out.push_str("  why: the first compiler error is inside it; nothing runs until the crate compiles.\n");
        out.push_str(&format!("  Knowledge component: {kc}.\n\n"));
    } else {
        out.push_str("Start here: the first compiler error below (it is not inside a hole).\n\n");
    }
    let plural = if diags.len() == 1 { "" } else { "s" };
    out.push_str(&format!("Build: does not compile ({} error{plural})\n", diags.len()));
    for d in diags.iter().take(3) {
        let code = d.code.as_deref().map(|c| format!("[{c}]")).unwrap_or_default();
        out.push_str(&format!("  {}:{}:{}: error{code}: {}\n", d.file, d.line, d.col, d.message));
        if let Some(n) = d.code.as_deref().and_then(diagnostic_note) {
            out.push_str(&format!("       {code}: {n}\n"));
        }
    }
    if diags.len() > 3 {
        out.push_str(&format!("  ... and {} more (fix the first one first: later errors are often its echoes)\n", diags.len() - 3));
    }
    out.push('\n');
}

fn line_offset(text: &str, line: usize) -> usize {
    text.split_inclusive('\n').take(line.saturating_sub(1)).map(str::len).sum()
}

#[allow(clippy::too_many_arguments)]
fn explain_tests(
    out: &mut String,
    p: &Project,
    map: &HoleMap,
    run: &TestRun,
    src: &str,
    holes: &[&scaffold::Hole],
    open: &BTreeMap<String, bool>,
    notes: &BTreeMap<String, Vec<String>>,
    stage: u32,
) {
    let failing = run.failing();
    let mut rows: Vec<HoleRow> = holes
        .iter()
        .enumerate()
        .map(|(order, h)| {
            let (mut ef, mut ep, mut sole) = (0, 0, 0);
            for t in &run.tests {
                let deps = map.deps(&t.id);
                if deps.contains(&h.id) {
                    if t.passed {
                        ep += 1
                    } else {
                        ef += 1;
                        if deps.iter().filter(|d| holes.iter().any(|x| &x.id == *d)).all(|d| d == &h.id) {
                            sole += 1;
                        }
                    }
                }
            }
            HoleRow {
                id: &h.id,
                score: ochiai(ef, ep, failing.len()),
                sole,
                ef,
                ep,
                open: open.get(&h.id).copied().unwrap_or(false),
                line: scaffold::hole_line(src, &h.id).unwrap_or(0),
                order,
                kc: &h.kc,
                measured: notes.get(&h.id).cloned().unwrap_or_default(),
            }
        })
        .collect();
    // Sole-cause evidence first (a failing test that no other hole at this
    // stage can explain), then Ochiai, then teaching order.
    rows.sort_by(|a, b| {
        (b.sole > 0)
            .cmp(&(a.sole > 0))
            .then(b.score.total_cmp(&a.score))
            .then(a.order.cmp(&b.order))
    });

    if failing.is_empty() {
        out.push_str(&format!("All {} tests pass at stage {stage}.\n\n", run.tests.len()));
    } else if let Some(top) = rows.first().filter(|r| r.score > 0.0) {
        out.push_str(&format!("Start here: hole `{}` at {}:{}\n", top.id, p.file, top.line));
        let mut why = format!(
            "  why: {} of the {} failing tests depend on it (Ochiai {})",
            top.ef,
            failing.len(),
            fmt2(top.score)
        );
        if top.ep > 0 {
            why.push_str(&format!("; {} of its dependent tests pass", top.ep));
        }
        why.push_str(".\n");
        if top.sole > 0 {
            why.push_str(&format!(
                "       {} failing {} on no other hole at this stage: only this hole can explain {}.\n",
                top.sole,
                if top.sole == 1 { "test depends" } else { "tests depend" },
                if top.sole == 1 { "it" } else { "them" }
            ));
        }
        out.push_str(&why);
        if top.open {
            out.push_str("  It is still open (`todo!`): its tests fail by reaching the `todo!`.\n");
        }
        for m in &top.measured {
            out.push_str(&format!("  measured: {m}\n"));
        }
        out.push_str(&format!("  Read its doc comment: it is the specification. Knowledge component: {}.\n\n", top.kc));
    } else {
        out.push_str("Start here: the failing tests below; no hole at this stage explains them (they depend only on code outside the holes).\n\n");
    }

    out.push_str(&format!("Tests: {}/{} pass\n", run.tests.len() - failing.len(), run.tests.len()));
    for t in &failing {
        let deps = map.deps(&t.id);
        let deps: Vec<&String> = deps.iter().filter(|d| holes.iter().any(|h| &h.id == *d)).collect();
        let deps_s = if deps.is_empty() { "none at this stage".to_string() } else { deps.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") };
        out.push_str(&format!("  FAIL {}  [depends on: {deps_s}]\n", t.id));
        if let Some(loc) = &t.location {
            let msg = t.message.first().cloned().unwrap_or_default();
            out.push_str(&format!("       {loc}: {msg}\n"));
            for m in t.message.iter().skip(1).take(1) {
                out.push_str(&format!("       {m}\n"));
            }
        }
    }
    out.push('\n');

    out.push_str(&format!(
        "Holes at stage {stage} (ranked by sole-cause evidence, then Ochiai over the measured test-to-hole map):\n"
    ));
    let w = rows.iter().map(|r| r.id.len()).max().unwrap_or(0);
    for (i, r) in rows.iter().enumerate() {
        out.push_str(&format!(
            "  {}. {:<w$}  {}  {:<11}  failing-deps={} passing-deps={} sole={}  {}:{}\n",
            i + 1,
            r.id,
            fmt2(r.score),
            if r.open { "open" } else { "implemented" },
            r.ef,
            r.ep,
            r.sole,
            p.file,
            r.line,
        ));
        for m in &r.measured {
            out.push_str(&format!("       measured: {m}\n"));
        }
    }
    out.push('\n');
}

/// Measurements that differ from the reference, attached to the holes whose
/// code drives them. Called after the measure example ran.
pub fn measurement_notes(p: &Project, ms: &[(String, String)]) -> BTreeMap<String, Vec<String>> {
    let base = load_baseline(&p.project);
    let mut notes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, v) in ms {
        if let Some(b) = base.get(k) {
            if b != v {
                for h in p.hole.iter().filter(|h| h.measures.contains(k)) {
                    notes.entry(h.id.clone()).or_default().push(format!("{k}: {v} (reference {b})"));
                }
            }
        }
    }
    notes
}
