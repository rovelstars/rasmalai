use fx::{
    decode_args, decode_call_body, decode_frame, decode_id_body, encode_frame, encode_ret_value,
    parse_fn_spec, ArgType, ArgValue, FnSpec, HandleTable, RetType, FX_FOREIGN, FX_FRAME_CAP,
    FX_MAX_ARGS, FX_MSG_CALL, FX_MSG_RELEASE, FX_MSG_RETAIN, FX_OK, FX_TRANSPORT, FX_T_BAD_INDEX,
    FX_T_MALFORMED, FX_T_STALE,
};
use std::ffi::CString;

include!(concat!(env!("OUT_DIR"), "/calls.rs"));

const EXIT_STARTUP: i32 = 3;

fn last_errno() -> u32 {
    #[cfg(target_os = "linux")]
    unsafe {
        unsafe extern "C" {
            fn __errno_location() -> *mut i32;
        }
        (*__errno_location()).max(0) as u32
    }
    #[cfg(target_os = "macos")]
    unsafe {
        unsafe extern "C" {
            fn __error() -> *mut i32;
        }
        (*__error()).max(0) as u32
    }
    #[cfg(windows)]
    unsafe {
        unsafe extern "C" {
            fn GetLastError() -> u32;
        }
        GetLastError()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    0
}

struct Config {
    lib: String,
    connect: String,
    specs: Vec<FnSpec>,
    allow: Vec<String>,
}

fn parse_args() -> Result<Config, String> {
    let mut lib = None;
    let mut connect = None;
    let mut specs = Vec::new();
    let mut allow = Vec::new();
    let mut it = std::env::args().skip(1).peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--lib" => lib = Some(it.next().ok_or("missing value for `--lib`")?),
            "--connect" => connect = Some(it.next().ok_or("missing value for `--connect`")?),
            "--fn" => {
                let text = it.next().ok_or("missing value for `--fn`")?;
                specs.push(parse_fn_spec(&text).map_err(|e| format!("E208: {e}"))?);
            }
            "--allow" => allow.push(it.next().ok_or("missing value for `--allow`")?),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(Config {
        lib: lib.ok_or("missing `--lib <path>`")?,
        connect: connect.ok_or("missing `--connect <path>`")?,
        specs,
        allow,
    })
}

#[cfg(target_os = "linux")]
mod linux_sandbox {
    use std::ffi::CString;

    const RULE_PATH_BENEATH: u64 = 1;
    const VERSION_FLAG: u64 = 1;

    #[repr(C)]
    struct RulesetAttr {
        handled_fs: u64,
        handled_net: u64,
        scoped: u64,
    }

    #[repr(C, packed)]
    struct PathBeneathAttr {
        allowed: u64,
        parent_fd: i32,
    }

    const READ_FILE: u64 = 1 << 2;
    const READ_DIR: u64 = 1 << 3;
    const EXECUTE: u64 = 1 << 0;

    fn landlock_abi() -> i64 {
        let abi =
            unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, 0usize, 0u32, VERSION_FLAG) };
        if abi < 0 {
            return 0;
        }
        abi as i64
    }

    fn try_ruleset(fs: u64, net: u64, size: usize) -> i64 {
        let attr = RulesetAttr { handled_fs: fs, handled_net: net, scoped: 0 };
        let fd = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr as *const RulesetAttr,
                size,
                0u32,
            )
        };
        if fd < 0 {
            return -1;
        }
        fd as i64
    }

    fn open_ruleset(abi: i64) -> Result<i64, String> {
        let tcp: u64 = (1 << 0) | (1 << 1);
        let udp: u64 = (1 << 2) | (1 << 3);
        let mut cands: Vec<(u64, u64, usize)> = Vec::new();
        if abi >= 10 {
            cands.push((0x1ffff, tcp | udp, 24));
        }
        if abi >= 9 {
            cands.push((0x1ffff, tcp, 24));
        }
        if abi >= 5 {
            cands.push((0xffff, tcp, 24));
        }
        if abi >= 4 {
            cands.push((0x7fff, tcp, 24));
        }
        if abi >= 1 {
            cands.push((0x3ff, 0, 8));
        }
        for (fs, net, size) in cands {
            let fd = try_ruleset(fs, net, size);
            if fd >= 0 {
                return Ok(fd);
            }
        }
        Err("E209: kernel Landlock probe failed".to_string())
    }

    fn allow_path(ruleset: i64, path: &str, access: u64) -> Result<(), String> {
        let raw = CString::new(path).map_err(|_| format!("bad allow path `{path}`"))?;
        let fd = unsafe { libc::open(raw.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(format!("cannot open allow path `{path}`"));
        }
        let attr = PathBeneathAttr { allowed: access, parent_fd: fd };
        let rule = unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset,
                RULE_PATH_BENEATH,
                &attr as *const PathBeneathAttr,
                0u32,
            )
        };
        unsafe {
            libc::close(fd);
        }
        if rule != 0 {
            return Err(format!("cannot allow path `{path}`"));
        }
        Ok(())
    }

    fn allow_optional(ruleset: i64, path: &str, access: u64) {
        let Ok(raw) = CString::new(path) else { return };
        let fd = unsafe { libc::open(raw.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if fd < 0 {
            return;
        }
        let attr = PathBeneathAttr { allowed: access, parent_fd: fd };
        unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset,
                RULE_PATH_BENEATH,
                &attr as *const PathBeneathAttr,
                0u32,
            );
            libc::close(fd);
        }
    }

    pub fn apply(lib: &str, allow: &[String]) -> Result<(), String> {
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1u64, 0u64, 0u64, 0u64) } != 0 {
            return Err("E209: cannot set no_new_privs".to_string());
        }
        let abi = landlock_abi();
        if abi <= 0 {
            return Err("E209: kernel has no Landlock; refusing to run unsandboxed".to_string());
        }
        let ruleset = open_ruleset(abi)?;
        let access = READ_FILE | READ_DIR | EXECUTE;
        let lib_dir = std::path::Path::new(lib)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| ".".to_string());
        allow_path(ruleset, &lib_dir, access)?;
        for dir in allow {
            allow_path(ruleset, dir, access)?;
        }
        for dir in ["/usr/lib", "/usr/lib64", "/lib", "/lib64"] {
            allow_optional(ruleset, dir, access);
        }
        allow_optional(ruleset, "/etc/ld.so.cache", READ_FILE);
        let restricted = unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset, 0u32) };
        unsafe {
            libc::close(ruleset as i32);
        }
        if restricted != 0 {
            return Err("E209: Landlock restrict_self failed".to_string());
        }
        Ok(())
    }

    #[repr(C)]
    struct SockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    #[repr(C)]
    struct SockFprog {
        len: u16,
        filter: *const SockFilter,
    }

    pub fn apply_seccomp() -> Result<(), String> {
        #[cfg(target_arch = "x86_64")]
        const ARCH: u32 = 0xC000003E;
        #[cfg(target_arch = "aarch64")]
        const ARCH: u32 = 0xC00000B7;
        #[cfg(target_arch = "x86_64")]
        const NUMBERS: [u32; 3] = [42, 59, 322];
        #[cfg(target_arch = "aarch64")]
        const NUMBERS: [u32; 3] = [203, 221, 281];
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        const ARCH: u32 = 0;
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        const NUMBERS: [u32; 3] = [0, 0, 0];
        if ARCH == 0 {
            return Ok(());
        }
        let allow = SockFilter { code: 0x06, jt: 0, jf: 0, k: 0x7fff0000 };
        let deny = SockFilter { code: 0x06, jt: 0, jf: 0, k: 0x00050001 };
        let prog = [
            SockFilter { code: 0x20, jt: 0, jf: 0, k: 4 },
            SockFilter { code: 0x15, jt: 0, jf: 5, k: ARCH },
            SockFilter { code: 0x20, jt: 0, jf: 0, k: 0 },
            SockFilter { code: 0x15, jt: 3, jf: 0, k: NUMBERS[0] },
            SockFilter { code: 0x15, jt: 2, jf: 0, k: NUMBERS[1] },
            SockFilter { code: 0x15, jt: 1, jf: 0, k: NUMBERS[2] },
            allow,
            deny,
        ];
        let fprog = SockFprog { len: prog.len() as u16, filter: prog.as_ptr() };
        let applied = unsafe {
            libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &fprog as *const SockFprog)
        };
        if applied != 0 {
            return Err("E209: seccomp filter install failed".to_string());
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn macos_reexec() -> Result<(), String> {
    if std::env::var("RNX_FX_SANDBOXED").is_ok() {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| format!("cannot find helper binary: {e}"))?;
    let mut profile = String::from("(version 1)(deny default)");
    profile.push_str("(allow file-read* (subpath \"/usr/lib\")(subpath \"/System\")(subpath \"/private/tmp\"))");
    profile.push_str("(allow file-read* file-write* (subpath \"/private/tmp\"))");
    profile.push_str("(allow sysctl-read)(allow mach-lookup)");
    let status = std::process::Command::new("sandbox-exec")
        .arg("-p")
        .arg(&profile)
        .arg(&exe)
        .args(std::env::args().skip(1))
        .env("RNX_FX_SANDBOXED", "1")
        .status()
        .map_err(|e| format!("cannot start sandbox-exec: {e}"))?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(windows)]
fn windows_contained() -> Result<(), String> {
    unsafe {
        unsafe extern "C" {
            fn GetCurrentProcess() -> *mut core::ffi::c_void;
            fn OpenProcessToken(
                process: *mut core::ffi::c_void,
                access: u32,
                token: *mut *mut core::ffi::c_void,
            ) -> i32;
            fn GetTokenInformation(
                token: *mut core::ffi::c_void,
                class: u32,
                info: *mut core::ffi::c_void,
                len: u32,
                needed: *mut u32,
            ) -> i32;
            fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
        }
        let mut token: *mut core::ffi::c_void = core::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), 8, &mut token) == 0 {
            return Err("E209: cannot query process token".to_string());
        }
        let mut contained: u32 = 0;
        let mut needed: u32 = 0;
        let ok = GetTokenInformation(
            token,
            29,
            &mut contained as *mut u32 as *mut core::ffi::c_void,
            4,
            &mut needed,
        );
        CloseHandle(token);
        if ok == 0 || contained == 0 {
            return Err(
                "E209: helper runs outside an AppContainer; spawn it through the host container profile"
                    .to_string(),
            );
        }
        Ok(())
    }
}

