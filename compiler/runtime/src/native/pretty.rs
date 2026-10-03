use super::common::*;
use super::collections::*;

pub const PRETTY_MAX_DEPTH: usize = 3;

pub const KIND_INT: u64 = 1;
pub const KIND_BOOL: u64 = 2;
pub const KIND_FLOAT: u64 = 3;
pub const KIND_STR: u64 = 4;
pub const KIND_ANY: u64 = 5;
pub const KIND_ARRAY: u64 = 6;
pub const KIND_OBJ: u64 = 7;
pub const KIND_ENUM: u64 = 8;

pub fn pretty_profile() -> u64 {
    if std::env::var_os("NO_COLOR").is_some() {
        return 0;
    }
    match std::env::var("COLORTERM") {
        Ok(ct) if ct == "truecolor" || ct == "24bit" => return 3,
        _ => {}
    }
    match std::env::var("TERM") {
        Ok(term) if term.contains("256color") => return 2,
        Ok(term) if !term.is_empty() && term != "dumb" => return 1,
        _ => return 0,
    }
}

pub fn pretty_color_on(fd: i64) -> bool {
    super::io::io_is_tty_impl(fd) && pretty_profile() != 0
}

static ARRAY_KIND: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeMap<usize, (u64, u64)>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

static CLASS_FIELDS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<u64, Vec<(String, String)>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

static ENUM_SCHEMA: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<(u64, u64), (String, Vec<String>)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

static ENUM_INST: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<usize, (u64, u64)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

static NAMESPACES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<u64, (String, Vec<(String, String)>)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

