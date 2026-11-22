use std::path::{Path, PathBuf};

fn artifact_from(stdout: &str) -> Option<PathBuf> {
    for line in stdout.lines() {
        let Some(rest) = line.trim().strip_prefix("artifact: ") else {
            continue;
        };
        let rest = rest.strip_suffix(" (fresh)").unwrap_or(rest);
        let path = match rest.find(" [") {
            Some(i) => &rest[..i],
            None => rest,
        };
        return Some(PathBuf::from(path.trim()));
    }
    None
}

pub(super) fn run_bench(
    package: Option<String>,
    filter: Option<String>,
    jobs: Option<usize>,
    memory_cap: Option<u64>,
    verbose: bool,
    quiet: bool,
) {
    let targets = cli::test_targets(package.as_deref()).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("rnx bench: cannot find the running binary: {e}");
        std::process::exit(1);
    });
    let mut code = 0;
    for (pkg_root, _) in &targets {
        let artifact = match build_harness(&exe, pkg_root, filter.as_deref(), jobs, memory_cap, verbose, quiet) {
            Ok(path) => path,
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        };
        if run_artifact(&artifact, pkg_root, verbose, quiet, filter.as_deref()) != 0 {
            code = 1;
        }
    }
    std::process::exit(code);
}

fn run_artifact(artifact: &Path, pkg_root: &Path, verbose: bool, quiet: bool, filter: Option<&str>) -> i32 {
    use std::io::{BufRead, BufReader, Write};

    let package = frontend::project::load_manifest(pkg_root)
        .ok()
        .flatten()
        .and_then(|m| m.project.map(|p| p.name))
        .or_else(|| pkg_root.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "package".to_string());
    let theme = diagnostics::theme::AuraTheme::active();
    let mut live = cli::term::Live::new(quiet);
    let started = std::time::Instant::now();
    let mut child = match std::process::Command::new(artifact)
        .current_dir(pkg_root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("rnx bench: cannot run {}: {e}", artifact.display());
            return 1;
        }
    };
    if verbose {
        eprintln!("rnx bench: {}", artifact.display());
    }
    let stdout = match child.stdout.take() {
        Some(pipe) => pipe,
        None => return 1,
    };
    let mut reader = BufReader::new(stdout);
    let mut buf = String::new();
    let mut current: Option<String>;
    let mut done = 0u64;
    let mut recent: Vec<(String, String)> = Vec::new();
    let mut frame = 0usize;
    let mut saw_result = false;
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                live.finish();
                eprintln!("rnx bench: cannot read results: {e}");
                let _ = child.wait();
                return 1;
            }
        }
        let line = buf.trim_end_matches(['\n', '\r']).to_string();
        if let Some(name) = line.strip_prefix("  ↻ bench ") {
            current = Some(name.trim().to_string());
            live.set(bench_rows(
                &theme,
                &package,
                done,
                started.elapsed().as_secs_f64() * 1000.0,
                current.as_deref(),
                &recent,
                frame,
            ));
            frame += 1;
            continue;
        }
        if let Some((name, detail)) = parse_bench_line(&line) {
            saw_result = true;
            done += 1;
            current = None;
            recent.push((name, detail));
            if recent.len() > 4 {
                recent.remove(0);
            }
            live.suspend();
            println!("{}", format_bench_line(&theme, &line));
            let _ = std::io::stdout().flush();
            if live.enabled() {
                live.set(bench_rows(
                    &theme,
                    &package,
                    done,
                    started.elapsed().as_secs_f64() * 1000.0,
                    current.as_deref(),
                    &recent,
                    frame,
                ));
            }
        } else if !line.trim().is_empty() {
            live.suspend();
            println!("{line}");
            let _ = std::io::stdout().flush();
            live.resume();
        }
    }
    let status = child.wait();
    live.finish();
    if !saw_result {
        if status.as_ref().map(|s| s.success()).unwrap_or(false) {
            match filter {
                Some(name) => eprintln!("No bench blocks matched `{name}`."),
                None => eprintln!("No bench blocks found."),
            }
            return if filter.is_some() { 1 } else { 0 };
        }
        return 1;
    }
    if !status.map(|s| s.success()).unwrap_or(false) {
        return 1;
    }
    if !quiet {
        eprintln!(
            "Ran {} benchmark block{} in {}.",
            done,
            if done == 1 { "" } else { "s" },
            cli::term::human_duration_ms(started.elapsed().as_secs_f64() * 1000.0)
        );
    }
    0
}

struct BenchResult<'a> {
    name: &'a str,
    mean: &'a str,
    sd: &'a str,
    iters: &'a str,
}