trait IpcStream {
    fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()>;
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()>;
}

#[cfg(unix)]
struct UnixIpc(std::os::unix::net::UnixStream);

#[cfg(unix)]
impl IpcStream for UnixIpc {
    fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()> {
        use std::io::Read;
        self.0.read_exact(buf)
    }
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        self.0.write_all(buf)?;
        self.0.flush()
    }
}

#[cfg(windows)]
struct PipeIpc {
    handle: *mut core::ffi::c_void,
}

#[cfg(windows)]
impl IpcStream for PipeIpc {
    fn read_exact(&mut self, mut buf: &mut [u8]) -> std::io::Result<()> {
        unsafe {
            unsafe extern "C" {
                fn ReadFile(
                    handle: *mut core::ffi::c_void,
                    out: *mut u8,
                    len: u32,
                    done: *mut u32,
                    over: *mut core::ffi::c_void,
                ) -> i32;
            }
            while !buf.is_empty() {
                let mut done: u32 = 0;
                let chunk = buf.len().min(u32::MAX as usize) as u32;
                let ok = ReadFile(
                    self.handle,
                    buf.as_mut_ptr(),
                    chunk,
                    &mut done,
                    core::ptr::null_mut(),
                );
                if ok == 0 || done == 0 {
                    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "pipe closed"));
                }
                buf = &mut buf[done as usize..];
            }
            Ok(())
        }
    }
    fn write_all(&mut self, mut buf: &[u8]) -> std::io::Result<()> {
        unsafe {
            unsafe extern "C" {
                fn WriteFile(
                    handle: *mut core::ffi::c_void,
                    data: *const u8,
                    len: u32,
                    done: *mut u32,
                    over: *mut core::ffi::c_void,
                ) -> i32;
            }
            while !buf.is_empty() {
                let mut done: u32 = 0;
                let chunk = buf.len().min(u32::MAX as usize) as u32;
                let ok = WriteFile(
                    self.handle,
                    buf.as_ptr(),
                    chunk,
                    &mut done,
                    core::ptr::null_mut(),
                );
                if ok == 0 {
                    return Err(std::io::Error::other("pipe write failed"));
                }
                buf = &buf[done as usize..];
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
fn open_pipe(name: &str) -> Result<PipeIpc, String> {
    unsafe {
        unsafe extern "C" {
            fn CreateFileW(
                name: *const u16,
                access: u32,
                share: u32,
                attrs: *mut core::ffi::c_void,
                create: u32,
                flags: u32,
                template: *mut core::ffi::c_void,
            ) -> *mut core::ffi::c_void;
        }
        let wide: Vec<u16> = format!("\\\\.\\pipe\\{name}").encode_utf16().chain([0]).collect();
        let handle = CreateFileW(
            wide.as_ptr(),
            0xC0000000,
            0,
            core::ptr::null_mut(),
            3,
            0,
            core::ptr::null_mut(),
        );
        if handle as isize == -1 {
            return Err(format!("cannot open pipe `{name}`"));
        }
        Ok(PipeIpc { handle })
    }
}

fn reply(stream: &mut impl IpcStream, status: u8, id: u64, payload: &[u8]) -> Result<(), String> {
    let frame =
        encode_frame(status, id, payload, "").map_err(|e| format!("cannot frame reply: {e}"))?;
    stream.write_all(&frame).map_err(|e| format!("cannot write reply: {e}"))
}

fn reply_transport(stream: &mut impl IpcStream, id: u64, code: u32) -> Result<(), String> {
    reply(stream, FX_TRANSPORT, id, &code.to_le_bytes())
}

fn serve(mut stream: impl IpcStream, config: &Config) -> Result<(), String> {
    #[cfg(unix)]
    let lib_handle = unsafe {
        let raw = CString::new(config.lib.clone()).map_err(|_| "bad --lib path".to_string())?;
        let h = libc::dlopen(raw.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if h.is_null() {
            let err = libc::dlerror();
            let why = if err.is_null() {
                "unknown dlopen error".to_string()
            } else {
                std::ffi::CStr::from_ptr(err).to_string_lossy().into_owned()
            };
            return Err(format!("E209: cannot open `{}`: {why}", config.lib));
        }
        h
    };
    #[cfg(windows)]
    let lib_handle = unsafe {
        unsafe extern "C" {
            fn LoadLibraryA(name: *const u8) -> *mut core::ffi::c_void;
        }
        let mut raw = config.lib.clone().into_bytes();
        raw.push(0);
        let h = LoadLibraryA(raw.as_ptr());
        if h.is_null() {
            return Err(format!("E209: cannot open `{}`", config.lib));
        }
        h
    };
    let mut symbols: Vec<usize> = Vec::with_capacity(config.specs.len());
    for spec in &config.specs {
        let raw = CString::new(spec.name.clone()).map_err(|_| "bad symbol name".to_string())?;
        #[cfg(unix)]
        let addr = unsafe { libc::dlsym(lib_handle, raw.as_ptr()) } as usize;
        #[cfg(windows)]
        let addr = unsafe {
            unsafe extern "C" {
                fn GetProcAddress(
                    lib: *mut core::ffi::c_void,
                    name: *const u8,
                ) -> *mut core::ffi::c_void;
            }
            let mut name = spec.name.clone().into_bytes();
            name.push(0);
            GetProcAddress(lib_handle, name.as_ptr()) as usize
        };
        if addr == 0 {
            return Err(format!("E209: symbol `{}` not found in `{}`", spec.name, config.lib));
        }
        symbols.push(addr);
    }
    let mut table = HandleTable::new();
    let mut len = [0u8; 4];
    loop {
        match stream.read_exact(&mut len) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(format!("E209: helper IPC read failed: {e}")),
        }
        let total = u32::from_le_bytes(len) as usize;
        if total < 9 || total > FX_FRAME_CAP {
            let _ = total;
            return Err("E209: helper IPC frame has a bad length".to_string());
        }
        let mut body = vec![0u8; total];
        if stream.read_exact(&mut body).is_err() {
            return Err("E209: helper IPC frame truncated".to_string());
        }
        let (kind, id, payload) = match decode_frame(&body) {
            Ok(v) => v,
            Err(_) => {
                if body.len() >= 9 {
                    let mut raw = [0u8; 8];
                    raw.copy_from_slice(&body[1..9]);
                    let _ = reply_transport(&mut stream, u64::from_le_bytes(raw), FX_T_MALFORMED);
                    continue;
                }
                return Err("E209: helper IPC frame malformed".to_string());
            }
        };
        match kind {
            FX_MSG_CALL => {
                let (index, args_bytes) = match decode_call_body(payload) {
                    Ok(v) => v,
                    Err(_) => {
                        reply_transport(&mut stream, id, FX_T_MALFORMED)?;
                        continue;
                    }
                };
                let Some(spec) = config.specs.get(index as usize) else {
                    reply_transport(&mut stream, id, FX_T_BAD_INDEX)?;
                    continue;
                };
                if spec.params.len() > FX_MAX_ARGS {
                    reply_transport(&mut stream, id, FX_T_MALFORMED)?;
                    continue;
                }
                let args = match decode_args(spec, args_bytes, id) {
                    Ok(v) => v,
                    Err(_) => {
                        reply_transport(&mut stream, id, FX_T_MALFORMED)?;
                        continue;
                    }
                };
                let mut ints: Vec<u64> = Vec::with_capacity(args.len());
                let mut floats: Vec<f64> = Vec::new();
                let mut bufs: Vec<Vec<u8>> = Vec::new();
                let mut failure: Option<u32> = None;
                let mut pat: u64 = 0;
                for (j, (ty, value)) in spec.params.iter().zip(args.iter()).enumerate() {
                    match (ty, value) {
                        (ArgType::Int, ArgValue::Int(n)) => ints.push(*n as u64),
                        (ArgType::Float, ArgValue::Float(f)) => {
                            pat |= 1 << j;
                            floats.push(*f);
                        }
                        (ArgType::Byte, ArgValue::Byte(b)) => ints.push(u64::from(*b)),
                        (ArgType::Bool, ArgValue::Bool(b)) => ints.push(u64::from(*b)),
                        (ArgType::Buf, ArgValue::Buf(bytes)) => {
                            bufs.push(bytes.clone());
                            ints.push(bufs.last().unwrap().as_ptr() as u64);
                        }
                        (ArgType::Handle, ArgValue::Handle(h)) => match table.resolve(*h) {
                            Some(addr) => ints.push(addr as u64),
                            None => {
                                failure = Some(FX_T_STALE);
                                break;
                            }
                        },
                        _ => {
                            failure = Some(FX_T_MALFORMED);
                            break;
                        }
                    }
                }
                if let Some(code) = failure {
                    reply_transport(&mut stream, id, code)?;
                    continue;
                }
                let ret_float = matches!(spec.ret, RetType::Float);
                let (raw_int, raw_float) =
                    unsafe { raw_call(symbols[index as usize], ret_float, args.len(), pat, &ints, &floats) };
                let errno = last_errno();
                let status_body: Vec<u8> = match spec.ret {
                    RetType::Void => encode_ret_value(spec.ret, 0, 0.0),
                    RetType::Int => {
                        let wide = raw_int as i32 as i64;
                        if wide == -1 {
                            reply(&mut stream, FX_FOREIGN, id, &errno.to_le_bytes())?;
                            continue;
                        }
                        encode_ret_value(spec.ret, wide, 0.0)
                    }
                    RetType::Long | RetType::Handle => {
                        if raw_int == u64::MAX {
                            reply(&mut stream, FX_FOREIGN, id, &errno.to_le_bytes())?;
                            continue;
                        }
                        let value = if matches!(spec.ret, RetType::Handle) {
                            table.intern(raw_int as usize) as i64
                        } else {
                            raw_int as i64
                        };
                        encode_ret_value(spec.ret, value, 0.0)
                    }
                    RetType::Float => encode_ret_value(spec.ret, 0, raw_float),
                    RetType::Byte | RetType::Bool => {
                        encode_ret_value(spec.ret, raw_int as i64, 0.0)
                    }
                };
                reply(&mut stream, FX_OK, id, &status_body)?;
            }
            FX_MSG_RETAIN => {
                let Ok(handle) = decode_id_body(payload) else {
                    reply_transport(&mut stream, id, FX_T_MALFORMED)?;
                    continue;
                };
                if table.retain(handle) {
                    reply(&mut stream, FX_OK, id, &[])?;
                } else {
                    reply_transport(&mut stream, id, FX_T_STALE)?;
                }
            }
            FX_MSG_RELEASE => {
                let Ok(handle) = decode_id_body(payload) else {
                    reply_transport(&mut stream, id, FX_T_MALFORMED)?;
                    continue;
                };
                if table.release(handle) {
                    reply(&mut stream, FX_OK, id, &[])?;
                } else {
                    reply_transport(&mut stream, id, FX_T_STALE)?;
                }
            }
            _ => {
                reply_transport(&mut stream, id, FX_T_MALFORMED)?;
            }
        }
    }
}

fn main() {
    let code = match run() {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("rnx-fx-host: {message}");
            EXIT_STARTUP
        }
    };
    std::process::exit(code);
}

fn run() -> Result<(), String> {
    let config = parse_args().map_err(|e| format!("E209: {e}"))?;
    if config.specs.is_empty() {
        return Err("E209: no `--fn` table entries".to_string());
    }
    #[cfg(target_os = "macos")]
    macos_reexec()?;
    #[cfg(windows)]
    windows_contained()?;
    #[cfg(unix)]
    let stream = {
        let sock = std::os::unix::net::UnixStream::connect(&config.connect)
            .map_err(|e| format!("E209: cannot connect helper IPC: {e}"))?;
        UnixIpc(sock)
    };
    #[cfg(windows)]
    let stream = open_pipe(&config.connect)?;
    #[cfg(target_os = "linux")]
    {
        linux_sandbox::apply(&config.lib, &config.allow)?;
        linux_sandbox::apply_seccomp()?;
    }
    serve(stream, &config)
}
