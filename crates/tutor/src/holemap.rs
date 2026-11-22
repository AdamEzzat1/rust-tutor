//! The measured test-to-hole map and mutation testing.
//!
//! A test depends on a hole if opening that hole alone (in the reference)
//! makes the test fail. Every hole must be depended on; every trivial body
//! must compile and be caught; each starter stage must fail exactly at the
//! tests of its open holes.

use crate::runner::{self, Status, TestRun};
use crate::scaffold::{self, Project};
use crate::util::{self, Res};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Deserialize)]
pub struct TestDeps {
    pub id: String,
    pub depends: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Mutant {
    pub hole: String,
    pub index: usize,
    pub caught_by: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StageRow {
    pub stage: u32,
    pub failing: usize,
    pub unexplained: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HoleMap {
    pub test: Vec<TestDeps>,
    #[serde(default)]
    pub mutant: Vec<Mutant>,
    #[serde(default)]
    pub stage: Vec<StageRow>,
}

impl HoleMap {
    pub fn load(project: &str) -> Res<HoleMap> {
        util::parse_toml(&scaffold::project_dir(project).join("holemap.toml"))
    }
    pub fn deps(&self, test: &str) -> Vec<String> {
        self.test.iter().find(|t| t.id == test).map(|t| t.depends.clone()).unwrap_or_default()
    }
}

fn run_with(p: &Project, dir: &std::path::Path, text: &str) -> Res<TestRun> {
    util::write(&dir.join(&p.file), text)?;
    runner::run_tests(dir, &p.project)
}

fn failing_set(run: &TestRun) -> BTreeSet<String> {
    run.failing().into_iter().map(|t| t.id.clone()).collect()
}

fn toml_list(v: &[String]) -> String {
    let items: Vec<String> = v.iter().map(|s| format!("{s:?}")).collect();
    format!("[{}]", items.join(", "))
}

pub struct Outcome {
    pub text: String,
    pub problems: Vec<String>,
}

pub fn build(project: &str, log: &mut dyn FnMut(&str)) -> Res<Outcome> {
    let p = Project::load(project)?;
    let refdir = scaffold::reference_dir(project);
    let reference = util::read(&refdir.join(&p.file))?;
    let dir = util::scratch_dir(&format!("holemap-{project}"))?;
    util::copy_dir(&refdir, &dir)?;
    let mut problems = vec![];

    log("reference");
    let base = run_with(&p, &dir, &reference)?;
    if base.status != Status::Ran || !base.failing().is_empty() {
        return Err(format!("the reference does not pass its own tests: {:?}, failing {:?}", base.status, failing_set(&base)));
    }
    let all: Vec<String> = base.tests.iter().map(|t| t.id.clone()).collect();

    let mut deps: BTreeMap<String, BTreeSet<String>> = all.iter().map(|t| (t.clone(), BTreeSet::new())).collect();
    for h in &p.hole {
        log(&format!("hole {}", h.id));
        let text = scaffold::replace_body(&reference, &h.id, &scaffold::open_body(&h.id))?;
        let run = run_with(&p, &dir, &text)?;
        if run.status != Status::Ran {
            problems.push(format!("opening hole `{}` does not give a test run: {:?}", h.id, run.status));
            continue;
        }
        let f = failing_set(&run);
        if f.is_empty() {
            problems.push(format!("no test depends on hole `{}`", h.id));
        }
        for t in f {
            deps.entry(t).or_default().insert(h.id.clone());
        }
    }

    let mut mutants = vec![];
    for h in &p.hole {
        if h.trivial.is_empty() {
            problems.push(format!("hole `{}` has no trivial bodies", h.id));
        }
        for (i, body) in h.trivial.iter().enumerate() {
            log(&format!("mutant {}#{i}", h.id));
            let text = scaffold::replace_body(&reference, &h.id, body)?;
            let run = run_with(&p, &dir, &text)?;
            let caught = match &run.status {
                Status::Ran => failing_set(&run).len(),
                Status::Timeout => {
                    problems.push(format!("trivial body {}#{i} hangs the tests (timeout): make it terminate", h.id));
                    0
                }
                Status::CompileError(d) => {
                    let first = d.first().map(|d| format!("{}:{}: {}", d.file, d.line, d.message)).unwrap_or_default();
                    problems.push(format!("trivial body {}#{i} does not compile ({first}): an authoring error", h.id));
                    0
                }
                Status::Crashed(e) => {
                    problems.push(format!("trivial body {}#{i} crashed the run: {e}", h.id));
                    0
                }
            };
            if caught == 0 && run.status == Status::Ran {
                problems.push(format!("trivial body {}#{i} passes every test: the tests are too weak", h.id));
            }
            mutants.push((h.id.clone(), i, caught));
        }
    }

    let mut stages = vec![];
    for &s in &p.stages {
        log(&format!("starter stage {s}"));
        let text = scaffold::starter_source(&p, &reference, s)?;
        let run = run_with(&p, &dir, &text)?;
        let open: BTreeSet<String> = p.holes_at(s).iter().map(|h| h.id.clone()).collect();
        let f = failing_set(&run);
        let unexplained: Vec<String> =
            f.iter().filter(|t| deps.get(*t).is_none_or(|d| d.is_disjoint(&open))).cloned().collect();
        let expected: BTreeSet<String> =
            deps.iter().filter(|(_, d)| !d.is_disjoint(&open)).map(|(t, _)| t.clone()).collect();
        if !unexplained.is_empty() {
            problems.push(format!("stage {s} fails tests no open hole explains: {unexplained:?}"));
        }
        if f != expected {
            let missing: Vec<&String> = expected.difference(&f).collect();
            if !missing.is_empty() {
                problems.push(format!("stage {s}: tests that depend on an open hole still pass: {missing:?}"));
            }
        }
        stages.push((s, f.len(), unexplained));
    }

    let mut t = String::new();
    t.push_str(&format!(
        "# Generated by `tutor holemap {project}`; never edit by hand.\n\
         # A test depends on a hole if opening that hole alone in the reference makes it fail.\n\
         # Measured on rustc {}.\n\n",
        util::rustc_version().unwrap_or_default()
    ));
    for (test, d) in &deps {
        let d: Vec<String> = d.iter().cloned().collect();
        t.push_str(&format!("[[test]]\nid = {test:?}\ndepends = {}\n\n", toml_list(&d)));
    }
    for (hole, i, caught) in &mutants {
        t.push_str(&format!("[[mutant]]\nhole = {hole:?}\nindex = {i}\ncaught_by = {caught}\n\n"));
    }
    for (s, n, u) in &stages {
        t.push_str(&format!("[[stage]]\nstage = {s}\nfailing = {n}\nunexplained = {}\n\n", toml_list(u)));
    }
    Ok(Outcome { text: t.trim_end().to_string() + "\n", problems })
}
