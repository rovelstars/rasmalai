use std::path::Path;
use std::process::{Command, Stdio};

pub const WORKER_ENV: &str = "RNX_TEST_WORKER";

pub fn is_worker() -> bool {
    std::env::var_os(WORKER_ENV).is_some()
}

pub struct TargetRequest {
    pub pkg_root: std::path::PathBuf,
    pub filter: Option<String>,
    pub opt_level: String,
    pub backend: String,
    pub exact: bool,
    pub no_color: bool,
    pub quiet: bool,
    pub jobs: Option<usize>,
    pub memory_cap: Option<u64>,
}

fn child_args(req: &TargetRequest) -> Vec<String> {
    let mut args = vec!["test".to_string()];
    if let Some(f) = &req.filter {
        args.push(f.clone());
    }
    args.push("-O".to_string());
    args.push(req.opt_level.clone());
    args.push("--backend".to_string());
    args.push(req.backend.clone());
    if req.exact {
        args.push("--exact".to_string());
    }
    if req.no_color {
        args.push("--no-color".to_string());
    }
    if let Some(jobs) = req.jobs {
        args.push("--jobs".to_string());
        args.push(jobs.to_string());
    }
    if let Some(cap) = req.memory_cap {
        args.push("--memory-cap".to_string());
        args.push(cap.to_string());
    }
    args
}

struct StreamState {
    pending: Vec<String>,
    all: Vec<String>,
    in_failure: bool,
    failure_captured: Vec<String>,
    saw_summary: bool,
    failed: bool,
}

impl StreamState {
    fn new() -> StreamState {
        StreamState {
            pending: Vec::new(),
            all: Vec::new(),
            in_failure: false,
            failure_captured: Vec::new(),
            saw_summary: false,
            failed: false,
        }
    }
}

