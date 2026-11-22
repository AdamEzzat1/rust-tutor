//! The static checker: rules over `syn` syntax trees, never over text.
//!
//! A rule may only *point to* a likely problem (invariant 8: measured over
//! inferred). Each rule states its false-positive risk and what measures the
//! real cost. ADVICE never changes the exit code (invariant 2).

use proc_macro2::Span;
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Advice,
    Warn,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Advice => "ADVICE",
            Severity::Warn => "WARN",
            Severity::Error => "ERROR",
        })
    }
}

pub struct RuleMeta {
    pub id: &'static str,
    pub name: &'static str,
    /// Knowledge component the rule's misconception belongs to.
    pub kc: &'static str,
    pub severity: Severity,
    /// Honest false-positive (and false-negative) note.
    pub fp_risk: &'static str,
    /// What measures the real cost, if anything does.
    pub measured_by: &'static str,
    /// Level 1 asks a question, level 2 explains, level 3 shows the fix.
    pub hints: [&'static str; 3],
    /// Words that give the answer away: the level-1 hint must contain none.
    pub answer_tokens: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub rule: &'static str,
    pub severity: Severity,
    pub message: String,
}

pub struct FileCtx<'a> {
    pub path: String,
    pub file: &'a syn::File,
    /// Files under `tests/` are test code throughout.
    pub test_file: bool,
}

pub trait Rule {
    fn meta(&self) -> RuleMeta;
    fn check(&self, ctx: &FileCtx) -> Vec<Finding>;
}

pub fn finding(rule: &dyn Rule, ctx: &FileCtx, span: Span, message: String) -> Finding {
    let m = rule.meta();
    let s = span.start();
    Finding { file: ctx.path.clone(), line: s.line, col: s.column + 1, rule: m.id, severity: m.severity, message }
}

pub fn all_rules() -> Vec<Box<dyn Rule>> {
    crate::rules::all()
}

pub enum CheckError {
    Parse { file: String, line: usize, col: usize, message: String },
    Io(String),
}

/// Checks one file's text. Deterministic: findings are sorted.
pub fn check_source(path: &str, text: &str) -> Result<Vec<Finding>, CheckError> {
    let file = syn::parse_file(text).map_err(|e| {
        let s = e.span().start();
        CheckError::Parse { file: path.to_string(), line: s.line, col: s.column + 1, message: e.to_string() }
    })?;
    let norm = path.replace('\\', "/");
    let test_file = norm.starts_with("tests/") || norm.contains("/tests/");
    let ctx = FileCtx { path: norm, file: &file, test_file };
    let mut out = vec![];
    for r in all_rules() {
        out.extend(r.check(&ctx));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Every `.rs` file under the given paths (files or directories), sorted.
pub fn collect_files(paths: &[String]) -> Vec<std::path::PathBuf> {
    fn go(p: &Path, out: &mut Vec<std::path::PathBuf>) {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                return;
            }
            if let Ok(rd) = std::fs::read_dir(p) {
                for e in rd.filter_map(Result::ok) {
                    go(&e.path(), out);
                }
            }
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p.to_path_buf());
        }
    }
    let mut out = vec![];
    for p in paths {
        go(Path::new(p), &mut out);
    }
    out.sort();
    out
}

pub struct Report {
    pub findings: Vec<Finding>,
    pub parse_errors: Vec<String>,
}

/// Checks files; `base` makes reported paths relative.
pub fn check_paths(files: &[std::path::PathBuf], base: &Path) -> Report {
    let mut findings = vec![];
    let mut parse_errors = vec![];
    for f in files {
        let rel = f.strip_prefix(base).unwrap_or(f).to_string_lossy().replace('\\', "/");
        match std::fs::read_to_string(f) {
            Ok(text) => match check_source(&rel, &text) {
                Ok(v) => findings.extend(v),
                Err(CheckError::Parse { file, line, col, message }) => {
                    parse_errors.push(format!("{file}:{line}:{col}: parse error: {message}"))
                }
                Err(CheckError::Io(e)) => parse_errors.push(e),
            },
            Err(e) => parse_errors.push(format!("{rel}: {e}")),
        }
    }
    findings.sort();
    Report { findings, parse_errors }
}

/// Exit code: 1 if any WARN or ERROR (or a parse error); ADVICE never counts.
pub fn exit_code(r: &Report) -> i32 {
    let bad = r.findings.iter().any(|f| f.severity >= Severity::Warn) || !r.parse_errors.is_empty();
    i32::from(bad)
}

pub fn rule_meta(id: &str) -> Option<RuleMeta> {
    all_rules().into_iter().map(|r| r.meta()).find(|m| m.id == id)
}

pub fn format_finding(f: &Finding, hints: usize) -> String {
    let m = rule_meta(f.rule).expect("finding from a known rule");
    let mut s = format!("{}:{}:{}: {} {} {}: {}", f.file, f.line, f.col, f.severity, m.id, m.name, f.message);
    for (i, h) in m.hints.iter().take(hints).enumerate() {
        s.push_str(&format!("\n    hint {}: {h}", i + 1));
    }
    s
}

pub fn format_report(r: &Report, hints: usize) -> String {
    let mut s = String::new();
    for e in &r.parse_errors {
        s.push_str(e);
        s.push('\n');
    }
    for f in &r.findings {
        s.push_str(&format_finding(f, hints));
        s.push('\n');
    }
    let count = |sev| r.findings.iter().filter(|f| f.severity == sev).count();
    s.push_str(&format!(
        "{} findings: {} WARN, {} ADVICE{}\n",
        r.findings.len(),
        count(Severity::Warn),
        count(Severity::Advice),
        if r.parse_errors.is_empty() { String::new() } else { format!(", {} parse errors", r.parse_errors.len()) }
    ));
    s
}

pub fn format_rules() -> String {
    let mut s = String::new();
    for r in all_rules() {
        let m = r.meta();
        s.push_str(&format!("{} {} [{}] kc={}\n", m.id, m.name, m.severity, m.kc));
        s.push_str(&format!("  false positives: {}\n", m.fp_risk));
        s.push_str(&format!("  measured by:     {}\n", m.measured_by));
        for (i, h) in m.hints.iter().enumerate() {
            s.push_str(&format!("  hint {}: {h}\n", i + 1));
        }
        s.push('\n');
    }
    s
}
