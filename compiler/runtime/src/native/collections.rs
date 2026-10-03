use super::common::*;
use super::string::*;
use super::sync::*;
use super::io::*;
use super::json_tape::{
    array_count, array_iter, bool_at, float_at, int_at, key_at, object_count, object_iter,
    parse_tape, parse_typed, split_typed_desc, str_at, tape_handle_id, tape_is_handle,
    tape_key_eq, tape_materialize, tape_materialize_fast, tape_release, tape_release_fast, tape_resolve, tape_resolve_fast, tape_tag, tape_upgrade_get,
    tape_upgrade_set, tape_write_doc, JsonDoc, TypedVal, TAPE_ARRAY, TAPE_BOOL, TAPE_FLOAT,
    TAPE_INT, TAPE_NULL, TAPE_STR,
};

pub struct ByteBufferState {
    pub data: Vec<u8>,
}

pub(crate) fn bytes_state(buf: *mut u8) -> &'static mut ByteBufferState {
    if buf.is_null() {
        unsafe {
            rnx_panic(b"null byte buffer\0".as_ptr(), "null byte buffer".len());
        }
    }
    unsafe { &mut *(buf as *mut ByteBufferState) }
}

pub(crate) fn bytes_range(buf: *mut u8, off: i64, size: i64) -> (usize, usize) {
    let st = bytes_state(buf);
    if off < 0 || size < 0 {
        unsafe {
            rnx_panic(b"byte buffer out of bounds\0".as_ptr(), "byte buffer out of bounds".len());
        }
    }
    let start = off as usize;
    let end = start.saturating_add(size as usize);
    if end > st.data.len() {
        unsafe {
            rnx_panic(b"byte buffer out of bounds\0".as_ptr(), "byte buffer out of bounds".len());
        }
    }
    (start, end)
}

pub fn bytes_alloc_impl(cap: i64) -> *mut u8 {
    if cap < 0 {
        unsafe {
            rnx_panic(b"negative byte buffer capacity\0".as_ptr(), "negative byte buffer capacity".len());
        }
    }
    Box::into_raw(Box::new(ByteBufferState { data: vec![0u8; cap as usize] })) as *mut u8
}

pub fn bytes_len_impl(buf: *mut u8) -> i64 {
    bytes_state(buf).data.len() as i64
}

pub fn bytes_free_impl(buf: *mut u8) {
    if buf.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(buf as *mut ByteBufferState));
    }
}

pub fn bytes_copy_within_impl(buf: *mut u8, target: i64, start: i64, end: i64) {
    let (s, e) = bytes_range(buf, start, end - start);
    let len = e - s;
    let (t, te) = bytes_range(buf, target, len as i64);
    let st = bytes_state(buf);
    st.data.copy_within(s..e, t);
    let _ = te;
}

pub fn bytes_read_impl(buf: *mut u8, off: i64, size: i64) -> u64 {
    let (s, e) = bytes_range(buf, off, size);
    let st = bytes_state(buf);
    let mut v: u64 = 0;
    for (i, b) in st.data[s..e].iter().enumerate() {
        v |= (*b as u64) << (8 * i);
    }
    v
}

pub fn bytes_read_be_impl(buf: *mut u8, off: i64, size: i64) -> u64 {
    let (s, e) = bytes_range(buf, off, size);
    let st = bytes_state(buf);
    let mut v: u64 = 0;
    for b in st.data[s..e].iter() {
        v = (v << 8) | (*b as u64);
    }
    v
}

pub fn bytes_write_impl(buf: *mut u8, off: i64, size: i64, val: u64) {
    let (s, e) = bytes_range(buf, off, size);
    let st = bytes_state(buf);
    for (i, slot) in st.data[s..e].iter_mut().enumerate() {
        *slot = ((val >> (8 * i)) & 0xFF) as u8;
    }
}

pub fn bytes_write_be_impl(buf: *mut u8, off: i64, size: i64, val: u64) {
    let (s, e) = bytes_range(buf, off, size);
    let st = bytes_state(buf);
    let n = e - s;
    for (i, slot) in st.data[s..e].iter_mut().enumerate() {
        *slot = ((val >> (8 * (n - 1 - i))) & 0xFF) as u8;
    }
}

pub fn bytes_read_string_impl(buf: *mut u8, off: i64, len: i64) -> String {
    let (s, e) = bytes_range(buf, off, len);
    String::from_utf8_lossy(&bytes_state(buf).data[s..e]).into_owned()
}

