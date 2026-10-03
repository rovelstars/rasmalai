
pub(crate) use std::alloc::{alloc, dealloc, Layout};
pub(crate) use std::collections::HashMap;
pub(crate) use std::io::Write;
pub(crate) use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
pub(crate) use std::sync::{Arc, Condvar, LazyLock, Mutex, RwLock};
pub(crate) use std::time::Duration;
use super::collections::*;
use super::io::*;

// Mirrors threads::spawn_failed_msg (duplicated: this file also builds
// standalone as the `runtime_native` staticlib for JIT linking).
#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn_failed_msg() -> String {
    "Threading is disabled in the web playground. Rasmalai threads require a native OS target.".to_string()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn_failed_msg() -> String {
    "Thread failed to spawn (threading is unsupported on this platform or resource limit reached)".to_string()
}

pub(crate) static TEST_FAILED: AtomicBool = AtomicBool::new(false);

pub fn test_flag_set() {
    TEST_FAILED.store(true, Ordering::SeqCst);
}

pub fn test_flag_check() -> bool {
    TEST_FAILED.swap(false, Ordering::SeqCst)
}

pub(crate) static ASSERT_STRICT: AtomicBool = AtomicBool::new(false);

#[unsafe(no_mangle)]
pub extern "C" fn rnx_set_assert_strict(strict: bool) {
    ASSERT_STRICT.store(strict, Ordering::SeqCst);
}

