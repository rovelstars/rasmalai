//! Runtime archive for `wasm32-unknown-wasi`.
//!
//! The compiler emits the user program plus a `main` wrapper and the
//! `rnx_*` helper calls listed in this file. This crate supplies the WASI
//! entry point (`_start`), the linear-memory heap those helpers are built
//! on, and the console writer. It is a `no_std` staticlib built for
//! `wasm32-unknown-unknown` with the preview 1 imports declared by hand, so
//! the archive carries no preview 2 component imports.
//!
//! Implemented surface: allocation and reference counting, strings, arrays,
//! `Any` boxing, integer and float rendering, `println!` output through
//! `fd_write`, and process exit through `proc_exit`. Collection and process
//! features the module cannot have fail through `rnx_panic_str` instead of
//! pretending to work.

#![no_std]
// The mem* symbols are this crate's own definitions for what LLVM lowers
// struct copies to; the signatures use `u8` where libc uses `c_void`.
#![allow(suspicious_runtime_symbol_definitions)]

mod heap;
mod wasi;

use core::panic::PanicInfo;
use heap::*;

static mut FATAL: u32 = 0;
static mut ASSERT_STRICT: bool = false;

pub fn fail(message: &str) -> ! {
    heap::fail(message)
}

unsafe extern "C" {
    fn main(argc: i32, argv: *const *const u8) -> i32;
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let code = unsafe { main(0, core::ptr::null()) };
    wasi::exit(code)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    wasi::write_error_str("rnx: panic\n");
    wasi::exit(1)
}

fn emit(text: &str, err: bool) {
    if err {
        wasi::write_error_str(text);
    } else {
        wasi::write_str(text);
    }
}

fn emit_bytes(bytes: &[u8], err: bool) {
    if err {
        wasi::write_error_bytes(bytes);
    } else {
        wasi::write_bytes(bytes);
    }
}

fn write_value(value: u64, err: bool) {
    if let Some(addr) = any_box_addr(value) {
        let payload = box_payload(addr);
        match (value & ANY_BOX_MASK) as u32 {
            0b001 => {
                let mut out = [0u8; 32];
                emit_bytes(wasi::format_i64(payload as i64, &mut out), err);
            }
            0b010 => emit(if payload == 0 { "false" } else { "true" }, err),
            0b011 => {
                let mut out = [0u8; 32];
                emit_bytes(wasi::format_f64(f64::from_bits(payload), &mut out), err);
            }
            _ => write_str_value(payload as *const u8, err),
        }
        return;
    }
    if value == 0 {
        emit("null", err);
        return;
    }
    match heap_kind(value) {
        Some(HEAP_STR) => write_str_value(value as *const u8, err),
        Some(HEAP_ARRAY) => emit("<array>", err),
        Some(HEAP_OBJ) | Some(HEAP_ENUM) => emit("<object>", err),
        _ => emit("<value>", err),
    }
}

fn write_str_value(ptr: *const u8, err: bool) {
    if ptr.is_null() {
        emit("null", err);
        return;
    }
    emit(core::str::from_utf8(str_bytes(ptr)).unwrap_or("?"), err);
}

