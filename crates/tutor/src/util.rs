//! Small shared helpers: repository paths, the toolchain pin, processes with
//! a timeout, deterministic hashing and directory copies.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub type Res<T> = Result<T, String>;

/// The repository root: the nearest ancestor of the current directory (or of
/// the executable) that holds `RUST_VERSION`.
pub fn repo_root() -> PathBuf {
    if let Ok(r) = std::env::var("TUTOR_ROOT") {
        return PathBuf::from(r);
    }
    let mut starts = vec![];
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        starts.push(exe);
    }
    for s in starts {
        let mut d = s.as_path();
        loop {
            if d.join("RUST_VERSION").is_file() {
                return d.to_path_buf();
            }
            match d.parent() {
                Some(p) => d = p,
                None => break,
            }
        }
    }
    PathBuf::from(".")
}

pub fn read(path: &Path) -> Res<String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

pub fn write(path: &Path, text: &str) -> Res<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("cannot create {}: {e}", p.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub fn parse_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Res<T> {
    let text = read(path)?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The pinned toolchain version from `RUST_VERSION`.
pub fn pinned_rust() -> Res<String> {
    Ok(read(&repo_root().join("RUST_VERSION"))?.trim().to_string())
}

/// `rustc --version`'s version number, e.g. "1.97.1".
pub fn rustc_version() -> Res<String> {
    let out = Command::new("rustc")
        .arg("--version")
        .current_dir(repo_root())
        .output()
        .map_err(|e| format!("cannot run rustc: {e}"))?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.split_whitespace().nth(1).map(str::to_string).ok_or_else(|| format!("unexpected rustc output: {s}"))
}

/// Invariant 4: one pinned toolchain. Every command that compiles or runs
/// code refuses any other rustc.
pub fn require_pinned_toolchain() -> Res<String> {
    let want = pinned_rust()?;
    let have = rustc_version()?;
    if want != have {
        return Err(format!(
            "rustc {have} is not the pinned toolchain {want} (RUST_VERSION). \
             Run `rustup toolchain install {want}`; rust-toolchain.toml selects it inside this repository."
        ));
    }
    Ok(have)
}

pub struct RunOutput {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub elapsed: Duration,
}

/// Runs a command, killing it after `timeout`. stdout and stderr are read on
/// threads so a chatty child cannot deadlock on a full pipe.
pub fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Res<RunOutput> {
    let start = Instant::now();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {:?}: {e}", cmd.get_program()))?;
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let to = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        s
    });
    let te = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err.read_to_end(&mut s);
        s
    });
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.code(),
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    };
    let stdout = String::from_utf8_lossy(&to.join().unwrap_or_default()).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&te.join().unwrap_or_default()).replace("\r\n", "\n");
    Ok(RunOutput { status, stdout, stderr, timed_out, elapsed: start.elapsed() })
}

/// FNV-1a, 64 bit: a stable hash (std's `DefaultHasher` is explicitly not
/// stable across releases, so it must never reach a file).
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Copies a directory tree, skipping `target/` and `.tutor/`.
pub fn copy_dir(from: &Path, to: &Path) -> Res<()> {
    std::fs::create_dir_all(to).map_err(|e| format!("cannot create {}: {e}", to.display()))?;
    let mut entries: Vec<_> = std::fs::read_dir(from)
        .map_err(|e| format!("cannot read {}: {e}", from.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name();
        if name == "target" || name == ".tutor" || name == "Cargo.lock" {
            continue;
        }
        let src = e.path();
        let dst = to.join(&name);
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("cannot copy {}: {e}", src.display()))?;
        }
    }
    Ok(())
}

/// Every file under `dir` (relative paths with `/`), sorted; skips `target/`.
pub fn list_files(dir: &Path) -> Vec<String> {
    fn go(base: &Path, d: &Path, out: &mut Vec<String>) {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.filter_map(Result::ok) {
                let p = e.path();
                let name = e.file_name();
                if name == "target" || name == ".tutor" || name == "Cargo.lock" {
                    continue;
                }
                if p.is_dir() {
                    go(base, &p, out);
                } else if let Ok(rel) = p.strip_prefix(base) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let mut out = vec![];
    go(dir, dir, &mut out);
    out.sort();
    out
}

/// A fresh, empty scratch directory under `target/tutor-tmp/<name>`.
pub fn scratch_dir(name: &str) -> Res<PathBuf> {
    let d = repo_root().join("target").join("tutor-tmp").join(name);
    if d.exists() {
        std::fs::remove_dir_all(&d).map_err(|e| format!("cannot clear {}: {e}", d.display()))?;
    }
    std::fs::create_dir_all(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    Ok(d)
}

/// A shared cargo target dir per project, so mutants rebuild only the crate.
pub fn shared_target(project: &str) -> PathBuf {
    repo_root().join("target").join("tutor-cargo").join(project)
}

pub fn rel_display(p: &Path) -> String {
    let root = repo_root();
    p.strip_prefix(&root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// Unified-ish diff summary for `--check` commands: the first differing line.
pub fn first_difference(a: &str, b: &str) -> Option<String> {
    let la: Vec<&str> = a.lines().collect();
    let lb: Vec<&str> = b.lines().collect();
    for i in 0..la.len().max(lb.len()) {
        let x = la.get(i).copied().unwrap_or("<end>");
        let y = lb.get(i).copied().unwrap_or("<end>");
        if x != y {
            return Some(format!("line {}:\n  committed: {x}\n  generated: {y}", i + 1));
        }
    }
    None
}
