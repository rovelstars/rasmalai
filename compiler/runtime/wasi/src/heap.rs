//! Linear-memory allocator and the object layout generated code relies on.
//!
//! The module owns one linear memory. Static data ends at `__heap_base`,
//! which the linker resolves to the highest static address, and the arena
//! grows from there in 64 KiB pages. Every allocation carries a 16-byte
//! header in front of the returned pointer; the header stores the block it
//! came from, so a free returns the whole block to a first-fit list instead
//! of leaking it.
//!
//! Strings and arrays mirror the host runtime layout: a reference count at
//! offset 0, then the type fields documented at `STR_HEADER` and
//! `ARR_HEADER`. The generated code never reads those fields itself; every
//! access goes through a runtime call in this file, so one layout serves
//! every target.

use crate::wasi;

pub const HEADER: usize = 16;
pub const STR_HEADER: usize = 32;
pub const ARR_HEADER: usize = 40;

pub const TAG_INT: u32 = 0;
pub const TAG_BOOL: u32 = 1;
pub const TAG_FLOAT: u32 = 2;
pub const TAG_STR: u32 = 3;

pub const ANY_BOX_MASK: u64 = 0b111;
pub const ANY_BOX_INT: u64 = 0b001;
pub const ANY_BOX_BOOL: u64 = 0b010;
pub const ANY_BOX_FLOAT: u64 = 0b011;
pub const ANY_BOX_STR: u64 = 0b100;
pub const ANY_BOX_SIZE: usize = 16;

pub const HEAP_ARRAY: u64 = 1;
pub const HEAP_STR: u64 = 2;
pub const HEAP_OBJ: u64 = 3;
pub const HEAP_ENUM: u64 = 4;

const PAGE_SIZE: usize = 65536;
const MIN_BLOCK: usize = 32;

unsafe extern "C" {
    static __heap_base: u8;
}

static mut HEAP_TOP: usize = 0;
static mut FREE_HEAD: usize = 0;

fn align_up(value: usize, align: usize) -> usize {
    (value.wrapping_add(align - 1)) & !(align - 1)
}

fn memory_bytes() -> usize {
    core::arch::wasm32::memory_size(0).wrapping_mul(PAGE_SIZE)
}

fn grow_to(end: usize) -> bool {
    let have = memory_bytes();
    if end <= have {
        return true;
    }
    let pages = (end - have).div_ceil(PAGE_SIZE);
    core::arch::wasm32::memory_grow(0, pages) != usize::MAX
}

fn block_size(at: usize) -> usize {
    unsafe { (at as *const usize).read_unaligned() }
}

fn block_next(at: usize) -> usize {
    unsafe { ((at + 8) as *const usize).read_unaligned() }
}

fn write_block(at: usize, size: usize, next: usize) {
    unsafe {
        (at as *mut usize).write_unaligned(size);
        ((at + 8) as *mut usize).write_unaligned(next);
    }
}