fn print_args(to_stderr: bool, args: *const u8) {
    let len = arr_len(args);
    let data = arr_data(args);
    for i in 0..len {
        if i > 0 {
            emit(" ", to_stderr);
        }
        let value = unsafe { (data as *const u64).add(i).read_unaligned() };
        write_value(value, to_stderr);
    }
    emit("\n", to_stderr);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_wasi_println(args: *const u8) {
    print_args(false, args);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_wasi_eprintln(args: *const u8) {
    print_args(true, args);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_wasi_write(text: *const u8) {
    write_str_value(text, false);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_wasi_ewrite(text: *const u8) {
    write_str_value(text, true);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_alloc(size: u64, align: u64) -> *mut u8 {
    unsafe { alloc_bytes(size as usize, align as usize) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_free(ptr: *mut u8, size: u64) {
    unsafe { free_bytes(ptr, size as usize) }
}

#[unsafe(no_mangle)]
pub extern "C" fn rnx_init(_argc: i32, _argv: *const *const u8) {}

#[unsafe(no_mangle)]
pub extern "C" fn rnx_set_assert_strict(strict: bool) {
    unsafe {
        ASSERT_STRICT = strict;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_assert(cond: i64, message: u64) {
    if cond != 0 {
        return;
    }
    wasi::write_error_str("rnx: assertion failed: ");
    write_str_value(message as *const u8, true);
    wasi::write_error_str("\n");
    let strict = unsafe { ASSERT_STRICT };
    if strict {
        wasi::exit(1);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_panic_str(message: *const u8) {
    wasi::write_error_str("rnx: ");
    write_str_value(message, true);
    wasi::write_error_str("\n");
    wasi::exit(1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fatal_span(message: *const u8, _start: u64, _end: u64) {
    wasi::write_error_str("fatal: ");
    write_str_value(message, true);
    wasi::write_error_str("\n");
    unsafe {
        FATAL = 1;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rnx_report_fatal() -> i32 {
    unsafe { FATAL as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_error_take() -> u64 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_report_uncaught(error: u64) -> i32 {
    if error == 0 {
        return 0;
    }
    wasi::write_error_str("rnx: uncaught error\n");
    1
}

// The compiler declares these helpers with i64 parameters for every size
// and index, because LIR values are 64-bit on all targets. On wasm32 a
// `usize` parameter would be i32 and every call site would fail type
// validation, so each exported helper takes u64 and narrows once here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_retain(ptr: *mut u8) {
    retain(ptr);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_retain(any: u64) {
    if let Some(addr) = any_box_addr(any) {
        unsafe {
            let cell = box_count(addr);
            cell.write_unaligned(cell.read_unaligned() + 1);
        }
        return;
    }
    if heap_kind(any).is_some() {
        retain(any as *mut u8);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_release(any: u64) {
    if let Some(addr) = any_box_addr(any) {
        let cell = box_count(addr);
        let count = unsafe { cell.read_unaligned() };
        if count <= 1 {
            if any & ANY_BOX_MASK == ANY_BOX_STR {
                let payload = box_payload(addr);
                if payload != 0 && payload % 8 == 0 {
                    release_str(payload as *mut u8);
                }
            }
            boxes_forget(addr);
            unsafe { free_bytes(addr as *mut u8, ANY_BOX_SIZE) };
        } else {
            unsafe { cell.write_unaligned(count - 1) };
        }
        return;
    }
    if let Some(kind) = heap_kind(any) {
        let ptr = any as *mut u8;
        match kind {
            HEAP_ARRAY => release_array(ptr, 8, None),
            HEAP_STR => release_str(ptr),
            _ => {
                if release(ptr) {
                    unsafe { rnx_free(ptr, 0) };
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_release_ptr(any: u32) {
    unsafe { rnx_any_release(any as u64) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_box(tag: u64, payload: u64) -> u64 {
    if any_box_addr(payload).is_some() {
        unsafe { rnx_any_retain(payload) };
        return payload;
    }
    let marker = match tag as u32 {
        TAG_INT => ANY_BOX_INT,
        TAG_BOOL => ANY_BOX_BOOL,
        TAG_FLOAT => ANY_BOX_FLOAT,
        TAG_STR => ANY_BOX_STR,
        _ => 0,
    };
    if marker == 0 {
        return payload;
    }
    let out = unsafe { alloc_bytes(ANY_BOX_SIZE, 8) };
    if out.is_null() {
        return payload;
    }
    if marker == ANY_BOX_STR && payload != 0 && payload % 8 == 0 {
        retain(payload as *mut u8);
    }
    unsafe {
        (out as *mut u64).write_unaligned(1);
        (out.add(8) as *mut u64).write_unaligned(payload);
    }
    boxes_insert(out as usize);
    out as u64 | marker
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_unbox(any: u64) -> u64 {
    match any_box_addr(any) {
        Some(addr) => box_payload(addr),
        None => any,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_unbox_heap(any: u64) -> u64 {
    match any_box_addr(any) {
        Some(addr) if any & ANY_BOX_MASK == ANY_BOX_STR => box_payload(addr),
        _ => any,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_release_box(any: u64) {
    unsafe { rnx_any_release(any) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_heap_track(ptr: u64, kind: u64, aux1: u64, aux2: u64) {
    note_heap(ptr as *mut u8, kind, aux1, aux2);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_note_array_kind(ptr: u64, kind: u64, aux: u64) {
    note_array_kind(ptr as *mut u8, kind, aux);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_new(initial_cap: u64, elem_size: u64) -> *mut u8 {
    array_new(initial_cap as usize, elem_size as usize)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_len(arr: *const u8) -> u64 {
    arr_len(arr) as u64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_push(arr: *mut u8, value: u64, elem_size: u64) {
    array_push(arr, value, elem_size as usize)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_get(arr: *const u8, index: u64, elem_size: u64) -> u64 {
    array_get(arr, index as usize, elem_size as usize)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_release_array(
    arr: *mut u8,
    elem_size: u64,
    dtor: Option<unsafe extern "C" fn(*mut u8)>,
) {
    release_array(arr, elem_size as usize, dtor)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_release_str(ptr: *mut u8) {
    release_str(ptr)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_string_concat(a: *const u8, b: *const u8) -> *mut u8 {
    let (left, right) = (str_bytes(a), str_bytes(b));
    let total = left.len().wrapping_add(right.len());
    let out = str_alloc(total);
    if out.is_null() {
        return out;
    }
    unsafe {
        let dst = out.add(STR_HEADER);
        if !left.is_empty() {
            core::ptr::copy_nonoverlapping(left.as_ptr(), dst, left.len());
        }
        if !right.is_empty() {
            core::ptr::copy_nonoverlapping(right.as_ptr(), dst.add(left.len()), right.len());
        }
        dst.add(total).write(0);
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_genref_invalidate(_ptr: u64) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, count: usize) -> *mut u8 {
    let mut i = 0;
    while i < count {
        unsafe {
            let byte = core::ptr::read_volatile(src.add(i));
            core::ptr::write_volatile(dest.add(i), byte);
        }
        i += 1;
    }
    dest
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, count: usize) -> *mut u8 {
    if (dest as usize) <= (src as usize) || (dest as usize) >= (src as usize) + count {
        return unsafe { memcpy(dest, src, count) };
    }
    let mut i = count;
    while i > 0 {
        i -= 1;
        unsafe {
            let byte = core::ptr::read_volatile(src.add(i));
            core::ptr::write_volatile(dest.add(i), byte);
        }
    }
    dest
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dest: *mut u8, value: i32, count: usize) -> *mut u8 {
    let byte = value as u8;
    let mut i = 0;
    while i < count {
        unsafe { core::ptr::write_volatile(dest.add(i), byte) };
        i += 1;
    }
    dest
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, count: usize) -> i32 {
    let mut i = 0;
    while i < count {
        let a = unsafe { left.add(i).read() };
        let b = unsafe { right.add(i).read() };
        if a != b {
            return a as i32 - b as i32;
        }
        i += 1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcmp(left: *const u8, right: *const u8, count: usize) -> i32 {
    unsafe { memcmp(left, right, count) }
}

