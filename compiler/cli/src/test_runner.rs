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

fn emit_captured(lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    println!("│  captured stdout:");
    for line in lines {
        println!("│    {}", line.trim_end());
    }
}

pub fn run_captured(
    theme: &diagnostics::theme::AuraTheme,
    req: &TargetRequest,
    pkg_root: &Path,
) -> i32 {
    let exe = std::env::current_exe().unwrap_or_else(|_| Path::new("rnx").to_path_buf());
    let out = Command::new(&exe)
        .args(child_args(req))
        .env(WORKER_ENV, "1")
        .current_dir(pkg_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: cannot spawn test worker: {e}");
            return 1;
        }
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut st = StreamState::new();
    for raw in text.lines() {
        let line = raw.to_string();
        st.all.push(line.clone());
        if line.starts_with("  ✓") {
            if st.in_failure {
                emit_captured(&st.failure_captured);
                st.failure_captured.clear();
                st.in_failure = false;
            }
            st.pending.clear();
            if !req.quiet {
                println!("{}", crate::telemetry::colorize_test_line(theme, &line));
            }
        } else if line.starts_with("  ✗") {
            st.failed = true;
            if !req.quiet {
                println!("{}", crate::telemetry::colorize_test_line(theme, &line));
            }
            st.failure_captured = std::mem::take(&mut st.pending);
            st.in_failure = true;
        } else if line.starts_with("┌─ failure") || line.starts_with("└─") {
            if !req.quiet {
                println!("{}", crate::telemetry::colorize_test_line(theme, &line));
            }
            if line.starts_with("└─") && st.in_failure {
                emit_captured(&st.failure_captured);
                st.failure_captured.clear();
                st.in_failure = false;
            }
        } else if line.starts_with("tests:") {
            if st.in_failure {
                emit_captured(&st.failure_captured);
                st.failure_captured.clear();
                st.in_failure = false;
            }
            st.saw_summary = true;
            println!("{}", crate::telemetry::colorize_test_line(theme, &line));
        } else if line.is_empty() {
            if !req.quiet {
                println!();
            }
        } else if !line.contains(' ') && line.ends_with(".rnx") {
            if !req.quiet {
                println!("{line}");
            }
        } else {
            st.pending.push(line);
        }
    }
    if st.in_failure {
        emit_captured(&st.failure_captured);
    }
    if !st.saw_summary {
        for line in &st.all {
            println!("{line}");
        }
        return 1;
    }
    if !out.status.success() || st.failed {
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
