use diagnostics::{Code, Diagnostic};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

pub struct TarEntry {
    pub path: String,
    pub dir: bool,
    pub data: Vec<u8>,
}

fn octal(bytes: &[u8]) -> Result<u64, Diagnostic> {
    let mut text = Vec::new();
    for b in bytes {
        if *b == 0 || *b == b' ' {
            break;
        }
        if !b.is_ascii_digit() || *b == b'8' || *b == b'9' {
            return Err(Diagnostic::new(Code::E108, "bad tar number".to_string()));
        }
        text.push(*b);
    }
    if text.is_empty() {
        return Ok(0);
    }
    let s = String::from_utf8(text)
        .map_err(|_| Diagnostic::new(Code::E108, "bad tar number".to_string()))?;
    u64::from_str_radix(&s, 8)
        .map_err(|_| Diagnostic::new(Code::E108, "bad tar number".to_string()))
}

fn field_str(bytes: &[u8]) -> Result<String, Diagnostic> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(|s| s.to_string())
        .map_err(|_| Diagnostic::new(Code::E108, "bad tar name".to_string()))
}

fn safe_rel(name: &str) -> Result<String, Diagnostic> {
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return Err(Diagnostic::new(Code::E108, format!("unsafe tar path `{name}`")));
    }
    for part in trimmed.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(Diagnostic::new(Code::E108, format!("unsafe tar path `{name}`")));
        }
        if part.contains('\0') {
            return Err(Diagnostic::new(Code::E108, format!("unsafe tar path `{name}`")));
        }
    }
    Ok(trimmed.to_string())
}

pub fn parse_tar(data: &[u8]) -> Result<Vec<TarEntry>, Diagnostic> {
    let mut entries = Vec::new();
    let mut pos = 0usize;
    while pos + 512 <= data.len() {
        let head = &data[pos..pos + 512];
        if head.iter().all(|b| *b == 0) {
            break;
        }
        if &head[257..262] != b"ustar" {
            return Err(Diagnostic::new(Code::E108, "bad tar magic".to_string()));
        }
        let stored = octal(&head[148..156])?;
        let mut sum = 0u64;
        for (i, b) in head.iter().enumerate() {
            sum += if (148..156).contains(&i) { b' ' as u64 } else { *b as u64 };
        }
        if sum != stored {
            return Err(Diagnostic::new(Code::E108, "tar checksum mismatch".to_string()));
        }
        let mut name = field_str(&head[0..100])?;
        let prefix = field_str(&head[345..394])?;
        if !prefix.is_empty() {
            name = format!("{prefix}/{name}");
        }
        let size = octal(&head[124..136])?;
        let flag = head[156];
        if flag == b'L' || flag == b'K' {
            return Err(Diagnostic::new(Code::E108, "long tar names unsupported".to_string()));
        }
        let dir = flag == b'5';
        if !dir && flag != b'0' && flag != 0 {
            return Err(Diagnostic::new(Code::E108, "unsupported tar entry".to_string()));
        }
        let path = safe_rel(&name)?;
        let blocks = size.div_ceil(512) as usize;
        if pos + 512 + blocks * 512 > data.len() {
            return Err(Diagnostic::new(Code::E108, "truncated tar archive".to_string()));
        }
        let body = if dir {
            Vec::new()
        } else {
            data[pos + 512..pos + 512 + size as usize].to_vec()
        };
        entries.push(TarEntry { path, dir, data: body });
        pos += 512 + blocks * 512;
    }
    Ok(entries)
}

#[derive(Clone)]
struct Job {
    path: PathBuf,
    content: Vec<u8>,
}

static JOBS: LazyLock<Mutex<Vec<Job>>> = LazyLock::new(|| Mutex::new(Vec::new()));
static FAILS: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn record_fail(msg: String) {
    FAILS.lock().unwrap_or_else(|e| e.into_inner()).push(msg);
}

unsafe extern "C" fn unpack_one(idx: i64) -> i64 {
    let job = {
        let guard = JOBS.lock().unwrap_or_else(|e| e.into_inner());
        if idx < 0 {
            None
        } else {
            guard.get(idx as usize).cloned()
        }
    };
    let Some(job) = job else {
        record_fail(format!("bad job index {idx}"));
        return 1;
    };
    if std::fs::write(&job.path, &job.content).is_err() {
        record_fail(format!("cannot write {}", job.path.display()));
        return 1;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&job.path, std::fs::Permissions::from_mode(0o644));
    }
    0
}

fn write_files_pool(out_dir: &Path, files: &[(PathBuf, Vec<u8>)]) -> Result<(), Diagnostic> {
    {
        let mut jobs = JOBS.lock().unwrap_or_else(|e| e.into_inner());
        jobs.clear();
        jobs.extend(files.iter().map(|(p, c)| Job { path: p.clone(), content: c.clone() }));
        FAILS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let workers = workers.min(files.len()).max(1) as i64;
    unsafe {
        runtime::native::rnx_thread_pool_init(9400, workers);
        for i in 0..files.len() as i64 {
            runtime::native::rnx_thread_pool_submit(9400, unpack_one as *const () as usize, i, 1);
        }
        runtime::native::rnx_thread_pool_join(9400);
        runtime::native::rnx_thread_pool_shutdown(9400);
    }
    let fails = FAILS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(first) = fails.first() {
        return Err(Diagnostic::new(Code::E108, first.clone()));
    }
    let _ = out_dir;
    Ok(())
}

pub fn unpack_archive(archive: &Path, out_dir: &Path) -> Result<(usize, u64), Diagnostic> {
    let bytes = std::fs::read(archive)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read {}: {e}", archive.display())))?;
    let tar = if bytes.len() >= 2 && bytes[0] == 0x1F && bytes[1] == 0x8B {
        frontend::gzip::decompress_gzip(&bytes)?
    } else {
        bytes
    };
    let entries = parse_tar(&tar)?;
    std::fs::create_dir_all(out_dir).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write {}: {e}", out_dir.display()))
    })?;
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let mut total = 0u64;
    for e in entries {
        let dest = out_dir.join(&e.path);
        if e.dir {
            dirs.push(dest);
        } else {
            total += e.data.len() as u64;
            files.push((dest, e.data));
        }
    }
    dirs.sort();
    for d in &dirs {
        std::fs::create_dir_all(d)
            .map_err(|e| Diagnostic::new(Code::E108, format!("cannot write {}: {e}", d.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755));
        }
    }
    let mut parents: Vec<PathBuf> = files
        .iter()
        .filter_map(|(p, _)| p.parent().map(|q| q.to_path_buf()))
        .collect();
    parents.sort();
    parents.dedup();
    for d in &parents {
        std::fs::create_dir_all(d)
            .map_err(|e| Diagnostic::new(Code::E108, format!("cannot write {}: {e}", d.display())))?;
    }
    if files.len() > 2 {
        write_files_pool(out_dir, &files)?;
    } else {
        for (dest, content) in &files {
            std::fs::write(dest, content).map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot write {}: {e}", dest.display()))
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o644));
            }
        }
    }
    Ok((files.len(), total))
}
