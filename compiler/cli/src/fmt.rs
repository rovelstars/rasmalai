use std::path::{Path, PathBuf};

pub struct FmtOutcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

fn skip_dir(name: &str) -> bool {
    name == "target" || name == ".git" || name == ".rnx" || name.starts_with('.')
}

fn is_manifest_name(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n == frontend::project::MANIFEST_FILE)
}

fn is_lock_name(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n == frontend::deplock::LOCK_FILE)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !skip_dir(&entry.file_name().to_string_lossy()) {
                walk(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rnx")
            || is_manifest_name(&path)
            || is_lock_name(&path)
        {
            out.push(path);
        }
    }
}

pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if root.is_file() {
        if root.extension().is_some_and(|ext| ext == "rnx")
            || is_manifest_name(root)
            || is_lock_name(root)
        {
            files.push(root.to_path_buf());
        }
        return files;
    }
    walk(root, &mut files);
    files.sort();
    files
}

fn format_file(path: &Path, src: &str) -> Result<String, String> {
    if is_manifest_name(path) {
        return frontend::project::format_manifest_text(src);
    }
    if is_lock_name(path) {
        return match frontend::deplock::ProjectDepLock::parse(src) {
            Ok(lock) => Ok(lock.to_rnx()),
            Err(msg) => Err(msg),
        };
    }
    frontend::fmt::format_source(src)
}

fn lcs_table(a: &[&str], b: &[&str]) -> Vec<Vec<u32>> {
    let mut t = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for (i, line_a) in a.iter().enumerate() {
        for (j, line_b) in b.iter().enumerate() {
            t[i + 1][j + 1] = if line_a == line_b {
                t[i][j] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    t
}

pub fn unified_diff(path: &Path, old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let name = path.display().to_string();
    if a.len() as u64 * b.len() as u64 > 4_000_000 {
        let mut out = format!("--- {name}\n+++ {name}\n");
        for line in &a {
            out.push('-');
            out.push_str(line);
            out.push('\n');
        }
        for line in &b {
            out.push('+');
            out.push_str(line);
            out.push('\n');
        }
        return out;
    }
    let t = lcs_table(&a, &b);
    let mut ops: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (a.len(), b.len());
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            ops.push((' ', a[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || t[i][j - 1] >= t[i - 1][j]) {
            ops.push(('+', b[j - 1]));
            j -= 1;
        } else {
            ops.push(('-', a[i - 1]));
            i -= 1;
        }
    }
    ops.reverse();
    let first = ops.iter().position(|(c, _)| *c != ' ').unwrap_or(0);
    let last = ops.iter().rposition(|(c, _)| *c != ' ').unwrap_or(0);
    let lo = first.saturating_sub(3);
    let hi = (last + 4).min(ops.len());
    let mut old_start = 0usize;
    let mut new_start = 0usize;
    for (c, _) in ops.iter().take(lo) {
        match c {
            ' ' => {
                old_start += 1;
                new_start += 1;
            }
            '-' => old_start += 1,
            '+' => new_start += 1,
            _ => {}
        }
    }
    let (mut old_n, mut new_n) = (0usize, 0usize);
    for (c, _) in ops.iter().take(hi).skip(lo) {
        match c {
            ' ' => {
                old_n += 1;
                new_n += 1;
            }
            '-' => old_n += 1,
            '+' => new_n += 1,
            _ => {}
        }
    }
    let mut out = format!("--- {name}\n+++ {name}\n");
    out.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        old_start + 1,
        old_n,
        new_start + 1,
        new_n
    ));
    for (c, line) in ops.iter().take(hi).skip(lo) {
        out.push(*c);
        out.push_str(line);
        out.push('\n');
    }
    out
}

pub fn run_fmt(paths: &[PathBuf], check: bool, diff: bool) -> FmtOutcome {
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut files = Vec::new();
    for path in paths {
        if !path.exists() {
            stderr.push_str(&format!("path not found: {}\n", path.display()));
            return FmtOutcome {
                code: 1,
                stdout,
                stderr,
            };
        }
        files.extend(discover(path));
    }
    let mut changed = 0usize;
    let mut failed = false;
    for path in &files {
        let src = match std::fs::read_to_string(path) {
            Ok(src) => src,
            Err(e) => {
                stderr.push_str(&format!("cannot read {}: {e}\n", path.display()));
                failed = true;
                continue;
            }
        };
        let formatted = match format_file(path, &src) {
            Ok(formatted) => formatted,
            Err(e) => {
                stderr.push_str(&format!("cannot format {}: {e}\n", path.display()));
                failed = true;
                continue;
            }
        };
        if formatted == src {
            continue;
        }
        changed += 1;
        if check {
            stderr.push_str(&format!("unformatted: {}\n", path.display()));
        }
        if diff {
            stdout.push_str(&unified_diff(path, &src, &formatted));
        }
        if !check && !diff && let Err(e) = std::fs::write(path, &formatted) {
            stderr.push_str(&format!("cannot write {}: {e}\n", path.display()));
            failed = true;
        }
    }
    if failed {
        return FmtOutcome {
            code: 1,
            stdout,
            stderr,
        };
    }
    if check {
        return FmtOutcome {
            code: i32::from(changed > 0),
            stdout,
            stderr,
        };
    }
    if diff {
        return FmtOutcome {
            code: 0,
            stdout,
            stderr,
        };
    }
    if changed == 0 {
        stdout.push_str("All files already formatted\n");
    } else {
        stdout.push_str(&format!("Formatted {changed} files\n"));
    }
    FmtOutcome {
        code: 0,
        stdout,
        stderr,
    }
}