pub fn bytes_write_string_impl(buf: *mut u8, off: i64, text: &[u8]) -> i64 {
    let (s, e) = bytes_range(buf, off, text.len() as i64);
    bytes_state(buf).data[s..e].copy_from_slice(text);
    text.len() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_alloc(cap: i64) -> *mut u8 {
    bytes_alloc_impl(cap)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_free(buf: *mut u8) {
    bytes_free_impl(buf)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_len(buf: *mut u8) -> i64 {
    bytes_len_impl(buf)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_data(buf: *mut u8) -> *mut u8 {
    bytes_state(buf).data.as_mut_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_cap(buf: *mut u8) -> i64 {
    bytes_len_impl(buf)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_copy_within(buf: *mut u8, target: i64, start: i64, end: i64) {
    bytes_copy_within_impl(buf, target, start, end)
}

bytes_read_int!(rnx_bytes_read_u8, rnx_bytes_read_u8_be, 1, zero_extend);
bytes_read_int!(rnx_bytes_read_i8, rnx_bytes_read_i8_be, 1, sign_extend);
bytes_read_int!(rnx_bytes_read_u16le, rnx_bytes_read_u16be, 2, zero_extend);
bytes_read_int!(rnx_bytes_read_i16le, rnx_bytes_read_i16be, 2, sign_extend);
bytes_read_int!(rnx_bytes_read_u32le, rnx_bytes_read_u32be, 4, zero_extend);
bytes_read_int!(rnx_bytes_read_i32le, rnx_bytes_read_i32be, 4, sign_extend);
bytes_read_int!(rnx_bytes_read_i64le, rnx_bytes_read_i64be, 8, sign_extend);

bytes_write_int!(rnx_bytes_write_u8, rnx_bytes_write_u8_be, 1);
bytes_write_int!(rnx_bytes_write_u16le, rnx_bytes_write_u16be, 2);
bytes_write_int!(rnx_bytes_write_u32le, rnx_bytes_write_u32be, 4);
bytes_write_int!(rnx_bytes_write_u64le, rnx_bytes_write_u64be, 8);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_read_f32le(buf: *mut u8, off: i64) -> u64 {
    let bits = bytes_read_impl(buf, off, 4) as u32;
    (f32::from_bits(bits) as f64).to_bits()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_read_f32be(buf: *mut u8, off: i64) -> u64 {
    let bits = bytes_read_be_impl(buf, off, 4) as u32;
    (f32::from_bits(bits) as f64).to_bits()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_read_f64le(buf: *mut u8, off: i64) -> u64 {
    bytes_read_impl(buf, off, 8)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_read_f64be(buf: *mut u8, off: i64) -> u64 {
    bytes_read_be_impl(buf, off, 8)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_write_f32le(buf: *mut u8, off: i64, bits: u64) {
    let v = f64::from_bits(bits) as f32;
    bytes_write_impl(buf, off, 4, v.to_bits() as u64)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_write_f32be(buf: *mut u8, off: i64, bits: u64) {
    let v = f64::from_bits(bits) as f32;
    bytes_write_be_impl(buf, off, 4, v.to_bits() as u64)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_write_f64le(buf: *mut u8, off: i64, bits: u64) {
    bytes_write_impl(buf, off, 8, bits)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_write_f64be(buf: *mut u8, off: i64, bits: u64) {
    bytes_write_be_impl(buf, off, 8, bits)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_bytes_read_string(buf: *mut u8, off: i64, len: i64) -> *mut u8 {
    let text = bytes_read_string_impl(buf, off, len);
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
pub unsafe extern "C" fn rnx_bytes_write_string(buf: *mut u8, off: i64, s: *const u8) -> i64 {
    bytes_write_string_impl(buf, off, str_bytes(s))
}

pub const GKEY_INT: u8 = 1;
pub const GKEY_FLOAT: u8 = 2;
pub const GKEY_BOOL: u8 = 3;
pub const GKEY_STR: u8 = 4;
pub const GKEY_OBJ: u8 = 5;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GKey {
    pub kind: u8,
    pub bits: u64,
    pub text: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GVal {
    pub kind: u8,
    pub bits: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SplitMixHasher(u64);

impl std::hash::Hasher for SplitMixHasher {
    fn write(&mut self, bytes: &[u8]) -> () {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.0 = splitmix64(self.0.wrapping_add(u64::from_le_bytes(word)));
        }
    }
    fn finish(&self) -> u64 {
        splitmix64(self.0)
    }
}

pub(crate) fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SplitMixBuild;

impl std::hash::BuildHasher for SplitMixBuild {
    type Hasher = SplitMixHasher;
    fn build_hasher(&self) -> SplitMixHasher {
        SplitMixHasher(0x9E3779B97F4A7C15)
    }
}

pub(crate) const GMAP_INDEX_THRESHOLD: usize = 8;

pub struct GenericMapState {
    inner: RwLock<GenericMapStateInner>,
}

pub(crate) struct GenericMapStateInner {
    pub(crate) entries: Vec<GEntry>,
    pub(crate) index: Option<std::collections::HashMap<GKey, usize, SplitMixBuild>>,
    pub(crate) count: usize,
}

pub(crate) struct GEntry {
    #[allow(dead_code)]
    pub(crate) key: GKey,
    pub(crate) krepr: u64,
    pub(crate) val: GVal,
    pub(crate) active: bool,
}

pub(crate) fn gkey_canon_float(bits: u64) -> u64 {
    if f64::from_bits(bits).is_nan() {
        f64::NAN.to_bits()
    } else {
        bits
    }
}

pub(crate) fn gkey_of(rep: u64) -> GKey {
    match any_box_addr(rep) {
        Some(addr) => {
            let payload = unsafe { ((addr as *const u8).add(8) as *const u64).read_unaligned() };
            match (rep & ANY_BOX_MASK) as u32 {
                x if x == ANY_BOX_INT as u32 => GKey { kind: GKEY_INT, bits: payload, text: None },
                x if x == ANY_BOX_BOOL as u32 => GKey { kind: GKEY_BOOL, bits: payload & 1, text: None },
                x if x == ANY_BOX_FLOAT as u32 => {
                    GKey { kind: GKEY_FLOAT, bits: gkey_canon_float(payload), text: None }
                }
                _ => {
                    let text = native_str(payload as *const u8);
                    let bits = fnv1a(&text);
                    GKey { kind: GKEY_STR, bits, text: Some(text) }
                }
            }
        }
        None => GKey { kind: GKEY_OBJ, bits: rep, text: None },
    }
}

pub(crate) fn gval_of(rep: u64) -> GVal {
    match any_box_addr(rep) {
        Some(_) => {
            let kind = match (rep & ANY_BOX_MASK) as u32 {
                x if x == ANY_BOX_INT as u32 => GKEY_INT,
                x if x == ANY_BOX_BOOL as u32 => GKEY_BOOL,
                x if x == ANY_BOX_FLOAT as u32 => GKEY_FLOAT,
                _ => GKEY_STR,
            };
            GVal { kind, bits: rep }
        }
        None => GVal { kind: GKEY_OBJ, bits: rep },
    }
}

pub(crate) fn gval_is_box(v: &GVal) -> bool {
    v.bits != 0 && any_box_addr(v.bits).is_some()
}

pub(crate) fn gval_retain(v: &GVal) {
    if gval_is_box(v) {
        unsafe { rnx_any_retain(v.bits) };
        return;
    }
    if v.bits != 0 && heap_tracked(v.bits).is_some() {
        unsafe { rnx_any_retain(v.bits) };
    }
}

pub(crate) fn gkey_retain(krepr: u64) {
    if krepr != 0 && any_box_addr(krepr).is_some() {
        unsafe { rnx_any_retain(krepr) };
        return;
    }
    if krepr != 0 && heap_tracked(krepr).is_some() {
        unsafe { rnx_any_retain(krepr) };
    }
}

pub(crate) fn gkey_release(krepr: u64) {
    if krepr != 0 && any_box_addr(krepr).is_some() {
        unsafe { rnx_any_release(krepr) };
        return;
    }
    if krepr != 0 && heap_tracked(krepr).is_some() {
        unsafe { rnx_any_release(krepr) };
    }
}

pub(crate) fn gval_release(v: &GVal) {
    if gval_is_box(v) {
        unsafe { rnx_any_release(v.bits) };
        return;
    }
    if v.bits != 0 && heap_tracked(v.bits).is_some() {
        unsafe { rnx_any_release(v.bits) };
    }
}

impl GenericMapState {
    pub fn new() -> GenericMapState {
        GenericMapState {
            inner: RwLock::new(GenericMapStateInner {
                entries: Vec::new(),
                index: None,
                count: 0,
            }),
        }
    }

    fn read<R>(&self, f: impl FnOnce(&GenericMapStateInner) -> R) -> R {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        f(&inner)
    }

    fn write<R>(&self, f: impl FnOnce(&mut GenericMapStateInner) -> R) -> R {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        f(&mut inner)
    }

    pub fn set(&self, krepr: u64, vrepr: u64) {
        self.write(|st| st.set_inner(krepr, vrepr));
    }

    pub fn get(&self, krepr: u64) -> Option<u64> {
        self.read(|st| st.get_inner(krepr))
    }

    pub fn has(&self, krepr: u64) -> bool {
        self.read(|st| st.has_inner(krepr))
    }

    pub fn delete(&self, krepr: u64) -> bool {
        self.write(|st| st.delete_inner(krepr))
    }

    pub fn len(&self) -> usize {
        self.read(|st| st.count)
    }

    pub fn clear(&self) {
        self.write(|st| st.clear_inner());
    }

    pub fn live_key_reprs(&self) -> Vec<u64> {
        self.read(|st| {
            let mut out = Vec::with_capacity(st.count);
            for e in st.entries.iter().filter(|e| e.active) {
                gkey_retain(e.krepr);
                out.push(e.krepr);
            }
            out
        })
    }

    pub fn live_vals(&self) -> Vec<u64> {
        self.read(|st| {
            let mut out = Vec::with_capacity(st.count);
            for e in st.entries.iter().filter(|e| e.active) {
                gval_retain(&e.val);
                out.push(e.val.bits);
            }
            out
        })
    }

    pub fn take_all(&self) -> Vec<GEntry> {
        self.write(|st| {
            let entries = std::mem::replace(&mut st.entries, Vec::new());
            st.index = None;
            st.count = 0;
            entries
        })
    }

    pub fn snapshot(&self) -> Vec<(u64, u64)> {
        self.read(|st| {
            let mut out = Vec::with_capacity(st.count);
            for e in st.entries.iter().filter(|e| e.active) {
                gkey_retain(e.krepr);
                gval_retain(&e.val);
                out.push((e.krepr, e.val.bits));
            }
            out
        })
    }
}

impl GenericMapStateInner {
    fn lookup(&self, key: &GKey) -> Option<usize> {
        if let Some(index) = self.index.as_ref() {
            return index.get(key).copied();
        }
        self.entries
            .iter()
            .enumerate()
            .find(|(_, e)| e.active && e.key == *key)
            .map(|(i, _)| i)
    }

    fn ensure_index(&mut self) {
        if self.index.is_some() || self.entries.len() <= GMAP_INDEX_THRESHOLD {
            return;
        }
        let mut index = std::collections::HashMap::with_hasher(SplitMixBuild);
        for (i, e) in self.entries.iter().enumerate() {
            if e.active {
                index.insert(e.key.clone(), i);
            }
        }
        self.index = Some(index);
    }

    fn set_inner(&mut self, krepr: u64, vrepr: u64) {
        let key = gkey_of(krepr);
        let val = gval_of(vrepr);
        if let Some(ix) = self.lookup(&key) {
            gval_retain(&val);
            gval_release(&self.entries[ix].val);
            self.entries[ix].val = val;
            if self.entries[ix].krepr != krepr {
                gkey_retain(krepr);
                gkey_release(self.entries[ix].krepr);
                self.entries[ix].krepr = krepr;
            }
            return;
        }
        let ix = self.entries.len();
        gkey_retain(krepr);
        gval_retain(&val);
        self.entries.push(GEntry { key: key.clone(), krepr, val, active: true });
        if let Some(index) = self.index.as_mut() {
            index.insert(key, ix);
        }
        self.count += 1;
        self.ensure_index();
    }

    fn get_inner(&self, krepr: u64) -> Option<u64> {
        let key = gkey_of(krepr);
        self.lookup(&key).and_then(|ix| {
            let e = &self.entries[ix];
            if e.active {
                gval_retain(&e.val);
                Some(e.val.bits)
            } else {
                None
            }
        })
    }

    fn has_inner(&self, krepr: u64) -> bool {
        let key = gkey_of(krepr);
        self.lookup(&key)
            .is_some_and(|ix| self.entries[ix].active)
    }

    fn delete_inner(&mut self, krepr: u64) -> bool {
        let key = gkey_of(krepr);
        let found = match self.index.as_mut() {
            Some(index) => index.remove(&key),
            None => self
                .entries
                .iter()
                .enumerate()
                .find(|(_, e)| e.active && e.key == key)
                .map(|(i, _)| i),
        };
        match found {
            Some(ix) => {
                if self.entries[ix].active {
                    self.entries[ix].active = false;
                    gkey_release(self.entries[ix].krepr);
                    gval_release(&self.entries[ix].val);
                    self.count -= 1;
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    }

    fn clear_inner(&mut self) {
        for e in self.entries.iter() {
            if e.active {
                gkey_release(e.krepr);
                gval_release(&e.val);
            }
        }
        self.entries.clear();
        self.index = None;
        self.count = 0;
    }

}

pub(crate) fn gmap_state(map: *mut u8) -> &'static GenericMapState {
    if map.is_null() {
        unsafe {
            rnx_panic(b"null map handle\0".as_ptr(), "null map handle".len());
        }
    }
    unsafe { &*(map as *mut GenericMapState) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_gmap_new() -> *mut u8 {
    Box::into_raw(Box::new(GenericMapState::new())) as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_gmap_free(map: *mut u8) {
    if map.is_null() {
        return;
    }
    if json_map_contains(map as u64) {
        unsafe { json_deep_free_map(map) };
        return;
    }
    if tape_is_handle(map as u64) {
        unsafe { tape_free_map(map as u64) };
        return;
    }
    let st = gmap_state(map);
    st.clear();
    unsafe {
        drop(Box::from_raw(map as *mut GenericMapState));
    }
}

unsafe fn tape_read(handle: u64, key: u64) -> Option<u64> {
    let up = tape_upgrade_get(handle);
    if !up.is_null() {
        return gmap_state(up).get(key);
    }
    let (doc, root) = match tape_resolve_fast(handle).or_else(|| tape_resolve(handle).map(|t| (t.doc, t.root))) {
        Some(v) => v,
        None => return None,
    };
    let root = root as usize;
    for (k, v) in object_iter(&doc.nodes, root) {
        if unsafe { tape_key_matches(key_at(&doc, k), key) } {
            return Some(unsafe { tape_to_any(&doc, v) });
        }
    }
    None
}

unsafe fn tape_has_key(handle: u64, key: u64) -> bool {
    let up = tape_upgrade_get(handle);
    if !up.is_null() {
        return gmap_state(up).has(key);
    }
    let (doc, root) = match tape_resolve_fast(handle).or_else(|| tape_resolve(handle).map(|t| (t.doc, t.root))) {
        Some(v) => v,
        None => return false,
    };
    let root = root as usize;
    for (k, _) in object_iter(&doc.nodes, root) {
        if unsafe { tape_key_matches(key_at(&doc, k), key) } {
            return true;
        }
    }
    false
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_gmap_set(map: *mut u8, key: u64, val: u64) {
    if tape_is_handle(map as u64) {
        let live = unsafe { tape_upgrade(map as u64) };
        gmap_state(live).set(key, val);
        return;
    }
    gmap_state(map).set(key, val)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_gmap_get(map: *mut u8, key: u64) -> u64 {
    if tape_is_handle(map as u64) {
        return unsafe { tape_read(map as u64, key) }.unwrap_or(0);
    }
    gmap_state(map).get(key).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_gmap_has(map: *const u8, key: u64) -> bool {
    if tape_is_handle(map as u64) {
        return unsafe { tape_has_key(map as u64, key) };
    }
    gmap_state(map as *mut u8).has(key)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_gmap_delete(map: *mut u8, key: u64) -> bool {
    if tape_is_handle(map as u64) {
        let live = unsafe { tape_upgrade(map as u64) };
        return gmap_state(live).delete(key);
    }
    gmap_state(map).delete(key)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_gmap_len(map: *const u8) -> usize {
    if tape_is_handle(map as u64) {
        let up = tape_upgrade_get(map as u64);
        if !up.is_null() {
            return gmap_state(up).len();
        }
        return match tape_resolve(map as u64) {
            Some(t) => object_count(&t.doc.nodes, t.root as usize),
            None => 0,
        };
    }
    gmap_state(map as *mut u8).len()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_gmap_clear(map: *mut u8) {
    if tape_is_handle(map as u64) {
        let live = unsafe { tape_upgrade(map as u64) };
        gmap_state(live).clear();
        return;
    }
    gmap_state(map).clear()
}

pub(crate) fn gmap_box_array(items: &[u64], release: fn(u64)) -> *mut u8 {
    let out = unsafe { rnx_array_new(items.len(), 8) };
    if out.is_null() {
        for v in items {
            release(*v);
        }
        return out;
    }
    for v in items {
        unsafe {
            rnx_array_push(out, *v, 8);
        }
    }
    unsafe { super::pretty::rnx_note_array_kind(out, super::pretty::KIND_ANY, 0) };
    out
}

fn gmap_key_release(krepr: u64) {
    gkey_release(krepr);
}

fn gmap_val_release(bits: u64) {
    gval_release(&GVal { kind: GKEY_OBJ, bits });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_gmap_keys(map: *const u8) -> *mut u8 {
    if tape_is_handle(map as u64) {
        let live = unsafe { tape_upgrade(map as u64) };
        let reprs = gmap_state(live).live_key_reprs();
        return gmap_box_array(&reprs, gmap_key_release);
    }
    let reprs = gmap_state(map as *mut u8).live_key_reprs();
    gmap_box_array(&reprs, gmap_key_release)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_gmap_values(map: *const u8) -> *mut u8 {
    if tape_is_handle(map as u64) {
        let live = unsafe { tape_upgrade(map as u64) };
        let vals = gmap_state(live).live_vals();
        return gmap_box_array(&vals, gmap_val_release);
    }
    let vals = gmap_state(map as *mut u8).live_vals();
    gmap_box_array(&vals, gmap_val_release)
}

const JSON_MAX_DEPTH: usize = 64;

pub(crate) static JSON_MAPS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<usize>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

pub(crate) fn json_map_add(map: *mut u8) {
    JSON_MAPS.lock().unwrap_or_else(|e| e.into_inner()).insert(map as usize);
}

pub(crate) fn json_map_remove(map: *mut u8) {
    JSON_MAPS.lock().unwrap_or_else(|e| e.into_inner()).remove(&(map as usize));
}

pub(crate) fn json_map_contains(bits: u64) -> bool {
    if bits == 0 || bits & ANY_BOX_MASK != 0 {
        return false;
    }
    JSON_MAPS.lock().unwrap_or_else(|e| e.into_inner()).contains(&(bits as usize))
}

pub(crate) fn json_panic_oom() -> ! {
    unsafe {
        rnx_panic(b"json out of memory\0".as_ptr(), "json out of memory".len());
    }
}

pub(crate) fn json_box_str(s: &str) -> u64 {
    let p = alloc_str(s);
    if p.is_null() {
        json_panic_oom();
    }
    let b = unsafe { rnx_any_box(TAG_STR, p as u64) };
    unsafe { rnx_release_str(p) };
    b
}

pub(crate) fn json_tape_err(msg: &str, offset: usize, is_depth: bool) -> ! {
    if is_depth {
        unsafe {
            rnx_panic(
                b"json max depth exceeded\0".as_ptr(),
                "json max depth exceeded".len(),
            );
        }
    }
    let text = format!("json parse error: {msg} at offset {offset}");
    unsafe {
        rnx_panic(text.as_ptr(), text.len());
    }
}

unsafe fn tape_box_tape_str(bytes: &[u8]) -> u64 {
    match std::str::from_utf8(bytes) {
        Ok(s) => json_box_str(s),
        Err(_) => json_box_str(&String::from_utf8_lossy(bytes)),
    }
}

unsafe fn tape_to_any(doc: &Arc<JsonDoc>, idx: usize) -> u64 {
    unsafe {
        match tape_tag(doc.nodes[idx]) {
            TAPE_NULL => 0,
            TAPE_BOOL => rnx_any_box(TAG_BOOL, bool_at(doc, idx) as u64),
            TAPE_INT => rnx_any_box(TAG_INT, int_at(doc, idx) as u64),
            TAPE_FLOAT => rnx_any_box(TAG_FLOAT, float_at(doc, idx).to_bits()),
            TAPE_STR => tape_box_tape_str(str_at(doc, idx)),
            TAPE_ARRAY => {
                let n = array_count(&doc.nodes, idx);
                let arr = rnx_array_new(n, 8);
                if arr.is_null() {
                    json_panic_oom();
                }
                for e in array_iter(&doc.nodes, idx) {
                    let v = tape_to_any(doc, e);
                    rnx_array_push(arr, v, 8);
                }
                rnx_heap_track(arr, HEAP_ARRAY, 8, json_elem_dtor_addr());
                arr as u64
            }
            _ => tape_materialize_fast(doc, idx).unwrap_or_else(|| tape_materialize(doc, idx)),
        }
    }
}

unsafe fn tape_key_matches(stored: &[u8], key: u64) -> bool {
    if unsafe { rnx_any_tag(key) } != TAG_STR as u64 {
        return false;
    }
    let p = unsafe { rnx_any_unbox(key) } as *const u8;
    if p.is_null() {
        return stored.is_empty();
    }
    tape_key_eq(stored, str_bytes(p))
}

unsafe fn tape_upgrade(handle: u64) -> *mut u8 {
    let live = tape_upgrade_get(handle);
    if !live.is_null() {
        return live;
    }
    let t = match tape_resolve(handle) {
        Some(t) => t,
        None => {
            unsafe {
                rnx_panic(b"null map handle\0".as_ptr(), "null map handle".len());
            }
        }
    };
    unsafe {
        let map = rnx_gmap_new();
        if map.is_null() {
            json_panic_oom();
        }
        json_map_add(map);
        tape_upgrade_set(handle, map);
        let root = t.root as usize;
        for (k, v) in object_iter(&t.doc.nodes, root) {
            let kb = tape_box_tape_str(key_at(&t.doc, k));
            let vb = tape_to_any(&t.doc, v);
            rnx_gmap_set(map, kb, vb);
            rnx_any_release(kb);
            rnx_any_release(vb);
        }
        map
    }
}

unsafe fn tape_free_map(handle: u64) {
    if let Some(up) = tape_release(handle) {
        if !up.is_null() {
            unsafe { json_deep_free_map(up) };
        }
    }
}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_json_parse_typed(text: *const u8, desc: *const u8) -> *mut u8 {
    let bytes = str_bytes(text);
    let dbytes = str_bytes(desc);
    let desc_text = match std::str::from_utf8(dbytes) {
        Ok(s) => s,
        Err(_) => "",
    };
    let fields = split_typed_desc(desc_text);
    let slots = match parse_typed(bytes, &fields) {
        Ok(s) => s,
        Err(e) => json_tape_err(&e.msg, e.offset, e.is_depth),
    };
    let arr = unsafe { rnx_array_new(slots.len(), 8) };
    if arr.is_null() {
        json_panic_oom();
    }
    for v in slots {
        let bits = unsafe {
            match v {
                TypedVal::Null => 0,
                TypedVal::Bool(b) => rnx_any_box(TAG_BOOL, b as u64),
                TypedVal::Int(n) => rnx_any_box(TAG_INT, n as u64),
                TypedVal::Float(f) => rnx_any_box(TAG_FLOAT, f.to_bits()),
                TypedVal::Str(bytes) => match String::from_utf8(bytes) {
                    Ok(s) => json_box_str(&s),
                    Err(e) => json_box_str(&String::from_utf8_lossy(e.as_bytes())),
                },
                TypedVal::Nested(doc) => tape_to_any(&Arc::new(doc), 0),
            }
        };
        unsafe { rnx_array_push(arr, bits, 8) };
    }
    unsafe { rnx_heap_track(arr, HEAP_ARRAY, 8, json_elem_dtor_addr()) };
    arr
}
pub(crate) fn json_elem_dtor_addr() -> u64 {
    rnx_json_release_elem as *const () as u64
}

pub(crate) fn json_proxy_dtor_addr() -> u64 {
    rnx_json_proxy_release as *const () as u64
}

unsafe fn json_release_bits(bits: u64) {
    if json_map_contains(bits) {
        unsafe { json_deep_free_map(bits as *mut u8) };
        return;
    }
    if unsafe { tape_release_elem(bits) } {
        return;
    }
    unsafe { rnx_any_release(bits) };
}

unsafe fn tape_release_elem(bits: u64) -> bool {
    if tape_release_fast(bits) {
        return true;
    }
    if !tape_is_handle(bits) {
        return false;
    }
    unsafe { tape_free_map(bits) };
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_json_release_elem(slot: *mut u8) {    if slot.is_null() {
        return;
    }
    let bits = unsafe { (slot as *const u64).read_unaligned() };
    unsafe { json_release_bits(bits) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_json_proxy_release(slot: *mut u8) {
    if slot.is_null() {
        return;
    }
    let bits = unsafe { (slot as *const u64).read_unaligned() };
    if json_map_contains(bits) {
        unsafe { json_deep_free_map(bits as *mut u8) };
    } else if unsafe { tape_release_elem(bits) } {
    } else {
        unsafe { rnx_any_release(bits) };
    }
}

unsafe fn json_deep_free_map(map: *mut u8) {
    if map.is_null() {
        return;
    }
    json_map_remove(map);
    let entries = gmap_state(map).take_all();
    for e in entries {
        if !e.active {
            continue;
        }
        unsafe { rnx_any_release(e.krepr) };
        let vb = e.val.bits;
        if json_map_contains(vb) {
            unsafe { json_deep_free_map(vb as *mut u8) };
        } else if unsafe { tape_release_elem(vb) } {
        } else {
            unsafe { rnx_any_release(vb) };
        }
    }
    unsafe {
        drop(Box::from_raw(map as *mut GenericMapState));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_json_parse(text: *const u8) -> u64 {
    let bytes = str_bytes(text);
    let doc = match parse_tape(bytes) {
        Ok(doc) => Arc::new(doc),
        Err(e) => json_tape_err(&e.msg, e.offset, e.is_depth),
    };
    let bits = unsafe { tape_to_any(&doc, 0) };
    if json_map_contains(bits) || tape_is_handle(bits) {
        let proxy = unsafe { rnx_array_new(1, 8) };
        if proxy.is_null() {
            unsafe { json_release_bits(bits) };
            json_panic_oom();
        }
        unsafe { rnx_array_push(proxy, bits, 8) };
        unsafe { rnx_heap_track(proxy, HEAP_ARRAY, 8, json_proxy_dtor_addr()) };
        return proxy as u64;
    }
    bits
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_json_unwrap(any: u64) -> u64 {
    if any == 0 {
        unsafe {
            rnx_panic(b"json object expected\0".as_ptr(), "json object expected".len());
        }
    }
    if json_map_contains(any) || tape_is_handle(any) {
        return any;
    }
    let tracked = heap_tracked(any);
    let is_proxy = match tracked {
        Some(t) => t.kind == HEAP_ARRAY && t.aux1 == 8 && t.aux2 == json_proxy_dtor_addr(),
        None => false,
    };
    if !is_proxy {
        unsafe {
            rnx_panic(b"json object expected\0".as_ptr(), "json object expected".len());
        }
    }
    let arr = any as *mut u8;
    if unsafe { rnx_array_len(arr as *const u8) } != 1 {
        unsafe {
            rnx_panic(b"json object expected\0".as_ptr(), "json object expected".len());
        }
    }
    let bits = unsafe { rnx_array_get(arr as *const u8, 0, 8) };
    heap_untrack(arr);
    unsafe { rnx_release_array(arr, 8, None) };
    if !json_map_contains(bits) && !tape_is_handle(bits) {
        unsafe {
            rnx_panic(b"json object expected\0".as_ptr(), "json object expected".len());
        }
    }
    bits
}

pub trait JsonSink {
    fn push_str(&mut self, s: &str);
    fn push_byte(&mut self, b: u8);
    fn reserve(&mut self, additional: usize);
}

impl JsonSink for String {
    fn push_str(&mut self, s: &str) {
        String::push_str(self, s);
    }
    fn push_byte(&mut self, b: u8) {
        String::push(self, b as char);
    }
    fn reserve(&mut self, additional: usize) {
        String::reserve(self, additional);
    }
}

pub struct JsonBufSink {
    ptr: *mut u8,
    cap: usize,
    len: usize,
}

impl JsonBufSink {
    fn write_bytes(&mut self, bytes: &[u8]) {
        if self.len.saturating_add(bytes.len()) > self.cap {
            unsafe {
                rnx_panic(
                    b"byte buffer out of bounds\0".as_ptr(),
                    "byte buffer out of bounds".len(),
                );
            }
        }
        if !bytes.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.add(self.len), bytes.len());
            }
            self.len += bytes.len();
        }
    }
}

impl JsonSink for JsonBufSink {
    fn push_str(&mut self, s: &str) {
        self.write_bytes(s.as_bytes());
    }
    fn push_byte(&mut self, b: u8) {
        self.write_bytes(&[b]);
    }
    fn reserve(&mut self, _additional: usize) {}
}

pub(crate) fn json_push_int(out: &mut impl JsonSink, n: i64) {
    if n == 0 {
        out.push_byte(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut mag = if n < 0 { (n as u64).wrapping_neg() } else { n as u64 };
    let mut i = 20;
    while mag > 0 {
        i -= 1;
        buf[i] = b'0' + (mag % 10) as u8;
        mag /= 10;
    }
    if n < 0 {
        i -= 1;
        buf[i] = b'-';
    }
    out.push_str(unsafe { std::str::from_utf8_unchecked(&buf[i..]) });
}

pub fn json_escape_into(out: &mut impl JsonSink, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => {
                let mut buf = [0u8; 4];
                out.push_str(c.encode_utf8(&mut buf));
            }
        }
    }
}

pub(crate) fn json_fmt_float(out: &mut impl JsonSink, f: f64) {
    if !f.is_finite() {
        out.push_str("null");
        return;
    }
    out.push_str(&fmt_float(f));
}

unsafe fn json_write_gmap(out: &mut impl JsonSink, map: *mut u8, depth: usize) {
    let pairs = gmap_state(map).snapshot();
    out.push_byte(b'{');
    let mut first = true;
    for (krepr, vbits) in &pairs {
        let krepr = *krepr;
        let vbits = *vbits;
        if any_box_addr(krepr).is_none()
            || (krepr & ANY_BOX_MASK) as u32 != ANY_BOX_STR as u32
        {
            continue;
        }
        let key = unsafe { rnx_any_unbox(krepr) } as *const u8;
        if key.is_null() {
            continue;
        }
        let ks = native_str(key);
        if !first {
            out.push_byte(b',');
        }
        first = false;
        out.push_byte(b'"');
        json_escape_into(out, &ks);
        out.push_byte(b'"');
        out.push_byte(b':');
        unsafe { json_write_native(out, vbits, depth + 1) };
    }
    out.push_byte(b'}');
    for (krepr, vbits) in pairs {
        gkey_release(krepr);
        gval_release(&GVal { kind: GKEY_OBJ, bits: vbits });
    }
}

unsafe fn json_write_tape_map(out: &mut impl JsonSink, handle: u64, depth: usize) {
    if let Some((doc, root)) = tape_resolve_fast(handle) {
        let up = tape_upgrade_get(handle);
        if !up.is_null() {
            unsafe { json_write_gmap(out, up, depth) };
            return;
        }
        let root = root as usize;
        out.reserve(object_count(&doc.nodes, root) * 64 + 16);
        tape_write_doc(out, &doc, root, depth);
        return;
    }
    let t = match tape_resolve(handle) {
        Some(t) => t,
        None => {
            out.push_str("null");
            return;
        }
    };
    if !t.upgraded.is_null() {
        unsafe { json_write_gmap(out, t.upgraded, depth) };
        return;
    }
    let root = t.root as usize;
    out.reserve(object_count(&t.doc.nodes, root) * 64 + 16);
    tape_write_doc(out, &t.doc, root, depth);
}

unsafe fn json_write_native(out: &mut impl JsonSink, any: u64, depth: usize) {
    if depth > JSON_MAX_DEPTH {
        unsafe {
            rnx_panic(b"json max depth exceeded\0".as_ptr(), "json max depth exceeded".len());
        }
    }
    if let Some(addr) = any_box_addr(any) {
        let payload = unsafe { ((addr as *const u8).add(8) as *const u64).read_unaligned() };
        match (any & ANY_BOX_MASK) as u32 {
            x if x == ANY_BOX_INT as u32 => json_push_int(out, payload as i64),
            x if x == ANY_BOX_BOOL as u32 => {
                out.push_str(if payload != 0 { "true" } else { "false" });
            }
            x if x == ANY_BOX_FLOAT as u32 => json_fmt_float(out, f64::from_bits(payload)),
            _ => {
                let s = native_str(payload as *const u8);
                out.push_byte(b'"');
                json_escape_into(out, &s);
                out.push_byte(b'"');
            }
        }
        return;
    }
    if any == 0 {
        out.push_str("null");
        return;
    }
    if tape_handle_id(any) != 0 && tape_resolve_fast(any).is_some() {
        unsafe { json_write_tape_map(out, any, depth) };
        return;
    }
    if json_map_contains(any) {
        unsafe { json_write_gmap(out, any as *mut u8, depth) };
        return;
    }
    if tape_is_handle(any) {
        unsafe { json_write_tape_map(out, any, depth) };
        return;
    }
    match heap_tracked(any) {
        Some(t) if t.kind == HEAP_ARRAY && t.aux1 == 8 && t.aux2 == json_elem_dtor_addr() => {
            let arr = any as *const u8;
            let n = arr_len(arr);
            let data = arr_data(arr);
            out.push_byte(b'[');
            for i in 0..n {
                if i > 0 {
                    out.push_byte(b',');
                }
                let bits = unsafe { (data.add(i.wrapping_mul(8)) as *const u64).read_unaligned() };
                unsafe { json_write_native(out, bits, depth + 1) };
            }
            out.push_byte(b']');
            return;
        }
        Some(t) if t.kind == HEAP_ARRAY && t.aux1 == 8 && t.aux2 == json_proxy_dtor_addr() => {
            let arr = any as *const u8;
            if arr_len(arr) == 1 {
                let data = arr_data(arr);
                let bits = unsafe { (data as *const u64).read_unaligned() };
                unsafe { json_write_native(out, bits, depth + 1) };
                return;
            }
        }
        _ => {}
    }
    out.push_str("null");
}

std::thread_local! {
    static JSON_FMT_BUF: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
}

pub(crate) fn json_fmt_begin() -> String {
    JSON_FMT_BUF.with(|b| std::mem::take(&mut *b.borrow_mut()))
}

pub(crate) fn json_fmt_end(out: String) {
    JSON_FMT_BUF.with(|b| {
        let mut slot = b.borrow_mut();
        if slot.capacity() < out.capacity() {
            *slot = out;
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_json_stringify(any: u64) -> *mut u8 {
    let mut out = json_fmt_begin();
    out.clear();
    unsafe { json_write_native(&mut out, any, 0) };
    let p = alloc_str(&out);
    json_fmt_end(out);
    p
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_json_stringify_into(any: u64, buf: *mut u8, pos: i64) -> i64 {
    if buf.is_null() || pos < 0 {
        unsafe {
            rnx_panic(b"byte buffer out of bounds\0".as_ptr(), "byte buffer out of bounds".len());
        }
    }
    let blen = bytes_len_impl(buf) as usize;
    let start = (pos as usize).min(blen);
    let mut out = JsonBufSink {
        ptr: unsafe { bytes_state(buf).data.as_mut_ptr().add(start) },
        cap: blen - start,
        len: 0,
    };
    unsafe { json_write_native(&mut out, any, 0) };
    out.len as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_free_roundtrip() {
        unsafe {
            let p = rnx_alloc(64, 8);
            assert!(!p.is_null());
            assert_eq!(p as usize % 8, 0);
            std::ptr::write_bytes(p, 0xAB, 64);
            assert_eq!(std::ptr::read(p), 0xAB);
            rnx_free(p, 64);
        }
    }

    #[test]
    fn alloc_zero_size() {
        unsafe {
            let p = rnx_alloc(0, 8);
            assert!(!p.is_null());
            rnx_free(p, 0);
        }
    }

    #[test]
    fn free_null_is_noop() {
        unsafe {
            rnx_free(std::ptr::null_mut(), 0);
        }
    }

    #[test]
    fn align_below_minimum_promoted() {
        unsafe {
            let p = rnx_alloc(16, 1);
            assert!(!p.is_null());
            assert_eq!(p as usize % 8, 0);
            rnx_free(p, 16);
        }
    }

    #[test]
    fn print_entry_points_run() {
        rnx_print_i64(42);
        unsafe {
            rnx_print_str(b"abc".as_ptr(), 3);
            rnx_print_str(std::ptr::null(), 0);
        }
    }

    #[test]
    fn retain_release_balanced() {
        unsafe {
            let p = rnx_alloc(16, 8);
            assert!(!p.is_null());
            (p as *mut u32).write(1);
            rnx_retain(p);
            rnx_retain(p);
            rnx_release(p, 16, None);
            rnx_release(p, 16, None);
            rnx_release(p, 16, None);
        }
    }

    #[test]
    fn retain_release_null_is_noop() {
        unsafe {
            rnx_retain(std::ptr::null_mut());
            rnx_release(std::ptr::null_mut(), 0, None);
        }
    }

    #[test]
    fn release_runs_dtor_once() {
        static HITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        unsafe extern "C" fn dtor(_ptr: *mut u8) {
            HITS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        unsafe {
            let p = rnx_alloc(16, 8);
            assert!(!p.is_null());
            (p as *mut u32).write(1);
            rnx_release(p, 16, Some(dtor));
            assert_eq!(HITS.load(std::sync::atomic::Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn closure_descs_parallel_churn_unique_ids() {
        const THREADS: usize = 16;
        const ITERS: usize = 1000;
        let seen = std::sync::Mutex::new(Vec::<u64>::new());
        std::thread::scope(|s| {
            for _ in 0..THREADS {
                s.spawn(|| unsafe {
                    let mut local = Vec::with_capacity(ITERS);
                    for i in 0..ITERS {
                        let b = rnx_closure_new(7, 2);
                        assert!(!b.is_null());
                        let id = closure_box_id(b);
                        assert_ne!(id, 0);
                        local.push(id);
                        let sp = alloc_str("churn");
                        rnx_closure_set(b, 0, TAG_STR as u64, sp as u64, 0, 0, 0, 0);
                        let obj = rnx_alloc(24, 8);
                        (obj as *mut u32).write(1);
                        rnx_closure_set(b, 1, TAG_OBJ as u64, obj as u64, 24, 0, 0, 0);
                        let _ = i;
                        rnx_closure_release(b);
                    }
                    seen.lock().unwrap().extend(local);
                });
            }
        });
        let ids = seen.lock().unwrap();
        assert_eq!(ids.len(), THREADS * ITERS);
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "closure box ids must be unique");
        let map = CLOSURE_DESCS.lock().unwrap();
        assert!(
            ids.iter().all(|id| !map.keys().any(|(bid, _)| bid == id)),
            "churned boxes must leave no CLOSURE_DESCS entries"
        );
    }
}

#[repr(C)]
pub(crate) struct GenSlot {
    pub(crate) target: usize,
    pub(crate) epoch: u32,
    pub(crate) _pad: u32,
}

pub(crate) const SLOT_POOL: usize = 1024;

pub(crate) struct SlotTable {
    slots: Vec<GenSlot>,
    free: Vec<u32>,
}

impl SlotTable {
    fn with_pool() -> SlotTable {
        let mut slots = Vec::with_capacity(SLOT_POOL);
        slots.resize_with(SLOT_POOL, || GenSlot {
            target: 0,
            epoch: 0,
            _pad: 0,
        });
        let mut free: Vec<u32> = (0..SLOT_POOL as u32).collect();
        free.reverse();
        SlotTable { slots, free }
    }

    fn create(&mut self, ptr: *mut u8, obj_epoch: u32) -> u64 {
        let idx = match self.free.pop() {
            Some(i) => i as usize,
            None => {
                let idx = self.slots.len();
                self.slots.push(GenSlot {
                    target: 0,
                    epoch: 0,
                    _pad: 0,
                });
                idx
            }
        };
        let slot = &mut self.slots[idx];
        let fresh = slot.epoch == 0 && slot.target == 0;
        slot.target = ptr as usize;
        slot.epoch = if fresh {
            obj_epoch
        } else {
            slot.epoch.wrapping_add(1).max(obj_epoch).max(1)
        };
        ((idx as u64 + 1) << 32) | (slot.epoch as u64)
    }

    fn get(&self, packed: u64) -> *mut u8 {
        let sid = (packed >> 32) as u32;
        let want = packed as u32;
        if sid == 0 {
            return std::ptr::null_mut();
        }
        match self.slots.get(sid as usize - 1) {
            Some(s) if s.target != 0 && s.epoch == want => s.target as *mut u8,
            _ => std::ptr::null_mut(),
        }
    }

    fn invalidate(&mut self, ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        let words = ptr as usize;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.target == words {
                slot.target = 0;
                slot.epoch = slot.epoch.wrapping_add(1).max(1);
                self.free.push(i as u32);
            }
        }
    }
}

pub(crate) static GENREF_TABLE: std::sync::LazyLock<std::sync::Mutex<SlotTable>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(SlotTable::with_pool()));

pub(crate) fn fnv1a(key: &str) -> u64 {
    let mut h: u64 = 0xCBF29CE484222325;
    for b in key.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001B3);
    }
    h
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotState {
    Empty,
    Live,
    Dead,
}

#[derive(Clone, Debug)]
pub(crate) struct MapSlot {
    pub(crate) hash: u64,
    pub(crate) index: usize,
    pub(crate) state: SlotState,
}

#[derive(Clone, Debug)]
pub struct MapEntry {
    pub key: String,
    pub val: u64,
    pub active: bool,
}

#[derive(Debug)]
pub struct NativeMapState {
    inner: RwLock<NativeMapStateInner>,
}

impl Clone for NativeMapState {
    fn clone(&self) -> NativeMapState {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        NativeMapState { inner: RwLock::new(inner.clone()) }
    }
}

impl Default for NativeMapState {
    fn default() -> NativeMapState {
        NativeMapState::new()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct NativeMapStateInner {
    pub(crate) entries: Vec<MapEntry>,
    pub(crate) slots: Vec<MapSlot>,
    pub(crate) count: usize,
}

impl NativeMapState {
    pub fn new() -> NativeMapState {
        NativeMapState {
            inner: RwLock::new(NativeMapStateInner {
                entries: Vec::new(),
                slots: vec![
                    MapSlot { hash: 0, index: 0, state: SlotState::Empty };
                    8
                ],
                count: 0,
            }),
        }
    }

    fn read<R>(&self, f: impl FnOnce(&NativeMapStateInner) -> R) -> R {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        f(&inner)
    }

    fn write<R>(&self, f: impl FnOnce(&mut NativeMapStateInner) -> R) -> R {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        f(&mut inner)
    }

    pub fn set(&self, key: &str, val: u64) {
        self.write(|st| st.set_inner(key, val));
    }

    pub fn get(&self, key: &str) -> u64 {
        self.read(|st| st.get_inner(key))
    }

    pub fn has(&self, key: &str) -> bool {
        self.read(|st| st.has_inner(key))
    }

    pub fn delete(&self, key: &str) -> bool {
        self.write(|st| st.delete_inner(key))
    }

    pub fn len(&self) -> usize {
        self.read(|st| st.count)
    }

    pub fn clear(&self) {
        self.write(|st| st.clear_inner());
    }

    pub fn keys(&self) -> Vec<String> {
        self.read(|st| st.keys_inner())
    }

    pub fn values(&self) -> Vec<u64> {
        self.read(|st| st.values_inner())
    }
}

impl NativeMapStateInner {

    fn find(&self, key: &str, hash: u64) -> Option<usize> {
        let mask = self.slots.len() - 1;
        let mut i = (hash as usize) & mask;
        loop {
            match self.slots[i].state {
                SlotState::Empty => return None,
                SlotState::Live => {
                    if self.slots[i].hash == hash && self.entries[self.slots[i].index].key == key {
                        return Some(self.slots[i].index);
                    }
                }
                SlotState::Dead => {}
            }
            i = (i + 1) & mask;
        }
    }

    fn place(&mut self, hash: u64, index: usize) {
        let mask = self.slots.len() - 1;
        let mut i = (hash as usize) & mask;
        loop {
            if self.slots[i].state != SlotState::Live {
                self.slots[i] = MapSlot { hash, index, state: SlotState::Live };
                return;
            }
            i = (i + 1) & mask;
        }
    }

    fn grow(&mut self) {
        let live: Vec<(u64, usize)> = self
            .slots
            .iter()
            .filter(|s| s.state == SlotState::Live)
            .map(|s| (s.hash, s.index))
            .collect();
        self.slots = vec![
            MapSlot { hash: 0, index: 0, state: SlotState::Empty };
            self.slots.len() * 2
        ];
        for (hash, index) in live {
            self.place(hash, index);
        }
    }

    fn set_inner(&mut self, key: &str, val: u64) {
        let hash = fnv1a(key);
        if let Some(ix) = self.find(key, hash) {
            self.entries[ix].val = val;
            return;
        }
        let ix = self.entries.len();
        self.entries.push(MapEntry { key: key.to_string(), val, active: true });
        self.count += 1;
        if self.count * 4 > self.slots.len() * 3 {
            self.grow();
        }
        self.place(hash, ix);
    }

    fn get_inner(&self, key: &str) -> u64 {
        let hash = fnv1a(key);
        self.find(key, hash).map(|ix| self.entries[ix].val).unwrap_or(0)
    }

    fn has_inner(&self, key: &str) -> bool {
        let hash = fnv1a(key);
        self.find(key, hash).is_some()
    }

    fn delete_inner(&mut self, key: &str) -> bool {
        let hash = fnv1a(key);
        let mask = self.slots.len() - 1;
        let mut i = (hash as usize) & mask;
        loop {
            match self.slots[i].state {
                SlotState::Empty => return false,
                SlotState::Live => {
                    if self.slots[i].hash == hash && self.entries[self.slots[i].index].key == key {
                        self.slots[i].state = SlotState::Dead;
                        self.entries[self.slots[i].index].active = false;
                        self.count -= 1;
                        return true;
                    }
                }
                SlotState::Dead => {}
            }
            i = (i + 1) & mask;
        }
    }

    fn clear_inner(&mut self) {
        self.entries.clear();
        for s in self.slots.iter_mut() {
            *s = MapSlot { hash: 0, index: 0, state: SlotState::Empty };
        }
        self.count = 0;
    }

    fn keys_inner(&self) -> Vec<String> {
        self.entries.iter().filter(|e| e.active).map(|e| e.key.clone()).collect()
    }

    fn values_inner(&self) -> Vec<u64> {
        self.entries.iter().filter(|e| e.active).map(|e| e.val).collect()
    }
}

pub(crate) fn map_of(map: *mut u8) -> Option<&'static NativeMapState> {
    if map.is_null() {
        return None;
    }
    Some(unsafe { &*(map as *mut NativeMapState) })
}

pub(crate) fn need_map(map: *mut u8) -> &'static NativeMapState {
    match map_of(map) {
        Some(m) => m,
        None => unsafe {
            rnx_panic(b"null map handle\0".as_ptr(), "null map handle".len());
        },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_map_new() -> *mut u8 {
    Box::into_raw(Box::new(NativeMapState::new())) as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_map_set(map: *mut u8, key: *const u8, val: u64) {
    need_map(map).set(&native_str(key), val);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_map_get(map: *const u8, key: *const u8) -> u64 {
    need_map(map as *mut u8).get(&native_str(key))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_map_has(map: *const u8, key: *const u8) -> bool {
    need_map(map as *mut u8).has(&native_str(key))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_map_delete(map: *mut u8, key: *const u8) -> bool {
    need_map(map).delete(&native_str(key))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_map_len(map: *const u8) -> usize {
    need_map(map as *mut u8).len()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_map_clear(map: *mut u8) {
    need_map(map).clear();
}

pub(crate) fn map_str_array(items: &[String]) -> *mut u8 {
    let out = unsafe { rnx_array_new(items.len(), 8) };
    if out.is_null() {
        return out;
    }
    for k in items {
        let s = alloc_str(k);
        if s.is_null() {
            return std::ptr::null_mut();
        }
        unsafe {
            rnx_array_push(out, s as u64, 8);
        }
    }
    out
}

pub(crate) fn map_int_array(items: &[u64]) -> *mut u8 {
    let out = unsafe { rnx_array_new(items.len(), 8) };
    if out.is_null() {
        return out;
    }
    for v in items {
        unsafe {
            rnx_array_push(out, *v, 8);
        }
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_map_keys(map: *const u8) -> *mut u8 {
    map_str_array(&need_map(map as *mut u8).keys())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_map_values(map: *const u8) -> *mut u8 {
    map_int_array(&need_map(map as *mut u8).values())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_release_map(map: *mut u8) {
    if map.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(map as *mut NativeMapState));
    }
}

pub(crate) fn sync_mutex(id: i64) -> std::sync::Arc<MutexEntry> {
    let mut table = SYNC_MUTEXES.lock().unwrap_or_else(|e| e.into_inner());
    table
        .entry(id)
        .or_insert_with(|| {
            std::sync::Arc::new(MutexEntry {
                state: std::sync::Mutex::new(false),
                cv: std::sync::Condvar::new(),
            })
        })
        .clone()
}

pub(crate) fn sync_rwlock(id: i64) -> std::sync::Arc<RwLockEntry> {
    let mut table = SYNC_RWLOCKS.lock().unwrap_or_else(|e| e.into_inner());
    table
        .entry(id)
        .or_insert_with(|| {
            std::sync::Arc::new(RwLockEntry {
                state: std::sync::Mutex::new(RwState { readers: 0, writer: false }),
                cv: std::sync::Condvar::new(),
            })
        })
        .clone()
}

pub(crate) fn lock_state(e: &std::sync::Arc<MutexEntry>) -> std::sync::MutexGuard<'_, bool> {
    e.state.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn task_live(handle: *mut u8) -> bool {
    !handle.is_null()
        && LIVE_TASKS.lock().unwrap_or_else(|e| e.into_inner()).contains(&(handle as usize))
}

pub(crate) fn task_box_of(handle: *mut u8) -> Option<&'static TaskBox> {
    if handle.is_null() {
        return None;
    }
    unsafe { Some(&*(handle as *const TaskBox)) }
}

pub(crate) fn task_complete(tb: &TaskBox, tag: u32, payload: u64, err: Option<String>) {
    let mut s = tb.slot.lock().unwrap_or_else(|e| e.into_inner());
    s.tag = tag;
    s.payload = payload;
    s.err = err;
    s.done = true;
    tb.cv.notify_all();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_genref_create(ptr: *mut u8) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    let obj_epoch = unsafe { ((ptr as *const u8).add(4) as *const u32).read_unaligned() };
    match GENREF_TABLE.lock() {
        Ok(mut t) => t.create(ptr, obj_epoch),
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_genref_get(packed_ref: u64) -> *mut u8 {
    match GENREF_TABLE.lock() {
        Ok(t) => t.get(packed_ref),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_genref_invalidate(ptr: *mut u8) {
    if let Ok(mut t) = GENREF_TABLE.lock() {
        t.invalidate(ptr);
    }
}

#[cfg(test)]
mod genref_tests {
    use super::*;

    fn live_ptr() -> *mut u8 {
        unsafe {
            let p = rnx_alloc(32, 8);
            assert!(!p.is_null());
            (p as *mut u32).write(1);
            ((p as *mut u8).add(4) as *mut u32).write(0);
            p
        }
    }

    #[test]
    fn create_get_roundtrip() {
        unsafe {
            let p = live_ptr();
            let packed = rnx_genref_create(p);
            assert_ne!(packed, 0);
            assert_eq!(rnx_genref_get(packed), p);
            rnx_genref_invalidate(p);
            assert!(rnx_genref_get(packed).is_null());
            rnx_release(p, 32, None);
        }
    }

    #[test]
    fn null_inputs_stay_null() {
        unsafe {
            assert_eq!(rnx_genref_create(std::ptr::null_mut()), 0);
            assert!(rnx_genref_get(0).is_null());
            assert!(rnx_genref_get(u64::MAX).is_null());
            rnx_genref_invalidate(std::ptr::null_mut());
        }
    }

    #[test]
    fn invalidate_clears_all_matches() {
        unsafe {
            let p = live_ptr();
            let a = rnx_genref_create(p);
            let b = rnx_genref_create(p);
            assert_ne!(a, b);
            assert_eq!(rnx_genref_get(a), p);
            rnx_genref_invalidate(p);
            assert!(rnx_genref_get(a).is_null());
            assert!(rnx_genref_get(b).is_null());
            rnx_release(p, 32, None);
        }
    }

    #[test]
    fn reused_slot_rejects_stale_epoch() {
        unsafe {
            let p = live_ptr();
            let stale = rnx_genref_create(p);
            rnx_genref_invalidate(p);
            rnx_release(p, 32, None);
            let q = live_ptr();
            let fresh = rnx_genref_create(q);
            assert!(rnx_genref_get(stale).is_null());
            assert_eq!(rnx_genref_get(fresh), q);
            rnx_genref_invalidate(q);
            rnx_release(q, 32, None);
        }
    }
}

pub(crate) fn arr_len(ptr: *const u8) -> usize {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (ptr.add(16) as *const u64).read_unaligned() as usize }
}

pub(crate) fn arr_cap(ptr: *const u8) -> usize {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (ptr.add(24) as *const u64).read_unaligned() as usize }
}

pub(crate) fn arr_data(ptr: *const u8) -> *mut u8 {
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { (ptr.add(32) as *const *mut u8).read_unaligned() }
}

pub(crate) fn arr_write(ptr: *mut u8, len: usize, cap: usize, data: *mut u8) {
    unsafe {
        (ptr.add(16) as *mut u64).write_unaligned(len as u64);
        (ptr.add(24) as *mut u64).write_unaligned(cap as u64);
        (ptr.add(32) as *mut *mut u8).write_unaligned(data);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_new(initial_cap: usize, elem_size: usize) -> *mut u8 {
    let out = unsafe { rnx_alloc(ARR_HEADER, 8) };
    if out.is_null() {
        return out;
    }
    unsafe {
        (out as *mut u32).write_unaligned(1);
        ((out as *mut u8).add(4) as *mut u32).write_unaligned(0);
        ((out as *mut u8).add(8) as *mut u64).write_unaligned(0);
    }
    let data = if initial_cap > 0 && elem_size > 0 {
        unsafe { rnx_alloc(initial_cap.wrapping_mul(elem_size), 8) }
    } else {
        std::ptr::null_mut()
    };
    if initial_cap > 0 && elem_size > 0 && data.is_null() {
        unsafe { rnx_free(out, ARR_HEADER) };
        return std::ptr::null_mut();
    }
    arr_write(out, 0, initial_cap, data);
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_len(arr: *const u8) -> usize {
    arr_len(arr)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_push(arr: *mut u8, elem_val: u64, elem_size: usize) {
    if arr.is_null() || elem_size == 0 {
        return;
    }
    unsafe {
        let (mut len, mut cap) = (arr_len(arr), arr_cap(arr));
        let mut data = arr_data(arr);
        if len == cap {
            let ncap = if cap == 0 { 4 } else { cap.wrapping_mul(2) };
            let nbytes = ncap.wrapping_mul(elem_size);
            let grown = rnx_alloc(nbytes, 8);
            if grown.is_null() {
                rnx_panic(b"array push out of memory\0".as_ptr(), "array push out of memory".len());
            }
            if !data.is_null() && len > 0 {
                std::ptr::copy_nonoverlapping(data, grown, len.wrapping_mul(elem_size));
                rnx_free(data, cap.wrapping_mul(elem_size));
            }
            data = grown;
            cap = ncap;
        }
        let slot = data.add(len.wrapping_mul(elem_size));
        if elem_size == 8 {
            (slot as *mut u64).write_unaligned(elem_val);
        } else {
            std::ptr::copy_nonoverlapping(
                &elem_val as *const u64 as *const u8,
                slot,
                elem_size.min(8),
            );
        }
        len += 1;
        arr_write(arr, len, cap, data);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_slice(
    arr: *const u8,
    start: i64,
    end: i64,
    inclusive: i64,
    elem_size: usize,
    retain_elems: u64,
) -> *mut u8 {
    let n = arr_len(arr) as i64;
    let mut lo = start.clamp(0, n);
    let mut hi = if inclusive != 0 { end.saturating_add(1) } else { end }.clamp(0, n);
    if lo >= hi {
        lo = 0;
        hi = 0;
    }
    let count = (hi - lo) as usize;
    let out = unsafe { rnx_array_new(count, elem_size) };
    if out.is_null() || count == 0 || elem_size == 0 {
        return out;
    }
    unsafe {
        let src = arr_data(arr).add((lo as usize).wrapping_mul(elem_size));
        let dst = arr_data(out);
        std::ptr::copy_nonoverlapping(src, dst, count.wrapping_mul(elem_size));
        arr_write(out, count, count, dst);
        if retain_elems != 0 && elem_size == 8 {
            for i in 0..count {
                let p = (dst.add(i.wrapping_mul(8)) as *const u64).read_unaligned();
                if !(p as *const u8).is_null() {
                    rnx_retain(p as *mut u8);
                }
            }
        }
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_pop(arr: *mut u8, elem_size: usize) -> u64 {    if arr.is_null() || elem_size == 0 {
        return 0;
    }
    unsafe {
        let len = arr_len(arr);
        if len == 0 {
            return 0;
        }
        let data = arr_data(arr);
        if data.is_null() {
            return 0;
        }
        let slot = data.add(len.wrapping_sub(1).wrapping_mul(elem_size));
        let out = if elem_size == 8 {
            (slot as *const u64).read_unaligned()
        } else {
            let mut buf: u64 = 0;
            std::ptr::copy_nonoverlapping(
                slot,
                &mut buf as *mut u64 as *mut u8,
                elem_size.min(8),
            );
            buf
        };
        arr_write(arr, len - 1, arr_cap(arr), data);
        out
    }
}

// Boxed-scalar carrier for `Any` slots on native backends. A scalar stored
// as `Any` is heap-boxed and the pointer is tagged in its low 3 bits
// (all native heap allocations are 8-aligned, so genuine pointers always
// have zero low bits). Box layout is [count: AtomicU64][payload: u64].
// The count is atomic because boxes cross threads through channels, tasks,
// and shared maps; retain is fetch_add and release frees exactly when
// fetch_sub returns 1, so only one thread observes the terminal transition.
// Membership in ANY_BOXES gates every access, so arbitrary integers or
// foreign pointers are never dereferenced and pass through untouched.
pub(crate) const ANY_BOX_MASK: u64 = 0b111;
pub(crate) const ANY_BOX_INT: u64 = 0b001;
pub(crate) const ANY_BOX_BOOL: u64 = 0b010;
pub(crate) const ANY_BOX_FLOAT: u64 = 0b011;
pub(crate) const ANY_BOX_STR: u64 = 0b100;
pub(crate) const ANY_BOX_SIZE: usize = 16;

static ANY_BOXES: std::sync::LazyLock<std::sync::Mutex<BoxSet>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(BoxSet::new()));

pub(crate) const BOX_EMPTY: usize = 0;
pub(crate) const BOX_TOMBSTONE: usize = usize::MAX;

pub(crate) struct BoxSet {
    table: Vec<usize>,
    len: usize,
    tomb: usize,
}

fn box_hash(addr: usize) -> u64 {
    let mut x = ((addr >> 4) as u64).wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

impl BoxSet {
    fn new() -> BoxSet {
        BoxSet { table: vec![BOX_EMPTY; 16], len: 0, tomb: 0 }
    }

    fn slot(&self, addr: usize) -> usize {
        (box_hash(addr) as usize) & (self.table.len() - 1)
    }

    fn contains(&self, addr: usize) -> bool {
        if addr == BOX_EMPTY || addr == BOX_TOMBSTONE {
            return false;
        }
        let mut i = self.slot(addr);
        loop {
            let v = self.table[i];
            if v == BOX_EMPTY {
                return false;
            }
            if v == addr {
                return true;
            }
            i = (i + 1) & (self.table.len() - 1);
        }
    }

    fn grow(&mut self) {
        let new_len = self.table.len() * 2;
        let old = std::mem::replace(&mut self.table, vec![BOX_EMPTY; new_len]);
        self.len = 0;
        self.tomb = 0;
        for v in old {
            if v != BOX_EMPTY && v != BOX_TOMBSTONE {
                self.insert_raw(v);
            }
        }
    }

    fn insert_raw(&mut self, addr: usize) {
        let mut i = self.slot(addr);
        loop {
            let v = self.table[i];
            if v == BOX_TOMBSTONE {
                self.table[i] = addr;
                self.len += 1;
                self.tomb -= 1;
                return;
            }
            if v == BOX_EMPTY {
                self.table[i] = addr;
                self.len += 1;
                return;
            }
            if v == addr {
                return;
            }
            i = (i + 1) & (self.table.len() - 1);
        }
    }

    fn insert(&mut self, addr: usize) {
        if addr == BOX_EMPTY || addr == BOX_TOMBSTONE {
            return;
        }
        // Tombstones are dead probe stops: contains and remove skip past
        // them and only terminate on EMPTY, so a table with no EMPTY slot
        // loops forever. removes never consume EMPTY, only inserts do, and
        // every insert passes through here, so counting tombstones toward
        // the load keeps at least one EMPTY slot alive at all times.
        if (self.len + self.tomb) * 4 >= self.table.len() * 3 {
            self.grow();
        }
        self.insert_raw(addr);
    }

    fn remove(&mut self, addr: usize) {
        if addr == BOX_EMPTY || addr == BOX_TOMBSTONE {
            return;
        }
        let mut i = self.slot(addr);
        loop {
            let v = self.table[i];
            if v == BOX_EMPTY {
                return;
            }
            if v == addr {
                self.table[i] = BOX_TOMBSTONE;
                self.len -= 1;
                self.tomb += 1;
                return;
            }
            i = (i + 1) & (self.table.len() - 1);
        }
    }
}

// Tracked-heap registry for erased (`Any`-typed) ownership on native
// backends. The interpreter tags every value, but JIT/LLVM traffic
// untagged u64, so an `Any`-typed slot holding a heap object cannot be
// distinguished from a scalar statically. Registration records how to
// destroy the object; any_retain/any_release consult the registry so
// erased stores and drops pair exactly like their typed counterparts.
// Untracked values pass through untouched, preserving prior behavior.
pub const HEAP_ARRAY: u64 = 1;
pub const HEAP_STR: u64 = 2;
pub const HEAP_OBJ: u64 = 3;
pub const HEAP_ENUM: u64 = 4;
pub const HEAP_BOX: u64 = 5;

pub(crate) struct TrackInfo {
    kind: u64,
    aux1: u64,
    aux2: u64,
}

static HEAP_TRACK: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeMap<usize, TrackInfo>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(crate) fn heap_kind_of(any: u64) -> Option<u64> {
    heap_tracked(any).map(|t| t.kind)
}

pub(crate) fn gmap_live_ptr(handle: *mut u8) -> *mut u8 {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        if tape_is_handle(handle as u64) {
            tape_upgrade(handle as u64)
        } else {
            handle
        }
    }
}

pub(crate) fn heap_tracked(any: u64) -> Option<TrackInfo> {
    if any & ANY_BOX_MASK != 0 {
        return None;
    }
    HEAP_TRACK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(any as usize))
        .map(|t| TrackInfo { kind: t.kind, aux1: t.aux1, aux2: t.aux2 })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_heap_track(ptr: *mut u8, kind: u64, aux1: u64, aux2: u64) {
    if ptr.is_null() || kind == 0 {
        return;
    }
    HEAP_TRACK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(ptr as usize, TrackInfo { kind, aux1, aux2 });
}

pub(crate) fn heap_untrack(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    HEAP_TRACK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(ptr as usize));
    super::pretty::pretty_untrack(ptr as usize);
}

pub(crate) fn heap_destroy(ptr: *mut u8, info: &TrackInfo) {
    unsafe {
        match info.kind {
            HEAP_ARRAY => {
                let cap = arr_cap(ptr);
                let len = arr_len(ptr);
                let data = arr_data(ptr);
                if info.aux2 != 0 {
                    let run: unsafe extern "C" fn(*mut u8) = std::mem::transmute(info.aux2 as usize);
                    for i in 0..len {
                        run(data.add(i.wrapping_mul(info.aux1 as usize)));
                    }
                }
                if !data.is_null() {
                    rnx_free(data, cap.wrapping_mul(info.aux1 as usize));
                }
                rnx_free(ptr, ARR_HEADER);
            }
            HEAP_STR => {
                let len = str_len(ptr);
                rnx_free(ptr, str_body_size(len));
            }
            HEAP_OBJ | HEAP_ENUM => {
                if info.aux1 != 0 {
                    let run: unsafe extern "C" fn(*mut u8) = std::mem::transmute(info.aux1 as usize);
                    run(ptr);
                }
                rnx_free(ptr, info.aux2 as usize);
            }
            HEAP_BOX => {
                rnx_closure_dtor(ptr);
                let ncaps = (ptr.add(16) as *const u64).read_unaligned() as usize;
                rnx_free(ptr, closure_box_size(ncaps));
            }
            _ => {}
        }
    }
}

pub(crate) fn any_box_marker(tag: u32) -> u64 {
    match tag {
        TAG_INT => ANY_BOX_INT,
        TAG_BOOL => ANY_BOX_BOOL,
        TAG_FLOAT => ANY_BOX_FLOAT,
        TAG_STR => ANY_BOX_STR,
        _ => 0,
    }
}

pub(crate) fn any_box_addr(any: u64) -> Option<usize> {
    if any & ANY_BOX_MASK == 0 {
        return None;
    }
    let addr = (any & !ANY_BOX_MASK) as usize;
    if ANY_BOXES.lock().unwrap_or_else(|e| e.into_inner()).contains(addr) {
        Some(addr)
    } else {
        None
    }
}

pub(crate) fn box_count(addr: usize) -> &'static AtomicU64 {
    debug_assert_eq!(addr % 8, 0);
    unsafe { &*(addr as *const AtomicU64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_box(tag: u32, payload: u64) -> u64 {
    if any_box_addr(payload).is_some() {
        unsafe { rnx_any_retain(payload) };
        return payload;
    }
    let marker = any_box_marker(tag);
    if marker == 0 {
        return payload;
    }
    unsafe {
        let out = rnx_alloc(ANY_BOX_SIZE, 8);
        if out.is_null() {
            return payload;
        }
        if marker == ANY_BOX_STR && payload != 0 && payload % 8 == 0 {
            rnx_retain(payload as *mut u8);
        }
        (out as *mut u64).write_unaligned(1);
        (out.add(8) as *mut u64).write_unaligned(payload);
        ANY_BOXES.lock().unwrap_or_else(|e| e.into_inner()).insert(out as usize);
        out as u64 | marker
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_unbox(any: u64) -> u64 {
    match any_box_addr(any) {
        Some(addr) => unsafe { ((addr as *const u8).add(8) as *const u64).read_unaligned() },
        None => any,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_unbox_heap(any: u64) -> u64 {
    match any_box_addr(any) {
        Some(addr) if (any & ANY_BOX_MASK) as u32 == ANY_BOX_STR as u32 => unsafe {
            ((addr as *const u8).add(8) as *const u64).read_unaligned()
        },
        _ => any,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_release_box(any: u64) {
    if let Some(addr) = any_box_addr(any) {
        if box_count(addr).fetch_sub(1, Ordering::SeqCst) == 1 {
            unsafe {
                if any & ANY_BOX_MASK == ANY_BOX_STR {
                    let payload = ((addr as *const u8).add(8) as *const u64).read_unaligned();
                    if payload != 0 && payload % 8 == 0 {
                        rnx_release_str(payload as *mut u8);
                    }
                }
                ANY_BOXES.lock().unwrap_or_else(|e| e.into_inner()).remove(addr);
                rnx_free(addr as *mut u8, ANY_BOX_SIZE);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_tag(any: u64) -> u64 {
    if any_box_addr(any).is_some() {
        match (any & ANY_BOX_MASK) as u32 {
            x if x == ANY_BOX_INT as u32 => TAG_INT as u64,
            x if x == ANY_BOX_BOOL as u32 => TAG_BOOL as u64,
            x if x == ANY_BOX_FLOAT as u32 => TAG_FLOAT as u64,
            _ => TAG_STR as u64,
        }
    } else {
        TAG_PTR as u64
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_obj_class(ptr: *const u8) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    if any_box_addr(ptr as u64).is_some() {
        return 0;
    }
    unsafe { ((ptr.add(8)) as *const u64).read_unaligned() }
}

thread_local! {
    static PENDING_ERROR: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[inline]
pub(crate) fn error_is_object(w: u64) -> bool {
    w & 1 == 1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_error_set(payload: u64) {
    PENDING_ERROR.with(|c| c.set(payload));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_error_take() -> u64 {
    PENDING_ERROR.with(|c| c.replace(0))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_error_class(w: u64) -> u64 {
    if !error_is_object(w) {
        return 0;
    }
    unsafe { rnx_obj_class((w ^ 1) as *const u8) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_error_unbox(w: u64) -> u64 {
    if error_is_object(w) {
        w ^ 1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_error_str(w: u64) -> *mut u8 {    if error_is_object(w) {
        let id = unsafe { rnx_obj_class((w ^ 1) as *const u8) };
        if id == 0 {
            return alloc_str("<object>");
        }
        static ERROR_NAMES: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeMap<u64, u64>>> =
            std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));
        let mut names = ERROR_NAMES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = names.get(&id).copied() {
            return p as *mut u8;
        }
        let name = TYPE_NAMES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned()
            .unwrap_or_else(|| "<object>".to_string());
        let p = alloc_str(&name) as u64;
        names.insert(id, p);
        return p as *mut u8;
    }
    if w == 0 {
        return alloc_str("");
    }
    w as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_error_release(w: u64) {
    if w == 0 {
        return;
    }
    if error_is_object(w) {
        unsafe { rnx_any_release(w ^ 1) };
    } else {
        unsafe { rnx_release_str(w as *mut u8) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_report_uncaught(w: u64) -> i32 {
    if w == 0 {
        return 0;
    }
    let msg = if error_is_object(w) {
        let id = unsafe { rnx_obj_class((w ^ 1) as *const u8) };
        if id == 0 {
            "<object>".to_string()
        } else {
            TYPE_NAMES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "<object>".to_string())
        }
    } else {
        String::from_utf8_lossy(str_bytes(w as *const u8)).into_owned()
    };
    eprintln!("Uncaught exception: {msg}");
    if !error_is_object(w) {
        unsafe { rnx_release_str(w as *mut u8) };
    }
    1
}

pub fn take_uncaught_message() -> Option<String> {
    let w = PENDING_ERROR.with(|c| c.replace(0));
    if w == 0 {
        return None;
    }
    let msg = if error_is_object(w) {
        let id = unsafe { rnx_obj_class((w ^ 1) as *const u8) };
        if id == 0 {
            "<object>".to_string()
        } else {
            TYPE_NAMES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "<object>".to_string())
        }
    } else {
        String::from_utf8_lossy(str_bytes(w as *const u8)).into_owned()
    };
    if !error_is_object(w) {
        unsafe { rnx_release_str(w as *mut u8) };
    }
    Some(msg)
}

pub(crate) static TYPE_NAMES: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeMap<u64, String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_note_type(idx: u64, name: *const u8) {
    if name.is_null() {
        return;
    }
    let text = native_str(name).to_string();
    TYPE_NAMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(idx)
        .or_insert(text);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_type_name(idx: u64) -> *mut u8 {
    let found = TYPE_NAMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&idx)
        .cloned();
    match found {
        Some(name) => alloc_str(&name),
        None => alloc_str("Unknown"),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_typeof_any(val: u64) -> *mut u8 {
    unsafe {
        let tag = rnx_any_tag(val);
        if tag == TAG_INT as u64 {
            return alloc_str("Int");
        }
        if tag == TAG_BOOL as u64 {
            return alloc_str("Bool");
        }
        if tag == TAG_FLOAT as u64 {
            return alloc_str("Float");
        }
        if tag == TAG_STR as u64 {
            return alloc_str("String");
        }
        let idx = rnx_obj_class(val as *const u8);
        if idx == 0 {
            return alloc_str("Unknown");
        }
        rnx_type_name(idx)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_retain(any: u64) {
    if let Some(addr) = any_box_addr(any) {
        box_count(addr).fetch_add(1, Ordering::SeqCst);
        return;
    }
    if heap_tracked(any).is_some() {
        retain_count(any as *mut u8);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_any_release(any: u64) {
    if let Some(addr) = any_box_addr(any) {
        if box_count(addr).fetch_sub(1, Ordering::SeqCst) == 1 {
            unsafe {
                if any & ANY_BOX_MASK == ANY_BOX_STR {
                    let payload = ((addr as *const u8).add(8) as *const u64).read_unaligned();
                    if payload != 0 && payload % 8 == 0 {
                        rnx_release_str(payload as *mut u8);
                    }
                }
                ANY_BOXES.lock().unwrap_or_else(|e| e.into_inner()).remove(addr);
                rnx_free(addr as *mut u8, ANY_BOX_SIZE);
            }
        }
        return;
    }
    let info = match heap_tracked(any) {
        Some(t) => t,
        None => return,
    };
    let ptr = any as *mut u8;
    if !release_count(ptr) {
        return;
    }
    heap_untrack(ptr);
    heap_destroy(ptr, &info);
}

pub(crate) fn any_box_tag(any: u64) -> Option<u32> {
    if any_box_addr(any).is_none() {
        return None;
    }
    match any & ANY_BOX_MASK {
        ANY_BOX_INT => Some(TAG_INT),
        ANY_BOX_BOOL => Some(TAG_BOOL),
        ANY_BOX_FLOAT => Some(TAG_FLOAT),
        ANY_BOX_STR => Some(TAG_STR),
        _ => None,
    }
}

pub(crate) static CLOSURE_CODE: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeMap<u64, usize>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

// Closure dispatch tags. JIT backends compile many modules in one
// process (tests, REPL, hosts), and every module numbers its closures
// from zero, so a bare function index is not a unique key. Each in-process
// JIT compile claims a fresh epoch; boxes and registry entries carry
// `(epoch << 32) | fid`, which can never collide across modules.
// Epoch 0 means "plain fid" and is used for ahead-of-time objects, which
// are self-consistent single-module processes and must stay byte
// deterministic across builds. Entries are never removed: a stale entry
// is inert (its tag can never be re-minted), and dropping a JIT while
// its detached workers still run keeps the same behavior as before
// instead of failing closed.

pub const CLOSURE_FID_MASK: u64 = 0xFFFF_FFFF;

pub(crate) static CLOSURE_EPOCH_NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

thread_local! {
    static CLOSURE_TAG_EPOCH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub fn rnx_claim_closure_epoch() -> u64 {
    let mut e = CLOSURE_EPOCH_NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if e == 0 || e > CLOSURE_FID_MASK {
        e = CLOSURE_EPOCH_NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst) & CLOSURE_FID_MASK;
        if e == 0 {
            e = 1;
        }
    }
    e
}

pub fn rnx_set_closure_epoch(epoch: u64) {
    CLOSURE_TAG_EPOCH.set(epoch);
}

pub fn rnx_closure_tag(fid: u64) -> u64 {
    let e = CLOSURE_TAG_EPOCH.get() & CLOSURE_FID_MASK;
    if e == 0 {
        return fid;
    }
    (e << 32) | (fid & CLOSURE_FID_MASK)
}

#[unsafe(no_mangle)]
pub fn rnx_closure_register(fid: u64, addr: usize) {
    CLOSURE_CODE.lock().unwrap_or_else(|e| e.into_inner()).insert(fid, addr);
}

pub(crate) fn closure_code(fid: u64) -> Option<usize> {
    CLOSURE_CODE.lock().unwrap_or_else(|e| e.into_inner()).get(&fid).copied()
}

pub(crate) fn closure_args(box_ptr: *const u8) -> Option<(u64, Vec<u64>)> {
    if box_ptr.is_null() {
        return None;
    }
    unsafe {
        let fid = (box_ptr.add(8) as *const u64).read_unaligned();
        let ncaps = (box_ptr.add(16) as *const u64).read_unaligned() as usize;
        if ncaps > 8 {
            return None;
        }
        let mut caps = Vec::with_capacity(ncaps);
        for i in 0..ncaps {
            caps.push((box_ptr.add(32).add(i.wrapping_mul(16)) as *const u64).read_unaligned());
        }
        Some((fid, caps))
    }
}

pub(crate) fn closure_call(code: usize, caps: &[u64]) -> u64 {
    unsafe {
        match caps.len() {
            0 => {
                let f: unsafe extern "C" fn() -> u64 = std::mem::transmute(code);
                f()
            }
            1 => {
                let f: unsafe extern "C" fn(u64) -> u64 = std::mem::transmute(code);
                f(caps[0])
            }
            2 => {
                let f: unsafe extern "C" fn(u64, u64) -> u64 = std::mem::transmute(code);
                f(caps[0], caps[1])
            }
            3 => {
                let f: unsafe extern "C" fn(u64, u64, u64) -> u64 = std::mem::transmute(code);
                f(caps[0], caps[1], caps[2])
            }
            4 => {
                let f: unsafe extern "C" fn(u64, u64, u64, u64) -> u64 = std::mem::transmute(code);
                f(caps[0], caps[1], caps[2], caps[3])
            }
            5 => {
                let f: unsafe extern "C" fn(u64, u64, u64, u64, u64) -> u64 =
                    std::mem::transmute(code);
                f(caps[0], caps[1], caps[2], caps[3], caps[4])
            }
            6 => {
                let f: unsafe extern "C" fn(u64, u64, u64, u64, u64, u64) -> u64 =
                    std::mem::transmute(code);
                f(caps[0], caps[1], caps[2], caps[3], caps[4], caps[5])
            }
            7 => {
                #[allow(clippy::too_many_arguments)]
                let f: unsafe extern "C" fn(u64, u64, u64, u64, u64, u64, u64) -> u64 =
                    std::mem::transmute(code);
                f(caps[0], caps[1], caps[2], caps[3], caps[4], caps[5], caps[6])
            }
            _ => {
                #[allow(clippy::too_many_arguments)]
                let f: unsafe extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> u64 =
                    std::mem::transmute(code);
                f(caps[0], caps[1], caps[2], caps[3], caps[4], caps[5], caps[6], caps[7])
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_closure_invoke0(box_ptr: *mut u8) -> u64 {
    match closure_args(box_ptr) {
        Some((fid, caps)) => match closure_code(fid) {
            Some(code) => closure_call(code, &caps),
            None => unsafe {
                rnx_panic(b"unknown closure\0".as_ptr(), "unknown closure".len());
            }
        },
        None => unsafe {
            rnx_panic(b"bad closure\0".as_ptr(), "bad closure".len());
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_closure_invoke1(box_ptr: *mut u8, arg: u64) -> u64 {
    match closure_args(box_ptr) {
        Some((fid, caps)) => match closure_code(fid) {
            Some(code) => {
                let mut full = Vec::with_capacity(caps.len() + 1);
                full.push(arg);
                full.extend(caps);
                closure_call(code, &full)
            }
            None => unsafe {
                rnx_panic(b"unknown closure\0".as_ptr(), "unknown closure".len());
            }
        },
        None => unsafe {
            rnx_panic(b"bad closure\0".as_ptr(), "bad closure".len());
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_closure_new(fid: u64, ncaps: usize) -> *mut u8 {
    let size = 32usize.wrapping_add(ncaps.wrapping_mul(16));
    unsafe {
        let out = rnx_alloc(size, 8);
        if out.is_null() {
            return out;
        }
        let id = NEXT_CLOSURE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = if id == 0 {
            NEXT_CLOSURE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        } else {
            id
        };
        (out as *mut u32).write_unaligned(1);
        (out.add(8) as *mut u64).write_unaligned(fid);
        (out.add(16) as *mut u64).write_unaligned(ncaps as u64);
        (out.add(24).add(ncaps.wrapping_mul(16)) as *mut u64).write_unaligned(id);
        out
    }
}

pub(crate) static NEXT_CLOSURE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) fn closure_box_ncaps(b: *const u8) -> usize {
    if b.is_null() {
        return 0;
    }
    unsafe { (b.add(16) as *const u64).read_unaligned() as usize }
}

pub(crate) fn closure_box_id(b: *const u8) -> u64 {
    if b.is_null() {
        return 0;
    }
    unsafe {
        (b.add(24).add(closure_box_ncaps(b).wrapping_mul(16)) as *const u64).read_unaligned()
    }
}

pub(crate) fn closure_box_size(ncaps: usize) -> usize {
    32usize.wrapping_add(ncaps.wrapping_mul(16))
}

pub(crate) const CLOSURE_CAPTURE_BASE: usize = 24;

#[derive(Clone, Copy)]
struct CaptureDesc {
    size: usize,
    dtor: usize,
    elem_size: usize,
    elem_dtor: usize,
}

static CLOSURE_DESCS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<(u64, usize), CaptureDesc>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(crate) fn capture_desc_fn(addr: usize) -> Option<unsafe extern "C" fn(*mut u8)> {
    if addr == 0 {
        None
    } else {
        Some(unsafe { std::mem::transmute(addr) })
    }
}

fn release_capture(tag: u32, val: *mut u8, d: &CaptureDesc) {
    if val.is_null() {
        return;
    }
    unsafe {
        if tag == TAG_OBJ {
            rnx_release(val, d.size, capture_desc_fn(d.dtor));
        } else if tag == TAG_ARRAY {
            rnx_release_array(val, d.elem_size, capture_desc_fn(d.elem_dtor));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_closure_set(
    b: *mut u8,
    idx: usize,
    tag: u64,
    val: u64,
    size: usize,
    dtor: usize,
    elem_size: usize,
    elem_dtor: usize,
) {
    if b.is_null() {
        return;
    }
    unsafe {
        let base = b.add(CLOSURE_CAPTURE_BASE).add(idx.wrapping_mul(16));
        let old_tag = (base as *const u64).read_unaligned() as u32;
        if old_tag == TAG_OBJ || old_tag == TAG_ARRAY {
            let old_val = (base.add(8) as *const u64).read_unaligned() as *mut u8;
            let key = (closure_box_id(b), idx);
            if let Some(old) = CLOSURE_DESCS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&key)
            {
                release_capture(old_tag, old_val, &old);
            }
        }
        (base as *mut u64).write_unaligned(tag);
        (base.add(8) as *mut u64).write_unaligned(val);
        if tag as u32 == TAG_OBJ || tag as u32 == TAG_ARRAY {
            CLOSURE_DESCS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    (closure_box_id(b), idx),
                    CaptureDesc { size, dtor, elem_size, elem_dtor },
                );
        }
    }
}

unsafe extern "C" fn rnx_closure_dtor(b: *mut u8) {
    unsafe {
        let id = closure_box_id(b);
        let ncaps = (b.add(16) as *const u64).read_unaligned() as usize;
        for i in 0..ncaps {
            let base = b.add(CLOSURE_CAPTURE_BASE).add(i.wrapping_mul(16));
            let tag = (base as *const u64).read_unaligned() as u32;
            let val = (base.add(8) as *const u64).read_unaligned() as *mut u8;
            if tag == TAG_STR {
                rnx_release_str(val);
            } else if tag == TAG_CLOSURE {
                rnx_closure_release(val);
            } else if tag == TAG_OBJ || tag == TAG_ARRAY {
                if let Some(d) = CLOSURE_DESCS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&(id, i))
                {
                    release_capture(tag, val, &d);
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_closure_release(b: *mut u8) {
    if b.is_null() {
        return;
    }
    unsafe {
        let ncaps = (b.add(16) as *const u64).read_unaligned() as usize;
        rnx_release(b, closure_box_size(ncaps), Some(rnx_closure_dtor));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_get(arr: *const u8, index: usize, elem_size: usize) -> u64 {
    unsafe {
        if arr.is_null() || index >= arr_len(arr) {
            rnx_panic(b"index out of bounds\0".as_ptr(), "index out of bounds".len());
        }
        let slot = arr_data(arr).add(index.wrapping_mul(elem_size));
        if elem_size == 8 {
            (slot as *const u64).read_unaligned()
        } else {
            let mut out: u64 = 0;
            std::ptr::copy_nonoverlapping(
                slot,
                &mut out as *mut u64 as *mut u8,
                elem_size.min(8),
            );
            out
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_set(arr: *mut u8, index: usize, elem_val: u64, elem_size: usize) {
    unsafe {
        if arr.is_null() || index >= arr_len(arr) {
            rnx_panic(b"index out of bounds\0".as_ptr(), "index out of bounds".len());
        }
        let slot = arr_data(arr).add(index.wrapping_mul(elem_size));
        if elem_size == 8 {
            (slot as *mut u64).write_unaligned(elem_val);
        } else {
            std::ptr::copy_nonoverlapping(
                &elem_val as *const u64 as *const u8,
                slot,
                elem_size.min(8),
            );
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_get_unchecked(arr: *const u8, index: usize, elem_size: usize) -> u64 {
    unsafe {
        let slot = arr_data(arr).add(index.wrapping_mul(elem_size));
        if elem_size == 8 {
            (slot as *const u64).read_unaligned()
        } else {
            let mut out: u64 = 0;
            std::ptr::copy_nonoverlapping(
                slot,
                &mut out as *mut u64 as *mut u8,
                elem_size.min(8),
            );
            out
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_array_set_unchecked(
    arr: *mut u8,
    index: usize,
    elem_val: u64,
    elem_size: usize,
) {
    unsafe {
        let slot = arr_data(arr).add(index.wrapping_mul(elem_size));
        if elem_size == 8 {
            (slot as *mut u64).write_unaligned(elem_val);
        } else {
            std::ptr::copy_nonoverlapping(
                &elem_val as *const u64 as *const u8,
                slot,
                elem_size.min(8),
            );
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_release_array(
    arr: *mut u8,
    elem_size: usize,
    elem_dtor: Option<unsafe extern "C" fn(*mut u8)>,
) {
    if arr.is_null() {
        return;
    }
    unsafe {
        if !release_count(arr) {
            return;
        }
        let (len, cap, data) = (arr_len(arr), arr_cap(arr), arr_data(arr));
        if let Some(run) = elem_dtor {
            for i in 0..len {
                let slot = data.add(i.wrapping_mul(elem_size));
                run((slot as *const *mut u8).read_unaligned());
            }
        }
        if !data.is_null() {
            rnx_free(data, cap.wrapping_mul(elem_size));
        }
        rnx_free(arr, ARR_HEADER);
    }
}

#[cfg(test)]
mod array_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static DTOR_HITS: AtomicU32 = AtomicU32::new(0);

    unsafe extern "C" fn counting_dtor(_ptr: *mut u8) {
        DTOR_HITS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn new_push_get_len() {
        unsafe {
            let a = rnx_array_new(0, 8);
            assert!(!a.is_null());
            assert_eq!(rnx_array_len(a), 0);
            assert_eq!(rnx_array_len(std::ptr::null()), 0);
            for i in 0..10u64 {
                rnx_array_push(a, 10 + i * 10, 8);
            }
            assert_eq!(rnx_array_len(a), 10);
            for i in 0..10usize {
                assert_eq!(rnx_array_get(a, i, 8), 10 + i as u64 * 10);
            }
            rnx_release_array(a, 8, None);
        }
    }

    #[test]
    fn pop_returns_last_and_empties_to_zero() {
        unsafe {
            let a = rnx_array_new(0, 8);
            for i in 1..=3u64 {
                rnx_array_push(a, i * 10, 8);
            }
            assert_eq!(rnx_array_pop(a, 8), 30);
            assert_eq!(rnx_array_pop(a, 8), 20);
            assert_eq!(rnx_array_len(a), 1);
            assert_eq!(rnx_array_pop(a, 8), 10);
            assert_eq!(rnx_array_pop(a, 8), 0);
            assert_eq!(rnx_array_len(a), 0);
            rnx_release_array(a, 8, None);
        }
    }

    #[test]
    fn new_with_cap_set() {
        unsafe {
            let a = rnx_array_new(4, 8);
            assert!(!a.is_null());
            rnx_array_push(a, 1, 8);
            rnx_array_push(a, 2, 8);
            rnx_array_set(a, 0, 9, 8);
            assert_eq!(rnx_array_get(a, 0, 8), 9);
            assert_eq!(rnx_array_get(a, 1, 8), 2);
            rnx_release_array(a, 8, None);
        }
    }

    #[test]
    fn release_runs_dtor_per_element() {
        unsafe {
            DTOR_HITS.store(0, Ordering::SeqCst);
            let a = rnx_array_new(0, 8);
            for i in 0..5u64 {
                rnx_array_push(a, 0x1000 + i, 8);
            }
            rnx_retain(a);
            rnx_release_array(a, 8, Some(counting_dtor));
            assert_eq!(DTOR_HITS.load(Ordering::SeqCst), 0);
            rnx_release_array(a, 8, Some(counting_dtor));
            assert_eq!(DTOR_HITS.load(Ordering::SeqCst), 5);
        }
    }

    #[test]
    fn release_null_is_noop() {
        unsafe {
            rnx_release_array(std::ptr::null_mut(), 8, None);
        }
    }

    #[test]
    fn empty_no_data_buffer() {
        unsafe {
            let a = rnx_array_new(0, 8);
            assert!(!a.is_null());
            assert!(arr_data(a).is_null());
            rnx_release_array(a, 8, None);
        }
    }
}

#[cfg(test)]
mod float_tests {
    use super::*;

    #[test]
    fn float_specials_print_cleanly() {
        unsafe {
            rnx_print_val((-0.0f64).to_bits(), TAG_FLOAT);
            rnx_print_val(f64::NAN.to_bits(), TAG_FLOAT);
            rnx_print_val(f64::INFINITY.to_bits(), TAG_FLOAT);
            rnx_print_val(f64::NEG_INFINITY.to_bits(), TAG_FLOAT);
        }
    }

    #[test]
    fn float_to_str_values() {
        for (v, want) in [
            (3.0, "3.0"),
            (-0.0, "-0.0"),
            (41.25, "41.25"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
        ] {
            let p = unsafe { rnx_float_to_str(v.to_bits()) };
            assert!(!p.is_null());
            assert_eq!(str_bytes(p), want.as_bytes());
            unsafe { rnx_release(p, str_body_size(str_len(p)), None) };
        }
        let p = unsafe { rnx_float_to_str(f64::NAN.to_bits()) };
        assert!(!p.is_null());
        assert_eq!(str_bytes(p), "NaN".as_bytes());
        unsafe { rnx_release(p, str_body_size(str_len(p)), None) };
    }
}

#[cfg(test)]
mod map_tests {
    use super::*;

    fn k(s: &str) -> *const u8 {
        let p = alloc_str(s);
        assert!(!p.is_null());
        p
    }

    #[test]
    fn insert_get_update_delete() {
        unsafe {
            let m = rnx_map_new();
            assert!(!m.is_null());
            assert_eq!(rnx_map_len(m), 0);
            assert!(!rnx_map_has(m, k("a")));
            assert_eq!(rnx_map_get(m, k("a")), 0);
            rnx_map_set(m, k("a"), 10);
            rnx_map_set(m, k("b"), 20);
            assert!(rnx_map_has(m, k("a")));
            assert_eq!(rnx_map_get(m, k("a")), 10);
            assert_eq!(rnx_map_len(m), 2);
            rnx_map_set(m, k("a"), 11);
            assert_eq!(rnx_map_get(m, k("a")), 11);
            assert_eq!(rnx_map_len(m), 2);
            assert!(rnx_map_delete(m, k("a")));
            assert!(!rnx_map_has(m, k("a")));
            assert_eq!(rnx_map_get(m, k("a")), 0);
            assert!(!rnx_map_delete(m, k("a")));
            assert_eq!(rnx_map_len(m), 1);
            rnx_map_clear(m);
            assert_eq!(rnx_map_len(m), 0);
            assert!(!rnx_map_has(m, k("b")));
            rnx_release_map(m);
            rnx_release_map(std::ptr::null_mut());
        }
    }

    #[test]
    fn insertion_order_and_resize() {
        unsafe {
            let m = rnx_map_new();
            for i in 0..64u64 {
                let name = format!("k{i:02}");
                rnx_map_set(m, k(&name), 100 + i);
            }
            assert_eq!(rnx_map_len(m), 64);
            for i in 0..64u64 {
                let name = format!("k{i:02}");
                assert_eq!(rnx_map_get(m, k(&name)), 100 + i);
            }
            let keys = rnx_map_keys(m);
            assert_eq!(rnx_array_len(keys), 64);
            let first = rnx_array_get(keys, 0, 8) as *const u8;
            assert_eq!(str_bytes(first), b"k00");
            let last = rnx_array_get(keys, 63, 8) as *const u8;
            assert_eq!(str_bytes(last), b"k63");
            rnx_map_delete(m, k("k00"));
            let keys2 = rnx_map_keys(m);
            assert_eq!(rnx_array_len(keys2), 63);
            let first2 = rnx_array_get(keys2, 0, 8) as *const u8;
            assert_eq!(str_bytes(first2), b"k01");
            let vals = rnx_map_values(m);
            assert_eq!(rnx_array_len(vals), 63);
            assert_eq!(rnx_array_get(vals, 0, 8), 101);
            rnx_release_map(m);
        }
    }
}

#[cfg(test)]
mod any_box_tests {
    use super::*;

    #[test]
    fn scalar_roundtrip() {
        unsafe {
            let b = rnx_any_box(TAG_INT, 42);
            assert_ne!(b & ANY_BOX_MASK, 0);
            assert_eq!(rnx_any_unbox(b), 42);
            rnx_any_release(b);
            let b = rnx_any_box(TAG_BOOL, 1);
            assert_eq!(rnx_any_unbox(b), 1);
            rnx_any_release(b);
            let f = 2.5f64.to_bits();
            let b = rnx_any_box(TAG_FLOAT, f);
            assert_eq!(rnx_any_unbox(b), f);
            rnx_any_release(b);
        }
    }

    #[test]
    fn passthrough_is_identity() {
        unsafe {
            let b = rnx_any_box(TAG_STR, 0x1234);
            assert_ne!(b & ANY_BOX_MASK, 0);
            rnx_any_release(b);
            assert_eq!(rnx_any_unbox(42), 42);
            assert_eq!(rnx_any_unbox(0), 0);
            assert_eq!(rnx_any_unbox(u64::MAX), u64::MAX);
            rnx_any_release(42);
            rnx_any_release(0);
            rnx_any_release(0x7f0012345679);
            let b = rnx_any_box(TAG_INT, 7);
            rnx_any_retain(b);
            rnx_any_release(b);
            assert_eq!(rnx_any_unbox(b), 7);
            rnx_any_release(b);
            rnx_any_release(b);
        }
    }

    #[test]
    fn print_decodes_boxes() {
        unsafe {
            let i = rnx_any_box(TAG_INT, 42);
            rnx_print_val(i, TAG_PTR);
            let t = rnx_any_box(TAG_BOOL, 1);
            rnx_print_val(t, TAG_PTR);
            let s = alloc_str("boxed");
            let b = rnx_any_box(TAG_STR, s as u64);
            rnx_print_val(b, TAG_PTR);
            assert_eq!(rnx_any_unbox(b), s as u64);
            rnx_any_release(i);
            rnx_any_release(t);
            rnx_any_release(b);
            rnx_release_str(s);
        }
    }

    #[test]
    fn insert_remove_cycles_keep_an_empty_slot() {
        // Regression test for the 16-worker hang: each spawn/join round
        // inserts one box per worker and removes them all at join. With
        // fresh addresses per round the removes leave tombstones behind,
        // and the old live-only growth check let tombstones crowd out every
        // EMPTY slot. The next contains probe for an absent address then
        // never terminates, spinning forever while holding the ANY_BOXES
        // mutex. Every probe loop below only terminates on EMPTY, so this
        // asserts the invariant directly instead of hanging.
        let mut bs = BoxSet::new();
        let mut addr = 0x10000usize;
        for _ in 0..10 {
            let mut live = Vec::new();
            for _ in 0..16 {
                addr += 64;
                bs.insert(addr);
                live.push(addr);
            }
            assert!(bs.table.iter().any(|&v| v == BOX_EMPTY), "lost EMPTY slot with 16 live");
            for a in &live {
                assert!(bs.contains(*a));
            }
            assert!(!bs.contains(addr + 8));
            for a in live {
                bs.remove(a);
            }
            assert!(bs.table.iter().any(|&v| v == BOX_EMPTY), "lost EMPTY slot after drain");
        }
    }
}
