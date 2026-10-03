use std::cell::Cell;

#[cfg(unix)]
use super::{query_stack_floor, STACK_EXHAUSTED_MSG};

#[cfg(unix)]
const ALT_STACK_SIZE: usize = 32 * 1024;

#[cfg(unix)]
const GUARD_ZONE_BYTES: usize = 64 * 1024;

#[cfg(unix)]
const ALT_ZERO: Cell<u8> = Cell::new(0);

#[cfg(unix)]
thread_local! {
    static ALT_STACK: [Cell<u8>; ALT_STACK_SIZE] = [ALT_ZERO; ALT_STACK_SIZE];
    static BOUNDS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
    static THREAD_READY: Cell<bool> = const { Cell::new(false) };
}

#[cfg(unix)]
static INSTALLED: std::sync::Once = std::sync::Once::new();

pub fn guard_install() {
    #[cfg(unix)]
    INSTALLED.call_once(install_handler);
    guard_thread_init();
}

pub fn guard_thread_init() {
    #[cfg(unix)]
    {
        if THREAD_READY.get() {
            return;
        }
        THREAD_READY.set(true);
        install_altstack();
        cache_bounds();
    }
}

#[cfg(unix)]
fn install_altstack() {
    ALT_STACK.with(|stack| {
        let mut ss: libc::stack_t = unsafe { std::mem::zeroed() };
        ss.ss_sp = stack.as_ptr() as *mut libc::c_void;
        ss.ss_size = ALT_STACK_SIZE;
        unsafe {
            libc::sigaltstack(&ss, std::ptr::null_mut());
        }
    });
}

#[cfg(unix)]
fn cache_bounds() {
    let (low, size) = query_stack_floor();
    if size != 0 {
        BOUNDS.set((low, low + size));
    }
}

#[cfg(unix)]
fn install_handler() {
    unsafe {
        let mut act: libc::sigaction = std::mem::zeroed();
        act.sa_sigaction = guard_handler as *const () as usize;
        libc::sigemptyset(&mut act.sa_mask);
        act.sa_flags = libc::SA_ONSTACK | libc::SA_SIGINFO;
        libc::sigaction(libc::SIGSEGV, &act, std::ptr::null_mut());
        libc::sigaction(libc::SIGBUS, &act, std::ptr::null_mut());
    }
}

#[cfg(unix)]
unsafe extern "C" fn guard_handler(sig: libc::c_int, info: *mut libc::siginfo_t, uc: *mut libc::c_void) {
    let fault = unsafe { (*info).si_addr() } as usize;
    let (low, high) = BOUNDS.get();
    if low != 0
        && fault >= low.saturating_sub(GUARD_ZONE_BYTES)
        && fault < low
        && fault_rsp_in_bounds(uc as *mut libc::ucontext_t, low, high)
    {
        write_all(libc::STDERR_FILENO, STACK_EXHAUSTED_MSG.as_bytes());
        write_all(libc::STDERR_FILENO, b"\n");
        unsafe { libc::_exit(1) };
    }
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

#[cfg(all(unix, target_arch = "x86_64"))]
fn fault_rsp_in_bounds(uc: *mut libc::ucontext_t, low: usize, high: usize) -> bool {
    if uc.is_null() {
        return true;
    }
    let rsp = unsafe { (*uc).uc_mcontext.gregs[libc::REG_RSP as usize] } as usize;
    rsp >= low.saturating_sub(GUARD_ZONE_BYTES) && rsp <= high
}

#[cfg(all(unix, not(target_arch = "x86_64")))]
fn fault_rsp_in_bounds(_uc: *mut libc::ucontext_t, _low: usize, _high: usize) -> bool {
    true
}

#[cfg(unix)]
fn write_all(fd: libc::c_int, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let n = unsafe { libc::write(fd, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
        if n <= 0 {
            return;
        }
        bytes = &bytes[n as usize..];
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_guard_install() {
    guard_install();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_guard_thread_init() {
    guard_thread_init();
}
