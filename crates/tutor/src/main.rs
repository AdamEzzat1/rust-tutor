use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tutor::util::{self, Res};
use tutor::*;

const USAGE: &str = "\
tutor: a deterministic Rust tutor for scientific computing (no LLM in the loop)

Projects (hole-scaffolded crates; projects/<p>/):
  tutor start <p> <stage> <dir>      copy a starter stage to work on (work/ is gitignored)
  tutor test <dir>                   per-test results
  tutor explain <dir> [--hints=N] [--no-record]
                                     tests + compiler errors + findings + measurements + where to start
  tutor progress                     learner model: BKT per KC, hole attempts, lesson reviews due
Checker:
  tutor check <paths...> [--hints=N] static checker; exit 1 on WARN, ADVICE never
  tutor rules                        every rule with its false-positive note and hints
Lessons and course:
  tutor lessons [list|verify]        predict-the-outcome lessons; verify runs every key on the pinned rustc
  tutor lesson <id>                  take a lesson (answers on stdin)
  tutor courses                      the courses, units, outcomes
  tutor course list|record|verify [unit]
  tutor practice list | <id> [file.rs]   show a problem / grade a solution (correctness, compile errors,
                                     allocations, speed relative to the reference)
  tutor run <file.rs>                compile and run one file as the grader does
Authoring:
  tutor scaffold <p> [--check]       (re)generate / diff starter stages
  tutor holemap <p> [--check]        measured test->hole map + mutation tests (slow)
  tutor baseline <p> [--check]       measurements of the reference
  tutor fixtures <p> [--check]       explain on buggy fixtures vs golden outputs
";

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn opt(args: &[String], name: &str) -> Option<String> {
    let pre = format!("{name}=");
    args.iter().find_map(|a| a.strip_prefix(&pre).map(str::to_string))
}

fn positional(args: &[String]) -> Vec<String> {
    args.iter().filter(|a| !a.starts_with("--")).cloned().collect()
}

fn need<'a>(pos: &'a [String], i: usize, what: &str) -> Res<&'a str> {
    pos.get(i).map(String::as_str).ok_or_else(|| format!("missing argument: {what}\n\n{USAGE}"))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

fn log(s: &str) {
    eprintln!("  .. {s}");
}

fn report_problems(what: &str, problems: &[String]) -> i32 {
    if problems.is_empty() {
        println!("{what}: OK");
        0
    } else {
        for p in problems {
            println!("PROBLEM: {p}");
        }
        println!("{what}: {} problems", problems.len());
        1
    }
}

