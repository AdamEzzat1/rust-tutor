//! Courses and units. `course/courses/<id>.toml` lists units, prerequisites
//! and outcomes; `course/units/<id>.toml` holds sections, examples, concept
//! questions and practice problems; `course/outputs/<id>.toml` is generated
//! by `tutor course record` and re-checked by `tutor course verify`.

use crate::practice::{self, Problem};
use crate::snippet::{self, Profile};
use crate::util::{self, Res};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Example {
    #[serde(default)]
    pub caption: String,
    #[serde(default)]
    pub notice: String,
    pub code: String,
    #[serde(default)]
    pub profile: Option<String>,
    /// false: shown and recorded, never compared (e.g. prints a timing).
    #[serde(default = "yes")]
    pub check: bool,
}

fn yes() -> bool {
    true
}

impl Example {
    fn profile(&self) -> Profile {
        if self.profile.as_deref() == Some("release") {
            Profile::Release
        } else {
            Profile::Debug
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Analogy {
    pub title: String,
    pub text: String,
    /// Where the analogy stops being true (required).
    pub breaks: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Section {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub example: Vec<Example>,
    #[serde(default)]
    pub analogy: Vec<Analogy>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Question {
    pub id: String,
    /// "output": the key is measured by running `code`; "choice": a reviewer's judgement.
    pub kind: String,
    pub prompt: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    pub options: Vec<String>,
    pub answer: usize,
    pub explanation: String,
    #[serde(default)]
    pub misconception: Vec<crate::lessons::Misconception>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reference {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Unit {
    pub id: String,
    pub title: String,
    pub track: String,
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub kcs: Vec<String>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    #[serde(default)]
    pub outline: Vec<String>,
    #[serde(default)]
    pub section: Vec<Section>,
    #[serde(default)]
    pub question: Vec<Question>,
    #[serde(default)]
    pub problem: Vec<Problem>,
    #[serde(default)]
    pub reference: Vec<Reference>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Outcome {
    pub text: String,
    #[serde(default)]
    pub assessed_by: Vec<String>,
    #[serde(default)]
    pub planned: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Course {
    pub id: String,
    pub title: String,
    pub order: u32,
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub requires: Vec<String>,
    pub units: Vec<String>,
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub outcome: Vec<Outcome>,
}

fn dir(sub: &str) -> std::path::PathBuf {
    util::repo_root().join("course").join(sub)
}

fn toml_files(d: &std::path::Path) -> Res<Vec<std::path::PathBuf>> {
    let mut v: Vec<_> = std::fs::read_dir(d)
        .map_err(|e| format!("cannot read {}: {e}", d.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    v.sort();
    Ok(v)
}

pub fn load_units() -> Res<Vec<Unit>> {
    toml_files(&dir("units"))?.iter().map(|f| util::parse_toml(f)).collect()
}

pub fn load_courses() -> Res<Vec<Course>> {
    let mut v: Vec<Course> = toml_files(&dir("courses"))?.iter().map(|f| util::parse_toml(f)).collect::<Res<_>>()?;
    v.sort_by_key(|c| c.order);
    Ok(v)
}

pub fn format_courses() -> Res<String> {
    let units: BTreeMap<String, Unit> = load_units()?.into_iter().map(|u| (u.id.clone(), u)).collect();
    let mut s = String::new();
    for c in load_courses()? {
        let ready = c.units.iter().filter(|u| units.get(*u).is_some_and(|u| u.status == "ready")).count();
        s.push_str(&format!("{} ({}) [{}]: {} of {} units ready\n  {}\n", c.title, c.id, c.status, ready, c.units.len(), c.summary));
        if !c.requires.is_empty() {
            s.push_str(&format!("  requires: {}\n", c.requires.join(", ")));
        }
        for u in &c.units {
            match units.get(u) {
                Some(unit) => s.push_str(&format!("    {:<8} {:<28} {}\n", unit.status, unit.id, unit.title)),
                None => s.push_str(&format!("    MISSING  {u}\n")),
            }
        }
        if !c.projects.is_empty() {
            s.push_str(&format!("  projects: {}\n", c.projects.join(", ")));
        }
        for o in &c.outcome {
            s.push_str(&format!("  outcome: {}\n    assessed by: {}\n", o.text, if o.assessed_by.is_empty() { "nothing yet".into() } else { o.assessed_by.join(", ") }));
            if !o.planned.is_empty() {
                s.push_str(&format!("    planned: {}\n", o.planned.join(", ")));
            }
        }
        s.push('\n');
    }
    Ok(s)
}

pub fn format_units() -> Res<String> {
    let mut s = String::new();
    let units = load_units()?;
    for c in load_courses()? {
        s.push_str(&format!("{}:\n", c.title));
        for id in &c.units {
            if let Some(u) = units.iter().find(|u| &u.id == id) {
                let ex: usize = u.section.iter().map(|x| x.example.len()).sum();
                s.push_str(&format!(
                    "  {:<28} {:<8} {} sections, {} examples, {} questions, {} problems\n",
                    u.id,
                    u.status,
                    u.section.len(),
                    ex,
                    u.question.len(),
                    u.problem.len()
                ));
            }
        }
    }
    Ok(s)
}

fn outputs_path(unit: &str) -> std::path::PathBuf {
    dir("outputs").join(format!("{unit}.toml"))
}

fn example_key(e: &Example) -> String {
    format!("{:016x}", util::fnv1a(&format!("{:?}\n{}", e.profile, e.code)))
}

fn run_example(e: &Example) -> Res<String> {
    Ok(snippet::run_program(&snippet::as_program(&e.code), e.profile())?.key())
}

fn select(unit: Option<&str>) -> Res<Vec<Unit>> {
    let all = load_units()?;
    Ok(match unit {
        Some(id) => {
            let v: Vec<Unit> = all.into_iter().filter(|u| u.id == id).collect();
            if v.is_empty() {
                return Err(format!("no unit {id}"));
            }
            v
        }
        None => all.into_iter().filter(|u| u.status == "ready").collect(),
    })
}

/// Runs every example and writes `course/outputs/<unit>.toml`.
pub fn record(unit: Option<&str>, log: &mut dyn FnMut(&str)) -> Res<()> {
    let rustc = util::rustc_version()?;
    for u in select(unit)? {
        let mut text = format!("# Generated by `tutor course record`; never edit by hand.\n# Recorded on rustc {rustc}.\n\n");
        for (si, s) in u.section.iter().enumerate() {
            for (ei, e) in s.example.iter().enumerate() {
                log(&format!("{} section {} example {}", u.id, si + 1, ei + 1));
                let out = run_example(e)?;
                text.push_str(&format!(
                    "[[example]]\nsection = {}\nindex = {}\nkey = \"{}\"\noutput = '''\n{}\n'''\n\n",
                    si + 1,
                    ei + 1,
                    example_key(e),
                    out.replace("'''", "' ' '")
                ));
            }
        }
        util::write(&outputs_path(&u.id), text.trim_end_matches('\n'))?;
        util::write(&outputs_path(&u.id), &(text.trim_end().to_string() + "\n"))?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct Recorded {
    #[serde(default)]
    example: Vec<RecordedExample>,
}

#[derive(Deserialize)]
struct RecordedExample {
    section: usize,
    index: usize,
    key: String,
    output: String,
}

pub fn recorded_output(unit: &str, section: usize, index: usize) -> Option<String> {
    let r: Recorded = util::parse_toml(&outputs_path(unit)).ok()?;
    r.example.into_iter().find(|e| e.section == section && e.index == index).map(|e| e.output.trim_matches('\n').to_string())
}

/// Length bias in choice questions (the cheapest give-away): in a unit
/// with 4+ choice-style items, at most 40% may have the key as the unique
/// longest option.
fn length_bias(u: &Unit) -> Option<String> {
    let qs: Vec<&Question> = u.question.iter().filter(|q| q.options.len() >= 3).collect();
    if qs.len() < 4 {
        return None;
    }
    let longest = qs
        .iter()
        .filter(|q| {
            let k = q.options[q.answer - 1].len();
            q.options.iter().enumerate().all(|(i, o)| i + 1 == q.answer || o.len() < k)
        })
        .count();
    if longest * 10 > qs.len() * 4 {
        Some(format!("{}: {longest} of {} questions have the key as the unique longest option", u.id, qs.len()))
    } else {
        None
    }
}

pub fn verify(unit: Option<&str>, log: &mut dyn FnMut(&str)) -> Res<Vec<String>> {
    let mut problems = vec![];
    let courses = load_courses()?;
    let all = load_units()?;
    // Every unit is in exactly one course, and every listed unit exists.
    if unit.is_none() {
        for u in &all {
            let n = courses.iter().filter(|c| c.units.contains(&u.id)).count();
            if n != 1 {
                problems.push(format!("unit {} is in {n} courses (must be exactly one)", u.id));
            }
        }
        for c in &courses {
            for id in &c.units {
                if !all.iter().any(|u| &u.id == id) {
                    problems.push(format!("course {} lists missing unit {id}", c.id));
                }
            }
        }
    }
    for u in select(unit)? {
        if u.status != "ready" {
            continue;
        }
        let refs: Vec<&str> = u.reference.iter().map(|r| r.id.as_str()).collect();
        let recorded: Option<Recorded> = util::parse_toml(&outputs_path(&u.id)).ok();
        if recorded.is_none() {
            problems.push(format!("{}: no recorded outputs (run `tutor course record {}`)", u.id, u.id));
        }
        for (si, s) in u.section.iter().enumerate() {
            for r in &s.refs {
                if !refs.contains(&r.as_str()) {
                    problems.push(format!("{} section {}: cites unknown reference {r}", u.id, si + 1));
                }
            }
            for a in &s.analogy {
                if a.breaks.trim().is_empty() {
                    problems.push(format!("{} section {}: analogy {:?} does not say where it breaks", u.id, si + 1, a.title));
                }
            }
            for (ei, e) in s.example.iter().enumerate() {
                log(&format!("{} section {} example {}", u.id, si + 1, ei + 1));
                let rec = recorded.as_ref().and_then(|r| r.example.iter().find(|x| x.section == si + 1 && x.index == ei + 1));
                match rec {
                    None => problems.push(format!("{} section {} example {}: not recorded", u.id, si + 1, ei + 1)),
                    Some(r) if r.key != example_key(e) => {
                        problems.push(format!("{} section {} example {}: code changed since recording", u.id, si + 1, ei + 1))
                    }
                    Some(r) if e.check => {
                        let now = run_example(e)?;
                        if now != r.output.trim_matches('\n') {
                            problems.push(format!(
                                "{} section {} example {}: output changed, {}",
                                u.id,
                                si + 1,
                                ei + 1,
                                util::first_difference(r.output.trim_matches('\n'), &now).unwrap_or_default()
                            ));
                        }
                    }
                    Some(_) => {}
                }
            }
        }
        for q in &u.question {
            let at = format!("{} question {}", u.id, q.id);
            if q.answer == 0 || q.answer > q.options.len() {
                problems.push(format!("{at}: answer out of range"));
                continue;
            }
            for i in 1..=q.options.len() {
                if i != q.answer && !q.misconception.iter().any(|m| m.option == i) {
                    problems.push(format!("{at}: wrong option {i} has no misconception"));
                }
            }
            if q.kind == "output" {
                log(&at);
                let Some(code) = &q.code else {
                    problems.push(format!("{at}: kind output needs code"));
                    continue;
                };
                let prof = if q.profile.as_deref() == Some("release") { Profile::Release } else { Profile::Debug };
                let key = snippet::run_program(&snippet::as_program(code), prof)?.key();
                if key != q.options[q.answer - 1] {
                    problems.push(format!("{at}: measured {key:?}, key {:?}", q.options[q.answer - 1]));
                }
            }
        }
        if let Some(b) = length_bias(&u) {
            problems.push(b);
        }
        for p in &u.problem {
            let at = format!("{} problem {}", u.id, p.id);
            log(&format!("{at}: reference"));
            let (g, notes) = practice::grade_for_verify(p, &p.reference, &["correct".into()])?;
            for n in notes {
                log(&format!("{at}: {n}"));
            }
            if !g.verdicts.contains(&"correct".into()) {
                problems.push(format!("{at}: the reference is graded {:?}: {:?}", g.verdicts, g.lines));
            }
            if p.perf.is_some() && !g.verdicts.contains(&"fast".into()) {
                log(&format!("{at}: NEAR: the reference against itself measured ratio {:.2} (not <= {})", g.ratio.unwrap_or(f64::NAN), practice::FAST_RATIO));
            }
            log(&format!("{at}: starter"));
            let want = p.starter_expect.clone().unwrap_or_else(|| vec!["incorrect".into()]);
            let (g, _) = practice::grade_for_verify(p, &p.starter, &want)?;
            let m = practice::missing(&want, &g);
            if !m.is_empty() {
                problems.push(format!("{at}: the starter lacks verdicts {m:?} (got {:?})", g.verdicts));
            }
            for v in &p.variant {
                log(&format!("{at}: variant {:?}", v.label));
                let (g, notes) = practice::grade_for_verify(p, &v.code, &v.expect)?;
                for n in notes {
                    log(&format!("{at}: {n}"));
                }
                let m = practice::missing(&v.expect, &g);
                if !m.is_empty() {
                    problems.push(format!(
                        "{at} variant {:?}: expected {m:?}, got {:?} (ratio {:.2})",
                        v.label,
                        g.verdicts,
                        g.ratio.unwrap_or(f64::NAN)
                    ));
                }
            }
            // The starter must not be the reference, and the first hint must not quote it.
            if p.starter.trim() == p.reference.trim() {
                problems.push(format!("{at}: starter equals the reference"));
            }
        }
    }
    Ok(problems)
}
