//! Holes cut from the reference by `// HOLE: id` markers, located with `syn`
//! spans (the marker must sit directly above the function's doc comment).
//! Starter stages are generated, never hand-edited (invariant 6).

use crate::util::{self, Res};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Hole {
    pub id: String,
    pub stage: u32,
    pub kc: String,
    pub summary: String,
    #[serde(default)]
    pub trivial: Vec<String>,
    /// Measurement keys (examples/measure.rs) that this hole's code drives.
    #[serde(default)]
    pub measures: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub project: String,
    #[serde(rename = "crate")]
    pub crate_name: String,
    /// The one source file holding every hole, relative to the crate root.
    pub file: String,
    pub stages: Vec<u32>,
    pub hole: Vec<Hole>,
}

impl Project {
    pub fn load(name: &str) -> Res<Project> {
        let p: Project = util::parse_toml(&project_dir(name).join("holes.toml"))?;
        if p.project != name {
            return Err(format!("holes.toml names project {:?}, expected {name:?}", p.project));
        }
        Ok(p)
    }
    pub fn holes_at(&self, stage: u32) -> Vec<&Hole> {
        self.hole.iter().filter(|h| h.stage <= stage).collect()
    }
    pub fn hole(&self, id: &str) -> Option<&Hole> {
        self.hole.iter().find(|h| h.id == id)
    }
}

pub fn project_dir(name: &str) -> PathBuf {
    util::repo_root().join("projects").join(name)
}

pub fn reference_dir(name: &str) -> PathBuf {
    project_dir(name).join("reference")
}

pub fn stage_dir(name: &str, stage: u32) -> PathBuf {
    project_dir(name).join("starter").join(format!("stage{stage}"))
}

#[derive(Debug, Clone)]
pub struct HoleLoc {
    pub id: String,
    /// 1-based line of the `fn` keyword's item (first doc line).
    pub line: usize,
    /// Byte offsets just inside the body braces.
    pub body_start: usize,
    pub body_end: usize,
}

pub fn open_body(id: &str) -> String {
    format!("todo!(\"TODO(hole:{id})\")")
}

/// Byte offset of a proc-macro2 line/column (column counts chars).
fn offset(text: &str, line: usize, col: usize) -> usize {
    let mut off = 0;
    for (i, l) in text.split_inclusive('\n').enumerate() {
        if i + 1 == line {
            return off + l.char_indices().nth(col).map_or(l.len(), |(b, _)| b);
        }
        off += l.len();
    }
    text.len()
}

/// Every hole marker in `text` with its function's body span.
pub fn find_holes(text: &str) -> Res<Vec<HoleLoc>> {
    let file = syn::parse_file(text).map_err(|e| {
        let s = e.span().start();
        format!("parse error at {}:{}: {e}", s.line, s.column + 1)
    })?;
    let lines: Vec<&str> = text.lines().collect();
    let mut markers = vec![];
    for (i, l) in lines.iter().enumerate() {
        if let Some(rest) = l.trim().strip_prefix("// HOLE:") {
            markers.push((i + 1, rest.trim().to_string()));
        }
    }
    let mut out = vec![];
    for (mline, id) in markers {
        let mut found = None;
        for item in &file.items {
            if let syn::Item::Fn(f) = item {
                let first = f
                    .attrs
                    .first()
                    .map(|a| a.pound_token.span.start().line)
                    .unwrap_or_else(|| f.sig.fn_token.span.start().line);
                if first == mline + 1 {
                    let open = f.block.brace_token.span.open().end();
                    let close = f.block.brace_token.span.close().start();
                    found = Some(HoleLoc {
                        id: id.clone(),
                        line: first,
                        body_start: offset(text, open.line, open.column),
                        body_end: offset(text, close.line, close.column),
                    });
                }
            }
        }
        match found {
            Some(h) => out.push(h),
            None => return Err(format!("marker `// HOLE: {id}` at line {mline} is not directly above a top-level fn")),
        }
    }
    Ok(out)
}

/// Line of the fn whose hole is `id` (for messages).
pub fn hole_line(text: &str, id: &str) -> Option<usize> {
    find_holes(text).ok()?.into_iter().find(|h| h.id == id).map(|h| h.line)
}

