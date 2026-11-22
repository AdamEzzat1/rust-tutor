//! The learner model over time, in `.tutor/state.toml` (gitignored).
//!
//! Evidence from code: one observation per distinct version of a hole (FNV
//! hash of its body), and only unambiguous evidence: correct when every test
//! depending on the hole passes; incorrect only when a failing test depends
//! on this hole and on no other hole the learner wrote. Failures shared
//! between holes are not recorded at all.

use crate::explain::Evidence;
use crate::holemap::HoleMap;
use crate::models::{self, Bkt, Sm2};
use crate::runner::Status;
use crate::scaffold::{self, Project};
use crate::util::{self, Res};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub kc: BTreeMap<String, Kc>,
    #[serde(default)]
    pub attempt: Vec<Attempt>,
    #[serde(default)]
    pub lesson: BTreeMap<String, LessonState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kc {
    pub p_known: f64,
    pub observations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub project: String,
    pub hole: String,
    pub hash: String,
    pub correct: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonState {
    pub repetitions: u32,
    pub interval_days: u32,
    pub easiness: f64,
    /// Days since the Unix epoch when the next review is due.
    pub due_day: u64,
    pub correct: u32,
    pub answered: u32,
}

fn state_path() -> std::path::PathBuf {
    util::repo_root().join(".tutor").join("state.toml")
}

pub fn load() -> State {
    std::fs::read_to_string(state_path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(s: &State) -> Res<()> {
    let text = toml::to_string(s).map_err(|e| e.to_string())?;
    util::write(&state_path(), &text)
}

pub fn observe(state: &mut State, kc: &str, p: Bkt, correct: bool) {
    let e = state.kc.entry(kc.to_string()).or_insert(Kc { p_known: p.p_init, observations: 0 });
    e.p_known = models::bkt_update(p, e.p_known, correct);
    e.observations += 1;
}

/// Records the unambiguous hole attempts in `ev`. Returns one line per
/// recorded observation.
pub fn record(state: &mut State, p: &Project, map: &HoleMap, src: &str, ev: &Evidence) -> Vec<String> {
    let mut lines = vec![];
    if ev.run.status != Status::Ran {
        return lines;
    }
    let holes = p.holes_at(ev.stage);
    let ids: Vec<&str> = holes.iter().map(|h| h.id.as_str()).collect();
    for h in &holes {
        if ev.open.get(&h.id).copied().unwrap_or(true) {
            continue;
        }
        let Ok(body) = scaffold::hole_body(src, &h.id) else { continue };
        let hash = format!("{:016x}", util::fnv1a(&body));
        if state.attempt.iter().any(|a| a.project == p.project && a.hole == h.id && a.hash == hash) {
            continue;
        }
        let dependents: Vec<_> = ev.run.tests.iter().filter(|t| map.deps(&t.id).contains(&h.id)).collect();
        if dependents.is_empty() {
            continue;
        }
        let correct = dependents.iter().all(|t| t.passed);
        let unambiguous_failure = dependents.iter().any(|t| {
            !t.passed && map.deps(&t.id).iter().filter(|d| ids.contains(&d.as_str())).all(|d| *d == h.id)
        });
        if !correct && !unambiguous_failure {
            continue;
        }
        observe(state, &h.kc, models::CODE_BKT, correct);
        state.attempt.push(Attempt { project: p.project.clone(), hole: h.id.clone(), hash, correct });
        lines.push(format!("recorded: {} attempt at `{}` ({})", if correct { "correct" } else { "incorrect" }, h.id, h.kc));
    }
    lines
}

pub fn today() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

pub fn review_lesson(state: &mut State, id: &str, quality: u8, day: u64, all_correct: bool) {
    let e = state.lesson.entry(id.to_string()).or_insert(LessonState {
        repetitions: 0,
        interval_days: 0,
        easiness: 2.5,
        due_day: day,
        correct: 0,
        answered: 0,
    });
    let s = models::sm2_review(Sm2 { repetitions: e.repetitions, interval_days: e.interval_days, easiness: e.easiness }, quality);
    e.repetitions = s.repetitions;
    e.interval_days = s.interval_days;
    e.easiness = s.easiness;
    e.due_day = day + s.interval_days as u64;
    e.answered += 1;
    if all_correct {
        e.correct += 1;
    }
}

pub fn format(state: &State) -> String {
    let mut s = String::new();
    s.push_str("Knowledge components (BKT; parameters are priors, not fitted):\n");
    if state.kc.is_empty() {
        s.push_str("  no observations yet\n");
    }
    for (k, v) in &state.kc {
        let bar = "#".repeat((v.p_known * 20.0).round() as usize);
        s.push_str(&format!("  {k:<30} {:.2} {bar:<20} ({} observations)\n", v.p_known, v.observations));
    }
    s.push_str("\nHole attempts:\n");
    let mut by: BTreeMap<(String, String), (u32, u32)> = BTreeMap::new();
    for a in &state.attempt {
        let e = by.entry((a.project.clone(), a.hole.clone())).or_default();
        e.0 += 1;
        if a.correct {
            e.1 += 1;
        }
    }
    if by.is_empty() {
        s.push_str("  none\n");
    }
    for ((p, h), (n, c)) in by {
        s.push_str(&format!("  {p}/{h}: {n} distinct versions, {c} correct\n"));
    }
    s.push_str("\nLessons:\n");
    let day = today();
    if state.lesson.is_empty() {
        s.push_str("  none taken\n");
    }
    for (id, l) in &state.lesson {
        let due = if l.due_day <= day { "due now".to_string() } else { format!("due in {} days", l.due_day - day) };
        s.push_str(&format!("  {id}: answered {} times, all correct {} times; {due}\n", l.answered, l.correct));
    }
    s
}
