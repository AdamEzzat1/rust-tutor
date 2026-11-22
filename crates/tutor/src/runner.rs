//! Runs a crate's tests in a fresh `cargo test` process and parses libtest's
//! stable, human-readable output (`--format json` is nightly-only).
//!
//! Determinism: one test thread, no colour, no backtraces; thread ids are
//! stripped from panic messages; results are sorted by test id.

use crate::util::{self, Res};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    /// `<binary>::<path::to::test>`, e.g. `behaviour::rk4::order_is_four`.
    pub id: String,
    pub passed: bool,
    /// `tests/behaviour.rs:215` of the panic, if any.
    pub location: Option<String>,
    /// First lines of the panic message.
    pub message: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Ran,
    CompileError(Vec<Diagnostic>),
    Timeout,
    Crashed(String),
}

#[derive(Debug, Clone)]
pub struct TestRun {
    pub status: Status,
    pub tests: Vec<TestResult>,
    pub elapsed: Duration,
}

impl TestRun {
    pub fn failing(&self) -> Vec<&TestResult> {
        self.tests.iter().filter(|t| !t.passed).collect()
    }
    pub fn passed(&self, id: &str) -> Option<bool> {
        self.tests.iter().find(|t| t.id == id).map(|t| t.passed)
    }
}

pub const TEST_TIMEOUT: Duration = Duration::from_secs(300);

pub fn cargo(dir: &Path, project: &str) -> Command {
    let mut c = Command::new("cargo");
    c.current_dir(dir)
        .env("CARGO_TARGET_DIR", util::shared_target(project))
        .env("RUST_BACKTRACE", "0")
        .env("CARGO_TERM_COLOR", "never")
        .env_remove("RUSTFLAGS");
    c
}