fn run(args: &[String]) -> Res<i32> {
    let Some(cmd) = args.first() else {
        print!("{USAGE}");
        return Ok(0);
    };
    let rest = &args[1..];
    let pos = positional(rest);
    let hints: usize = opt(rest, "--hints").and_then(|h| h.parse().ok()).unwrap_or(0);
    match cmd.as_str() {
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(0)
        }
        "check" => {
            if pos.is_empty() {
                return Err(format!("check needs paths\n\n{USAGE}"));
            }
            let files = checker::collect_files(&pos);
            let r = checker::check_paths(&files, &std::env::current_dir().unwrap_or_default());
            print!("{}", checker::format_report(&r, hints));
            Ok(checker::exit_code(&r))
        }
        "rules" => {
            print!("{}", checker::format_rules());
            Ok(0)
        }
        "scaffold" => {
            let p = need(&pos, 0, "project")?;
            let check = flag(rest, "--check");
            let problems = scaffold::scaffold(p, check)?;
            if check {
                Ok(report_problems("scaffold --check", &problems))
            } else {
                println!("wrote projects/{p}/starter/");
                Ok(0)
            }
        }
        "start" => {
            let p = need(&pos, 0, "project")?;
            let stage: u32 = need(&pos, 1, "stage")?.parse().map_err(|_| "stage must be a number".to_string())?;
            let dir = PathBuf::from(need(&pos, 2, "dir")?);
            let src = scaffold::stage_dir(p, stage);
            if !src.is_dir() {
                return Err(format!("no stage {stage} for project {p} (run `tutor scaffold {p}`)"));
            }
            if dir.exists() && std::fs::read_dir(&dir).map(|mut d| d.next().is_some()).unwrap_or(false) {
                return Err(format!("{} exists and is not empty", dir.display()));
            }
            util::copy_dir(&src, &dir)?;
            let _ = std::fs::remove_file(dir.join("STARTER.md"));
            explain::write_stage(&dir, p, stage)?;
            let proj = scaffold::Project::load(p)?;
            println!("Started {p} stage {stage} in {}. Open holes:", dir.display());
            for h in proj.holes_at(stage) {
                println!("  {:<16} {}", h.id, h.summary);
            }
            println!("\nThen: tutor explain {} --hints=1", dir.display());
            Ok(0)
        }
        "test" => {
            util::require_pinned_toolchain()?;
            let dir = PathBuf::from(need(&pos, 0, "dir")?);
            let st = explain::read_stage(&dir)?;
            let run = runner::run_tests(&dir, &st.project)?;
            match &run.status {
                runner::Status::CompileError(d) => {
                    println!("does not compile:");
                    for d in d {
                        println!("  {}:{}:{}: error[{}]: {}", d.file, d.line, d.col, d.code.as_deref().unwrap_or("-"), d.message);
                    }
                    return Ok(1);
                }
                runner::Status::Timeout => println!("TIMEOUT"),
                runner::Status::Crashed(e) => println!("crashed: {e}"),
                runner::Status::Ran => {}
            }
            for t in &run.tests {
                println!("{} {}", if t.passed { "pass" } else { "FAIL" }, t.id);
                if let Some(l) = &t.location {
                    println!("       {l}: {}", t.message.first().cloned().unwrap_or_default());
                }
            }
            let f = run.failing().len();
            println!("{}/{} pass", run.tests.len() - f, run.tests.len());
            Ok(i32::from(f > 0 || run.status != runner::Status::Ran))
        }
        "explain" => {
            util::require_pinned_toolchain()?;
            let dir = PathBuf::from(need(&pos, 0, "dir")?);
            let ex = explain::explain(&dir, hints.max(if opt(rest, "--hints").is_none() { 1 } else { 0 }))?;
            print!("{}", ex.text);
            if !flag(rest, "--no-record") {
                let st = explain::read_stage(&dir)?;
                let p = scaffold::Project::load(&st.project)?;
                let map = holemap::HoleMap::load(&st.project)?;
                let src = util::read(&dir.join(&p.file))?;
                let mut state = progress::load();
                let lines = progress::record(&mut state, &p, &map, &src, &ex.evidence);
                progress::save(&state)?;
                for l in lines {
                    println!("{l}");
                }
            }
            Ok(i32::from(!ex.evidence.run.failing().is_empty() || ex.evidence.run.status != runner::Status::Ran))
        }
        "progress" => {
            print!("{}", progress::format(&progress::load()));
            Ok(0)
        }
        "holemap" => {
            util::require_pinned_toolchain()?;
            let p = need(&pos, 0, "project")?;
            let out = holemap::build(p, &mut log)?;
            let path = scaffold::project_dir(p).join("holemap.toml");
            let mut problems = out.problems;
            if flag(rest, "--check") {
                let old = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
                if old != out.text {
                    problems.push(format!(
                        "holemap.toml is stale, {}",
                        util::first_difference(&old, &out.text).unwrap_or_default()
                    ));
                }
            } else {
                util::write(&path, &out.text)?;
                println!("wrote {}", util::rel_display(&path));
            }
            Ok(report_problems("holemap", &problems))
        }
        "baseline" => {
            util::require_pinned_toolchain()?;
            let p = need(&pos, 0, "project")?;
            let ms = runner::run_example(&scaffold::reference_dir(p), p, "measure")?;
            let mut text = format!(
                "# Generated by `tutor baseline {p}`: examples/measure.rs on the reference.\n# Measured on rustc {}.\n\n[measurement]\n",
                util::rustc_version()?
            );
            for (k, v) in &ms {
                text.push_str(&format!("{k:?} = {v:?}\n"));
            }
            let path = scaffold::project_dir(p).join("measure_baseline.toml");
            if flag(rest, "--check") {
                let old = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
                let problems: Vec<String> =
                    util::first_difference(&old, &text).map(|d| vec![format!("baseline is stale, {d}")]).unwrap_or_default();
                Ok(report_problems("baseline", &problems))
            } else {
                util::write(&path, &text)?;
                print!("{text}");
                Ok(0)
            }
        }
        "fixtures" => {
            util::require_pinned_toolchain()?;
            let p = need(&pos, 0, "project")?;
            let check = flag(rest, "--check");
            let problems = fixtures::run(p, check, &mut log)?;
            Ok(report_problems(if check { "fixtures --check" } else { "fixtures" }, &problems))
        }
        "lessons" => {
            let sub = pos.first().map(String::as_str).unwrap_or("list");
            match sub {
                "list" => {
                    for l in lessons::load_all()? {
                        println!("{}  {}  ({} checks, verified on {})", l.id, l.title, l.check.len(), l.verified_on);
                    }
                    Ok(0)
                }
                "verify" => {
                    util::require_pinned_toolchain()?;
                    let problems = lessons::verify_all(&mut log)?;
                    Ok(report_problems("lessons verify", &problems))
                }
                other => Err(format!("unknown lessons subcommand {other}")),
            }
        }
        "lesson" => {
            let id = need(&pos, 0, "lesson id")?;
            lessons::interactive(id)
        }
        "courses" => {
            print!("{}", course::format_courses()?);
            Ok(0)
        }
        "course" => {
            let sub = need(&pos, 0, "list|record|verify")?;
            let unit = pos.get(1).map(String::as_str);
            match sub {
                "list" => {
                    print!("{}", course::format_units()?);
                    Ok(0)
                }
                "record" => {
                    util::require_pinned_toolchain()?;
                    course::record(unit, &mut log)?;
                    Ok(0)
                }
                "verify" => {
                    util::require_pinned_toolchain()?;
                    let problems = course::verify(unit, &mut log)?;
                    Ok(report_problems("course verify", &problems))
                }
                other => Err(format!("unknown course subcommand {other}")),
            }
        }
        "practice" => {
            let sub = need(&pos, 0, "list|<problem id>")?;
            if sub == "list" {
                print!("{}", practice::format_list()?);
                return Ok(0);
            }
            match pos.get(1) {
                None => {
                    print!("{}", practice::format_problem(sub)?);
                    Ok(0)
                }
                Some(file) => {
                    util::require_pinned_toolchain()?;
                    let code = util::read(Path::new(file))?;
                    let (text, ok) = practice::grade_submission(sub, &code)?;
                    print!("{text}");
                    Ok(i32::from(!ok))
                }
            }
        }
        "run" => {
            util::require_pinned_toolchain()?;
            let file = need(&pos, 0, "file.rs")?;
            let code = util::read(Path::new(file))?;
            let o = snippet::run_program(&code, snippet::Profile::Debug)?;
            print!("{}", o.render());
            Ok(i32::from(!o.ok()))
        }
        "version" => {
            println!("tutor {} (pinned rustc {})", env!("CARGO_PKG_VERSION"), util::pinned_rust()?);
            Ok(0)
        }
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    }
}