fn parse_bench_parts(line: &str) -> Option<BenchResult<'_>> {
    let rest = line.strip_prefix("bench ")?;
    let (name, tail) = rest.split_once(" ... ")?;
    let mean_end = tail.find(" ns/iter")?;
    let mean = &tail[..mean_end];
    let after = &tail[mean_end + " ns/iter".len()..];
    let sd = after.strip_prefix(" (+/- ").and_then(|r| r.split(')').next())?;
    let iters = line
        .rsplit('[')
        .next()
        .and_then(|s| s.strip_suffix(" iters]"))?;
    Some(BenchResult {
        name: name.trim(),
        mean,
        sd,
        iters,
    })
}

fn bench_number(text: &str) -> String {
    match text.trim().parse::<f64>() {
        Ok(v) if v >= 100.0 => format!("{v:.1}"),
        Ok(v) if v >= 1.0 => format!("{v:.2}"),
        Ok(v) if v >= 0.01 => format!("{v:.3}"),
        Ok(v) => format!("{v:.4}"),
        Err(_) => text.trim().to_string(),
    }
}

fn parse_bench_line(line: &str) -> Option<(String, String)> {
    let parts = parse_bench_parts(line)?;
    let iters = parts
        .iters
        .parse::<u64>()
        .map(cli::term::human_count)
        .unwrap_or_else(|_| parts.iters.to_string());
    Some((
        parts.name.to_string(),
        format!("{} ns/iter {iters} iters", bench_number(parts.mean)),
    ))
}

fn format_bench_line(theme: &diagnostics::theme::AuraTheme, line: &str) -> String {
    let Some(parts) = parse_bench_parts(line) else {
        return line.to_string();
    };
    let iters = parts
        .iters
        .parse::<u64>()
        .map(cli::term::human_count)
        .unwrap_or_else(|_| parts.iters.to_string());
    format!(
        "{} ... {}{}{}{}",
        theme.paint(
            diagnostics::theme::AuraColor::Cyan,
            &format!("bench {}", parts.name)
        ),
        theme.paint(
            diagnostics::theme::AuraColor::Orange,
            &bench_number(parts.mean)
        ),
        theme.paint(diagnostics::theme::AuraColor::Muted, " ns/iter"),
        theme.paint(
            diagnostics::theme::AuraColor::Muted,
            &format!(" (+/- {})", bench_number(parts.sd))
        ),
        theme.paint(
            diagnostics::theme::AuraColor::Muted,
            &format!(" [{iters} iters]")
        )
    )
}

fn bench_rows(
    theme: &diagnostics::theme::AuraTheme,
    package: &str,
    done: u64,
    elapsed_ms: f64,
    current: Option<&str>,
    recent: &[(String, String)],
    frame: usize,
) -> Vec<String> {
    let width = cli::term::width();
    let mark = if current.is_some() {
        cli::term::Mark::Running(frame)
    } else {
        cli::term::Mark::Done
    };
    let mut rows = vec![cli::term::row(
        theme,
        mark,
        &format!("bench {package}"),
        &format!(
            "{done} done  {}",
            cli::term::human_duration_ms(elapsed_ms)
        ),
        width,
    )];
    if let Some(name) = current {
        rows.push(cli::term::row(theme, mark, name, "", width));
    }
    for (name, detail) in recent {
        rows.push(cli::term::row(
            theme,
            cli::term::Mark::Done,
            name,
            detail,
            width,
        ));
    }
    rows
}

fn build_harness(
    exe: &Path,
    pkg_root: &Path,
    filter: Option<&str>,
    jobs: Option<usize>,
    memory_cap: Option<u64>,
    verbose: bool,
    quiet: bool,
) -> Result<PathBuf, String> {
    let mut build = std::process::Command::new(exe);
    build
        .arg("build")
        .arg("--release")
        .arg("--bench-harness")
        .arg(filter.unwrap_or(""))
        .current_dir(pkg_root);
    if let Some(j) = jobs {
        build.arg("--jobs").arg(j.to_string());
    }
    if let Some(m) = memory_cap {
        build.arg("--memory-cap").arg(m.to_string());
    }
    if quiet {
        build.arg("--quiet");
    }
    let out = build
        .output()
        .map_err(|e| format!("rnx bench: cannot start the compiler: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(format!("{stdout}{stderr}").trim_end().to_string());
    }
    if !stderr.trim().is_empty() {
        eprint!("{stderr}");
    }
    let artifact = artifact_from(&stdout)
        .ok_or_else(|| format!("rnx bench: no artifact for {}", pkg_root.display()))?;
    if verbose {
        eprintln!("rnx bench: {}", artifact.display());
    }
    Ok(artifact)
}