/// `cargo test` over the crate's unit and integration tests.
pub fn run_tests(dir: &Path, project: &str) -> Res<TestRun> {
    let mut c = cargo(dir, project);
    c.args(["test", "--tests", "--no-fail-fast", "--message-format=short", "--", "--test-threads=1", "--color", "never"]);
    let out = util::run_with_timeout(&mut c, TEST_TIMEOUT)?;
    let mut run = parse(&out.stdout, &out.stderr);
    run.elapsed = out.elapsed;
    if out.timed_out {
        run.status = Status::Timeout;
    } else if run.tests.is_empty() && run.status == Status::Ran {
        let tail: Vec<&str> = out.stderr.lines().rev().take(5).collect();
        run.status = Status::Crashed(tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    Ok(run)
}

pub fn parse_diagnostics(stderr: &str) -> Vec<Diagnostic> {
    let mut out = vec![];
    for l in stderr.lines() {
        // short format: `src\lib.rs:120:5: error[E0308]: mismatched types`
        let Some(pos) = l.find(": error") else { continue };
        let (loc, rest) = l.split_at(pos);
        let mut parts = loc.rsplitn(3, ':');
        let (Some(col), Some(line), Some(file)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let (Ok(line), Ok(col)) = (line.parse(), col.parse()) else { continue };
        let rest = rest.trim_start_matches(": error");
        let (code, message) = if let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']').unwrap_or(0);
            (Some(r[..end].to_string()), r[end + 1..].trim_start_matches(':').trim().to_string())
        } else {
            (None, rest.trim_start_matches(':').trim().to_string())
        };
        out.push(Diagnostic { file: file.trim().replace('\\', "/"), line, col, code, message });
    }
    out
}

fn binary_names(stderr: &str) -> Vec<String> {
    let mut out = vec![];
    for l in stderr.lines() {
        let t = l.trim();
        if let Some(rest) = t.strip_prefix("Running ") {
            let path = rest.split(" (").next().unwrap_or(rest).replace('\\', "/");
            let name = if path.starts_with("unittests ") {
                "unit".to_string()
            } else {
                Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(path)
            };
            out.push(name);
        }
    }
    out
}

fn strip_thread_id(s: &str) -> String {
    // `thread 'name' (12345) panicked at` -> `thread 'name' panicked at`
    if let (Some(a), Some(b)) = (s.find("' ("), s.find(") panicked")) {
        if a < b {
            return format!("{}'{}", &s[..a], &s[b + 1..]);
        }
    }
    s.to_string()
}

pub fn parse(stdout: &str, stderr: &str) -> TestRun {
    let diags = parse_diagnostics(stderr);
    let compiled_nothing = !stdout.contains("running ");
    if compiled_nothing && (!diags.is_empty() || stderr.contains("could not compile")) {
        return TestRun { status: Status::CompileError(diags), tests: vec![], elapsed: Duration::ZERO };
    }
    let binaries = binary_names(stderr);
    let mut tests = vec![];
    let mut block = 0usize;
    let mut current = String::new();
    let lines: Vec<&str> = stdout.lines().collect();
    let mut i = 0;
    // failure details by (binary, test path)
    let mut details: std::collections::BTreeMap<String, (Option<String>, Vec<String>)> = Default::default();
    while i < lines.len() {
        let l = lines[i];
        if l.starts_with("running ") && l.ends_with(" tests") || l == "running 1 test" {
            current = binaries.get(block).cloned().unwrap_or_else(|| format!("bin{block}"));
            block += 1;
        } else if let Some(rest) = l.strip_prefix("test ") {
            if let Some((name, verdict)) = rest.rsplit_once(" ... ") {
                let name = name.trim_end_matches(" - should panic").trim();
                let verdict = verdict.trim();
                if verdict == "ok" || verdict == "FAILED" {
                    tests.push(TestResult {
                        id: format!("{current}::{name}"),
                        passed: verdict == "ok",
                        location: None,
                        message: vec![],
                    });
                }
            }
        } else if let Some(rest) = l.strip_prefix("---- ") {
            if let Some(name) = rest.strip_suffix(" stdout ----") {
                let key = format!("{current}::{name}");
                let mut loc = None;
                let mut msg = vec![];
                let mut j = i + 1;
                while j < lines.len() && !lines[j].starts_with("---- ") && lines[j] != "failures:" {
                    let t = strip_thread_id(lines[j]);
                    if let Some(p) = t.find("panicked at ") {
                        let at = t[p + "panicked at ".len()..].trim_end_matches(':').replace('\\', "/");
                        // keep file:line, drop the column
                        let mut parts: Vec<&str> = at.split(':').collect();
                        if parts.len() >= 3 {
                            parts.pop();
                        }
                        loc = Some(parts.join(":"));
                        let mut k = j + 1;
                        while k < lines.len() && !lines[k].trim().is_empty() && !lines[k].starts_with("note:") && msg.len() < 3 {
                            let mut m = lines[k].trim().to_string();
                            if m.len() > 220 {
                                let mut cut = 217;
                                while !m.is_char_boundary(cut) {
                                    cut -= 1;
                                }
                                m.truncate(cut);
                                m.push_str("...");
                            }
                            msg.push(m);
                            k += 1;
                        }
                        break;
                    }
                    j += 1;
                }
                details.insert(key, (loc, msg));
            }
        }
        i += 1;
    }
    for t in &mut tests {
        if let Some((loc, msg)) = details.remove(&t.id) {
            t.location = loc;
            t.message = msg;
        }
    }
    tests.sort_by(|a, b| a.id.cmp(&b.id));
    TestRun { status: Status::Ran, tests, elapsed: Duration::ZERO }
}

/// Runs `cargo run --release --example <name>` and parses `key = value` lines.
pub fn run_example(dir: &Path, project: &str, example: &str) -> Res<Vec<(String, String)>> {
    let mut c = cargo(dir, project);
    c.args(["run", "--quiet", "--release", "--example", example]);
    let out = util::run_with_timeout(&mut c, Duration::from_secs(300))?;
    if out.status != Some(0) {
        return Err(format!("example {example} failed: {}", out.stderr.lines().last().unwrap_or("")));
    }
    Ok(out
        .stdout
        .lines()
        .filter_map(|l| l.split_once(" = ").map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect())
}
