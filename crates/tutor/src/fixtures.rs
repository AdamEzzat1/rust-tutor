//! Explain fixtures: a realistic bug per hole, put into an otherwise solved
//! starter stage, with the explain output pinned as a golden file.

use crate::explain;
use crate::scaffold::{self, Project};
use crate::util::{self, Res};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Fixture {
    pub name: String,
    pub stage: u32,
    pub hole: String,
    /// What the learner got wrong, in one line (for authors; not printed).
    pub bug: String,
    pub body: String,
}

#[derive(Debug, Deserialize)]
struct File {
    fixture: Vec<Fixture>,
}

pub fn load(project: &str) -> Res<Vec<Fixture>> {
    let f: File = util::parse_toml(&scaffold::project_dir(project).join("fixtures.toml"))?;
    Ok(f.fixture)
}

fn golden_path(project: &str, name: &str) -> std::path::PathBuf {
    scaffold::project_dir(project).join("fixtures").join(format!("{name}.golden.txt"))
}

/// Builds the fixture's work dir: the reference with `hole` replaced by the
/// buggy body, as a starter stage (`.tutor-stage` set).
pub fn materialise(p: &Project, fx: &Fixture) -> Res<std::path::PathBuf> {
    let refdir = scaffold::reference_dir(&p.project);
    let reference = util::read(&refdir.join(&p.file))?;
    let dir = util::scratch_dir(&format!("fixture-{}-{}", p.project, fx.name))?;
    util::copy_dir(&refdir, &dir)?;
    util::write(&dir.join(&p.file), &scaffold::replace_body(&reference, &fx.hole, &fx.body)?)?;
    explain::write_stage(&dir, &p.project, fx.stage)?;
    Ok(dir)
}

/// Runs every fixture. With `check`, compares with the goldens and returns
/// the mismatches; otherwise rewrites the goldens.
pub fn run(project: &str, check: bool, log: &mut dyn FnMut(&str)) -> Res<Vec<String>> {
    let p = Project::load(project)?;
    let mut problems = vec![];
    let fixtures = load(project)?;
    for h in &p.hole {
        if !fixtures.iter().any(|f| f.hole == h.id) {
            problems.push(format!("hole `{}` has no explain fixture", h.id));
        }
    }
    for fx in &fixtures {
        log(&fx.name);
        if p.hole(&fx.hole).is_none_or(|h| h.stage > fx.stage) {
            problems.push(format!("fixture {}: hole `{}` is not open at stage {}", fx.name, fx.hole, fx.stage));
            continue;
        }
        let dir = materialise(&p, fx)?;
        let text = explain::explain(&dir, 1)?.text;
        // The fixture must point at its own hole.
        let points = text.contains(&format!("Start here: hole `{}`", fx.hole));
        if !points {
            problems.push(format!("fixture {}: explain does not start at hole `{}`", fx.name, fx.hole));
        }
        let gp = golden_path(project, &fx.name);
        if check {
            let old = std::fs::read_to_string(&gp).unwrap_or_default().replace("\r\n", "\n");
            if old != text {
                let d = util::first_difference(&old, &text).unwrap_or_else(|| "missing golden".into());
                problems.push(format!("fixture {}: output differs from {}, {d}", fx.name, util::rel_display(&gp)));
            }
        } else {
            util::write(&gp, &text)?;
        }
    }
    Ok(problems)
}