fn parse_pairs(desc: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in desc.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once(':') {
            Some((name, kind)) => out.push((name.trim().to_string(), kind.trim().to_string())),
            None => out.push((part.to_string(), String::new())),
        }
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_note_array_kind(ptr: *mut u8, kind: u64, aux: u64) {
    if ptr.is_null() {
        return;
    }
    ARRAY_KIND
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(ptr as usize, (kind, aux));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_note_fields(idx: u64, desc: *const u8) {
    if desc.is_null() {
        return;
    }
    let fields = parse_pairs(&native_str(desc));
    CLASS_FIELDS.lock().unwrap_or_else(|e| e.into_inner()).insert(idx, fields);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_note_enum(ptr: *mut u8, enu: u64, variant: u64, desc: *const u8) {
    if desc.is_null() {
        return;
    }
    let text = native_str(desc);
    let (name, kinds) = match text.split_once(':') {
        Some((n, rest)) => (n.trim().to_string(), rest.split(',').map(|s| s.trim().to_string()).collect()),
        None => (text.trim().to_string(), Vec::new()),
    };
    ENUM_SCHEMA.lock().unwrap_or_else(|e| e.into_inner()).insert((enu, variant), (name, kinds));
    if !ptr.is_null() {
        ENUM_INST.lock().unwrap_or_else(|e| e.into_inner()).insert(ptr as usize, (enu, variant));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_note_namespace(idx: u64, desc: *const u8) {
    if desc.is_null() {
        return;
    }
    let text = native_str(desc);
    let Some((alias, rest)) = text.split_once('|') else {
        return;
    };
    let mut exports = Vec::new();
    for part in split_escaped(rest, ',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((name, kind)) = part.split_once(':') else {
            continue;
        };
        exports.push((unescape(&name.trim().to_string()), kind.trim().to_string()));
    }
    NAMESPACES.lock().unwrap_or_else(|e| e.into_inner()).insert(idx, (unescape(&alias.trim().to_string()), exports));
}

fn split_escaped(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some(o) => {
                    cur.push('\\');
                    cur.push(o);
                }
                None => cur.push('\\'),
            }
        } else if c == sep {
            out.push(cur);
            cur = String::new();
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('p') => out.push('|'),
                Some('s') => out.push(';'),
                Some('c') => out.push(':'),
                Some('m') => out.push(','),
                Some('e') => out.push('='),
                Some('\\') => out.push('\\'),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub(crate) fn pretty_untrack(ptr: usize) {
    ARRAY_KIND.lock().unwrap_or_else(|e| e.into_inner()).remove(&ptr);
    ENUM_INST.lock().unwrap_or_else(|e| e.into_inner()).remove(&ptr);
}

fn array_kind_of(ptr: usize) -> Option<(u64, u64)> {
    ARRAY_KIND.lock().unwrap_or_else(|e| e.into_inner()).get(&ptr).copied()
}

fn class_fields_of(idx: u64) -> Option<Vec<(String, String)>> {
    CLASS_FIELDS.lock().unwrap_or_else(|e| e.into_inner()).get(&idx).cloned()
}

fn enum_schema_of(enu: u64, variant: u64) -> Option<(String, Vec<String>)> {
    ENUM_SCHEMA.lock().unwrap_or_else(|e| e.into_inner()).get(&(enu, variant)).cloned()
}

fn enum_inst_of(ptr: usize) -> Option<(u64, u64)> {
    ENUM_INST.lock().unwrap_or_else(|e| e.into_inner()).get(&ptr).copied()
}

fn namespace_of(idx: u64) -> Option<(String, Vec<(String, String)>)> {
    NAMESPACES.lock().unwrap_or_else(|e| e.into_inner()).get(&idx).cloned()
}

fn type_name_of(idx: u64) -> String {
    TYPE_NAMES.lock().unwrap_or_else(|e| e.into_inner()).get(&idx).cloned().unwrap_or_else(|| "Unknown".to_string())
}

fn short_name(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_string()
}

fn box_payload(any: u64) -> Option<(u64, u64)> {
    let addr = any_box_addr(any)?;
    let payload = unsafe { ((addr as *const u8).add(8) as *const u64).read_unaligned() };
    Some((payload, any & ANY_BOX_MASK))
}

fn read_word(ptr: *const u8, off: usize) -> u64 {
    unsafe { (ptr.add(off) as *const u64).read_unaligned() }
}

struct Walker {
    color: bool,
    stack: Vec<usize>,
}

impl Walker {
    fn paint(&self, code: &str, text: &str, out: &mut String) {
        if self.color {
            out.push_str("\x1b[");
            out.push_str(code);
            out.push('m');
            out.push_str(text);
            out.push_str("\x1b[0m");
        } else {
            out.push_str(text);
        }
    }

    fn num(&self, text: &str, out: &mut String) {
        self.paint("36", text, out);
    }

    fn str_tok(&self, text: &str, out: &mut String) {
        self.paint("32", text, out);
    }

    fn boolean(&self, text: &str, out: &mut String) {
        self.paint("35", text, out);
    }

    fn key<F>(&mut self, render: F, out: &mut String)
    where
        F: FnOnce(&mut Walker, &mut String),
    {
        if self.color {
            let saved = self.color;
            self.color = false;
            let mut inner = String::new();
            render(self, &mut inner);
            self.color = saved;
            self.paint("33", &inner, out);
        } else {
            render(self, out);
        }
    }

    fn fmt_null(&self, out: &mut String) {
        self.boolean("null", out);
    }

    fn fmt_scalar_box(&self, payload: u64, marker: u64, out: &mut String) {
        match marker as u32 {
            x if x == ANY_BOX_INT as u32 => self.num(&(payload as i64).to_string(), out),
            x if x == ANY_BOX_BOOL as u32 => self.boolean(if payload & 1 != 0 { "true" } else { "false" }, out),
            x if x == ANY_BOX_FLOAT as u32 => self.num(&fmt_float(f64::from_bits(payload)), out),
            _ => {
                if payload == 0 {
                    self.fmt_null(out);
                } else {
                    self.str_tok(&String::from_utf8_lossy(str_bytes(payload as *const u8)), out);
                }
            }
        }
    }

    fn fmt_any(&mut self, bits: u64, depth: usize, out: &mut String) {
        if bits == 0 {
            self.fmt_null(out);
            return;
        }
        if let Some((payload, marker)) = box_payload(bits) {
            self.fmt_scalar_box(payload, marker, out);
            return;
        }
        if let Some(kind) = heap_kind_of(bits) {
            let ptr = bits as *mut u8;
            match kind {
                HEAP_STR => self.str_tok(&String::from_utf8_lossy(str_bytes(ptr as *const u8)), out),
                HEAP_ARRAY => self.fmt_array_ptr(ptr, depth, out),
                HEAP_OBJ => self.fmt_obj_ptr(ptr, depth, out),
                HEAP_ENUM => self.fmt_enum_ptr(ptr, depth, out),
                _ => out.push_str("<fn>"),
            }
            return;
        }
        if let Some((kind, aux)) = array_kind_of(bits as usize) {
            let _ = (kind, aux);
            self.fmt_array_ptr(bits as *mut u8, depth, out);
            return;
        }
        if enum_inst_of(bits as usize).is_some() {
            self.fmt_enum_ptr(bits as *mut u8, depth, out);
            return;
        }
        let idx = unsafe { rnx_obj_class(bits as *const u8) };
        if idx == 0 {
            self.str_tok(&String::from_utf8_lossy(str_bytes(bits as *const u8)), out);
        } else if class_fields_of(idx).is_some() {
            self.fmt_obj_ptr(bits as *mut u8, depth, out);
        } else {
            out.push_str("<unknown>");
        }
    }

    fn fmt_elem(&mut self, kind: u64, aux: u64, bits: u64, depth: usize, out: &mut String) {
        match kind {
            KIND_INT => self.num(&(bits as i64).to_string(), out),
            KIND_BOOL => self.boolean(if bits & 1 != 0 { "true" } else { "false" }, out),
            KIND_FLOAT => self.num(&fmt_float(f64::from_bits(bits)), out),
            KIND_STR => {
                if bits == 0 {
                    self.fmt_null(out);
                } else {
                    self.str_tok(&String::from_utf8_lossy(str_bytes(bits as *const u8)), out);
                }
            }
            KIND_ARRAY => {
                if bits == 0 {
                    self.fmt_null(out);
                } else {
                    self.fmt_array_ptr(bits as *mut u8, depth, out);
                }
            }
            KIND_OBJ => {
                if bits == 0 {
                    self.fmt_null(out);
                } else {
                    self.fmt_obj_ptr(bits as *mut u8, depth, out);
                }
            }
            KIND_ENUM => {
                if bits == 0 {
                    self.fmt_null(out);
                } else {
                    self.fmt_enum_noted(bits as *mut u8, aux, depth, out);
                }
            }
            _ => self.fmt_any(bits, depth, out),
        }
    }

    fn fmt_array_ptr(&mut self, ptr: *mut u8, depth: usize, out: &mut String) {
        if depth >= PRETTY_MAX_DEPTH {
            out.push_str("...");
            return;
        }
        let key = ptr as usize;
        if self.stack.contains(&key) {
            out.push_str("<cycle>");
            return;
        }
        let (kind, aux) = array_kind_of(key).unwrap_or((KIND_ANY, 0));
        let len = arr_len(ptr as *const u8);
        self.stack.push(key);
        out.push('[');
        for i in 0..len {
            if i > 0 {
                out.push_str(", ");
            }
            let bits = unsafe { rnx_array_get(ptr as *const u8, i, 8) };
            self.fmt_elem(kind, aux, bits, depth + 1, out);
        }
        out.push(']');
        self.stack.pop();
    }

    fn fmt_obj_ptr(&mut self, ptr: *mut u8, depth: usize, out: &mut String) {
        if depth >= PRETTY_MAX_DEPTH {
            out.push_str("...");
            return;
        }
        let key = ptr as usize;
        if self.stack.contains(&key) {
            out.push_str("<cycle>");
            return;
        }
        let idx = unsafe { rnx_obj_class(ptr as *const u8) };
        if let Some((alias, exports)) = namespace_of(idx) {
            self.stack.push(key);
            self.fmt_namespace(&alias, &exports, depth, out);
            self.stack.pop();
            return;
        }
        let name = short_name(&type_name_of(idx));
        if name == "Map" {
            self.stack.push(key);
            self.fmt_gmap(ptr, depth, out);
            self.stack.pop();
            return;
        }
        if name == "Set" {
            self.stack.push(key);
            self.fmt_set(ptr, depth, out);
            self.stack.pop();
            return;
        }
        let fields = class_fields_of(idx);
        match fields {
            None => {
                out.push_str(&name);
                out.push_str("{...}");
            }
            Some(fields) => {
                self.stack.push(key);
                out.push_str(&name);
                out.push('{');
                for (i, (fname, fkind)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let slot = read_word(ptr as *const u8, 16 + i * 8);
                    self.key(|_w, o| o.push_str(fname), out);
                    out.push_str(": ");
                    self.fmt_field(fkind, slot, depth + 1, out);
                }
                out.push('}');
                self.stack.pop();
            }
        }
    }

    fn fmt_namespace(&mut self, alias: &str, exports: &[(String, String)], depth: usize, out: &mut String) {
        if depth >= PRETTY_MAX_DEPTH {
            out.push_str("...");
            return;
        }
        out.push_str("[Module ");
        out.push_str(alias);
        if exports.is_empty() {
            out.push_str("] {}");
            return;
        }
        out.push_str("] { ");
        for (i, (name, kind)) in exports.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            self.key(|_w, o| o.push_str(name), out);
            out.push_str(": ");
            if kind == "fn" {
                out.push_str(&format!("[Function: {name}]"));
            } else if kind == "class" {
                out.push_str(&format!("[Class: {name}]"));
            } else if kind == "enum" {
                out.push_str(&format!("[Enum: {name}]"));
            } else if kind == "null" {
                self.fmt_null(out);
            } else if let Some(v) = kind.strip_prefix("int=") {
                self.num(&v.parse::<i64>().unwrap_or(0).to_string(), out);
            } else if let Some(v) = kind.strip_prefix("bool=") {
                self.boolean(if v == "true" { "true" } else { "false" }, out);
            } else if let Some(v) = kind.strip_prefix("float=") {
                let bits = v.parse::<u64>().unwrap_or(0);
                self.num(&fmt_float(f64::from_bits(bits)), out);
            } else if let Some(v) = kind.strip_prefix("str=") {
                self.str_tok(&unescape(v), out);
            } else {
                out.push_str("<unknown>");
            }
        }
        out.push_str(" }");
    }

    fn fmt_field(&mut self, fkind: &str, slot: u64, depth: usize, out: &mut String) {
        let (kind, aux) = field_kind(fkind);
        self.fmt_elem(kind, aux, slot, depth, out);
    }

    fn fmt_gmap(&mut self, ptr: *mut u8, depth: usize, out: &mut String) {
        let inner = match class_fields_of(unsafe { rnx_obj_class(ptr as *const u8) }) {
            Some(fields) => fields.iter().position(|(n, _)| n == "handle").map(|i| read_word(ptr as *const u8, 16 + i * 8)),
            None => None,
        };
        let handle = match inner {
            Some(h) if h != 0 => h as *mut u8,
            _ => {
                out.push_str("{}");
                return;
            }
        };
        let live = gmap_live_ptr(handle);
        if live.is_null() {
            out.push_str("{}");
            return;
        }
        let pairs = gmap_state(live).snapshot();
        out.push('{');
        for (i, (k, v)) in pairs.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            self.key(|w, o| w.fmt_any(*k, depth + 1, o), out);
            out.push_str(": ");
            self.fmt_any(*v, depth + 1, out);
            unsafe {
                rnx_any_release(*k);
                rnx_any_release(*v);
            }
        }
        out.push('}');
    }

    fn fmt_set(&mut self, ptr: *mut u8, depth: usize, out: &mut String) {
        let inner = match class_fields_of(unsafe { rnx_obj_class(ptr as *const u8) }) {
            Some(fields) => fields.iter().position(|(n, _)| n == "inner").map(|i| read_word(ptr as *const u8, 16 + i * 8)),
            None => None,
        };
        let map_ptr = match inner {
            Some(p) if p != 0 => p as *mut u8,
            _ => {
                out.push_str("Set{...}");
                return;
            }
        };
        let midx = unsafe { rnx_obj_class(map_ptr as *const u8) };
        let mfields = class_fields_of(midx);
        let handle = mfields.as_ref().and_then(|fields| {
            fields.iter().position(|(n, _)| n == "handle").map(|i| read_word(map_ptr as *const u8, 16 + i * 8))
        });
        let handle = match handle {
            Some(h) if h != 0 => h as *mut u8,
            _ => {
                out.push_str("Set{...}");
                return;
            }
        };
        let live = gmap_live_ptr(handle);
        if live.is_null() {
            out.push_str("Set{...}");
            return;
        }
        let pairs = gmap_state(live).snapshot();
        out.push('[');
        for (i, (k, v)) in pairs.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            self.fmt_any(*k, depth + 1, out);
            unsafe {
                rnx_any_release(*k);
                rnx_any_release(*v);
            }
        }
        out.push(']');
    }

    fn fmt_enum_noted(&mut self, ptr: *mut u8, enu_hint: u64, depth: usize, out: &mut String) {
        if depth >= PRETTY_MAX_DEPTH {
            out.push_str("...");
            return;
        }
        let key = ptr as usize;
        if self.stack.contains(&key) {
            out.push_str("<cycle>");
            return;
        }
        let variant = read_word(ptr as *const u8, 16);
        let enu = enum_inst_of(key).map(|(e, _)| e).unwrap_or(enu_hint);
        self.stack.push(key);
        self.fmt_enum_parts(ptr, enu, variant, depth, out);
        self.stack.pop();
    }

    fn fmt_enum_ptr(&mut self, ptr: *mut u8, depth: usize, out: &mut String) {
        if depth >= PRETTY_MAX_DEPTH {
            out.push_str("...");
            return;
        }
        let key = ptr as usize;
        if self.stack.contains(&key) {
            out.push_str("<cycle>");
            return;
        }
        let variant = read_word(ptr as *const u8, 16);
        let enu = enum_inst_of(key).map(|(e, _)| e).unwrap_or(u64::MAX);
        self.stack.push(key);
        self.fmt_enum_parts(ptr, enu, variant, depth, out);
        self.stack.pop();
    }

    fn fmt_enum_parts(&mut self, ptr: *mut u8, enu: u64, variant: u64, depth: usize, out: &mut String) {
        match enum_schema_of(enu, variant) {
            None => {
                out.push_str("Unknown");
            }
            Some((name, kinds)) => {
                out.push_str(&short_name(&name));
                if !kinds.is_empty() {
                    out.push('(');
                    for (i, k) in kinds.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        let slot = read_word(ptr as *const u8, 24 + i * 8);
                        let (kind, aux) = field_kind(k);
                        self.fmt_elem(kind, aux, slot, depth + 1, out);
                    }
                    out.push(')');
                }
            }
        }
    }
}

