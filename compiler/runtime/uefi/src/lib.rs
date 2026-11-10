//! Runtime archive for `x86_64-unknown-uefi`.
//!
//! The compiler emits the user program plus a `Main` entry and the
//! `rnx_*` helper calls listed in the target's manifest. This crate supplies
//! the firmware entry point (`efi_main`), the heap primitives those helpers
//! are built on, and the console writer. It is a `no_std` staticlib linked
//! into the final PE image; nothing here needs a host C toolchain.
//!
//! Implemented surface: allocation and reference counting, strings, arrays,
//! `Any` boxing, integer and float rendering, plain `println!` output, and
//! explicit failure for the collection and process features a UEFI image
//! cannot have.

#![no_std]
// This target has no libc. Defining the memory primitives the Rust standard
// library lowers to is the point of the crate; the implementations below use
// `rep` string instructions, so they cannot recurse into themselves.
#![allow(suspicious_runtime_symbol_definitions)]

mod firmware;
mod heap;

use core::fmt::Write as _;
use core::panic::PanicInfo;
use firmware::{SystemTable, EFI_ABORTED, EFI_SUCCESS};
use heap::*;

static mut FATAL: u32 = 0;
static mut ASSERT_STRICT: bool = false;

pub fn fail(message: &str) -> ! {
    firmware::write_str("rnx: ");
    firmware::write_str(message);
    firmware::write_str("\n");
    firmware::exit_image(EFI_ABORTED)
}

unsafe extern "C" {
    fn Main() -> i64;
}

#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(image_handle: usize, system_table: *mut SystemTable) -> usize {
    firmware::install_system_table(image_handle, system_table);
    let code = unsafe { Main() };
    if code == 0 {
        EFI_SUCCESS
    } else {
        EFI_ABORTED
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    firmware::write_str("rnx: panic: ");
    let mut text = Text::new();
    let _ = write!(text, "{}", info.message());
    firmware::write_bytes(text.slice());
    firmware::write_str("\n");
    firmware::exit_image(EFI_ABORTED)
}

struct Text {
    data: [u8; 512],
    len: usize,
}

impl Text {
    fn new() -> Text {
        Text {
            data: [0; 512],
            len: 0,
        }
    }