fn take_free(size: usize, align: usize) -> Option<(usize, usize)> {
    let head = unsafe { FREE_HEAD };
    let mut prev = 0usize;
    let mut cur = head;
    while cur != 0 {
        let total = block_size(cur);
        let next = block_next(cur);
        let start = align_up(cur + HEADER, align);
        let end = start + size;
        if end <= cur + total {
            let tail = align_up(end, 8);
            let mut span = total;
            if cur + total >= tail + MIN_BLOCK {
                write_block(tail, cur + total - tail, next);
                if prev == 0 {
                    unsafe { FREE_HEAD = tail };
                } else {
                    unsafe { ((prev + 8) as *mut usize).write_unaligned(tail) };
                }
                span = tail - cur;
            } else if prev == 0 {
                unsafe { FREE_HEAD = next };
            } else {
                unsafe { ((prev + 8) as *mut usize).write_unaligned(next) };
            }
            return Some((start, span));
        }
        prev = cur;
        cur = next;
    }
    None
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn alloc_bytes(size: usize, align: usize) -> *mut u8 {
    let align = if align < 8 { 8 } else { align };
    if !align.is_power_of_two() {
        return core::ptr::null_mut();
    }
    let size = size.max(1);
    let (start, span) = match take_free(size, align) {
        Some(found) => found,
        None => {
            let mut top = unsafe { HEAP_TOP };
            if top == 0 {
                top = align_up(core::ptr::addr_of!(__heap_base) as usize, 16);
            }
            let start = align_up(top + HEADER, align);
            let end = start + size;
            if !grow_to(end) {
                return core::ptr::null_mut();
            }
            unsafe { HEAP_TOP = end };
            (start, end - (start - HEADER))
        }
    };
    let base = start - HEADER;
    write_block(base, span, 0);
    unsafe { ((base + 8) as *mut usize).write_unaligned(size) };
    start as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_bytes(ptr: *mut u8, _size: usize) {
    if ptr.is_null() {
        return;
    }
    let at = ptr as usize;
    if at < HEADER {
        return;
    }
    let base = at - HEADER;
    let total = block_size(base);
    if total < MIN_BLOCK {
        return;
    }
    let head = unsafe { FREE_HEAD };
    write_block(base, total, head);
    unsafe { FREE_HEAD = base };
}

pub fn read_u64(ptr: *const u8, offset: usize) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (ptr.add(offset) as *const u64).read_unaligned() }
}

pub fn write_u64(ptr: *mut u8, offset: usize, value: u64) {
    if ptr.is_null() {
        return;
    }
    unsafe { (ptr.add(offset) as *mut u64).write_unaligned(value) };
}

pub fn write_u32(ptr: *mut u8, offset: usize, value: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (ptr.add(offset) as *mut u32).write_unaligned(value) };
}

pub fn str_len(ptr: *const u8) -> usize {
    read_u64(ptr, 16) as usize
}

pub fn str_bytes<'a>(ptr: *const u8) -> &'a [u8] {
    if ptr.is_null() {
        return b"";
    }
    let len = str_len(ptr);
    unsafe { core::slice::from_raw_parts(ptr.add(STR_HEADER), len) }
}

pub fn str_body_size(len: usize) -> usize {
    STR_HEADER + len + 1
}

pub fn str_alloc(len: usize) -> *mut u8 {
    let out = unsafe { alloc_bytes(str_body_size(len), 8) };
    if out.is_null() {
        return out;
    }
    write_u32(out, 0, 1);
    write_u32(out, 4, 0);
    write_u64(out, 8, 0);
    write_u64(out, 16, len as u64);
    write_u64(out, 24, 0);
    out
}

pub fn arr_len(ptr: *const u8) -> usize {
    read_u64(ptr, 16) as usize
}

pub fn arr_cap(ptr: *const u8) -> usize {
    read_u64(ptr, 24) as usize
}

pub fn arr_data(ptr: *const u8) -> *mut u8 {
    read_u64(ptr, 32) as *mut u8
}

pub fn arr_write(ptr: *mut u8, len: usize, cap: usize, data: *mut u8) {
    write_u64(ptr, 16, len as u64);
    write_u64(ptr, 24, cap as u64);
    write_u64(ptr, 32, data as u64);
}

pub fn ref_count_ptr(ptr: *mut u8) -> *mut u32 {
    ptr as *mut u32
}

pub fn retain(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let cell = ref_count_ptr(ptr);
        let count = cell.read_unaligned();
        if count == u32::MAX || count == 0 {
            return;
        }
        cell.write_unaligned(count + 1);
    }
}

pub fn release(ptr: *mut u8) -> bool {
    if ptr.is_null() {
        return false;
    }
    unsafe {
        let cell = ref_count_ptr(ptr);
        let count = cell.read_unaligned();
        if count == u32::MAX || count == 0 {
            return false;
        }
        cell.write_unaligned(count - 1);
        count == 1
    }
}

const SET_CAPACITY: usize = 4096;

struct PtrSet {
    slots: [usize; SET_CAPACITY],
    len: usize,
}

impl PtrSet {
    const fn new() -> PtrSet {
        PtrSet {
            slots: [0; SET_CAPACITY],
            len: 0,
        }
    }

