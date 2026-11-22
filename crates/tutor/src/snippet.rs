//! Compile and run one self-contained Rust file with the pinned `rustc`
//! (no cargo, std only). Used by lessons, course examples and the grader.
//!
//! Every run has a canonical outcome key, compared verbatim with lesson
//! answers and recorded outputs:
//! * `compile error E0382` (the first error code; `compile error` if none),
//! * `panic: <first line of the panic message>` (stdout before it is kept
//!   separately),
//! * otherwise the program's stdout, trailing whitespace trimmed.

use crate::runner::{self, Diagnostic};
use crate::util::{self, Res};
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// opt-level 0 with debug assertions and overflow checks (what `cargo test` uses).
    Debug,
    /// opt-level 3, no overflow checks (what `cargo build --release` uses).
    Release,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    CompileError(Vec<Diagnostic>),
    Ran { stdout: String, stderr: String, exit: Option<i32>, panic: Option<String>, timed_out: bool },
}

impl Outcome {
    pub fn ok(&self) -> bool {
        matches!(self, Outcome::Ran { exit: Some(0), .. })
    }

    pub fn key(&self) -> String {
        match self {
            Outcome::CompileError(d) => match d.iter().find_map(|d| d.code.clone()) {
                Some(c) => format!("compile error {c}"),
                None => "compile error".to_string(),
            },
            Outcome::Ran { timed_out: true, .. } => "timeout".to_string(),
            Outcome::Ran { panic: Some(p), .. } => format!("panic: {p}"),
            Outcome::Ran { stdout, exit, .. } => {
                let s = stdout.trim_end().to_string();
                if *exit == Some(0) {
                    s
                } else {
                    format!("{s}\nexit code {}", exit.map_or("killed".to_string(), |c| c.to_string()))
                }
            }
        }
    }

    pub fn render(&self) -> String {
        match self {
            Outcome::CompileError(d) => {
                let mut s = String::from("does not compile:\n");
                for d in d {
                    s.push_str(&format!("  line {}:{}: error{}: {}\n", d.line, d.col, d.code.as_deref().map(|c| format!("[{c}]")).unwrap_or_default(), d.message));
                }
                s
            }
            Outcome::Ran { stdout, panic, timed_out, .. } => {
                let mut s = stdout.clone();
                if let Some(p) = panic {
                    s.push_str(&format!("panicked: {p}\n"));
                }
                if *timed_out {
                    s.push_str("timed out\n");
                }
                s
            }
        }
    }
}

pub fn panic_message(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr.lines().collect();
    let i = lines.iter().position(|l| l.contains("panicked at"))?;
    lines.get(i + 1).map(|l| l.trim().to_string())
}

/// Compiles `code` to an executable, cached by (source, profile, rustc).
pub fn compile(code: &str, profile: Profile, extra: &[&str]) -> Res<Result<std::path::PathBuf, Vec<Diagnostic>>> {
    let rustc = util::rustc_version()?;
    let key = format!("{:016x}", util::fnv1a(&format!("{rustc}\n{profile:?}\n{extra:?}\n{code}")));
    let dir = util::repo_root().join("target").join("tutor-snippets").join(&key);
    let exe = dir.join(if cfg!(windows) { "snippet.exe" } else { "snippet" });
    if exe.is_file() {
        return Ok(Ok(exe));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let src = dir.join("snippet.rs");
    util::write(&src, code)?;
    let mut c = Command::new("rustc");
    c.current_dir(&dir).args(["--edition", "2021", "--error-format=short", "--crate-name", "snippet", "-A", "warnings"]);
    match profile {
        Profile::Debug => c.args(["-C", "opt-level=0", "-C", "debug-assertions=on", "-C", "overflow-checks=on"]),
        Profile::Release => c.args(["-C", "opt-level=3", "-C", "debug-assertions=off", "-C", "overflow-checks=off"]),
    };
    c.args(extra).arg("-o").arg(&exe).arg("snippet.rs");
    let out = util::run_with_timeout(&mut c, Duration::from_secs(120))?;
    if out.status != Some(0) {
        let mut d = runner::parse_diagnostics(&out.stderr);
        for x in &mut d {
            x.file = "snippet.rs".into();
        }
        if d.is_empty() {
            return Err(format!("rustc failed without diagnostics:\n{}", out.stderr));
        }
        let _ = std::fs::remove_file(&exe);
        return Ok(Err(d));
    }
    Ok(Ok(exe))
}

pub fn run_exe(exe: &std::path::Path, args: &[&str], timeout: Duration) -> Res<Outcome> {
    let mut c = Command::new(exe);
    c.args(args).env("RUST_BACKTRACE", "0");
    let o = util::run_with_timeout(&mut c, timeout)?;
    let panic = if o.status == Some(101) { panic_message(&o.stderr) } else { None };
    Ok(Outcome::Ran { stdout: o.stdout, stderr: o.stderr, exit: o.status, panic, timed_out: o.timed_out })
}

pub fn run_program(code: &str, profile: Profile) -> Res<Outcome> {
    match compile(code, profile, &[])? {
        Err(d) => Ok(Outcome::CompileError(d)),
        Ok(exe) => run_exe(&exe, &[], Duration::from_secs(20)),
    }
}

/// Wraps a snippet that has no `fn main` in one.
pub fn as_program(code: &str) -> String {
    let has_main = syn::parse_file(code)
        .map(|f| f.items.iter().any(|i| matches!(i, syn::Item::Fn(f) if f.sig.ident == "main")))
        .unwrap_or(false);
    if has_main {
        code.to_string()
    } else {
        format!("fn main() {{\n{code}\n}}\n")
    }
}
