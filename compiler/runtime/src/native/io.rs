use super::common::*;
use super::guard;
use super::collections::*;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_print_str(ptr: *const u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(ptr, len);
        let text = String::from_utf8_lossy(bytes);
        let _ = write!(std::io::stdout(), "{text}");
        let _ = std::io::stdout().flush();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rnx_print_i64(val: i64) {
    let _ = write!(std::io::stdout(), "{val}");
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_print_val(bits: u64, tag: u32) {
    let text = match tag {
        TAG_INT => (bits as i64).to_string(),
        TAG_BOOL => (if bits != 0 { "true" } else { "false" }).to_string(),
        TAG_FLOAT => fmt_float(f64::from_bits(bits)),
        TAG_STR => {
            if bits == 0 {
                "null".to_string()
            } else {
                String::from_utf8_lossy(str_bytes(bits as *const u8)).into_owned()
            }
        }
        _ => {
            if bits == 0 {
                "null".to_string()
            } else if let Some(inner) = any_box_tag(bits) {
                let payload = unsafe { rnx_any_unbox(bits) };
                match inner {
                    TAG_INT => (payload as i64).to_string(),
                    TAG_BOOL => (if payload != 0 { "true" } else { "false" }).to_string(),
                    TAG_STR => {
                        if payload == 0 {
                            "null".to_string()
                        } else {
                            String::from_utf8_lossy(str_bytes(payload as *const u8)).into_owned()
                        }
                    }
                    _ => fmt_float(f64::from_bits(payload)),
                }
            } else {
                super::pretty::pretty_any_colored(bits, 1)
            }
        }
    };
    let _ = write!(std::io::stdout(), "{text}");
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_panic(msg_ptr: *const u8, msg_len: usize) -> ! {
    let msg = if msg_ptr.is_null() || msg_len == 0 {
        "<null>".to_string()
    } else {
        unsafe {
            let bytes = std::slice::from_raw_parts(msg_ptr, msg_len);
            String::from_utf8_lossy(bytes).into_owned()
        }
    };
    let _ = writeln!(std::io::stderr(), "rnx panic: {msg}");
    let trace = std::backtrace::Backtrace::capture();
    if matches!(trace.status(), std::backtrace::BacktraceStatus::Captured) {
        let _ = writeln!(std::io::stderr(), "{trace}");
    }
    std::process::abort();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_init(argc: i32, argv: *const *const u8) {
    guard::guard_install();
    match GENREF_TABLE.lock() {
        Ok(guard) => drop(guard),
        Err(_) => {}
    }
    if !argv.is_null() && argc > 0 {
        let mut args = Vec::with_capacity(argc as usize);
        for i in 0..argc as usize {
            let p = unsafe { *argv.add(i) };
            if p.is_null() {
                break;
            }
            let mut len = 0usize;
            while unsafe { *p.add(len) } != 0 {
                len += 1;
            }
            let bytes = unsafe { std::slice::from_raw_parts(p, len) };
            args.push(String::from_utf8_lossy(bytes).into_owned());
        }
        rnx_set_args(&args);
    }
}

pub(crate) static ENV_ARGS: std::sync::LazyLock<std::sync::Mutex<Option<Vec<String>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

pub fn rnx_set_args(args: &[String]) {
    match ENV_ARGS.lock() {
        Ok(mut g) => *g = Some(args.to_vec()),
        Err(_) => {}
    }
}

pub(crate) fn env_args() -> Vec<String> {
    match ENV_ARGS.lock() {
        Ok(mut g) => {
            if g.is_none() {
                *g = Some(std::env::args().collect());
            }
            g.clone().unwrap_or_default()
        }
        Err(_) => Vec::new(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_clock_monotonic_nanos() -> u64 {
    static T0: std::sync::LazyLock<std::time::Instant> =
        std::sync::LazyLock::new(std::time::Instant::now);
    T0.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sleep_nanos(nanos: u64) {
    std::thread::sleep(std::time::Duration::from_nanos(nanos));
}

pub(crate) fn file_of(handle: *mut u8) -> Option<&'static mut std::fs::File> {
    if handle.is_null() || !file_is_live(handle) {
        return None;
    }
    #[cfg(target_arch = "wasm32")]
    if is_stdio_sentinel(handle) {
        return None;
    }
    Some(unsafe { &mut *(handle as *mut std::fs::File) })
}

pub(crate) static FILE_LIVE: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeSet<usize>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeSet::new()));

fn file_is_live(handle: *mut u8) -> bool {
    FILE_LIVE.lock().unwrap_or_else(|e| e.into_inner()).contains(&(handle as usize))
}

pub(crate) static FILE_READABLE: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeSet<usize>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeSet::new()));

pub fn file_open_impl(path: &str, mode: u64) -> *mut u8 {
    if matches!(mode, 1 | 2 | 3 | 4 | 5) {
        check_protected_write(path);
    }
    let file = match mode {
        1 => std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(path),
        2 => std::fs::OpenOptions::new().append(true).create(true).open(path),
        3 => std::fs::OpenOptions::new().read(true).write(true).open(path),
        4 => std::fs::OpenOptions::new().write(true).create_new(true).open(path),
        5 => std::fs::OpenOptions::new().write(true).create(true).open(path),
        _ => std::fs::OpenOptions::new().read(true).open(path),
    };
    match file {
        Ok(f) => {
            let handle = Box::into_raw(Box::new(f)) as *mut u8;
            FILE_LIVE.lock().unwrap_or_else(|e| e.into_inner()).insert(handle as usize);
            if !matches!(mode, 1 | 2 | 4 | 5) {
                FILE_READABLE.lock().unwrap_or_else(|e| e.into_inner()).insert(handle as usize);
            }
            handle
        }
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_open(path: *const u8, mode: i64) -> *mut u8 {
    file_open_impl(&native_str(path), mode as u64)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_close(handle: *mut u8) {
    if handle.is_null() {
        return;
    }
    if !FILE_LIVE.lock().unwrap_or_else(|e| e.into_inner()).remove(&(handle as usize)) {
        return;
    }
    FILE_READABLE.lock().unwrap_or_else(|e| e.into_inner()).remove(&(handle as usize));
    stdio_lock().remove(&(handle as usize));
    #[cfg(target_arch = "wasm32")]
    if is_stdio_sentinel(handle) {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle as *mut std::fs::File));
    }
}

pub fn file_write_impl(handle: *mut u8, bytes: &[u8]) -> i64 {
    let file = match file_of(handle) {
        Some(f) => f,
        None => return -1,
    };
    match std::io::Write::write_all(file, bytes) {
        Ok(()) => bytes.len() as i64,
        Err(_) => -1,
    }
}

pub fn file_read_impl(handle: *mut u8) -> Result<String, String> {
    let file = match file_of(handle) {
        Some(f) => f,
        None => return Err("readText failed: closed handle".to_string()),
    };
    let mut text = String::new();
    match std::io::Read::read_to_string(file, &mut text) {
        Ok(_) => Ok(text),
        Err(e) => Err(format!("readText failed: {e}")),
    }
}

pub fn file_read_text_err_impl(handle: *mut u8) -> String {
    if handle.is_null() {
        return "readText failed: closed handle".to_string();
    }
    if !FILE_READABLE.lock().unwrap_or_else(|e| e.into_inner()).contains(&(handle as usize)) {
        return "readText failed: handle not open for reading".to_string();
    }
    String::new()
}

pub fn file_read_bytes_impl(fhandle: *mut u8, buf: *mut u8, off: i64, len: i64) -> i64 {
    let file = match file_of(fhandle) {
        Some(f) => f,
        None => return -1,
    };
    let cap = bytes_len_impl(buf);
    if off < 0 || off > cap {
        unsafe {
            rnx_panic(b"byte buffer out of bounds\0".as_ptr(), "byte buffer out of bounds".len());
        }
    }
    let want = if len == -1 { cap - off } else { len };
    if want < 0 || off + want > cap {
        unsafe {
            rnx_panic(b"byte buffer out of bounds\0".as_ptr(), "byte buffer out of bounds".len());
        }
    }
    if want == 0 {
        return 0;
    }
    let st = bytes_state(buf);
    let dst = &mut st.data[off as usize..(off + want) as usize];
    match std::io::Read::read(file, dst) {
        Ok(0) => 0,
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

pub fn file_write_bytes_impl(fhandle: *mut u8, buf: *mut u8, off: i64, len: i64) -> i64 {
    let file = match file_of(fhandle) {
        Some(f) => f,
        None => return -1,
    };
    let have = bytes_len_impl(buf);
    if off < 0 || off > have {
        unsafe {
            rnx_panic(b"byte buffer out of bounds\0".as_ptr(), "byte buffer out of bounds".len());
        }
    }
    let want = if len == -1 { have - off } else { len };
    if want < 0 || off + want > have {
        unsafe {
            rnx_panic(b"byte buffer out of bounds\0".as_ptr(), "byte buffer out of bounds".len());
        }
    }
    if want == 0 {
        return 0;
    }
    let st = bytes_state(buf);
    let src = &st.data[off as usize..(off + want) as usize];
    match std::io::Write::write_all(file, src) {
        Ok(()) => want,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_read_bytes(fhandle: *mut u8, buf: *mut u8, off: i64, len: i64) -> i64 {
    file_read_bytes_impl(fhandle, buf, off, len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_write_bytes(fhandle: *mut u8, buf: *mut u8, off: i64, len: i64) -> i64 {
    file_write_bytes_impl(fhandle, buf, off, len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_read_text(handle: *mut u8) -> *mut u8 {
    let text = file_read_impl(handle).unwrap_or_default();
    let out = str_alloc(text.len());
    if out.is_null() {
        return out;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), out.add(STR_HEADER), text.len());
        out.add(STR_HEADER).add(text.len()).write(0);
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_read_text_err(handle: *mut u8) -> *mut u8 {
    alloc_str(&file_read_text_err_impl(handle))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_write_text(handle: *mut u8, text: *const u8) -> i64 {
    file_write_impl(handle, str_bytes(text))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_flush(handle: *mut u8) {
    if let Some(file) = file_of(handle) {
        let _ = std::io::Write::flush(file);
    }
}

pub fn file_seek_impl(handle: *mut u8, pos: i64) -> i64 {
    if pos < 0 {
        return -1;
    }
    let file = match file_of(handle) {
        Some(f) => f,
        None => return -1,
    };
    match std::io::Seek::seek(file, std::io::SeekFrom::Start(pos as u64)) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_seek(handle: *mut u8, pos: i64) -> i64 {
    file_seek_impl(handle, pos)
}

pub fn file_tell_impl(handle: *mut u8) -> i64 {
    let file = match file_of(handle) {
        Some(f) => f,
        None => return -1,
    };
    match std::io::Seek::stream_position(file) {
        Ok(p) => p.min(i64::MAX as u64) as i64,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_tell(handle: *mut u8) -> i64 {
    file_tell_impl(handle)
}

pub(crate) static STDIO_HANDLES: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeMap<usize, i64>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

fn stdio_lock() -> std::sync::MutexGuard<'static, std::collections::BTreeMap<usize, i64>> {
    STDIO_HANDLES.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn stdio_fd_of(handle: *mut u8) -> Option<i64> {
    if handle.is_null() {
        return None;
    }
    stdio_lock().get(&(handle as usize)).copied()
}

pub fn io_is_tty_impl(fd: i64) -> bool {
    if fd < 0 || fd > i32::MAX as i64 {
        return false;
    }
    #[cfg(unix)]
    {
        unsafe { libc::isatty(fd as libc::c_int) == 1 }
    }
    #[cfg(windows)]
    {
        io_is_tty_windows(fd as i32)
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

pub fn io_winsize_impl() -> (i64, i64) {
    io_winsize_of(1)
}

pub fn io_winsize_of(fd: i64) -> (i64, i64) {
    #[cfg(unix)]
    {
        if fd >= 0 && fd <= i32::MAX as i64 {
            let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
            if unsafe { libc::ioctl(fd as libc::c_int, libc::TIOCGWINSZ, &mut ws) } == 0 {
                let cols = ws.ws_col as i64;
                let rows = ws.ws_row as i64;
                if cols > 0 && rows > 0 {
                    return (cols, rows);
                }
            }
        }
        (80, 24)
    }
    #[cfg(windows)]
    {
        io_winsize_windows()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = fd;
        (80, 24)
    }
}

pub fn io_set_raw_impl(fd: i64, enabled: bool) -> Result<(), String> {
    if fd < 0 || fd > i32::MAX as i64 {
        return Err(format!("setRawMode failed: bad descriptor {fd}"));
    }
    #[cfg(unix)]
    {
        io_set_raw_unix(fd as libc::c_int, enabled)
    }
    #[cfg(windows)]
    {
        io_set_raw_windows(fd as i32, enabled)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = enabled;
        Err("setRawMode failed: raw mode is not supported on this platform".to_string())
    }
}

#[cfg(unix)]
static SAVED_TERMIOS: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeMap<i32, libc::termios>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

#[cfg(unix)]
fn restore_all_raw_modes() {
    let saved: Vec<(i32, libc::termios)> = match SAVED_TERMIOS.lock() {
        Ok(mut g) => {
            let out: Vec<(i32, libc::termios)> =
                g.iter().map(|(fd, tio)| (*fd, tio.clone())).collect();
            g.clear();
            out
        }
        Err(_) => return,
    };
    for (fd, tio) in saved {
        unsafe {
            libc::tcsetattr(fd, libc::TCSANOW, &tio);
        }
    }
}

#[cfg(unix)]
fn raw_exit_hook_install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        extern "C" fn restore_all() {
            restore_all_raw_modes();
        }
        libc::atexit(restore_all);
    });
}

#[cfg(unix)]
fn io_set_raw_unix(fd: libc::c_int, enabled: bool) -> Result<(), String> {
    if unsafe { libc::isatty(fd) } != 1 {
        return Err("setRawMode failed: stdin is not a TTY".to_string());
    }
    if enabled {
        let mut tio: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut tio) } != 0 {
            return Err(format!("setRawMode failed: {}", std::io::Error::last_os_error()));
        }
        SAVED_TERMIOS.lock().unwrap_or_else(|e| e.into_inner()).insert(fd, tio.clone());
        raw_exit_hook_install();
        tio.c_lflag &= !(libc::ICANON | libc::ECHO);
        tio.c_cc[libc::VMIN as usize] = 1;
        tio.c_cc[libc::VTIME as usize] = 0;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &tio) } != 0 {
            return Err(format!("setRawMode failed: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    } else if let Some(orig) = SAVED_TERMIOS.lock().unwrap_or_else(|e| e.into_inner()).remove(&fd) {
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &orig) } != 0 {
            return Err(format!("setRawMode failed: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    } else {
        let mut tio: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut tio) } != 0 {
            return Err(format!("setRawMode failed: {}", std::io::Error::last_os_error()));
        }
        tio.c_lflag |= libc::ICANON | libc::ECHO;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &tio) } != 0 {
            return Err(format!("setRawMode failed: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

#[cfg(windows)]
#[repr(C)]
struct WinConsoleInfo {
    size: [i16; 2],
    cursor: [i16; 2],
    attrs: u16,
    window: [i16; 4],
    max_size: [i16; 2],
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
    fn GetConsoleMode(handle: *mut std::ffi::c_void, mode: *mut u32) -> i32;
    fn SetConsoleMode(handle: *mut std::ffi::c_void, mode: u32) -> i32;
    fn GetConsoleScreenBufferInfo(handle: *mut std::ffi::c_void, info: *mut WinConsoleInfo) -> i32;
}

#[cfg(windows)]
unsafe extern "C" {
    fn _dup(fd: i32) -> i32;
    fn _get_osfhandle(fd: i32) -> isize;
}

#[cfg(windows)]
fn std_handle_for(fd: i32) -> *mut std::ffi::c_void {
    let which: u32 = match fd {
        0 => 0xfffffff6,
        1 => 0xfffffff5,
        _ => 0xfffffff4,
    };
    unsafe { GetStdHandle(which) }
}

#[cfg(windows)]
fn io_is_tty_windows(fd: i32) -> bool {
    let h = std_handle_for(fd);
    if h.is_null() || h as isize == -1 {
        return false;
    }
    let mut mode = 0u32;
    unsafe { GetConsoleMode(h, &mut mode) != 0 }
}

#[cfg(windows)]
fn io_winsize_windows() -> (i64, i64) {
    let h = std_handle_for(1);
    if !h.is_null() && h as isize != -1 {
        let mut info: WinConsoleInfo = unsafe { std::mem::zeroed() };
        if unsafe { GetConsoleScreenBufferInfo(h, &mut info) } != 0 {
            let cols = (info.window[2] - info.window[0] + 1) as i64;
            let rows = (info.window[3] - info.window[1] + 1) as i64;
            if cols > 0 && rows > 0 {
                return (cols, rows);
            }
        }
    }
    (80, 24)
}

#[cfg(windows)]
static SAVED_CONSOLE_MODE: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeMap<i32, u32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

#[cfg(windows)]
fn restore_all_raw_modes() {
    let saved: Vec<(i32, u32)> = match SAVED_CONSOLE_MODE.lock() {
        Ok(mut g) => {
            let out: Vec<(i32, u32)> = g.iter().map(|(fd, mode)| (*fd, *mode)).collect();
            g.clear();
            out
        }
        Err(_) => return,
    };
    for (fd, mode) in saved {
        let h = std_handle_for(fd);
        if !h.is_null() && h as isize != -1 {
            unsafe {
                SetConsoleMode(h, mode);
            }
        }
    }
}

#[cfg(windows)]
fn raw_exit_hook_install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        extern "C" fn restore_all() {
            restore_all_raw_modes();
        }
        libc::atexit(restore_all);
    });
}

#[cfg(windows)]
fn io_set_raw_windows(fd: i32, enabled: bool) -> Result<(), String> {
    const ENABLE_LINE_INPUT: u32 = 0x2;
    const ENABLE_ECHO_INPUT: u32 = 0x4;
    const ENABLE_PROCESSED_INPUT: u32 = 0x1;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x200;
    if fd != 0 {
        return Err(format!("setRawMode failed: bad descriptor {fd}"));
    }
    let h = std_handle_for(fd);
    if h.is_null() || h as isize == -1 {
        return Err("setRawMode failed: stdin is not a TTY".to_string());
    }
    let mut mode = 0u32;
    if unsafe { GetConsoleMode(h, &mut mode) } == 0 {
        return Err("setRawMode failed: stdin is not a TTY".to_string());
    }
    if enabled {
        SAVED_CONSOLE_MODE.lock().unwrap_or_else(|e| e.into_inner()).insert(fd, mode);
        raw_exit_hook_install();
        let next = (mode | ENABLE_VIRTUAL_TERMINAL_INPUT)
            & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT);
        if unsafe { SetConsoleMode(h, next) } == 0 {
            return Err("setRawMode failed: SetConsoleMode refused the change".to_string());
        }
        Ok(())
    } else if let Some(orig) = SAVED_CONSOLE_MODE.lock().unwrap_or_else(|e| e.into_inner()).remove(&fd) {
        if unsafe { SetConsoleMode(h, orig) } == 0 {
            return Err("setRawMode failed: SetConsoleMode refused the change".to_string());
        }
        Ok(())
    } else {
        let next = (mode | ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT)
            & !ENABLE_VIRTUAL_TERMINAL_INPUT;
        if unsafe { SetConsoleMode(h, next) } == 0 {
            return Err("setRawMode failed: SetConsoleMode refused the change".to_string());
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
fn stdio_sentinel(fd: i64) -> *mut u8 {
    static SLOTS: std::sync::LazyLock<[usize; 3]> = std::sync::LazyLock::new(|| {
        [Box::into_raw(Box::new(0u64)) as usize, Box::into_raw(Box::new(1u64)) as usize, Box::into_raw(Box::new(2u64)) as usize]
    });
    SLOTS[fd as usize] as *mut u8
}

#[cfg(target_arch = "wasm32")]
fn is_stdio_sentinel(handle: *mut u8) -> bool {
    let h = handle as usize;
    h == stdio_sentinel(0) as usize || h == stdio_sentinel(1) as usize || h == stdio_sentinel(2) as usize
}

pub fn file_from_handle_impl(fd: i64, readable: bool, _writable: bool) -> *mut u8 {
    if fd < 0 || fd > i32::MAX as i64 {
        return std::ptr::null_mut();
    }
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        let duped = unsafe { libc::dup(fd as libc::c_int) };
        if duped < 0 {
            return std::ptr::null_mut();
        }
        let file = unsafe { std::fs::File::from_raw_fd(duped) };
        let handle = Box::into_raw(Box::new(file)) as *mut u8;
        FILE_LIVE.lock().unwrap_or_else(|e| e.into_inner()).insert(handle as usize);
        if readable {
            FILE_READABLE.lock().unwrap_or_else(|e| e.into_inner()).insert(handle as usize);
        }
        stdio_lock().insert(handle as usize, fd);
        handle
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::FromRawHandle;
        let duped = unsafe { _dup(fd as i32) };
        if duped < 0 {
            return std::ptr::null_mut();
        }
        let os_handle = unsafe { _get_osfhandle(duped) };
        if os_handle == -1 {
            return std::ptr::null_mut();
        }
        let file = unsafe { std::fs::File::from_raw_handle(os_handle as *mut std::ffi::c_void) };
        let handle = Box::into_raw(Box::new(file)) as *mut u8;
        FILE_LIVE.lock().unwrap_or_else(|e| e.into_inner()).insert(handle as usize);
        if readable {
            FILE_READABLE.lock().unwrap_or_else(|e| e.into_inner()).insert(handle as usize);
        }
        stdio_lock().insert(handle as usize, fd);
        handle
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = _writable;
        if !(0..=2).contains(&fd) {
            return std::ptr::null_mut();
        }
        let handle = stdio_sentinel(fd);
        FILE_LIVE.lock().unwrap_or_else(|e| e.into_inner()).insert(handle as usize);
        if readable {
            FILE_READABLE.lock().unwrap_or_else(|e| e.into_inner()).insert(handle as usize);
        }
        stdio_lock().insert(handle as usize, fd);
        return handle;
    }
    #[cfg(not(any(unix, windows, target_arch = "wasm32")))]
    {
        let _ = (readable, _writable);
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_io_is_tty(fd: i64) -> bool {
    io_is_tty_impl(fd)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_io_winsize() -> *mut u8 {
    let (cols, rows) = io_winsize_impl();
    unsafe {
        let out = rnx_array_new(2, 8);
        if out.is_null() {
            return out;
        }
        rnx_array_push(out, cols as u64, 8);
        rnx_array_push(out, rows as u64, 8);
        out
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_io_set_raw(fd: i64, enabled: i64) -> *mut u8 {
    match io_set_raw_impl(fd, enabled != 0) {
        Ok(()) => alloc_str(""),
        Err(e) => alloc_str(&e),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_file_from_handle(fd: i64, readable: i64, writable: i64) -> *mut u8 {
    file_from_handle_impl(fd, readable != 0, writable != 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_path_exists(path: *const u8) -> bool {
    std::path::Path::new(&native_str(path)).exists()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_path_remove(path: *const u8) -> bool {
    let p = native_str(path);
    path_remove_impl(&p)
}

pub fn path_remove_impl(path: &str) -> bool {
    check_protected_write(path);
    std::fs::remove_file(path).is_ok()
}

pub(crate) fn s301_trap(display: &str) -> ! {
    eprintln!(
        "panic [S301]: Protected Path Violation\n  Attempted modifying access to protected file or directory: '{display}'\n  Protected project configurations and VCS metadata cannot be modified at runtime."
    );
    let _ = std::io::stderr().flush();
    std::process::exit(1);
}

pub(crate) fn normalized_abs(path: &str) -> std::path::PathBuf {
    use std::path::Component;
    let p = std::path::Path::new(path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(p)
    };
    if let Ok(c) = std::fs::canonicalize(&abs) {
        return c;
    }
    let mut out = std::path::PathBuf::new();
    for comp in abs.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

pub(crate) fn has_manifest_above(dir: &std::path::Path) -> bool {
    let mut d = dir.to_path_buf();
    loop {
        if d.join("Project.config").is_file() {
            return true;
        }
        if !d.pop() {
            return false;
        }
    }
}

pub(crate) fn is_protected_path(path: &str) -> bool {
    let abs = normalized_abs(path);
    if let Some(name) = abs.file_name().and_then(|n| n.to_str()) {
        if name.eq_ignore_ascii_case("Project.config") || name.eq_ignore_ascii_case("Project.deplock") {
            return true;
        }
    }
    let comps: Vec<String> = abs
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let has = |n: &str| comps.iter().any(|c| c.eq_ignore_ascii_case(n));
    if has(".git") || has(".rnx") {
        return true;
    }
    if has("target") {
        let parent = abs.parent().unwrap_or(abs.as_path());
        if has_manifest_above(parent) {
            return true;
        }
    }
    false
}

pub(crate) fn check_protected_write(path: &str) {
    if is_protected_path(path) {
        s301_trap(path);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_panic_str(s: *const u8) -> ! {
    let msg = native_str(s);
    unsafe {
        if msg.is_empty() {
            rnx_panic(std::ptr::null(), 0);
        }
        rnx_panic(msg.as_ptr(), msg.len());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_env_args_count() -> i64 {
    env_args().len() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_env_args_get(index: i64) -> *mut u8 {
    let args = env_args();
    match args.get(index as usize) {
        Some(a) => alloc_str(a),
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_env_get(key: *const u8) -> *mut u8 {
    alloc_str(&env_get_impl(&native_str(key)))
}

pub fn env_get_impl(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

pub fn env_set_impl(key: &str, val: &str) {
    unsafe {
        std::env::set_var(key, val);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_env_set(key: *const u8, val: *const u8) {
    env_set_impl(&native_str(key), &native_str(val));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_env_cwd() -> *mut u8 {
    match std::env::current_dir() {
        Ok(p) => alloc_str(&p.to_string_lossy()),
        Err(_) => alloc_str(""),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_host_version() -> *mut u8 {
    alloc_str(env!("CARGO_PKG_VERSION"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_env_exit(code: i64) -> ! {
    std::process::exit(code as i32);
}

pub(crate) fn os_string(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

pub(crate) fn os_uptime_secs() -> f64 {
    if cfg!(target_os = "linux")
        && let Ok(text) = std::fs::read_to_string("/proc/uptime")
        && let Some(first) = text.split_whitespace().next()
        && let Ok(secs) = first.parse::<f64>() {
            return secs;
        }
    0.0
}

pub(crate) struct ChildEntry {
    child: Option<std::process::Child>,
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
    code: Option<i64>,
    out: Vec<u8>,
    err: Vec<u8>,
    out_taken: bool,
    err_taken: bool,
}

pub(crate) struct ChildTable {
    next: i64,
    map: std::collections::HashMap<i64, ChildEntry>,
}

pub(crate) static CHILD_TABLE: std::sync::OnceLock<std::sync::Mutex<ChildTable>> = std::sync::OnceLock::new();

pub(crate) fn child_table() -> &'static std::sync::Mutex<ChildTable> {
    CHILD_TABLE.get_or_init(|| {
        std::sync::Mutex::new(ChildTable { next: 1, map: std::collections::HashMap::new() })
    })
}

pub(crate) fn child_table_lock() -> std::sync::MutexGuard<'static, ChildTable> {
    match child_table().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn process_panic(msg: &str) -> ! {
    unsafe {
        rnx_panic(msg.as_ptr(), msg.len());
    }
}

pub(crate) fn read_str_array(arr: *const u8) -> Vec<String> {
    if arr.is_null() {
        return Vec::new();
    }
    let n = unsafe { rnx_array_len(arr) };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let slot = unsafe { rnx_array_get(arr, i, 8) };
        if slot == 0 {
            continue;
        }
        out.push(native_str(slot as *const u8));
    }
    out
}

pub(crate) fn spawn_stdio(mode: i64) -> std::process::Stdio {
    match mode {
        2 => std::process::Stdio::null(),
        1 => std::process::Stdio::piped(),
        _ => std::process::Stdio::inherit(),
    }
}

pub(crate) fn apply_spawn_env(cmd: &mut std::process::Command, pairs: &[String]) {
    for pair in pairs {
        match pair.find('=') {
            Some(eq) => {
                cmd.env(&pair[..eq], &pair[eq + 1..]);
            }
            None => {
                cmd.env(pair, "");
            }
        }
    }
}

pub(crate) static SPAWN_STRICT: AtomicBool = AtomicBool::new(false);
pub(crate) static SPAWN_ALLOW: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

#[unsafe(no_mangle)]
pub extern "C" fn rnx_set_spawn_allowlist(csv: *const u8, len: usize) {
    let text = if csv.is_null() {
        String::new()
    } else {
        let bytes = unsafe { std::slice::from_raw_parts(csv, len) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    let list: Vec<String> = text
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if let Ok(mut guard) = SPAWN_ALLOW.lock() {
        *guard = list;
    }
    SPAWN_STRICT.store(true, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn rnx_clear_spawn_restriction() {
    if let Ok(mut guard) = SPAWN_ALLOW.lock() {
        guard.clear();
    }
    SPAWN_STRICT.store(false, Ordering::SeqCst);
}

pub(crate) fn spawn_basename(cmd: &str) -> &str {
    cmd.rsplit(['/', '\\']).next().unwrap_or(cmd)
}

pub(crate) fn spawn_allowed(cmd: &str) -> bool {
    if !SPAWN_STRICT.load(Ordering::SeqCst) {
        return true;
    }
    let base = spawn_basename(cmd.trim());
    SPAWN_ALLOW.lock()
        .map(|g| g.iter().any(|a| a == base))
        .unwrap_or(false)
}

pub(crate) fn s401_trap(cmd: &str) -> ! {
    eprintln!(
        "panic [S401]: Unapproved Child Process\n  Attempted to spawn `{cmd}` without a covering `sys:exec` grant.\n  Approve the binary in Project.deplock or run without `--locked`."
    );
    let _ = std::io::stderr().flush();
    std::process::exit(1);
}

pub fn process_spawn_impl(
    cmd: &str,
    args: &[String],
    cwd: &str,
    env_pairs: &[String],
    si: i64,
    so: i64,
    se: i64,
) -> i64 {
    if cmd.trim().is_empty() {
        s401_trap("(empty command)");
    }
    if !spawn_allowed(cmd) {
        s401_trap(cmd);
    }
    let mut command = std::process::Command::new(cmd);
    command.args(args);
    if !cwd.is_empty() {
        command.current_dir(cwd);
    }
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SYSTEMROOT") {
        command.env("SYSTEMROOT", root);
    }
    apply_spawn_env(&mut command, env_pairs);
    command.stdin(spawn_stdio(si)).stdout(spawn_stdio(so)).stderr(spawn_stdio(se));
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => process_panic(&format!("process spawn failed for `{cmd}`: {e}")),
    };
    let entry = ChildEntry {
        stdin: child.stdin.take(),
        stdout: child.stdout.take(),
        stderr: child.stderr.take(),
        child: Some(child),
        code: None,
        out: Vec::new(),
        err: Vec::new(),
        out_taken: false,
        err_taken: false,
    };
    let mut table = child_table_lock();
    let handle = table.next;
    table.next += 1;
    table.map.insert(handle, entry);
    handle
}

pub(crate) fn exit_code_of(status: &std::process::ExitStatus) -> i64 {
    if let Some(code) = status.code() {
        return code as i64;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return 128 + sig as i64;
        }
    }
    -1
}

pub(crate) fn reap_locked(table: &mut ChildTable, handle: i64, status: std::process::ExitStatus) -> i64 {
    let code = exit_code_of(&status);
    if let Some(entry) = table.map.get_mut(&handle) {
        entry.child = None;
        entry.code = Some(code);
    }
    code
}

pub fn process_wait_impl(handle: i64) -> i64 {
    let mut table = child_table_lock();
    if let Some(entry) = table.map.get_mut(&handle) {
        if let Some(code) = entry.code {
            return code;
        }
        if let Some(mut child) = entry.child.take() {
            drop(entry.stdin.take());
            match child.wait() {
                Ok(status) => {
                    drop(entry.stdout.take());
                    drop(entry.stderr.take());
                    return reap_locked(&mut table, handle, status);
                }
                Err(_) => {
                    entry.child = Some(child);
                    return -1;
                }
            }
        }
    }
    -1
}

pub fn process_try_wait_impl(handle: i64) -> i64 {
    let mut table = child_table_lock();
    let entry = match table.map.get_mut(&handle) {
        Some(e) => e,
        None => return -1,
    };
    if let Some(code) = entry.code {
        return code;
    }
    let status = match entry.child.as_mut() {
        Some(child) => match child.try_wait() {
            Ok(Some(s)) => s,
            Ok(None) => return -1,
            Err(_) => return -1,
        },
        None => return -1,
    };
    reap_locked(&mut table, handle, status)
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

pub fn process_kill_impl(handle: i64, sig: i64) -> i64 {
    let pid: Option<u32> = child_table_lock().map.get(&handle).and_then(|e| e.child.as_ref()).map(|c| c.id());
    let pid = match pid {
        Some(p) => p,
        None => return 0,
    };
    #[cfg(unix)]
    {
        if unsafe { kill(pid as i32, sig as i32) } == 0 {
            return 1;
        }
        0
    }
    #[cfg(not(unix))]
    {
        let _ = sig;
        let mut table = child_table_lock();
        match table.map.get_mut(&handle).and_then(|e| e.child.as_mut()) {
            Some(child) => match child.kill() {
                Ok(()) => 1,
                Err(_) => 0,
            },
            None => 0,
        }
    }
}

pub fn process_forget_impl(handle: i64) {
    child_table_lock().map.remove(&handle);
}

pub fn process_pid_of_impl(handle: i64) -> i64 {
    match child_table_lock().map.get(&handle).and_then(|e| e.child.as_ref()) {
        Some(child) => child.id() as i64,
        None => 0,
    }
}

pub fn process_exit_code_impl(handle: i64) -> i64 {
    match child_table_lock().map.get(&handle).and_then(|e| e.code) {
        Some(code) => code,
        None => -1,
    }
}

pub(crate) fn pipe_bounds(buf: *mut u8, off: i64, len: i64) -> (usize, usize) {
    let cap = bytes_len_impl(buf);
    if off < 0 || off > cap {
        process_panic("byte buffer out of bounds");
    }
    let want = if len == -1 { cap - off } else { len };
    if want < 0 || off + want > cap {
        process_panic("byte buffer out of bounds");
    }
    (off as usize, want as usize)
}

pub fn process_write_stdin_impl(handle: i64, buf: *mut u8, off: i64, len: i64) -> i64 {
    let (start, want) = pipe_bounds(buf, off, len);
    if want == 0 {
        return 0;
    }
    let data = bytes_state(buf).data[start..start + want].to_vec();
    let mut stdin = match child_table_lock().map.get_mut(&handle).and_then(|e| e.stdin.take()) {
        Some(s) => s,
        None => return -1,
    };
    let result = match std::io::Write::write(&mut stdin, &data) {
        Ok(n) => n as i64,
        Err(_) => -1,
    };
    child_table_lock().map.get_mut(&handle).map(|e| {
        e.stdin = Some(stdin);
    });
    result
}

pub fn process_read_pipe_impl(handle: i64, stream: i64, buf: *mut u8, off: i64, len: i64) -> i64 {
    let (start, want) = pipe_bounds(buf, off, len);
    if want == 0 {
        return 0;
    }
    enum Taken {
        Out(std::process::ChildStdout),
        Err(std::process::ChildStderr),
    }
    let mut table = child_table_lock();
    let taken = match table.map.get_mut(&handle) {
        Some(entry) => {
            if stream == 2 {
                entry.stderr.take().map(Taken::Err)
            } else {
                entry.stdout.take().map(Taken::Out)
            }
        }
        None => None,
    };
    drop(table);
    let mut taken = match taken {
        Some(t) => t,
        None => return -1,
    };
    let reader: &mut dyn std::io::Read = match &mut taken {
        Taken::Out(p) => p,
        Taken::Err(p) => p,
    };
    let dst = &mut bytes_state(buf).data[start..start + want];
    let result = match std::io::Read::read(reader, dst) {
        Ok(n) => n as i64,
        Err(_) => -1,
    };
    let mut table = child_table_lock();
    if let Some(entry) = table.map.get_mut(&handle) {
        match taken {
            Taken::Out(p) => entry.stdout = Some(p),
            Taken::Err(p) => entry.stderr = Some(p),
        }
    }
    result
}

pub fn process_close_stdin_impl(handle: i64) {
    let mut table = child_table_lock();
    if let Some(entry) = table.map.get_mut(&handle) {
        drop(entry.stdin.take());
    }
}

pub fn process_take_pipe_impl(handle: i64, stream: i64) -> i64 {
    let mut table = child_table_lock();
    let entry = match table.map.get_mut(&handle) {
        Some(e) => e,
        None => return 0,
    };
    let (taken, bytes) = if stream == 2 {
        (&mut entry.err_taken, std::mem::take(&mut entry.err))
    } else {
        (&mut entry.out_taken, std::mem::take(&mut entry.out))
    };
    if *taken {
        return 0;
    }
    *taken = true;
    let boxed = Box::new(ByteBufferState { data: bytes });
    Box::into_raw(boxed) as usize as i64
}

pub fn process_run_impl(
    cmd: &str,
    args: &[String],
    cwd: &str,
    env_pairs: &[String],
    si: i64,
    so: i64,
    se: i64,
) -> i64 {
    let handle = process_spawn_impl(cmd, args, cwd, env_pairs, si, so, se);
    process_close_stdin_impl(handle);
    {
        let mut table = child_table_lock();
        let entry = match table.map.get_mut(&handle) {
            Some(e) => e,
            None => return handle,
        };
        let mut out_pipe = entry.stdout.take();
        let mut err_pipe = entry.stderr.take();
        drop(table);
        std::thread::scope(|scope| {
            let out_thread = scope.spawn(move || {
                let mut buf = Vec::new();
                if let Some(pipe) = out_pipe.as_mut() {
                    let _ = std::io::Read::read_to_end(pipe, &mut buf);
                }
                buf
            });
            let mut err_buf = Vec::new();
            if let Some(pipe) = err_pipe.as_mut() {
                let _ = std::io::Read::read_to_end(pipe, &mut err_buf);
            }
            let out_buf = out_thread.join().unwrap_or_default();
            let mut table = child_table_lock();
            if let Some(entry) = table.map.get_mut(&handle) {
                entry.out = out_buf;
                entry.err = err_buf;
            }
        });
    }
    process_wait_impl(handle);
    handle
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_pid() -> i64 {
    std::process::id() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_remove_env(key: *const u8) {
    unsafe { std::env::remove_var(native_str(key)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_all_env_count() -> i64 {
    std::env::vars().count() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_all_env_get(index: i64) -> *mut u8 {
    match std::env::vars().nth(index as usize) {
        Some((k, v)) => alloc_str(&format!("{k}={v}")),
        None => alloc_str(""),
    }
}

pub fn process_chdir_impl(path: &str) -> i64 {
    match std::env::set_current_dir(path) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_chdir(path: *const u8) -> i64 {
    process_chdir_impl(&native_str(path))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_os_platform() -> *mut u8 {
    alloc_str(std::env::consts::OS)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_os_arch() -> *mut u8 {
    alloc_str(std::env::consts::ARCH)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_os_hostname() -> *mut u8 {
    let host = os_string("HOSTNAME");
    if !host.is_empty() {
        return alloc_str(&host);
    }
    if let Ok(text) = std::fs::read_to_string("/etc/hostname") {
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            return alloc_str(&trimmed);
        }
    }
    let win = os_string("COMPUTERNAME");
    if !win.is_empty() {
        return alloc_str(&win);
    }
    alloc_str("unknown")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_os_tmpdir() -> *mut u8 {
    for key in ["TMPDIR", "TEMP", "TMP"] {
        let val = os_string(key);
        if !val.is_empty() {
            return alloc_str(&val);
        }
    }
    alloc_str(&std::env::temp_dir().to_string_lossy())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_os_homedir() -> *mut u8 {
    for key in ["HOME", "USERPROFILE"] {
        let val = os_string(key);
        if !val.is_empty() {
            return alloc_str(&val);
        }
    }
    alloc_str("")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_os_cpu_count() -> i64 {
    std::thread::available_parallelism().map(|n| n.get() as i64).unwrap_or(1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_os_uptime() -> u64 {
    os_uptime_secs().to_bits()
}

fn spawn_args(cmd: *const u8, args: *const u8, cwd: *const u8, env: *const u8) -> (String, Vec<String>, String, Vec<String>) {
    (native_str(cmd), read_str_array(args), native_str(cwd), read_str_array(env))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_spawn(
    cmd: *const u8,
    args: *const u8,
    cwd: *const u8,
    env: *const u8,
    si: i64,
    so: i64,
    se: i64,
) -> i64 {
    let (c, a, w, e) = spawn_args(cmd, args, cwd, env);
    process_spawn_impl(&c, &a, &w, &e, si, so, se)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_run(
    cmd: *const u8,
    args: *const u8,
    cwd: *const u8,
    env: *const u8,
    si: i64,
    so: i64,
    se: i64,
) -> i64 {
    let (c, a, w, e) = spawn_args(cmd, args, cwd, env);
    process_run_impl(&c, &a, &w, &e, si, so, se)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_pid_of(handle: i64) -> i64 {
    process_pid_of_impl(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_write_stdin(handle: i64, buf: *mut u8, off: i64, len: i64) -> i64 {
    process_write_stdin_impl(handle, buf, off, len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_read_stdout(handle: i64, buf: *mut u8, off: i64, len: i64) -> i64 {
    process_read_pipe_impl(handle, 1, buf, off, len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_read_stderr(handle: i64, buf: *mut u8, off: i64, len: i64) -> i64 {
    process_read_pipe_impl(handle, 2, buf, off, len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_close_stdin(handle: i64) {
    process_close_stdin_impl(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_wait(handle: i64) -> i64 {
    process_wait_impl(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_try_wait(handle: i64) -> i64 {
    process_try_wait_impl(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_kill(handle: i64, sig: i64) -> i64 {
    process_kill_impl(handle, sig)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_take_stdout(handle: i64) -> i64 {
    process_take_pipe_impl(handle, 1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_take_stderr(handle: i64) -> i64 {
    process_take_pipe_impl(handle, 2)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_exit_code(handle: i64) -> i64 {
    process_exit_code_impl(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_process_forget(handle: i64) {
    process_forget_impl(handle)
}

#[cfg(test)]
mod clock_tests {
    use super::*;

    #[test]
    fn monotonic_non_decreasing() {
        unsafe {
            let a = rnx_clock_monotonic_nanos();
            let b = rnx_clock_monotonic_nanos();
            assert!(b >= a);
        }
    }

    #[test]
    fn sleep_advances_clock() {
        unsafe {
            let a = rnx_clock_monotonic_nanos();
            rnx_sleep_nanos(2_000_000);
            let b = rnx_clock_monotonic_nanos();
            assert!(b >= a + 1_000_000);
        }
    }
}

#[cfg(test)]
mod io_tests {
    use super::*;

    #[test]
    fn file_write_read_remove_roundtrip() {
        let dir = std::env::temp_dir().join("rnx-io-native-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("t.txt");
        let path_str = path.to_string_lossy().into_owned();
        let make_str = |text: &str| {
            let out = str_alloc(text.len());
            assert!(!out.is_null());
            unsafe {
                std::ptr::copy_nonoverlapping(text.as_ptr(), out.add(STR_HEADER), text.len());
                out.add(STR_HEADER).add(text.len()).write(0);
            }
            out
        };
        let p = make_str(&path_str);
        unsafe {
            assert!(!rnx_path_exists(p));
            let w = rnx_file_open(p, 1);
            assert!(!w.is_null());
            let t = make_str("hello io");
            assert_eq!(rnx_file_write_text(w, t), 8);
            rnx_file_flush(w);
            rnx_file_close(w);
            assert!(rnx_path_exists(p));
            let r = rnx_file_open(p, 0);
            assert!(!r.is_null());
            let back = rnx_file_read_text(r);
            assert_eq!(str_bytes(back), b"hello io");
            rnx_file_close(r);
            assert!(rnx_path_remove(p));
            assert!(!rnx_path_exists(p));
            assert!(rnx_file_open(p, 0).is_null());
            assert_eq!(rnx_file_write_text(std::ptr::null_mut(), t), -1);
            rnx_file_close(std::ptr::null_mut());
            rnx_file_flush(std::ptr::null_mut());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_read_null_and_write_only_returns_error() {
        unsafe {
            assert_eq!(str_bytes(rnx_file_read_text(std::ptr::null_mut())), b"");
            assert!(!file_read_text_err_impl(std::ptr::null_mut()).is_empty());
            assert!(file_read_impl(std::ptr::null_mut()).is_err());
        }
        let dir = std::env::temp_dir().join("rnx-io-native-read-err");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("w.txt");
        let path_str = path.to_string_lossy().into_owned();
        let out = str_alloc(path_str.len());
        assert!(!out.is_null());
        unsafe {
            std::ptr::copy_nonoverlapping(path_str.as_ptr(), out.add(STR_HEADER), path_str.len());
            out.add(STR_HEADER).add(path_str.len()).write(0);
            let w = rnx_file_open(out, 1);
            assert!(!w.is_null());
            assert!(file_read_impl(w).is_err());
            assert!(!file_read_text_err_impl(w).is_empty());
            rnx_file_close(w);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn foreign_handle_is_never_dereferenced() {
        let foreign = usize::MAX as *mut u8;
        unsafe {
            rnx_file_close(foreign);
            rnx_file_close(foreign);
            rnx_file_flush(foreign);
            let t = str_alloc(1);
            assert!(!t.is_null());
            std::ptr::copy_nonoverlapping(b"x".as_ptr(), t.add(STR_HEADER), 1);
            t.add(STR_HEADER).add(1).write(0);
            assert_eq!(rnx_file_write_text(foreign, t), -1);
            assert_eq!(rnx_file_seek(foreign, 0), -1);
            assert_eq!(rnx_file_tell(foreign), -1);
            assert_eq!(str_bytes(rnx_file_read_text(foreign)), b"");
        }
        assert!(file_of(foreign).is_none());
        assert!(file_read_impl(foreign).is_err());
        assert_eq!(file_write_impl(foreign, b"x"), -1);
        assert_eq!(file_seek_impl(foreign, 0), -1);
        assert_eq!(file_tell_impl(foreign), -1);
    }

    #[test]
    fn double_close_of_live_handle_is_noop() {
        let dir = std::env::temp_dir().join("rnx-io-native-double-close");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("d.txt");
        let path_str = path.to_string_lossy().into_owned();
        let out = str_alloc(path_str.len());
        assert!(!out.is_null());
        unsafe {
            std::ptr::copy_nonoverlapping(path_str.as_ptr(), out.add(STR_HEADER), path_str.len());
            out.add(STR_HEADER).add(path_str.len()).write(0);
            let w = rnx_file_open(out, 1);
            assert!(!w.is_null());
            rnx_file_close(w);
            assert!(file_of(w).is_none());
            assert_eq!(file_write_impl(w, b"x"), -1);
            rnx_file_close(w);
            rnx_file_close(w);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, unix))]
mod term_tests {
    use super::*;
    use std::os::fd::AsRawFd;

    fn devnull_fd() -> std::fs::File {
        std::fs::File::open("/dev/null").unwrap()
    }

    #[test]
    fn is_tty_false_off_tty() {
        assert!(!io_is_tty_impl(-1));
        assert!(!io_is_tty_impl(i64::MAX));
        let null = devnull_fd();
        assert!(!io_is_tty_impl(null.as_raw_fd() as i64));
        unsafe {
            assert!(!rnx_io_is_tty(-1));
            assert!(!rnx_io_is_tty(null.as_raw_fd() as i64));
        }
    }

    #[test]
    fn winsize_falls_back_off_tty() {
        let null = devnull_fd();
        assert_eq!(io_winsize_of(null.as_raw_fd() as i64), (80, 24));
        assert_eq!(io_winsize_of(-1), (80, 24));
        assert_eq!(io_winsize_impl(), io_winsize_of(1));
        unsafe {
            let arr = rnx_io_winsize();
            assert!(!arr.is_null());
            assert_eq!(rnx_array_len(arr), 2);
            let cols = rnx_array_get(arr, 0, 8) as i64;
            let rows = rnx_array_get(arr, 1, 8) as i64;
            assert!(cols > 0 && rows > 0);
        }
    }

    #[test]
    fn set_raw_errs_off_tty_never_aborts() {
        let null = devnull_fd();
        let fd = null.as_raw_fd() as i64;
        assert!(io_set_raw_impl(-1, true).is_err());
        assert!(io_set_raw_impl(fd, true).is_err());
        assert!(io_set_raw_impl(fd, false).is_err());
        unsafe {
            let err = rnx_io_set_raw(-1, 1);
            assert!(!str_bytes(err).is_empty());
            let err = rnx_io_set_raw(fd, 1);
            assert!(!str_bytes(err).is_empty());
            let ok = rnx_io_set_raw(fd, 0);
            assert!(!str_bytes(ok).is_empty());
        }
    }

    #[test]
    fn from_handle_bad_fd_is_null() {
        assert!(file_from_handle_impl(-1, true, true).is_null());
        assert!(file_from_handle_impl(1 << 40, true, true).is_null());
        unsafe {
            assert!(rnx_file_from_handle(-1, 1, 1).is_null());
        }
    }

    #[test]
    fn from_handle_dup_roundtrip() {
        let null = devnull_fd();
        let fd = null.as_raw_fd() as i64;
        let readable = file_from_handle_impl(fd, true, false);
        assert!(!readable.is_null());
        assert_eq!(stdio_fd_of(readable), Some(fd));
        assert!(file_read_text_err_impl(readable).is_empty());
        let write_only = file_from_handle_impl(fd, false, true);
        assert!(!write_only.is_null());
        assert!(!file_read_text_err_impl(write_only).is_empty());
        unsafe {
            rnx_file_close(readable);
            rnx_file_close(write_only);
        }
        assert_eq!(stdio_fd_of(readable), None);
    }

    #[test]
    fn exit_hook_drains_saved_modes_without_tty() {
        raw_exit_hook_install();
        SAVED_TERMIOS.lock().unwrap_or_else(|e| e.into_inner()).insert(-1, unsafe { std::mem::zeroed() });
        restore_all_raw_modes();
        assert!(!SAVED_TERMIOS.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&-1));
    }
}

#[cfg(test)]
mod env_tests {
    use super::*;

    #[test]
    fn set_get_roundtrip() {
        let k = alloc_str("RNX_NATIVE_TEST_KEY");
        let v = alloc_str("hello");
        unsafe {
            rnx_env_set(k, v);
            let back = rnx_env_get(k);
            assert_eq!(str_bytes(back), b"hello");
            let missing = alloc_str("RNX_NATIVE_TEST_MISSING_XYZ");
            let empty = rnx_env_get(missing);
            assert_eq!(str_bytes(empty), b"");
        }
    }

    #[test]
    fn args_default_nonempty_and_oob_null() {
        unsafe {
            assert!(rnx_env_args_count() >= 1);
            assert!(rnx_env_args_get(1 << 30).is_null());
            let first = rnx_env_args_get(0);
            assert!(!first.is_null());
        }
    }

    #[test]
    fn cwd_nonempty() {
        unsafe {
            let c = rnx_env_cwd();
            assert!(!str_bytes(c).is_empty());
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) static DL_HANDLES: LazyLock<Mutex<HashMap<String, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn dl_handle(lib: &str) -> Option<*mut libc::c_void> {
    if lib == "c" {
        return Some(libc::RTLD_DEFAULT as *mut libc::c_void);
    }
    {
        let guard = DL_HANDLES.lock().ok()?;
        if let Some(h) = guard.get(lib) {
            return Some(*h as *mut libc::c_void);
        }
    }
    let cands = [format!("lib{lib}.so"), format!("lib{lib}.so.1"), lib.to_string()];
    for cand in &cands {
        let Ok(path) = std::ffi::CString::new(cand.as_str()) else {
            continue;
        };
        let h = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if !h.is_null() {
            if let Ok(mut guard) = DL_HANDLES.lock() {
                guard.insert(lib.to_string(), h as usize);
            }
            return Some(h);
        }
    }
    None
}

/// Resolve a foreign C symbol for `from native "lib"` calls.
/// `lib` holds the cleaned name (`local:`/`lib` prefixes stripped).
/// Returns the symbol address, or `None` when the library or symbol is missing.
#[cfg(all(unix, not(target_arch = "wasm32")))]
pub fn resolve_foreign(lib: &str, symbol: &str) -> Option<*const u8> {
    let h = dl_handle(lib)?;
    let Ok(sym) = std::ffi::CString::new(symbol) else {
        return None;
    };
    let p = unsafe { libc::dlsym(h, sym.as_ptr()) };
    if p.is_null() {
        return None;
    }
    Some(p as *const u8)
}

#[cfg(target_arch = "wasm32")]
pub fn resolve_foreign(_lib: &str, _symbol: &str) -> Option<*const u8> {
    None
}

#[cfg(windows)]
unsafe extern "C" {
    fn LoadLibraryA(name: *const u8) -> *mut std::ffi::c_void;
    fn GetProcAddress(handle: *mut std::ffi::c_void, name: *const u8) -> *const u8;
}

#[cfg(windows)]
fn dl_handle(lib: &str) -> Option<*mut std::ffi::c_void> {
    {
        let guard = DL_HANDLES.lock().ok()?;
        if let Some(h) = guard.get(lib) {
            return Some(*h as *mut std::ffi::c_void);
        }
    }
    let cands = [format!("{lib}.dll"), lib.to_string()];
    for cand in &cands {
        let Ok(name) = std::ffi::CString::new(cand.as_str()) else {
            continue;
        };
        let h = unsafe { LoadLibraryA(name.as_ptr() as *const u8) };
        if !h.is_null() {
            if let Ok(mut guard) = DL_HANDLES.lock() {
                guard.insert(lib.to_string(), h as usize);
            }
            return Some(h);
        }
    }
    None
}

#[cfg(windows)]
pub fn resolve_foreign(lib: &str, symbol: &str) -> Option<*const u8> {
    let h = dl_handle(lib)?;
    let Ok(sym) = std::ffi::CString::new(symbol) else {
        return None;
    };
    let p = unsafe { GetProcAddress(h, sym.as_ptr() as *const u8) };
    if p.is_null() {
        return None;
    }
    Some(p)
}