    fn hash(value: usize) -> usize {
        let mut x = ((value >> 4) as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (x ^ (x >> 31)) as usize & (SET_CAPACITY - 1)
    }

    fn contains(&self, value: usize) -> bool {
        if value == 0 {
            return false;
        }
        let mut at = Self::hash(value);
        loop {
            let slot = self.slots[at];
            if slot == 0 {
                return false;
            }
            if slot == value {
                return true;
            }
            at = (at + 1) & (SET_CAPACITY - 1);
        }
    }

    fn insert(&mut self, value: usize) -> bool {
        if value == 0 {
            return false;
        }
        if self.len == SET_CAPACITY {
            return false;
        }
        let mut at = Self::hash(value);
        loop {
            let slot = self.slots[at];
            if slot == 0 {
                self.slots[at] = value;
                self.len += 1;
                return true;
            }
            if slot == value {
                return true;
            }
            at = (at + 1) & (SET_CAPACITY - 1);
        }
    }

    fn remove(&mut self, value: usize) -> bool {
        let mut at = Self::hash(value);
        let mut found = false;
        loop {
            let slot = self.slots[at];
            if slot == 0 {
                return found;
            }
            if slot == value {
                found = true;
                self.slots[at] = 0;
                self.len -= 1;
                break;
            }
            at = (at + 1) & (SET_CAPACITY - 1);
        }
        let mut next = (at + 1) & (SET_CAPACITY - 1);
        while self.slots[next] != 0 {
            let value = self.slots[next];
            self.slots[next] = 0;
            self.len -= 1;
            self.insert(value);
            next = (next + 1) & (SET_CAPACITY - 1);
        }
        found
    }
}

struct Boxes {
    set: PtrSet,
}

impl Boxes {
    const fn new() -> Boxes {
        Boxes {
            set: PtrSet::new(),
        }
    }
}

struct Kinds {
    ptrs: [usize; SET_CAPACITY],
    kinds: [u64; SET_CAPACITY],
    len: usize,
}

impl Kinds {
    const fn new() -> Kinds {
        Kinds {
            ptrs: [0; SET_CAPACITY],
            kinds: [0; SET_CAPACITY],
            len: 0,
        }
    }

    fn find(&self, ptr: usize) -> Option<usize> {
        let mut n = 0;
        while n < self.len {
            if self.ptrs[n] == ptr {
                return Some(n);
            }
            n += 1;
        }
        None
    }

    fn set(&mut self, ptr: usize, kind: u64) {
        if ptr == 0 {
            return;
        }
        if let Some(at) = self.find(ptr) {
            self.kinds[at] = kind;
            return;
        }
        if self.len == SET_CAPACITY {
            return;
        }
        self.ptrs[self.len] = ptr;
        self.kinds[self.len] = kind;
        self.len += 1;
    }

    fn kind(&self, ptr: usize) -> Option<u64> {
        self.find(ptr).map(|at| self.kinds[at])
    }
}

static mut BOXES: Boxes = Boxes::new();
static mut KINDS: Kinds = Kinds::new();

fn boxes() -> &'static mut Boxes {
    unsafe { &mut *core::ptr::addr_of_mut!(BOXES) }
}

fn kinds() -> &'static mut Kinds {
    unsafe { &mut *core::ptr::addr_of_mut!(KINDS) }
}

pub fn any_box_addr(any: u64) -> Option<usize> {
    if any & ANY_BOX_MASK == 0 {
        return None;
    }
    let addr = (any & !ANY_BOX_MASK) as usize;
    boxes().set.contains(addr).then_some(addr)
}

pub fn boxes_insert(addr: usize) -> bool {
    boxes().set.insert(addr)
}

pub fn boxes_forget(addr: usize) {
    boxes().set.remove(addr);
}

pub fn box_payload(addr: usize) -> u64 {
    read_u64(addr as *const u8, 8)
}

pub fn box_count(addr: usize) -> *mut u64 {
    addr as *mut u64
}

pub fn heap_kind(any: u64) -> Option<u64> {
    if any & ANY_BOX_MASK != 0 || any == 0 {
        return None;
    }
    kinds().kind(any as usize)
}

