//! Predict-the-outcome lessons. Keys are measured: `tutor lessons verify`
//! compiles and runs every check on the pinned rustc and compares the
//! outcome key (output, `panic: ...` or `compile error E....`) with the
//! answer. Every wrong option names a misconception.

use crate::models;
use crate::progress;
use crate::snippet::{self, Profile};
use crate::util::{self, Res};
use serde::Deserialize;
use std::io::BufRead;

#[derive(Debug, Clone, Deserialize)]
pub struct Misconception {
    pub option: usize,
    pub id: String,
    pub feedback: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Check {
    pub id: String,
    pub prompt: String,
    pub code: String,
    /// "debug" (default) or "release".
    #[serde(default)]
    pub profile: Option<String>,
    pub options: Vec<String>,
    /// 1-based.
    pub answer: usize,
    pub hint: String,
    pub explanation: String,
    #[serde(default)]
    pub misconception: Vec<Misconception>,
}

impl Check {
    pub fn profile(&self) -> Profile {
        if self.profile.as_deref() == Some("release") {
            Profile::Release
        } else {
            Profile::Debug
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Lesson {
    pub id: String,
    pub title: String,
    pub kcs: Vec<String>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    pub verified_on: String,
    pub body: String,
    pub check: Vec<Check>,
}

pub fn lessons_dir() -> std::path::PathBuf {
    util::repo_root().join("lessons")
}

pub fn load_all() -> Res<Vec<Lesson>> {
    let mut files: Vec<_> = std::fs::read_dir(lessons_dir())
        .map_err(|e| format!("cannot read lessons/: {e}"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    files.iter().map(|f| util::parse_toml::<Lesson>(f)).collect()
}

pub fn find(id: &str) -> Res<Lesson> {
    load_all()?
        .into_iter()
        .find(|l| l.id == id || l.id.split('-').next() == Some(id))
        .ok_or_else(|| format!("no lesson {id}"))
}

/// Static problems with a check (no code run).
pub fn static_problems(l: &Lesson, c: &Check) -> Vec<String> {
    let mut out = vec![];
    let at = format!("{}/{}", l.id, c.id);
    if c.answer == 0 || c.answer > c.options.len() {
        out.push(format!("{at}: answer {} is not an option", c.answer));
        return out;
    }
    for i in 1..=c.options.len() {
        if i != c.answer && !c.misconception.iter().any(|m| m.option == i) {
            out.push(format!("{at}: wrong option {i} has no misconception"));
        }
    }
    for m in &c.misconception {
        if m.option == c.answer {
            out.push(format!("{at}: the answer has a misconception attached"));
        }
    }
    let key = &c.options[c.answer - 1];
    if key.len() > 3 && c.hint.contains(key.as_str()) {
        out.push(format!("{at}: the hint contains the answer"));
    }
    let mut sorted = c.options.clone();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != c.options.len() {
        out.push(format!("{at}: duplicate options"));
    }
    out
}

pub fn verify_all(log: &mut dyn FnMut(&str)) -> Res<Vec<String>> {
    let pinned = util::pinned_rust()?;
    let mut problems = vec![];
    for l in load_all()? {
        if l.verified_on != pinned {
            problems.push(format!("{}: verified_on {} but the pinned rustc is {pinned}", l.id, l.verified_on));
        }
        for c in &l.check {
            log(&format!("{}/{}", l.id, c.id));
            let sp = static_problems(&l, c);
            if !sp.is_empty() {
                problems.extend(sp);
                continue;
            }
            let o = snippet::run_program(&snippet::as_program(&c.code), c.profile())?;
            let key = o.key();
            let want = &c.options[c.answer - 1];
            if &key != want {
                problems.push(format!("{}/{}: measured outcome {key:?}, answer key {want:?}", l.id, c.id));
            }
            for (i, opt) in c.options.iter().enumerate() {
                if i + 1 != c.answer && *opt == key {
                    problems.push(format!("{}/{}: wrong option {} equals the measured outcome", l.id, c.id, i + 1));
                }
            }
        }
    }
    Ok(problems)
}

pub fn interactive(id: &str) -> Res<i32> {
    let l = find(id)?;
    println!("== {} : {} ==\n", l.id, l.title);
    println!("{}", l.body.trim());
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let mut first_try = 0;
    let mut correct_total = 0;
    let mut state = progress::load();
    for (n, c) in l.check.iter().enumerate() {
        println!("\n-- check {} of {}: {}\n", n + 1, l.check.len(), c.prompt);
        println!("```rust\n{}\n```", c.code.trim_end());
        for (i, o) in c.options.iter().enumerate() {
            println!("  {}) {}", i + 1, o.replace('\n', "\n     "));
        }
        let mut got_it = false;
        for attempt in 0..2 {
            print!("answer> ");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let Some(Ok(line)) = lines.next() else { break };
            let pick: usize = line.trim().parse().unwrap_or(0);
            if pick == c.answer {
                println!("correct. {}", c.explanation.trim());
                got_it = true;
                if attempt == 0 {
                    first_try += 1;
                }
                break;
            }
            match c.misconception.iter().find(|m| m.option == pick) {
                Some(m) => println!("not quite ({}). {}", m.id, m.feedback.trim()),
                None => println!("not one of the options."),
            }
            if attempt == 0 {
                println!("hint: {}", c.hint.trim());
            } else {
                println!("the answer is {}. {}", c.answer, c.explanation.trim());
            }
        }
        if got_it {
            correct_total += 1;
        }
        for kc in &l.kcs {
            progress::observe(&mut state, kc, models::LESSON_BKT, got_it);
        }
    }
    let all_first = first_try == l.check.len();
    let quality = if all_first { 5 } else if correct_total == l.check.len() { 3 } else { 1 };
    progress::review_lesson(&mut state, &l.id, quality, progress::today(), all_first);
    progress::save(&state)?;
    println!("\n{} of {} right first time. Next review: see `tutor progress`.", first_try, l.check.len());
    Ok(0)
}