pub fn assert_strict() -> bool {
    ASSERT_STRICT.load(Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_assert(cond: i64, msg_bits: u64) {
    if cond != 0 {
        return;
    }
    if assert_strict() {
        if msg_bits != 0 {
            let text = native_str(msg_bits as *const u8);
            let _ = writeln!(std::io::stderr(), "{text}");
        }
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        std::process::exit(1);
    }
    if msg_bits != 0 {
        let text = native_str(msg_bits as *const u8);
        let _ = write!(std::io::stdout(), "{text}\n");
        let _ = std::io::stdout().flush();
    }
    test_flag_set();
}

#[unsafe(no_mangle)]
pub extern "C" fn rnx_test_check() -> bool {
    test_flag_check()
}

pub const IMMORTAL: u32 = 0xFFFF_FFFF;
pub const STR_HEADER: usize = 32;
pub const ARR_HEADER: usize = 40;

pub const TAG_INT: u32 = 0;
pub const TAG_BOOL: u32 = 1;
pub const TAG_FLOAT: u32 = 2;
pub const TAG_STR: u32 = 3;
pub const TAG_PTR: u32 = 4;

pub(crate) const HEADER_LEN: usize = 16;
pub(crate) const MIN_ALIGN: usize = 8;

pub(crate) fn body_layout(size: usize, align: usize) -> Option<Layout> {
    Layout::from_size_align(size.wrapping_add(HEADER_LEN), align.max(MIN_ALIGN)).ok()
}

pub(crate) fn read_header(ptr: *const u8) -> (usize, usize) {
    unsafe {
        let size = (ptr as *const u64).read_unaligned() as usize;
        let align = (ptr.add(8) as *const u64).read_unaligned() as usize;
        (size, align)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_alloc(size: usize, align: usize) -> *mut u8 {
    let layout = match body_layout(size, align) {
        Some(l) => l,
        None => return std::ptr::null_mut(),
    };
    unsafe {
        let base = alloc(layout);
        if base.is_null() {
            return base;
        }
        (base as *mut u64).write_unaligned(size as u64);
        (base.add(8) as *mut u64).write_unaligned(layout.align() as u64);
        LIVE_ALLOCS.fetch_add(1, Ordering::SeqCst);
        base.add(HEADER_LEN)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_free(ptr: *mut u8, size: usize) {
    if ptr.is_null() {
        return;
    }
    heap_untrack(ptr);
    unsafe {
        let base = ptr.sub(HEADER_LEN);
        let (stored_size, stored_align) = read_header(base);
        if stored_size != size {
            if std::env::var("RNX_FREE_DEBUG").is_ok() {
                eprintln!("RNX_FREE_DEBUG ptr={ptr:?} want={size} stored={stored_size}");
            }
            rnx_panic(
                b"rnx_free size mismatch\0".as_ptr(),
                "rnx_free size mismatch".len(),
            );
        }
        if let Ok(layout) = Layout::from_size_align(size.wrapping_add(HEADER_LEN), stored_align) {
            dealloc(base, layout);
            LIVE_ALLOCS.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

pub(crate) static LIVE_ALLOCS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_debug_live_count() -> i64 {
    LIVE_ALLOCS.load(Ordering::SeqCst)
}

pub(crate) fn ref_count(ptr: *mut u8) -> &'static AtomicU32 {
    debug_assert_eq!(ptr as usize % 8, 0);
    unsafe { &*(ptr as *const AtomicU32) }
}

pub(crate) fn retain_count(ptr: *mut u8) {
    let cell = ref_count(ptr);
    if cell.load(Ordering::SeqCst) == IMMORTAL {
        return;
    }
    cell.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn release_count(ptr: *mut u8) -> bool {
    let cell = ref_count(ptr);
    if cell.load(Ordering::SeqCst) == IMMORTAL {
        return false;
    }
    let prev = cell.fetch_sub(1, Ordering::SeqCst);
    if prev == 0 {
        unsafe {
            rnx_panic(
                b"rnx_release on zero count\0".as_ptr(),
                "rnx_release on zero count".len(),
            );
        }
    }
    prev == 1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_retain(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
retain_count(ptr);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_release(
    ptr: *mut u8,
    instance_size: usize,
    dtor: Option<unsafe extern "C" fn(*mut u8)>,
) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        if release_count(ptr) {
            if let Some(run) = dtor {
                run(ptr);
            }
            rnx_free(ptr, instance_size);
        }
    }
}

pub(crate) fn str_len(ptr: *const u8) -> usize {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (ptr.add(16) as *const u64).read_unaligned() as usize }
}

pub(crate) fn str_bytes<'a>(ptr: *const u8) -> &'a [u8] {
    if ptr.is_null() {
        return b"";
    }
    unsafe { std::slice::from_raw_parts(ptr.add(STR_HEADER), str_len(ptr)) }
}

pub fn str_body_size(len: usize) -> usize {
    STR_HEADER + len + 1
}

pub(crate) fn str_alloc(len: usize) -> *mut u8 {
    let out = unsafe { rnx_alloc(str_body_size(len), 8) };
    if out.is_null() {
        return out;
    }
    unsafe {
        (out as *mut u32).write_unaligned(1);
        ((out as *mut u8).add(4) as *mut u32).write_unaligned(0);
        ((out as *mut u8).add(8) as *mut u64).write_unaligned(0);
        ((out as *mut u8).add(16) as *mut u64).write_unaligned(len as u64);
        ((out as *mut u8).add(24) as *mut u64).write_unaligned(0);
    }
    out
}

pub(crate) fn str_ascii(ptr: *const u8) -> bool {
    if ptr.is_null() {
        return true;
    }
    unsafe {
        let flag = ptr.add(4) as *const AtomicU32;
        let v = (*flag).load(Ordering::Relaxed);
        if v == 1 {
            return true;
        }
        if v == 2 {
            return false;
        }
        let a = str_bytes(ptr).is_ascii();
        if ref_count(ptr as *mut u8).load(Ordering::Relaxed) != IMMORTAL {
            (*flag).store(if a { 1 } else { 2 }, Ordering::Relaxed);
        }
        a
    }
}

pub(crate) fn str_chars(ptr: *const u8) -> usize {
    if ptr.is_null() {
        return 0;
    }
    unsafe {
        let cell = ptr.add(24) as *const AtomicU64;
        let cached = (*cell).load(Ordering::Relaxed);
        if cached != 0 {
            return cached as usize - 1;
        }
        let count = String::from_utf8_lossy(str_bytes(ptr)).chars().count();
        (*cell).store(count as u64 + 1, Ordering::Relaxed);
        count
    }
}

pub(crate) fn libc_memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    unsafe {
        let (mut p, mut q) = (a, b);
        for _ in 0..n {
            let (x, y) = (*p, *q);
            if x != y {
                return x as i32 - y as i32;
            }
            p = p.add(1);
            q = q.add(1);
        }
        0
    }
}

pub fn fmt_float(v: f64) -> String {
    if v.fract() == 0.0 && v.is_finite() {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

macro_rules! bytes_read_int {
    ($name:ident, $be:ident, $size:expr, $signed:expr) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(buf: *mut u8, off: i64) -> i64 {
            let v = bytes_read_impl(buf, off, $size);
            $signed(v, $size) as i64
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $be(buf: *mut u8, off: i64) -> i64 {
            let v = bytes_read_be_impl(buf, off, $size);
            $signed(v, $size) as i64
        }
    };
}

pub(crate) fn sign_extend(v: u64, size: i64) -> i64 {
    let shift = 64 - 8 * size as u64;
    ((v << shift) as i64) >> shift
}

pub(crate) fn zero_extend(v: u64, _size: i64) -> i64 {
    v as i64
}

macro_rules! bytes_write_int {
    ($le:ident, $be:ident, $size:expr) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $le(buf: *mut u8, off: i64, val: i64) {
            bytes_write_impl(buf, off, $size, val as u64)
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $be(buf: *mut u8, off: i64, val: i64) {
            bytes_write_be_impl(buf, off, $size, val as u64)
        }
    };
}

pub fn native_str(ptr: *const u8) -> String {
    String::from_utf8_lossy(str_bytes(ptr)).into_owned()
}

pub fn alloc_str(text: &str) -> *mut u8 {
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

// Closure box layout (refcounted like every other heap object):
// [count: u32][pad: u32][fid: u64][ncaps: u64][tag: u64][val: u64] * ncaps[box_id: u64].
// `box_id` is a process-unique monotonic id (never a heap address) stored
// as a trailer AFTER the captures, so capture offsets stay fixed for
// backends. A freed box whose memory is recycled by the allocator can
// never collide with a live registry entry: ids are never reused.
// Tags reuse TAG_* plus TAG_CLOSURE for nested closures; TAG_OBJ and
// TAG_ARRAY carry heap objects and arrays. Backends retain heap captures
// at creation; dropping the box runs the destructor, which releases
// string, closure, object, and array captures. Object and array captures
// need per-capture release metadata (instance size, class dtor, element
// size/dtor) that does not fit the 16-byte pair, so backends record it in
// CLOSURE_DESCS keyed by (box_id, index) when storing the pair.

pub const TAG_CLOSURE: u32 = 5;
pub const TAG_NULL: u32 = 6;
pub const TAG_OBJ: u32 = 7;
pub const TAG_ARRAY: u32 = 8;