/// The text between a hole's braces, without surrounding blank lines.
pub fn hole_body(text: &str, id: &str) -> Res<String> {
    let h = find_holes(text)?.into_iter().find(|h| h.id == id).ok_or_else(|| format!("no hole `{id}`"))?;
    Ok(dedent(&text[h.body_start..h.body_end]))
}

pub fn is_open(text: &str, id: &str) -> bool {
    hole_body(text, id).is_ok_and(|b| b.trim() == open_body(id))
}

fn dedent(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let min = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let body: Vec<String> = lines.iter().map(|l| if l.len() >= min { l[min..].to_string() } else { l.trim().to_string() }).collect();
    body.join("\n").trim_matches('\n').to_string()
}

/// Replaces hole `id`'s body with `body` (indented four spaces).
pub fn replace_body(text: &str, id: &str, body: &str) -> Res<String> {
    let h = find_holes(text)?.into_iter().find(|h| h.id == id).ok_or_else(|| format!("no hole `{id}`"))?;
    let indented: Vec<String> = body
        .trim_matches('\n')
        .lines()
        .map(|l| if l.trim().is_empty() { String::new() } else { format!("    {l}") })
        .collect();
    Ok(format!("{}\n{}\n{}", &text[..h.body_start], indented.join("\n"), &text[h.body_end..]))
}

/// The starter text for `stage`: every hole with stage <= `stage` opened.
pub fn starter_source(p: &Project, reference: &str, stage: u32) -> Res<String> {
    let mut text = reference.to_string();
    for h in p.holes_at(stage) {
        text = replace_body(&text, &h.id, &open_body(&h.id))?;
    }
    Ok(text)
}

pub const STARTER_README: &str = "This directory is generated by `tutor scaffold` from the reference. \
Never edit it by hand: `tutor scaffold <project> --check` fails in CI if it differs.\n\
Copy it with `tutor start <project> <stage> <dir>` and work there.\n";

/// Writes (or, with `check`, compares) every starter stage. Returns the
/// problems found by `check` (empty when up to date).
pub fn scaffold(name: &str, check: bool) -> Res<Vec<String>> {
    let p = Project::load(name)?;
    let refdir = reference_dir(name);
    let reference = util::read(&refdir.join(&p.file))?;
    // Every hole in holes.toml has a marker, and vice versa.
    let locs = find_holes(&reference)?;
    for h in &p.hole {
        if !locs.iter().any(|l| l.id == h.id) {
            return Err(format!("hole `{}` is in holes.toml but has no marker in {}", h.id, p.file));
        }
    }
    for l in &locs {
        if p.hole(&l.id).is_none() {
            return Err(format!("marker `{}` has no entry in holes.toml", l.id));
        }
    }
    let mut problems = vec![];
    for &stage in &p.stages {
        let tmp = util::scratch_dir(&format!("scaffold-{name}-{stage}"))?;
        util::copy_dir(&refdir, &tmp)?;
        util::write(&tmp.join(&p.file), &starter_source(&p, &reference, stage)?)?;
        util::write(&tmp.join("STARTER.md"), STARTER_README)?;
        let dest = stage_dir(name, stage);
        if check {
            problems.extend(compare_dirs(&tmp, &dest));
        } else {
            if dest.exists() {
                std::fs::remove_dir_all(&dest).map_err(|e| format!("cannot clear {}: {e}", dest.display()))?;
            }
            util::copy_dir(&tmp, &dest)?;
        }
    }
    Ok(problems)
}

pub fn compare_dirs(generated: &Path, committed: &Path) -> Vec<String> {
    let mut out = vec![];
    let a = util::list_files(generated);
    let b = util::list_files(committed);
    for f in a.iter().filter(|f| !b.contains(f)) {
        out.push(format!("{}: missing {f}", util::rel_display(committed)));
    }
    for f in b.iter().filter(|f| !a.contains(f)) {
        out.push(format!("{}: unexpected {f}", util::rel_display(committed)));
    }
    for f in a.iter().filter(|f| b.contains(f)) {
        let x = std::fs::read_to_string(generated.join(f)).unwrap_or_default().replace("\r\n", "\n");
        let y = std::fs::read_to_string(committed.join(f)).unwrap_or_default().replace("\r\n", "\n");
        if x != y {
            let d = util::first_difference(&y, &x).unwrap_or_default();
            out.push(format!("{}/{f} differs from the generated version, {d}", util::rel_display(committed)));
        }
    }
    out
}
