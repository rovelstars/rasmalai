use super::collections::*;
use super::common::*;
use super::io::check_protected_write;

pub fn fs_exists_impl(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_exists(path: *const u8) -> bool {
    fs_exists_impl(&native_str(path))
}

pub fn fs_is_file_impl(path: &str) -> bool {
    std::path::Path::new(path).is_file()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_is_file(path: *const u8) -> bool {
    fs_is_file_impl(&native_str(path))
}

pub fn fs_is_dir_impl(path: &str) -> bool {
    std::path::Path::new(path).is_dir()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_is_dir(path: *const u8) -> bool {
    fs_is_dir_impl(&native_str(path))
}

pub fn fs_stat_err_impl(path: &str) -> String {
    match std::fs::symlink_metadata(path) {
        Ok(_) => String::new(),
        Err(e) => format!("stat failed for `{path}`: {e}"),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_stat_err(path: *const u8) -> *mut u8 {
    alloc_str(&fs_stat_err_impl(&native_str(path)))
}

pub fn fs_stat_fields_impl(path: &str) -> [i64; 6] {
    match std::fs::symlink_metadata(path) {
        Ok(m) => {
            let modified = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            [
                m.len() as i64,
                i64::from(m.is_file()),
                i64::from(m.is_dir()),
                i64::from(m.file_type().is_symlink()),
                modified,
                fs_mode_impl(&m),
            ]
        }
        Err(_) => [0, 0, 0, 0, 0, 0],
    }
}

#[cfg(unix)]
fn fs_mode_impl(m: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() as i64
}

#[cfg(not(unix))]
fn fs_mode_impl(_m: &std::fs::Metadata) -> i64 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_stat(path: *const u8) -> *mut u8 {
    let fields = fs_stat_fields_impl(&native_str(path));
    unsafe {
        let out = rnx_array_new(fields.len(), 8);
        if out.is_null() {
            return out;
        }
        for v in fields {
            rnx_array_push(out, v as u64, 8);
        }
        out
    }
}

pub fn fs_read_dir_impl(path: &str) -> Result<Vec<String>, String> {
    match std::fs::read_dir(path) {
        Ok(rd) => {
            let mut out = Vec::new();
            for entry in rd {
                match entry {
                    Ok(de) => out.push(de.file_name().to_string_lossy().into_owned()),
                    Err(e) => return Err(format!("readDir failed for `{path}`: {e}")),
                }
            }
            out.sort();
            Ok(out)
        }
        Err(e) => Err(format!("readDir failed for `{path}`: {e}")),
    }
}

fn fs_str_array(items: &[String]) -> *mut u8 {
    unsafe {
        let out = rnx_array_new(items.len(), 8);
        if out.is_null() {
            return out;
        }
        for item in items {
            rnx_array_push(out, alloc_str(item) as u64, 8);
        }
        out
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_read_dir(path: *const u8) -> *mut u8 {
    match fs_read_dir_impl(&native_str(path)) {
        Ok(items) => fs_str_array(&items),
        Err(_) => fs_str_array(&[]),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_read_dir_err(path: *const u8) -> *mut u8 {
    match fs_read_dir_impl(&native_str(path)) {
        Ok(_) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

fn fs_glob_match(pat: &[u8], name: &[u8]) -> bool {
    let (mut px, mut nx) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while nx < name.len() {
        if px < pat.len() && (pat[px] == b'?' || pat[px] == name[nx]) {
            px += 1;
            nx += 1;
        } else if px < pat.len() && pat[px] == b'*' {
            star = Some(px);
            mark = nx;
            px += 1;
        } else if let Some(s) = star {
            px = s + 1;
            mark += 1;
            nx = mark;
        } else {
            return false;
        }
    }
    while px < pat.len() && pat[px] == b'*' {
        px += 1;
    }
    px == pat.len()
}

fn fs_glob_walk(base: &str, comps: &[&str], out: &mut Vec<String>) -> Result<(), String> {
    if comps.is_empty() {
        if std::fs::symlink_metadata(base).is_ok() {
            out.push(base.to_string());
        }
        return Ok(());
    }
    if comps[0] == "**" {
        fs_glob_walk(base, &comps[1..], out)?;
        let mut dirs = Vec::new();
        match std::fs::read_dir(base) {
            Ok(rd) => {
                for entry in rd {
                    match entry {
                        Ok(de) => {
                            if de.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                dirs.push(de.file_name().to_string_lossy().into_owned());
                            }
                        }
                        Err(e) => return Err(format!("glob failed for `{base}`: {e}")),
                    }
                }
            }
            Err(e) => return Err(format!("glob failed for `{base}`: {e}")),
        }
        dirs.sort();
        for dir in dirs {
            fs_glob_walk(&format!("{base}/{dir}"), comps, out)?;
        }
        return Ok(());
    }
    if !comps[0].bytes().any(|b| b == b'*' || b == b'?') {
        let next = if base == "/" {
            format!("/{0}", comps[0])
        } else if base.is_empty() {
            comps[0].to_string()
        } else {
            format!("{base}/{0}", comps[0])
        };
        if std::fs::symlink_metadata(&next).is_ok() {
            return fs_glob_walk(&next, &comps[1..], out);
        }
        return Ok(());
    }
    let mut names = Vec::new();
    match std::fs::read_dir(base) {
        Ok(rd) => {
            for entry in rd {
                match entry {
                    Ok(de) => {
                        let name = de.file_name().to_string_lossy().into_owned();
                        if fs_glob_match(comps[0].as_bytes(), name.as_bytes()) {
                            names.push(name);
                        }
                    }
                    Err(e) => return Err(format!("glob failed for `{base}`: {e}")),
                }
            }
        }
        Err(e) => return Err(format!("glob failed for `{base}`: {e}")),
    }
    names.sort();
    for name in names {
        let next = if base.is_empty() {
            name
        } else {
            format!("{base}/{name}")
        };
        fs_glob_walk(&next, &comps[1..], out)?;
    }
    Ok(())
}

pub fn fs_glob_impl(pattern: &str) -> Result<Vec<String>, String> {
    if pattern.is_empty() {
        return Err("glob pattern must not be empty".to_string());
    }
    let absolute = pattern.starts_with('/');
    let comps: Vec<&str> = pattern.split('/').filter(|c| !c.is_empty()).collect();
    let mut literal = 0usize;
    while literal < comps.len()
        && !comps[literal].bytes().any(|b| b == b'*' || b == b'?')
        && comps[literal] != "**"
    {
        literal += 1;
    }
    let mut base = if absolute {
        "/".to_string()
    } else {
        String::new()
    };
    for comp in comps[..literal].iter() {
        if base == "/" {
            base.push_str(comp);
        } else if base.is_empty() {
            base.push_str(comp);
        } else {
            base.push('/');
            base.push_str(comp);
        }
    }
    if base.is_empty() {
        base.push('.');
    }
    if literal == comps.len() {
        if std::fs::symlink_metadata(pattern).is_ok() {
            return Ok(vec![pattern.to_string()]);
        }
        return Ok(Vec::new());
    }
    if literal > 0 && std::fs::symlink_metadata(&base).is_err() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    fs_glob_walk(&base, &comps[literal..], &mut out)?;
    out.sort();
    Ok(out)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_glob(pattern: *const u8) -> *mut u8 {
    match fs_glob_impl(&native_str(pattern)) {
        Ok(items) => fs_str_array(&items),
        Err(_) => fs_str_array(&[]),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_glob_err(pattern: *const u8) -> *mut u8 {
    match fs_glob_impl(&native_str(pattern)) {
        Ok(_) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_read_link_impl(path: &str) -> Result<String, String> {
    match std::fs::read_link(path) {
        Ok(target) => Ok(target.to_string_lossy().into_owned()),
        Err(e) => Err(format!("readLink failed for `{path}`: {e}")),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_read_link(path: *const u8) -> *mut u8 {
    match fs_read_link_impl(&native_str(path)) {
        Ok(target) => alloc_str(&target),
        Err(_) => alloc_str(""),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_read_link_err(path: *const u8) -> *mut u8 {
    match fs_read_link_impl(&native_str(path)) {
        Ok(_) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

fn fs_write_opts(mode: i64) -> std::fs::OpenOptions {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    match mode {
        0 => {
            opts.create_new(true);
        }
        1 => {
            opts.create(true);
        }
        3 => {
            opts.create(true);
            opts.append(true);
        }
        _ => {
            opts.create(true);
            opts.truncate(true);
        }
    }
    opts
}

pub fn fs_mkdir_impl(path: &str, recursive: bool) -> Result<(), String> {
    check_protected_write(path);
    let done = if recursive {
        std::fs::create_dir_all(path)
    } else {
        std::fs::create_dir(path)
    };
    done.map_err(|e| format!("mkdir failed for `{path}`: {e}"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_mkdir_err(path: *const u8, recursive: i64) -> *mut u8 {
    match fs_mkdir_impl(&native_str(path), recursive != 0) {
        Ok(()) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_remove_impl(path: &str) -> Result<bool, String> {
    check_protected_write(path);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => match std::fs::remove_dir(path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("remove failed for `{path}`: {e}")),
        },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_remove(path: *const u8) -> bool {
    fs_remove_impl(&native_str(path)).unwrap_or(false)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_remove_err(path: *const u8) -> *mut u8 {
    match fs_remove_impl(&native_str(path)) {
        Ok(_) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_remove_all_impl(path: &str) -> Result<bool, String> {
    check_protected_write(path);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => match std::fs::remove_dir_all(path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("removeAll failed for `{path}`: {e}")),
        },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_remove_all(path: *const u8) -> bool {
    fs_remove_all_impl(&native_str(path)).unwrap_or(false)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_remove_all_err(path: *const u8) -> *mut u8 {
    match fs_remove_all_impl(&native_str(path)) {
        Ok(_) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_copy_impl(from: &str, to: &str, mode: i64) -> Result<(), String> {
    check_protected_write(to);
    if mode == 1 && std::path::Path::new(to).exists() {
        return Ok(());
    }
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("copy failed from `{from}` to `{to}`: {e}"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_copy_err(from: *const u8, to: *const u8, mode: i64) -> *mut u8 {
    match fs_copy_impl(&native_str(from), &native_str(to), mode) {
        Ok(()) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_rename_impl(from: &str, to: &str) -> Result<(), String> {
    check_protected_write(from);
    check_protected_write(to);
    std::fs::rename(from, to).map_err(|e| format!("rename failed from `{from}` to `{to}`: {e}"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_rename_err(from: *const u8, to: *const u8) -> *mut u8 {
    match fs_rename_impl(&native_str(from), &native_str(to)) {
        Ok(()) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_move_impl(from: &str, to: &str) -> Result<(), String> {
    check_protected_write(from);
    check_protected_write(to);
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to)
                .map_err(|e| format!("move failed from `{from}` to `{to}`: {e}"))
                .and_then(|_| {
                    std::fs::remove_file(from)
                        .map_err(|e| format!("move failed from `{from}` to `{to}`: {e}"))
                })
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_move_err(from: *const u8, to: *const u8) -> *mut u8 {
    match fs_move_impl(&native_str(from), &native_str(to)) {
        Ok(()) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_truncate_impl(path: &str, len: i64) -> Result<(), String> {
    check_protected_write(path);
    if len < 0 {
        return Err(format!("truncate failed for `{path}`: negative length {len}"));
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| format!("truncate failed for `{path}`: {e}"))?;
    file.set_len(len as u64)
        .map_err(|e| format!("truncate failed for `{path}`: {e}"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_truncate_err(path: *const u8, len: i64) -> *mut u8 {
    match fs_truncate_impl(&native_str(path), len) {
        Ok(()) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_chmod_impl(path: &str, mode: i64) -> Result<(), String> {
    check_protected_write(path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(mode as u32);
        return std::fs::set_permissions(path, perm)
            .map_err(|e| format!("chmod failed for `{path}`: {e}"));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        return Err(format!("chmod failed for `{path}`: unsupported on this platform"));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_chmod_err(path: *const u8, mode: i64) -> *mut u8 {
    match fs_chmod_impl(&native_str(path), mode) {
        Ok(()) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_symlink_impl(target: &str, link: &str) -> Result<(), String> {
    check_protected_write(link);
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (target, link);
        return Err("symlink is not supported on this target".to_string());
    }
    #[cfg(unix)]
    {
        return std::os::unix::fs::symlink(target, link)
            .map_err(|e| format!("symlink failed from `{target}` to `{link}`: {e}"));
    }
    #[cfg(windows)]
    {
        if std::path::Path::new(target).is_dir() {
            return std::os::windows::fs::symlink_dir(target, link)
                .map_err(|e| format!("symlink failed from `{target}` to `{link}`: {e}"));
        }
        return std::os::windows::fs::symlink_file(target, link)
            .map_err(|e| format!("symlink failed from `{target}` to `{link}`: {e}"));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_symlink_err(target: *const u8, link: *const u8) -> *mut u8 {
    match fs_symlink_impl(&native_str(target), &native_str(link)) {
        Ok(()) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_fsync_impl(path: &str) -> Result<(), String> {
    check_protected_write(path);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|e| format!("fsync failed for `{path}`: {e}"))?;
    file.sync_all().map_err(|e| format!("fsync failed for `{path}`: {e}"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_fsync_err(path: *const u8) -> *mut u8 {
    match fs_fsync_impl(&native_str(path)) {
        Ok(()) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_read_text_impl(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("readText failed for `{path}`: {e}"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_read_text(path: *const u8) -> *mut u8 {
    match fs_read_text_impl(&native_str(path)) {
        Ok(text) => alloc_str(&text),
        Err(_) => alloc_str(""),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_read_text_err(path: *const u8) -> *mut u8 {
    match fs_read_text_impl(&native_str(path)) {
        Ok(_) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_write_text_impl(path: &str, text: &str, mode: i64) -> Result<i64, String> {
    check_protected_write(path);
    let mut file = fs_write_opts(mode)
        .open(path)
        .map_err(|e| format!("writeText failed for `{path}`: {e}"))?;
    std::io::Write::write_all(&mut file, text.as_bytes())
        .map(|_| text.len() as i64)
        .map_err(|e| format!("writeText failed for `{path}`: {e}"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_write_text(path: *const u8, text: *const u8, mode: i64) -> i64 {
    fs_write_text_impl(&native_str(path), &native_str(text), mode).unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_write_text_err(path: *const u8, text: *const u8, mode: i64) -> *mut u8 {
    match fs_write_text_impl(&native_str(path), &native_str(text), mode) {
        Ok(_) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_read_bytes_impl(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("readBytes failed for `{path}`: {e}"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_read_bytes(path: *const u8) -> *mut u8 {
    match fs_read_bytes_impl(&native_str(path)) {
        Ok(data) => Box::into_raw(Box::new(ByteBufferState { data })) as *mut u8,
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_read_bytes_err(path: *const u8) -> *mut u8 {
    match fs_read_bytes_impl(&native_str(path)) {
        Ok(_) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

pub fn fs_write_bytes_impl(path: &str, data: &[u8], mode: i64) -> Result<i64, String> {
    check_protected_write(path);
    let mut file = fs_write_opts(mode)
        .open(path)
        .map_err(|e| format!("writeBytes failed for `{path}`: {e}"))?;
    std::io::Write::write_all(&mut file, data)
        .map(|_| data.len() as i64)
        .map_err(|e| format!("writeBytes failed for `{path}`: {e}"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_write_bytes(path: *const u8, buf: *mut u8, mode: i64) -> i64 {
    let data = bytes_state(buf).data.clone();
    fs_write_bytes_impl(&native_str(path), &data, mode).unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_write_bytes_err(path: *const u8, buf: *mut u8, mode: i64) -> *mut u8 {
    let data = bytes_state(buf).data.clone();
    match fs_write_bytes_impl(&native_str(path), &data, mode) {
        Ok(_) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

std::thread_local! {
    static FS_POOL_DEPTH: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

pub fn fs_pool_depth_impl(delta: i64) -> i64 {
    FS_POOL_DEPTH.with(|d| {
        let next = (d.get() + delta).max(0);
        d.set(next);
        next
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_pool_depth(delta: i64) -> i64 {
    fs_pool_depth_impl(delta)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) enum MmapBacking {
    Read(memmap2::Mmap),
    Write(memmap2::MmapMut),
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct MmapState {
    backing: MmapBacking,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) static LIVE_MMAPS: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeSet<usize>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeSet::new()));

#[cfg(not(target_arch = "wasm32"))]
fn mmap_track(handle: *mut u8) {
    LIVE_MMAPS.lock().unwrap_or_else(|e| e.into_inner()).insert(handle as usize);
}

#[cfg(not(target_arch = "wasm32"))]
fn mmap_live(handle: *mut u8) -> bool {
    !handle.is_null()
        && LIVE_MMAPS.lock().unwrap_or_else(|e| e.into_inner()).contains(&(handle as usize))
}

#[cfg(not(target_arch = "wasm32"))]
fn mmap_of(handle: *mut u8) -> Option<&'static mut MmapState> {
    if !mmap_live(handle) {
        return None;
    }
    Some(unsafe { &mut *(handle as *mut MmapState) })
}

#[cfg(not(target_arch = "wasm32"))]
fn mmap_addr_of(state: &MmapState) -> i64 {
    match &state.backing {
        MmapBacking::Read(m) => m.as_ptr() as usize as i64,
        MmapBacking::Write(m) => m.as_ptr() as usize as i64,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn mmap_len_of(state: &MmapState) -> i64 {
    match &state.backing {
        MmapBacking::Read(m) => m.len() as i64,
        MmapBacking::Write(m) => m.len() as i64,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn fs_mmap_impl(path: &str, mode: i64) -> Result<*mut u8, String> {
    if mode != 0 {
        check_protected_write(path);
    }
    let file_len = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("mmap failed for `{path}`: {e}"))?;
    if file_len == 0 {
        return Err(format!("mmap failed for `{path}`: cannot map an empty file"));
    }
    let backing = if mode == 0 {
        let file =
            std::fs::File::open(path).map_err(|e| format!("mmap failed for `{path}`: {e}"))?;
        let map = unsafe { memmap2::Mmap::map(&file) }
            .map_err(|e| format!("mmap failed for `{path}`: {e}"))?;
        MmapBacking::Read(map)
    } else {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("mmap failed for `{path}`: {e}"))?;
        let map = unsafe { memmap2::MmapMut::map_mut(&file) }
            .map_err(|e| format!("mmap failed for `{path}`: {e}"))?;
        MmapBacking::Write(map)
    };
    let handle = Box::into_raw(Box::new(MmapState { backing })) as *mut u8;
    mmap_track(handle);
    Ok(handle)
}

#[cfg(target_arch = "wasm32")]
pub fn fs_mmap_impl(_path: &str, _mode: i64) -> Result<*mut u8, String> {
    Err("mmap is not supported on this target".to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_mmap(path: *const u8, mode: i64) -> *mut u8 {
    fs_mmap_impl(&native_str(path), mode).unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_mmap_err(path: *const u8, mode: i64) -> *mut u8 {
    match fs_mmap_impl(&native_str(path), mode) {
        Ok(handle) => {
            fs_mmap_close_impl(handle);
            alloc_str("")
        }
        Err(e) => alloc_str(&e),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn fs_mmap_anon_impl(len: i64) -> Result<*mut u8, String> {
    if len <= 0 {
        return Err(format!("mmapAnon failed: non-positive length {len}"));
    }
    let map = memmap2::MmapMut::map_anon(len as usize)
        .map_err(|e| format!("mmapAnon failed: {e}"))?;
    let handle = Box::into_raw(Box::new(MmapState {
        backing: MmapBacking::Write(map),
    })) as *mut u8;
    mmap_track(handle);
    Ok(handle)
}

#[cfg(target_arch = "wasm32")]
pub fn fs_mmap_anon_impl(_len: i64) -> Result<*mut u8, String> {
    Err("mmap is not supported on this target".to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_mmap_anon(len: i64) -> *mut u8 {
    fs_mmap_anon_impl(len).unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_mmap_anon_err(len: i64) -> *mut u8 {
    match fs_mmap_anon_impl(len) {
        Ok(handle) => {
            fs_mmap_close_impl(handle);
            alloc_str("")
        }
        Err(e) => alloc_str(&e),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn fs_mmap_addr_impl(handle: *mut u8) -> i64 {
    mmap_of(handle).map(|state| mmap_addr_of(state)).unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
pub fn fs_mmap_addr_impl(_handle: *mut u8) -> i64 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_mmap_addr(handle: *mut u8) -> i64 {
    fs_mmap_addr_impl(handle)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn fs_mmap_len_impl(handle: *mut u8) -> i64 {
    mmap_of(handle).map(|state| mmap_len_of(state)).unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
pub fn fs_mmap_len_impl(_handle: *mut u8) -> i64 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_mmap_len(handle: *mut u8) -> i64 {
    fs_mmap_len_impl(handle)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn fs_mmap_flush_impl(handle: *mut u8) -> Result<(), String> {
    match mmap_of(handle) {
        None => Ok(()),
        Some(state) => match &state.backing {
            MmapBacking::Read(_) => Ok(()),
            MmapBacking::Write(m) => m
                .flush()
                .map_err(|e| format!("mmap flush failed: {e}")),
        },
    }
}

#[cfg(target_arch = "wasm32")]
pub fn fs_mmap_flush_impl(_handle: *mut u8) -> Result<(), String> {
    Err("mmap is not supported on this target".to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_mmap_flush(handle: *mut u8) {
    let _ = fs_mmap_flush_impl(handle);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn fs_mmap_close_impl(handle: *mut u8) {
    if handle.is_null() {
        return;
    }
    if !LIVE_MMAPS.lock().unwrap_or_else(|e| e.into_inner()).remove(&(handle as usize)) {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle as *mut MmapState));
    }
}

#[cfg(target_arch = "wasm32")]
pub fn fs_mmap_close_impl(_handle: *mut u8) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fs_mmap_close(handle: *mut u8) {
    fs_mmap_close_impl(handle)
}

#[cfg(test)]
mod fs_tests {
    use super::*;

    fn mortal(text: &str) -> *mut u8 {
        let out = str_alloc(text.len());
        assert!(!out.is_null());
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), out.add(STR_HEADER), text.len());
            out.add(STR_HEADER).add(text.len()).write(0);
        }
        out
    }

    fn case_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-fs-native-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn case_str(dir: &std::path::Path, name: &str) -> String {
        if name.is_empty() {
            return dir.to_string_lossy().into_owned();
        }
        dir.join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn exists_is_file_is_dir_roundtrip() {
        let dir = case_dir("probe");
        let file = case_str(&dir, "a.txt");
        std::fs::write(&file, "hi").unwrap();
        let sub = case_str(&dir, "sub");
        std::fs::create_dir(&sub).unwrap();
        let missing = case_str(&dir, "nope.txt");
        let (f, s, m) = (mortal(&file), mortal(&sub), mortal(&missing));
        unsafe {
            assert!(rnx_fs_exists(f));
            assert!(rnx_fs_is_file(f));
            assert!(!rnx_fs_is_dir(f));
            assert!(rnx_fs_exists(s));
            assert!(!rnx_fs_is_file(s));
            assert!(rnx_fs_is_dir(s));
            assert!(!rnx_fs_exists(m));
            assert!(!rnx_fs_is_file(m));
            assert!(!rnx_fs_is_dir(m));
        }
        assert!(fs_exists_impl(&file));
        assert!(!fs_is_file_impl(&sub));
        assert!(fs_is_dir_impl(&sub));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stat_fields_and_missing_err() {
        let dir = case_dir("stat");
        let file = case_str(&dir, "b.bin");
        std::fs::write(&file, "12345678").unwrap();
        let f = fs_stat_fields_impl(&file);
        assert_eq!(f[0], 8);
        assert_eq!(f[1], 1);
        assert_eq!(f[2], 0);
        assert_eq!(f[3], 0);
        assert!(f[4] > 0);
        let missing = case_str(&dir, "gone.txt");
        assert_eq!(fs_stat_fields_impl(&missing), [0, 0, 0, 0, 0, 0]);
        assert!(fs_stat_err_impl(&file).is_empty());
        let err = fs_stat_err_impl(&missing);
        assert!(!err.is_empty());
        let p = mortal(&file);
        unsafe {
            assert_eq!(str_bytes(rnx_fs_stat_err(p)), b"");
            assert_eq!(rnx_array_len(rnx_fs_stat(p)), 6);
            assert_eq!(rnx_array_get(rnx_fs_stat(p), 0, 8), 8);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_dir_sorted_and_missing_err() {
        let dir = case_dir("readdir");
        let ds = case_str(&dir, "");
        for name in ["zeta.txt", "alpha.txt", "mid.txt"] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let items = fs_read_dir_impl(&ds).unwrap();
        assert_eq!(items, vec!["alpha.txt", "mid.txt", "zeta.txt"]);
        let missing = case_str(&dir, "gone-dir");
        assert!(fs_read_dir_impl(&missing).is_err());
        let p = mortal(&ds);
        unsafe {
            assert_eq!(str_bytes(rnx_fs_read_dir_err(p)), b"");
            assert_eq!(rnx_array_len(rnx_fs_read_dir(p)), 3);
            let q = mortal(&missing);
            assert!(!str_bytes(rnx_fs_read_dir_err(q)).is_empty());
            assert_eq!(rnx_array_len(rnx_fs_read_dir(q)), 0);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn glob_sorted_star_and_starstar() {
        let dir = case_dir("glob");
        std::fs::write(dir.join("b.rnx"), "x").unwrap();
        std::fs::write(dir.join("a.rnx"), "x").unwrap();
        std::fs::write(dir.join("skip.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("c.rnx"), "x").unwrap();
        let ds = case_str(&dir, "");
        let star = format!("{ds}/*.rnx");
        let got = fs_glob_impl(&star).unwrap();
        assert_eq!(got, vec![format!("{ds}/a.rnx"), format!("{ds}/b.rnx")]);
        let deep = format!("{ds}/**/*.rnx");
        let mut deep_got = fs_glob_impl(&deep).unwrap();
        deep_got.sort();
        assert_eq!(
            deep_got,
            vec![
                format!("{ds}/a.rnx"),
                format!("{ds}/b.rnx"),
                format!("{ds}/sub/c.rnx"),
            ]
        );
        assert!(fs_glob_impl("").is_err());
        let p = mortal(&star);
        unsafe {
            assert_eq!(str_bytes(rnx_fs_glob_err(p)), b"");
            assert_eq!(rnx_array_len(rnx_fs_glob(p)), 2);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mkdir_single_and_recursive() {
        let dir = case_dir("mkdir");
        let single = case_str(&dir, "one");
        assert!(fs_mkdir_impl(&single, false).is_ok());
        assert!(std::path::Path::new(&single).is_dir());
        assert!(fs_mkdir_impl(&single, false).is_err());
        let deep = case_str(&dir, "a/b/c");
        assert!(fs_mkdir_impl(&deep, false).is_err());
        assert!(fs_mkdir_impl(&deep, true).is_ok());
        assert!(std::path::Path::new(&deep).is_dir());
        let p = mortal(&case_str(&dir, "flat"));
        unsafe {
            assert_eq!(str_bytes(rnx_fs_mkdir_err(p, 0)), b"");
            assert!(!str_bytes(rnx_fs_mkdir_err(p, 0)).is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_and_remove_all_roundtrip() {
        let dir = case_dir("remove");
        let missing = case_str(&dir, "gone.txt");
        assert_eq!(fs_remove_impl(&missing), Ok(false));
        assert_eq!(fs_remove_all_impl(&missing), Ok(false));
        let file = case_str(&dir, "f.txt");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(fs_remove_impl(&file), Ok(true));
        assert!(!std::path::Path::new(&file).exists());
        let empty = case_str(&dir, "empty");
        std::fs::create_dir(&empty).unwrap();
        assert_eq!(fs_remove_impl(&empty), Ok(true));
        let full = case_str(&dir, "full");
        std::fs::create_dir_all(format!("{full}/sub")).unwrap();
        std::fs::write(format!("{full}/sub/x.txt"), "x").unwrap();
        assert!(fs_remove_impl(&full).is_err());
        assert_eq!(fs_remove_all_impl(&full), Ok(true));
        assert!(!std::path::Path::new(&full).exists());
        let p = mortal(&missing);
        unsafe {
            assert!(!rnx_fs_remove(p));
            assert!(!rnx_fs_remove_all(p));
            assert_eq!(str_bytes(rnx_fs_remove_err(p)), b"");
            assert_eq!(str_bytes(rnx_fs_remove_all_err(p)), b"");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_move_rename_roundtrip() {
        let dir = case_dir("cmr");
        let src = case_str(&dir, "src.txt");
        std::fs::write(&src, "payload").unwrap();
        let dst = case_str(&dir, "dst.txt");
        assert!(fs_copy_impl(&src, &dst, 0).is_ok());
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "payload");
        std::fs::write(&dst, "keep").unwrap();
        assert!(fs_copy_impl(&src, &dst, 1).is_ok());
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "keep");
        assert!(fs_copy_impl(&src, &dst, 0).is_ok());
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "payload");
        assert!(fs_copy_impl(&case_str(&dir, "missing.txt"), &dst, 0).is_err());
        let moved = case_str(&dir, "moved.txt");
        assert!(fs_move_impl(&dst, &moved).is_ok());
        assert!(!std::path::Path::new(&dst).exists());
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "payload");
        let renamed = case_str(&dir, "renamed.txt");
        assert!(fs_rename_impl(&moved, &renamed).is_ok());
        assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "payload");
        assert!(fs_rename_impl(&case_str(&dir, "missing.txt"), &renamed).is_err());
        let (a, b) = (mortal(&src), mortal(&renamed));
        unsafe {
            assert_eq!(str_bytes(rnx_fs_copy_err(a, b, 0)), b"");
            assert_eq!(str_bytes(rnx_fs_move_err(b, a, )), b"");
            assert_eq!(str_bytes(rnx_fs_rename_err(a, b)), b"");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncate_chmod_fsync_roundtrip() {
        let dir = case_dir("tcf");
        let file = case_str(&dir, "f.txt");
        std::fs::write(&file, "12345678").unwrap();
        assert!(fs_truncate_impl(&file, 3).is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "123");
        assert!(fs_truncate_impl(&file, -1).is_err());
        assert!(fs_truncate_impl(&case_str(&dir, "missing.txt"), 0).is_err());
        #[cfg(unix)]
        {
            assert!(fs_chmod_impl(&file, 0o600).is_ok());
            let f = fs_stat_fields_impl(&file);
            assert_eq!(f[5] & 0o777, 0o600);
        }
        assert!(fs_fsync_impl(&file).is_ok());
        assert!(fs_fsync_impl(&case_str(&dir, "missing.txt")).is_err());
        let p = mortal(&file);
        unsafe {
            assert_eq!(str_bytes(rnx_fs_truncate_err(p, 3)), b"");
            assert_eq!(str_bytes(rnx_fs_chmod_err(p, 0o644)), b"");
            assert_eq!(str_bytes(rnx_fs_fsync_err(p)), b"");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn symlink_roundtrip() {
        let dir = case_dir("symlink");
        let real = case_str(&dir, "real.txt");
        std::fs::write(&real, "data").unwrap();
        let link = case_str(&dir, "link.txt");
        assert!(fs_symlink_impl(&real, &link).is_ok());
        assert_eq!(fs_read_link_impl(&link).unwrap(), real);
        assert!(fs_symlink_impl(&real, &link).is_err());
        let (a, b) = (mortal(&real), mortal(&case_str(&dir, "link2.txt")));
        unsafe {
            assert_eq!(str_bytes(rnx_fs_symlink_err(a, b)), b"");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_bytes_oneshot_roundtrip() {
        let dir = case_dir("oneshot");
        let text = case_str(&dir, "t.txt");
        assert!(fs_write_text_impl(&text, "hello", 2).is_ok());
        assert_eq!(fs_read_text_impl(&text).unwrap(), "hello");
        assert!(fs_write_text_impl(&text, "HELLO", 0).is_err());
        assert!(fs_write_text_impl(&text, " world", 3).is_ok());
        assert_eq!(fs_read_text_impl(&text).unwrap(), "hello world");
        assert!(fs_write_text_impl(&text, "hi", 1).is_ok());
        assert_eq!(fs_read_text_impl(&text).unwrap(), "hillo world");
        assert!(fs_read_text_impl(&case_str(&dir, "missing.txt")).is_err());
        let bin = case_str(&dir, "b.bin");
        assert!(fs_write_bytes_impl(&bin, &[0, 1, 2, 255], 2).is_ok());
        assert_eq!(fs_read_bytes_impl(&bin).unwrap(), vec![0, 1, 2, 255]);
        assert!(fs_read_bytes_impl(&case_str(&dir, "missing.bin")).is_err());
        let (p, q) = (mortal(&text), mortal(&bin));
        let t = mortal("hello-bytes");
        unsafe {
            assert_eq!(str_bytes(rnx_fs_read_text_err(p)), b"");
            assert_eq!(str_bytes(rnx_fs_read_text(p)), b"hillo world");
            assert_eq!(rnx_fs_write_text(p, t, 2), 11);
            assert_eq!(str_bytes(rnx_fs_write_text_err(p, t, 2)), b"");
            assert_eq!(str_bytes(rnx_fs_read_bytes_err(q)), b"");
            let h = rnx_fs_read_bytes(q);
            assert!(!h.is_null());
            assert_eq!(bytes_len_impl(h), 4);
            bytes_free_impl(h);
            let buf = bytes_alloc_impl(2);
            bytes_state(buf).data.copy_from_slice(&[9, 9]);
            assert_eq!(rnx_fs_write_bytes(q, buf, 2), 2);
            assert_eq!(str_bytes(rnx_fs_write_bytes_err(q, buf, 2)), b"");
            bytes_free_impl(buf);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pool_depth_nests_and_stays_thread_local() {
        assert_eq!(fs_pool_depth_impl(0), 0);
        assert_eq!(fs_pool_depth_impl(1), 1);
        assert_eq!(fs_pool_depth_impl(1), 2);
        assert_eq!(fs_pool_depth_impl(0), 2);
        assert_eq!(fs_pool_depth_impl(-1), 1);
        assert_eq!(fs_pool_depth_impl(-5), 0);
        let seen = std::thread::spawn(|| fs_pool_depth_impl(0)).join().unwrap();
        assert_eq!(seen, 0);
        assert_eq!(fs_pool_depth_impl(0), 0);
    }

    #[test]
    fn protected_paths_trap_on_write() {
        use super::super::io::is_protected_path;
        assert!(is_protected_path("Project.config"));
        assert!(is_protected_path("Project.deplock"));
        assert!(is_protected_path(".git/refs/heads/main"));
        assert!(!is_protected_path("notes.txt"));
    }

    #[test]
    fn mmap_file_write_flush_read_back() {
        let dir = case_dir("mmap");
        let file = case_str(&dir, "seg.bin");
        std::fs::write(&file, "abcd").unwrap();
        assert!(fs_mmap_impl(&case_str(&dir, "missing.bin"), 3).is_err());
        let empty = case_str(&dir, "empty.bin");
        std::fs::write(&empty, "").unwrap();
        assert!(fs_mmap_impl(&empty, 3).is_err());
        let handle = fs_mmap_impl(&file, 3).unwrap();
        assert_eq!(fs_mmap_len_impl(handle), 4);
        let addr = fs_mmap_addr_impl(handle);
        assert_ne!(addr, 0);
        unsafe {
            std::ptr::write_bytes(addr as *mut u8, b'Z', 4);
        }
        assert!(fs_mmap_flush_impl(handle).is_ok());
        fs_mmap_close_impl(handle);
        assert_eq!(std::fs::read(&file).unwrap(), vec![b'Z'; 4]);
        let ro = fs_mmap_impl(&file, 0).unwrap();
        assert_eq!(fs_mmap_len_impl(ro), 4);
        assert_ne!(fs_mmap_addr_impl(ro), 0);
        assert!(fs_mmap_flush_impl(ro).is_ok());
        fs_mmap_close_impl(ro);
        let p = mortal(&file);
        unsafe {
            assert_eq!(str_bytes(rnx_fs_mmap_err(p, 3)), b"");
            let h = rnx_fs_mmap(p, 3);
            assert!(!h.is_null());
            assert_eq!(rnx_fs_mmap_len(h), 4);
            assert_ne!(rnx_fs_mmap_addr(h), 0);
            rnx_fs_mmap_flush(h);
            rnx_fs_mmap_close(h);
            assert_eq!(rnx_fs_mmap_addr(std::ptr::null_mut()), 0);
            assert_eq!(rnx_fs_mmap_len(std::ptr::null_mut()), 0);
            rnx_fs_mmap_flush(std::ptr::null_mut());
            rnx_fs_mmap_close(std::ptr::null_mut());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mmap_double_close_safe_and_addr_zero_after_close() {
        let dir = case_dir("mmap-close");
        let file = case_str(&dir, "seg.bin");
        std::fs::write(&file, "abcd").unwrap();
        let handle = fs_mmap_impl(&file, 3).unwrap();
        assert_ne!(fs_mmap_addr_impl(handle), 0);
        assert_eq!(fs_mmap_len_impl(handle), 4);
        fs_mmap_close_impl(handle);
        fs_mmap_close_impl(handle);
        assert_eq!(fs_mmap_addr_impl(handle), 0);
        assert_eq!(fs_mmap_len_impl(handle), 0);
        assert!(fs_mmap_flush_impl(handle).is_ok());
        let anon = fs_mmap_anon_impl(8).unwrap();
        fs_mmap_close_impl(anon);
        fs_mmap_close_impl(anon);
        assert_eq!(fs_mmap_addr_impl(anon), 0);
        assert_eq!(fs_mmap_len_impl(anon), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mmap_anon_roundtrip_and_bad_len() {
        assert!(fs_mmap_anon_impl(0).is_err());
        assert!(fs_mmap_anon_impl(-8).is_err());
        let handle = fs_mmap_anon_impl(16).unwrap();
        assert_eq!(fs_mmap_len_impl(handle), 16);
        let addr = fs_mmap_addr_impl(handle);
        assert_ne!(addr, 0);
        unsafe {
            std::ptr::write_bytes(addr as *mut u8, 0xAB, 16);
            for i in 0..16 {
                assert_eq!(std::ptr::read((addr as *const u8).add(i)), 0xAB);
            }
        }
        assert!(fs_mmap_flush_impl(handle).is_ok());
        fs_mmap_close_impl(handle);
        unsafe {
            assert_eq!(str_bytes(rnx_fs_mmap_anon_err(16)), b"");
            assert!(!str_bytes(rnx_fs_mmap_anon_err(0)).is_empty());
            let h = rnx_fs_mmap_anon(16);
            assert!(!h.is_null());
            assert_eq!(rnx_fs_mmap_len(h), 16);
            rnx_fs_mmap_close(h);
            assert!(rnx_fs_mmap_anon(0).is_null());
        }
    }

    #[test]
    fn read_link_roundtrip_and_missing_err() {
        let dir = case_dir("link");
        let real = case_str(&dir, "real.txt");
        std::fs::write(&real, "data").unwrap();
        let link = case_str(&dir, "link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(not(unix))]
        std::fs::write(&link, "data").unwrap();
        let target = fs_read_link_impl(&link);
        #[cfg(unix)]
        assert_eq!(target.unwrap(), real);
        let missing = case_str(&dir, "gone.txt");
        assert!(fs_read_link_impl(&missing).is_err());
        let p = mortal(&missing);
        unsafe {
            assert_eq!(str_bytes(rnx_fs_read_link(p)), b"");
            assert!(!str_bytes(rnx_fs_read_link_err(p)).is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