fn field_kind(desc: &str) -> (u64, u64) {
    let desc = desc.trim();
    if let Some(rest) = desc.strip_prefix("enum:") {
        return (KIND_ENUM, rest.trim().parse::<u64>().unwrap_or(u64::MAX));
    }
    match desc {
        "int" => (KIND_INT, 0),
        "bool" => (KIND_BOOL, 0),
        "float" => (KIND_FLOAT, 0),
        "str" => (KIND_STR, 0),
        "array" => (KIND_ARRAY, 0),
        "obj" => (KIND_OBJ, 0),
        "any" => (KIND_ANY, 0),
        _ => (KIND_ANY, 0),
    }
}

pub fn pretty_any_plain(bits: u64) -> String {
    let mut w = Walker { color: false, stack: Vec::new() };
    let mut out = String::new();
    w.fmt_any(bits, 0, &mut out);
    out
}

pub fn pretty_any_colored(bits: u64, fd: i64) -> String {
    let mut w = Walker { color: pretty_color_on(fd), stack: Vec::new() };
    let mut out = String::new();
    w.fmt_any(bits, 0, &mut out);
    out
}

pub fn pretty_scalar_tagged(bits: u64, tag: u32) -> String {
    match tag {
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
        _ => pretty_any_plain(bits),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_io_pretty(any: u64, fd: i64) -> *mut u8 {
    alloc_str(&pretty_any_colored(any, fd))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_rules_in_order() {
        unsafe { std::env::set_var("NO_COLOR", "1") };
        assert_eq!(pretty_profile(), 0);
        unsafe { std::env::set_var("NO_COLOR", "") };
        assert_eq!(pretty_profile(), 0);
        unsafe { std::env::remove_var("NO_COLOR") };
        unsafe { std::env::set_var("COLORTERM", "truecolor") };
        unsafe { std::env::set_var("TERM", "xterm") };
        assert_eq!(pretty_profile(), 3);
        unsafe { std::env::remove_var("COLORTERM") };
        unsafe { std::env::set_var("TERM", "xterm-256color") };
        assert_eq!(pretty_profile(), 2);
        unsafe { std::env::set_var("TERM", "dumb") };
        assert_eq!(pretty_profile(), 0);
        unsafe { std::env::remove_var("TERM") };
        assert_eq!(pretty_profile(), 0);
        unsafe { std::env::set_var("TERM", "xterm") };
        assert_eq!(pretty_profile(), 1);
        unsafe { std::env::remove_var("TERM") };
    }

    #[test]
    fn plain_scalars_have_no_escapes() {
        assert_eq!(pretty_any_plain(0), "null");
    }
}