fn emit_captured(live: &mut crate::term::Live, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    live.suspend();
    println!("│  captured stdout:");
    for line in lines {
        println!("│    {}", line.trim_end());
    }
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

fn parse_result_line(line: &str, marker: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix(marker)?.trim();
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    let duration = format!("{} {}", parts[parts.len() - 2], parts[parts.len() - 1]);
    let name = parts[..parts.len() - 2].join(" ");
    Some((name, duration))
}

fn live_rows(
    theme: &diagnostics::theme::AuraTheme,
    package: &str,
    passed: u64,
    failed: u64,
    elapsed_ms: f64,
    current: Option<&str>,
    recent: &[(bool, String, String)],
    frame: usize,
) -> Vec<String> {
    let width = crate::term::width();
    let mark = if current.is_some() {
        crate::term::Mark::Running(frame)
    } else if failed > 0 {
        crate::term::Mark::Failed
    } else {
        crate::term::Mark::Done
    };
    let counts = format!(
        "{passed} passed  {failed} failed  {}",
        crate::term::human_duration_ms(elapsed_ms)
    );
    let mut rows = vec![crate::term::row(
        theme,
        mark,
        &format!("test {package}"),
        &counts,
        width,
    )];
    if let Some(name) = current {
        rows.push(crate::term::row(theme, mark, name, "", width));
    }
    for (ok, name, duration) in recent {
        rows.push(crate::term::row(
            theme,
            if *ok {
                crate::term::Mark::Done
            } else {
                crate::term::Mark::Failed
            },
            name,
            duration,
            width,
        ));
    }
    rows
}

pub fn run_captured(
    theme: &diagnostics::theme::AuraTheme,
    req: &TargetRequest,
    pkg_root: &Path,
) -> i32 {
    use std::io::{BufRead, BufReader, Write};

    let exe = std::env::current_exe().unwrap_or_else(|_| Path::new("rnx").to_path_buf());
    let mut child = match Command::new(&exe)
        .args(child_args(req))
        .env(WORKER_ENV, "1")
        .current_dir(pkg_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("error: cannot spawn test worker: {e}");
            return 1;
        }
    };
    let stdout = match child.stdout.take() {
        Some(pipe) => pipe,
        None => {
            eprintln!("error: test worker produced no output pipe");
            return 1;
        }
    };
    let package = frontend::project::load_manifest(pkg_root)
        .ok()
        .flatten()
        .and_then(|m| m.project.map(|p| p.name))
        .or_else(|| pkg_root.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "package".to_string());
    let mut live = crate::term::Live::new(req.quiet);
    let echo_results = !std::io::IsTerminal::is_terminal(&std::io::stdout()) || !live.enabled();
    let started = std::time::Instant::now();
    let mut passed = 0u64;
    let mut failed = 0u64;
    let mut current: Option<String>;
    let mut recent: Vec<(bool, String, String)> = Vec::new();
    let mut frame = 0usize;
    let push_recent = |recent: &mut Vec<(bool, String, String)>, row: (bool, String, String)| {
        recent.push(row);
        if recent.len() > 4 {
            recent.remove(0);
        }
    };
    let mut st = StreamState::new();
    let mut reader = BufReader::new(stdout);
    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                live.finish();
                eprintln!("error: cannot read test output: {e}");
                let _ = child.wait();
                return 1;
            }
        }
        let line = buf.trim_end_matches(['\n', '\r']).to_string();
        st.all.push(line.clone());
        let plain = crate::term::strip_ansi(&line);
        // `  ↻` is a worker protocol line for the live block, never output.
        if plain.starts_with("  ↻") {
            current = Some(plain["  ↻".len()..].trim().to_string());
            live.set(live_rows(
                theme,
                &package,
                passed,
                failed,
                started.elapsed().as_secs_f64() * 1000.0,
                current.as_deref(),
                &recent,
                frame,
            ));
            frame += 1;
            continue;
        }
        if plain.starts_with("  ✓") {
            if st.in_failure {
                emit_captured(&mut live, &st.failure_captured);
                st.failure_captured.clear();
                st.in_failure = false;
            }
            st.pending.clear();
            passed += 1;
            if let Some((name, duration)) = parse_result_line(&plain, "  ✓") {
                push_recent(&mut recent, (true, name, duration));
            }
            current = None;
            live.set(live_rows(
                theme,
                &package,
                passed,
                failed,
                started.elapsed().as_secs_f64() * 1000.0,
                current.as_deref(),
                &recent,
                frame,
            ));
            if echo_results && !req.quiet {
                println!("{}", crate::telemetry::colorize_test_line(theme, &plain));
            }
        } else if plain.starts_with("  ✗") {
            st.failed = true;
            failed += 1;
            if let Some((name, duration)) = parse_result_line(&plain, "  ✗") {
                push_recent(&mut recent, (false, name, duration));
            }
            current = None;
            live.set(live_rows(
                theme,
                &package,
                passed,
                failed,
                started.elapsed().as_secs_f64() * 1000.0,
                current.as_deref(),
                &recent,
                frame,
            ));
            if !req.quiet {
                live.suspend();
                println!("{}", crate::telemetry::colorize_test_line(theme, &plain));
                let _ = std::io::stdout().flush();
            }
            st.failure_captured = std::mem::take(&mut st.pending);
            st.in_failure = true;
        } else if plain.starts_with("┌─ failure") || plain.starts_with("└─") {
            if !req.quiet {
                live.suspend();
                println!("{}", crate::telemetry::colorize_test_line(theme, &plain));
                let _ = std::io::stdout().flush();
            }
            if plain.starts_with("└─") && st.in_failure {
                emit_captured(&mut live, &st.failure_captured);
                st.failure_captured.clear();
                st.in_failure = false;
            }
        } else if plain.starts_with("tests:") {
            if st.in_failure {
                emit_captured(&mut live, &st.failure_captured);
                st.failure_captured.clear();
                st.in_failure = false;
            }
            st.saw_summary = true;
            live.finish();
            println!("{}", crate::telemetry::colorize_test_line(theme, &plain));
        } else if plain.is_empty() {
            if !req.quiet && !live.enabled() {
                println!();
            }
        } else if !plain.contains(' ') && plain.ends_with(".rnx") {
            if !req.quiet {
                live.set(Vec::new());
                live.suspend();
                println!("{}", theme.paint(diagnostics::theme::AuraColor::Cyan, &plain));
                let _ = std::io::stdout().flush();
            }
        } else {
            st.pending.push(line);
        }
    }
    let status = child.wait();
    live.finish();
    if st.in_failure {
        emit_captured(&mut live, &st.failure_captured);
    }
    if !st.saw_summary {
        for line in &st.all {
            println!("{line}");
        }
        return 1;
    }
    if !status.map(|s| s.success()).unwrap_or(false) || st.failed {
        return 1;
    }
    0
}

pub fn run_nocapture(req: &TargetRequest, pkg_root: &Path) -> i32 {
    let exe = std::env::current_exe().unwrap_or_else(|_| Path::new("rnx").to_path_buf());
    let status = Command::new(&exe)
        .args(child_args(req))
        .env(WORKER_ENV, "1")
        .current_dir(pkg_root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    match status {
        Ok(s) => {
            if s.success() {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("error: cannot spawn test worker: {e}");
            1
        }
    }
}