pub fn note_heap(ptr: *mut u8, kind: u64, aux1: u64, aux2: u64) {
    if ptr.is_null() || kind == 0 {
        return;
    }
    kinds().set(ptr as usize, kind);
    let _ = (aux1, aux2);
}

pub fn note_array_kind(ptr: *mut u8, kind: u64, aux: u64) {
    let _ = (ptr, kind, aux);
}

pub fn array_new(initial_cap: usize, elem_size: usize) -> *mut u8 {
    let out = unsafe { alloc_bytes(ARR_HEADER, 8) };
    if out.is_null() {
        return out;
    }
    write_u32(out, 0, 1);
    write_u32(out, 4, 0);
    write_u64(out, 8, 0);
    let data = if initial_cap > 0 && elem_size > 0 {
        unsafe { alloc_bytes(initial_cap.wrapping_mul(elem_size), 8) }
    } else {
        core::ptr::null_mut()
    };
    if initial_cap > 0 && elem_size > 0 && data.is_null() {
        unsafe { free_bytes(out, ARR_HEADER) };
        return core::ptr::null_mut();
    }
    arr_write(out, 0, initial_cap, data);
    out
}

pub fn array_push(arr: *mut u8, value: u64, elem_size: usize) {
    if arr.is_null() || elem_size == 0 {
        return;
    }
    let mut len = arr_len(arr);
    let mut cap = arr_cap(arr);
    let mut data = arr_data(arr);
    if len == cap {
        let next_cap = if cap == 0 { 4 } else { cap.wrapping_mul(2) };
        let bytes = next_cap.wrapping_mul(elem_size);
        let grown = unsafe { alloc_bytes(bytes, 8) };
        if grown.is_null() {
            crate::fail("array push out of memory");
        }
        if !data.is_null() && len > 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(data, grown, len.wrapping_mul(elem_size));
                free_bytes(data, cap.wrapping_mul(elem_size));
            }
        }
        data = grown;
        cap = next_cap;
    }
    unsafe {
        let slot = data.add(len.wrapping_mul(elem_size));
        if elem_size == 8 {
            (slot as *mut u64).write_unaligned(value);
        } else {
            core::ptr::copy_nonoverlapping(
                &value as *const u64 as *const u8,
                slot,
                elem_size.min(8),
            );
        }
    }
    len += 1;
    arr_write(arr, len, cap, data);
}

pub fn array_get(arr: *const u8, index: usize, elem_size: usize) -> u64 {
    if arr.is_null() || index >= arr_len(arr) {
        crate::fail("index out of bounds");
    }
    let data = arr_data(arr);
    unsafe {
        let slot = data.add(index.wrapping_mul(elem_size));
        if elem_size == 8 {
            (slot as *const u64).read_unaligned()
        } else {
            let mut out = [0u8; 8];
            let n = elem_size.min(out.len());
            core::ptr::copy_nonoverlapping(slot, out.as_mut_ptr(), n);
            u64::from_le_bytes(out)
        }
    }
}

pub fn release_array(arr: *mut u8, elem_size: usize, dtor: Option<unsafe extern "C" fn(*mut u8)>) {
    if arr.is_null() {
        return;
    }
    if !release(arr) {
        return;
    }
    let len = arr_len(arr);
    let cap = arr_cap(arr);
    let data = arr_data(arr);
    if let Some(run) = dtor {
        for i in 0..len {
            unsafe {
                let slot = data.add(i.wrapping_mul(elem_size));
                run((slot as *const *mut u8).read_unaligned());
            }
        }
    }
    if !data.is_null() {
        unsafe { free_bytes(data, cap.wrapping_mul(elem_size)) };
    }
    unsafe { free_bytes(arr, ARR_HEADER) };
}

pub fn release_str(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    let len = str_len(ptr);
    if release(ptr) {
        unsafe { free_bytes(ptr, str_body_size(len)) };
    }
}

pub fn fail(message: &str) -> ! {
    wasi::write_error_str("rnx: ");
    wasi::write_error_str(message);
    wasi::write_error_str("\n");
    wasi::exit(1)
}