    fn slice(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

impl core::fmt::Write for Text {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        for &byte in text.as_bytes() {
            if self.len == self.data.len() {
                return Err(core::fmt::Error);
            }
            self.data[self.len] = byte;
            self.len += 1;
        }
        Ok(())
    }
}

fn write_value(value: u64) {
    if let Some(addr) = any_box_addr(value) {
        let payload = box_payload(addr);
        match (value & ANY_BOX_MASK) as u32 {
            0b001 => {
                let mut out = [0u8; 32];
                firmware::write_bytes(firmware::format_i64(payload as i64, &mut out));
            }
            0b010 => firmware::write_str(if payload == 0 { "false" } else { "true" }),
            0b011 => {
                let mut out = [0u8; 32];
                firmware::write_bytes(firmware::format_f64(
                    f64::from_bits(payload),
                    &mut out,
                ));
            }
            _ => write_str_value(payload as *const u8),
        }
        return;
    }
    if value == 0 {
        firmware::write_str("null");
        return;
    }
    match heap_kind(value) {
        Some(HEAP_STR) => write_str_value(value as *const u8),
        Some(HEAP_ARRAY) => firmware::write_str("<array>"),
        Some(HEAP_OBJ) | Some(HEAP_ENUM) => firmware::write_str("<object>"),
        _ => firmware::write_str("<value>"),
    }
}

fn write_str_value(ptr: *const u8) {
    if ptr.is_null() {
        firmware::write_str("null");
        return;
    }
    firmware::write_str(core::str::from_utf8(str_bytes(ptr)).unwrap_or("?"));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_uefi_println(args: *const u8) {
    let len = arr_len(args);
    let data = arr_data(args);
    for i in 0..len {
        if i > 0 {
            firmware::write_str(" ");
        }
        let value = unsafe { (data as *const u64).add(i).read_unaligned() };
        write_value(value);
    }
    firmware::write_str("\n");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_uefi_write(text: *const u8) {
    write_str_value(text);
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
pub unsafe extern "C" fn rnx_assert(cond: i64, message: *const u8) {
    if cond != 0 {
        return;
    }
    firmware::write_str("rnx: assertion failed: ");
    write_str_value(message);
    firmware::write_str("\n");
    let strict = unsafe { ASSERT_STRICT };
    if strict {
        firmware::exit_image(EFI_ABORTED);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_panic_str(message: *const u8) {
    firmware::write_str("rnx: ");
    write_str_value(message);
    firmware::write_str("\n");
    firmware::exit_image(EFI_ABORTED)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_fatal_span(message: *const u8, _start: u64, _end: u64) {
    firmware::write_str("fatal: ");
    write_str_value(message);
    firmware::write_str("\n");
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
    firmware::write_str("rnx: uncaught error\n");
    1
}

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
            unsafe { rnx_free(addr as *mut u8, ANY_BOX_SIZE) };
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
pub unsafe extern "C" fn rnx_any_box(tag: u32, payload: u64) -> u64 {
    if any_box_addr(payload).is_some() {
        unsafe { rnx_any_retain(payload) };
        return payload;
    }
    let marker = match tag {
        TAG_INT => ANY_BOX_INT,
        TAG_BOOL => ANY_BOX_BOOL,
        TAG_FLOAT => ANY_BOX_FLOAT,
        TAG_STR => ANY_BOX_STR,
        _ => 0,
    };
    if marker == 0 {
        return payload;
    }
    let out = unsafe { rnx_alloc(ANY_BOX_SIZE, 8) };
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
pub unsafe extern "C" fn rnx_heap_track(ptr: *mut u8, kind: u64, aux1: u64, aux2: u64) {
    note_heap(ptr, kind, aux1, aux2);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_note_array_kind(ptr: *mut u8, kind: u64, aux: u64) {
    note_array_kind(ptr, kind, aux);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_new(initial_cap: usize, elem_size: usize) -> *mut u8 {
    array_new(initial_cap, elem_size)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_len(arr: *const u8) -> usize {
    arr_len(arr)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_push(arr: *mut u8, value: u64, elem_size: usize) {
    array_push(arr, value, elem_size)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_get(arr: *const u8, index: usize, elem_size: usize) -> u64 {
    array_get(arr, index, elem_size)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_release_array(
    arr: *mut u8,
    elem_size: usize,
    dtor: Option<unsafe extern "C" fn(*mut u8)>,
) {
    release_array(arr, elem_size, dtor)
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
pub unsafe extern "C" fn rnx_genref_invalidate(_ptr: *mut u8) {}

// The target has no libc, so these four are the only definitions of the
// symbols `core` and LLVM may lower copy and compare loops to. memcpy,
// memmove, and memset use `rep` string instructions so LLVM cannot turn the
// implementation back into a call to itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, count: usize) -> *mut u8 {
    unsafe {
        core::arch::asm!(
            "rep movsb",
            inout("rcx") count => _,
            inout("rdi") dest => _,
            inout("rsi") src => _,
            options(nostack, preserves_flags)
        );
    }
    dest
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, count: usize) -> *mut u8 {
    if (dest as usize) <= (src as usize) || (dest as usize) >= (src as usize) + count {
        return unsafe { memcpy(dest, src, count) };
    }
    unsafe {
        core::arch::asm!(
            "std",
            "rep movsb",
            "cld",
            inout("rcx") count => _,
            inout("rdi") (dest as usize + count - 1) as *mut u8 => _,
            inout("rsi") (src as usize + count - 1) as *const u8 => _,
            options(nostack)
        );
    }
    dest
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dest: *mut u8, value: i32, count: usize) -> *mut u8 {
    unsafe {
        core::arch::asm!(
            "rep stosb",
            inout("rcx") count => _,
            inout("rdi") dest => _,
            in("al") value as u8,
            options(nostack, preserves_flags)
        );
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
