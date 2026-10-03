use lir::instr::FloatKind;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64, FloatKind),
    Bool(bool),
    Str(String),
    Null,
    Array { id: usize },
    Obj { slot: usize, epoch: u32 },
    Struct { class: usize, fields: Vec<Value> },
    Enum { enu: usize, variant: usize, name: String, payload: Vec<Value> },
    Pointer(i64),
    StackToken(i64),
    Vec4f([f32; 4]),
    Vec4i([i32; 4]),
    Closure(ClosureVal),
    GenRef { slot: Option<usize>, epoch: u32 },
    Range { lo: i64, hi: i64, inclusive: bool, step: i64 },
    Error(ErrorVal),
    Module(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClosureVal {
    pub func: usize,
    pub captures: Vec<Value>,
    pub decay: bool,
    pub decay_this: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ErrorVal {
    pub tag: String,
    pub message: String,
}

impl Value {
    pub fn type_tag(&self, module: &lir::instr::Module) -> String {
        match self {
            Value::Int(_) => "Int".to_string(),
            Value::Float(_, _) => "Float".to_string(),
            Value::Bool(_) => "Bool".to_string(),
            Value::Str(_) => "String".to_string(),
            Value::Null => "Null".to_string(),
            Value::Array { .. } => "Array".to_string(),
            Value::Obj { .. } => "Object".to_string(),
            Value::Struct { class, .. } => module.classes.get(*class).map(|c| c.name.clone()).unwrap_or_default(),
            Value::Enum { enu, .. } => module.enums.get(*enu).map(|e| e.name.clone()).unwrap_or_default(),
            Value::Pointer(_) => "Pointer".to_string(),
            Value::StackToken(_) => "Pointer".to_string(),
            Value::Vec4f(_) => "Vec4f".to_string(),
            Value::Vec4i(_) => "Vec4i".to_string(),
            Value::Closure(_) => "Closure".to_string(),
            Value::GenRef { .. } => "GenRef".to_string(),
            Value::Range { .. } => "Range".to_string(),
            Value::Error(e) => e.tag.clone(),
            Value::Module(m) => m.clone(),
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Null => false,
            Value::Int(i) => *i != 0,
            _ => true,
        }
    }

    pub fn pretty(&self, ctx: &PrettyCtx, depth: usize, stack: &mut Vec<PrettyId>, out: &mut String) {
        match self {
            Value::Int(i) => ctx.num(&i.to_string(), out),
            Value::Float(f, _) => ctx.num(&fmt_float_val(*f), out),
            Value::Bool(b) => ctx.boolean(&b.to_string(), out),
            Value::Str(s) => ctx.str_tok(s, out),
            Value::Null => ctx.boolean("null", out),
            Value::Array { id } => {
                if depth >= PRETTY_MAX_DEPTH {
                    out.push_str("...");
                    return;
                }
                if stack.contains(&PrettyId::Array(*id)) {
                    out.push_str("<cycle>");
                    return;
                }
                match ctx.arrays.with_live(*id, |live| live.elems.clone()) {
                    None => out.push_str("<unknown>"),
                    Some(elems) => {
                        stack.push(PrettyId::Array(*id));
                        out.push('[');
                        for (i, e) in elems.iter().enumerate() {
                            if i > 0 {
                                out.push_str(", ");
                            }
                            e.pretty(ctx, depth + 1, stack, out);
                        }
                        out.push(']');
                        stack.pop();
                    }
                }
            }
            Value::Obj { slot, epoch } => {
                if depth >= PRETTY_MAX_DEPTH {
                    out.push_str("...");
                    return;
                }
                if stack.contains(&PrettyId::Obj(*slot, *epoch)) {
                    out.push_str("<cycle>");
                    return;
                }
                match ctx.arena.get(*slot, *epoch) {
                    None => ctx.boolean("null", out),
                    Some(class) => {
                        let full = ctx
                            .module
                            .classes
                            .get(class)
                            .map(|c| c.name.clone())
                            .unwrap_or_default();
                        if let Some(ns) = ctx.module.namespaces.get(&full) {
                            stack.push(PrettyId::Obj(*slot, *epoch));
                            Self::pretty_namespace(ctx, ns, depth, stack, out);
                            stack.pop();
                            return;
                        }
                        let name = ctx
                            .module
                            .classes
                            .get(class)
                            .map(|c| short_class_name(&c.name))
                            .unwrap_or_else(|| "Unknown".to_string());
                        if name == "Map" {
                            stack.push(PrettyId::Obj(*slot, *epoch));
                            self.pretty_gmap(ctx, depth, stack, out);
                            stack.pop();
                            return;
                        }
                        if name == "Set" {
                            stack.push(PrettyId::Obj(*slot, *epoch));
                            self.pretty_set(ctx, depth, stack, out);
                            stack.pop();
                            return;
                        }
                        match ctx.arena.fields(*slot, *epoch) {
                            None => ctx.boolean("null", out),
                            Some(fields) => {
                                stack.push(PrettyId::Obj(*slot, *epoch));
                                out.push_str(&name);
                                out.push('{');
                                let descs = ctx.module.classes.get(class);
                                for (i, v) in fields.iter().enumerate() {
                                    if i > 0 {
                                        out.push_str(", ");
                                    }
                                    let fname = descs
                                        .and_then(|d| d.fields.get(i))
                                        .map(|f| f.name.as_str())
                                        .unwrap_or("?");
                                    ctx.key(fname, out);
                                    out.push_str(": ");
                                    v.pretty(ctx, depth + 1, stack, out);
                                }
                                out.push('}');
                                stack.pop();
                            }
                        }
                    }
                }
            }
            Value::Struct { class, fields } => {
                if depth >= PRETTY_MAX_DEPTH {
                    out.push_str("...");
                    return;
                }
                let id = PrettyId::Struct(*class, value_digest(self, 0));
                if stack.contains(&id) {
                    out.push_str("<cycle>");
                    return;
                }
                stack.push(id);
                let desc = ctx.module.classes.get(*class);
                let name = desc.map(|c| short_class_name(&c.name)).unwrap_or_else(|| "Unknown".to_string());
                out.push_str(&name);
                out.push('{');
                for (i, v) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let fname = desc
                        .and_then(|d| d.fields.get(i))
                        .map(|f| f.name.as_str())
                        .unwrap_or("?");
                    ctx.key(fname, out);
                    out.push_str(": ");
                    v.pretty(ctx, depth + 1, stack, out);
                }
                out.push('}');
                stack.pop();
            }
            Value::Enum { enu, variant, name, payload } => {
                if depth >= PRETTY_MAX_DEPTH {
                    out.push_str("...");
                    return;
                }
                let id = PrettyId::Enum(*enu, *variant, value_digest(self, 0));
                if stack.contains(&id) {
                    out.push_str("<cycle>");
                    return;
                }
                stack.push(id);
                let vname = ctx
                    .module
                    .enums
                    .get(*enu)
                    .and_then(|e| e.variants.get(*variant))
                    .map(|v| short_class_name(&v.name))
                    .unwrap_or_else(|| short_class_name(name));
                out.push_str(&vname);
                if !payload.is_empty() {
                    out.push('(');
                    for (i, p) in payload.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        p.pretty(ctx, depth + 1, stack, out);
                    }
                    out.push(')');
                }
                stack.pop();
            }
            Value::GenRef { slot: None, .. } => out.push_str("<empty>"),
            Value::GenRef { slot: Some(s), epoch } => match ctx.arena.get(*s, *epoch) {
                None => ctx.boolean("null", out),
                Some(_) => Value::Obj { slot: *s, epoch: *epoch }.pretty(ctx, depth, stack, out),
            },
            Value::Range { lo, hi, .. } => {
                out.push_str(&format!("{lo}..{hi}"));
            }
            Value::Error(e) => ctx.str_tok(&e.message, out),
            Value::Closure(_) => out.push_str("<fn>"),
            Value::Pointer(h) => out.push_str(&format!("<ptr#{h}>")),
            Value::StackToken(h) => out.push_str(&format!("<stack#{h}>")),
            Value::Vec4f(v) => out.push_str(&format!("Vec4f({}, {}, {}, {})", v[0], v[1], v[2], v[3])),
            Value::Vec4i(v) => out.push_str(&format!("Vec4i({}, {}, {}, {})", v[0], v[1], v[2], v[3])),
            Value::Module(m) => out.push_str(&format!("<{m}>")),
        }
    }

    fn pretty_namespace(ctx: &PrettyCtx, ns: &lir::instr::NamespaceDesc, depth: usize, stack: &mut Vec<PrettyId>, out: &mut String) {
        if depth >= PRETTY_MAX_DEPTH {
            out.push_str("...");
            return;
        }
        out.push_str("[Module ");
        out.push_str(&ns.alias);
        if ns.exports.is_empty() {
            out.push_str("] {}");
            return;
        }
        out.push_str("] { ");
        for (i, e) in ns.exports.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            ctx.key(&e.name, out);
            out.push_str(": ");
            match &e.kind {
                lir::instr::NsKind::Function => {
                    out.push_str(&format!("[Function: {}]", e.name));
                }
                lir::instr::NsKind::Class => {
                    out.push_str(&format!("[Class: {}]", e.name));
                }
                lir::instr::NsKind::Enum => {
                    out.push_str(&format!("[Enum: {}]", e.name));
                }
                lir::instr::NsKind::Const(lit) => {
                    let v = match lit {
                        lir::instr::Lit::Int(n) => Value::Int(*n),
                        lir::instr::Lit::Float(x, k) => Value::Float(*x, *k),
                        lir::instr::Lit::Bool(b) => Value::Bool(*b),
                        lir::instr::Lit::Str(s) => Value::Str(s.clone()),
                        lir::instr::Lit::Null => Value::Null,
                    };
                    v.pretty(ctx, depth + 1, stack, out);
                }
            }
        }
        out.push_str(" }");
    }

    fn pretty_gmap(&self, ctx: &PrettyCtx, depth: usize, stack: &mut Vec<PrettyId>, out: &mut String) {
        let entries = match self {
            Value::Obj { slot, epoch } => ctx.arena.get(*slot, *epoch).and_then(|class| {
                let fields = ctx.arena.fields(*slot, *epoch)?;
                let handle = map_handle(ctx, class, &fields)?;
                ctx.gmaps.with(handle, |m| m.entries.clone())
            }),
            _ => None,
        };
        match entries {
            None => out.push_str("{}"),
            Some(entries) => {
                out.push('{');
                let mut first = true;
                for e in entries.iter().filter(|e| e.active) {
                    if !first {
                        out.push_str(", ");
                    }
                    first = false;
                    ctx.key_owned(gmap_key_string(ctx, &e.key, depth, stack), out);
                    out.push_str(": ");
                    e.val.pretty(ctx, depth + 1, stack, out);
                }
                out.push('}');
            }
        }
    }

    fn pretty_set(&self, ctx: &PrettyCtx, depth: usize, stack: &mut Vec<PrettyId>, out: &mut String) {
        let entries = match self {
            Value::Obj { slot, epoch } => ctx.arena.get(*slot, *epoch).and_then(|class| {
                let fields = ctx.arena.fields(*slot, *epoch)?;
                let inner = set_inner(ctx, class, &fields)?;
                match inner {
                    Value::Obj { slot, epoch } => {
                        let mclass = ctx.arena.get(slot, epoch)?;
                        let mfields = ctx.arena.fields(slot, epoch)?;
                        let handle = map_handle(ctx, mclass, &mfields)?;
                        ctx.gmaps.with(handle, |m| m.entries.clone())
                    }
                    _ => None,
                }
            }),
            _ => None,
        };
        match entries {
            None => out.push_str("Set{...}"),
            Some(entries) => {
                out.push('[');
                let mut first = true;
                for e in entries.iter().filter(|e| e.active) {
                    if !first {
                        out.push_str(", ");
                    }
                    first = false;
                    gmap_key_value(ctx, &e.key, depth, stack, out);
                }
                out.push(']');
            }
        }
    }

    pub fn display(&self) -> String {
        match self {
            Value::Int(i) => i.to_string(),
            Value::Float(f, _) => {
                if f.fract() == 0.0 && f.is_finite() {
                    format!("{f:.1}")
                } else {
                    format!("{f}")
                }
            }
            Value::Bool(b) => b.to_string(),
            Value::Str(s) => s.clone(),
            Value::Null => "null".to_string(),
            Value::Array { id } => format!("<array#{id}>"),
            Value::Obj { slot, .. } => format!("<obj#{slot}>"),
            Value::Struct { fields, .. } => {
                let items: Vec<String> =
                    fields.iter().map(|v| v.display()).collect();
                format!("({})", items.join(", "))
            }
            Value::Enum { name, payload, .. } => {
                if payload.is_empty() {
                    name.clone()
                } else {
                    let items: Vec<String> =
                        payload.iter().map(|v| v.display()).collect();
                    format!("{name}({})", items.join(", "))
                }
            }
            Value::Pointer(h) => format!("<ptr#{h}>"),
            Value::StackToken(h) => format!("<stack#{h}>"),
            Value::Vec4f(v) => format!("Vec4f({}, {}, {}, {})", v[0], v[1], v[2], v[3]),
            Value::Vec4i(v) => format!("Vec4i({}, {}, {}, {})", v[0], v[1], v[2], v[3]),
            Value::Closure(_) => "<fn>".to_string(),
            Value::GenRef { slot: None, .. } => "<empty>".to_string(),
            Value::GenRef { slot: Some(s), .. } => format!("<ref#{s}>"),
            Value::Range { lo, hi, .. } => format!("{lo}..{hi}"),
            Value::Error(e) => e.message.clone(),
            Value::Module(m) => format!("<{m}>"),
        }
    }
}

pub const PRETTY_MAX_DEPTH: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrettyId {
    Array(usize),
    Obj(usize, u32),
    Struct(usize, u64),
    Enum(usize, usize, u64),
}

fn value_digest(v: &Value, depth: usize) -> u64 {
    fn mix(h: u64, x: u64) -> u64 {
        (h ^ x).wrapping_mul(0x100000001b3)
    }
    const OFF: u64 = 0xcbf29ce484222325;
    const CAP: usize = 8;
    if depth > CAP {
        return mix(OFF, 0x9e3779b97f4a7c15);
    }
    let mut h = OFF;
    match v {
        Value::Int(i) => {
            h = mix(h, 1);
            h = mix(h, *i as u64);
        }
        Value::Float(f, _) => {
            h = mix(h, 3);
            h = mix(h, f.to_bits());
        }
        Value::Bool(b) => {
            h = mix(h, 2);
            h = mix(h, *b as u64);
        }
        Value::Str(s) => {
            h = mix(h, 4);
            for b in s.as_bytes() {
                h = mix(h, *b as u64);
            }
        }
        Value::Null => {
            h = mix(h, 5);
        }
        Value::Array { id } => {
            h = mix(h, 6);
            h = mix(h, *id as u64);
        }
        Value::Obj { slot, epoch } => {
            h = mix(h, 7);
            h = mix(h, *slot as u64);
            h = mix(h, *epoch as u64);
        }
        Value::Struct { class, fields } => {
            h = mix(h, 8);
            h = mix(h, *class as u64);
            for f in fields {
                h = mix(h, value_digest(f, depth + 1));
            }
        }
        Value::Enum { enu, variant, payload, .. } => {
            h = mix(h, 9);
            h = mix(h, *enu as u64);
            h = mix(h, *variant as u64);
            for p in payload {
                h = mix(h, value_digest(p, depth + 1));
            }
        }
        Value::Pointer(x) => {
            h = mix(h, 10);
            h = mix(h, *x as u64);
        }
        Value::StackToken(x) => {
            h = mix(h, 11);
            h = mix(h, *x as u64);
        }
        Value::Vec4f(a) => {
            h = mix(h, 12);
            for x in a {
                h = mix(h, x.to_bits() as u64);
            }
        }
        Value::Vec4i(a) => {
            h = mix(h, 13);
            for x in a {
                h = mix(h, *x as u64);
            }
        }
        Value::Closure(c) => {
            h = mix(h, 14);
            h = mix(h, c.func as u64);
            for x in &c.captures {
                h = mix(h, value_digest(x, depth + 1));
            }
        }
        Value::GenRef { slot, epoch } => {
            h = mix(h, 15);
            h = mix(h, slot.unwrap_or(usize::MAX) as u64);
            h = mix(h, *epoch as u64);
        }
        Value::Range { lo, hi, inclusive, step } => {
            h = mix(h, 16);
            h = mix(h, *lo as u64);
            h = mix(h, *hi as u64);
            h = mix(h, *inclusive as u64);
            h = mix(h, *step as u64);
        }
        Value::Error(e) => {
            h = mix(h, 17);
            for b in e.tag.as_bytes().iter().chain(e.message.as_bytes()) {
                h = mix(h, *b as u64);
            }
        }
        Value::Module(m) => {
            h = mix(h, 18);
            for b in m.as_bytes() {
                h = mix(h, *b as u64);
            }
        }
    }
    h
}

pub struct PrettyCtx<'a> {
    pub module: &'a lir::instr::Module,
    pub arena: &'a Arena,
    pub arrays: &'a ArrayTable,
    pub gmaps: &'a GenericMapTable,
    pub color: bool,
}

impl PrettyCtx<'_> {
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

    fn key(&self, text: &str, out: &mut String) {
        self.paint("33", text, out);
    }

    fn key_owned(&self, text: String, out: &mut String) {
        self.paint("33", &text, out);
    }
}

fn short_class_name(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_string()
}

fn fmt_float_val(f: f64) -> String {
    if f.fract() == 0.0 && f.is_finite() {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

fn map_handle(ctx: &PrettyCtx, class: usize, fields: &[Value]) -> Option<i64> {
    let desc = ctx.module.classes.get(class)?;
    let i = *desc.field_index.get("handle")?;
    match fields.get(i) {
        Some(Value::Int(h)) => Some(*h),
        _ => None,
    }
}

fn set_inner(ctx: &PrettyCtx, class: usize, fields: &[Value]) -> Option<Value> {
    let desc = ctx.module.classes.get(class)?;
    let i = *desc.field_index.get("inner")?;
    fields.get(i).cloned()
}

fn gmap_key_string(ctx: &PrettyCtx, key: &GMapKey, depth: usize, stack: &mut Vec<PrettyId>) -> String {
    let mut tmp = String::new();
    let plain = PrettyCtx {
        module: ctx.module,
        arena: ctx.arena,
        arrays: ctx.arrays,
        gmaps: ctx.gmaps,
        color: false,
    };
    gmap_key_value(&plain, key, depth, stack, &mut tmp);
    tmp
}

fn gmap_key_value(ctx: &PrettyCtx, key: &GMapKey, depth: usize, stack: &mut Vec<PrettyId>, out: &mut String) {
    match key {
        GMapKey::Int(i) => ctx.num(&i.to_string(), out),
        GMapKey::Float(bits) => ctx.num(&fmt_float_val(f64::from_bits(*bits)), out),
        GMapKey::Bool(b) => ctx.boolean(&b.to_string(), out),
        GMapKey::Str(s) => ctx.str_tok(s, out),
        GMapKey::Obj(slot, epoch) => Value::Obj { slot: *slot, epoch: *epoch }.pretty(ctx, depth, stack, out),
    }
}

pub struct Slot {
    pub epoch: u32,
    pub live: Option<LiveObj>,
}

pub struct LiveObj {
    pub class: usize,
    pub fields: Vec<Value>,
    pub count: AtomicI64,
}

struct ObjState {
    slots: Vec<Slot>,
    free: Vec<usize>,
}

pub struct Arena {
    state: Arc<Mutex<ObjState>>,
}

impl Arena {
    pub fn new() -> Arena {
        Arena {
            state: Arc::new(Mutex::new(ObjState { slots: Vec::new(), free: Vec::new() })),
        }
    }

    pub fn share(&self) -> Arena {
        Arena { state: Arc::clone(&self.state) }
    }

    pub fn same(&self, other: &Arena) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ObjState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn alloc(&mut self, class: usize, fields: Vec<Value>) -> (usize, u32) {
        let obj = LiveObj { class, fields, count: AtomicI64::new(1) };
        let mut st = self.lock();
        if let Some(s) = st.free.pop() {
            let epoch = st.slots[s].epoch;
            st.slots[s].live = Some(obj);
            (s, epoch)
        } else {
            let s = st.slots.len();
            st.slots.push(Slot { epoch: 0, live: Some(obj) });
            (s, 0)
        }
    }

    pub fn retain(&mut self, slot: usize) {
        let st = self.lock();
        if let Some(live) = st.slots.get(slot).and_then(|s| s.live.as_ref()) {
            live.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn release(&mut self, slot: usize) -> Option<usize> {
        let st = self.lock();
        let prev = match st.slots.get(slot).and_then(|s| s.live.as_ref()) {
            Some(live) => live.count.fetch_sub(1, Ordering::SeqCst),
            None => return None,
        };
        if prev <= 0 {
            if let Some(live) = st.slots.get(slot).and_then(|s| s.live.as_ref()) {
                live.count.fetch_add(1, Ordering::SeqCst);
            }
        }
        if prev > 1 {
            return None;
        }
        let class = st.slots.get(slot).and_then(|s| s.live.as_ref()).map(|l| l.class)?;
        let mut st = st;
        st.slots[slot].live = None;
        st.slots[slot].epoch += 1;
        st.free.push(slot);
        Some(class)
    }

    pub fn count(&self, slot: usize) -> Option<usize> {
        let st = self.lock();
        st.slots
            .get(slot)
            .and_then(|s| s.live.as_ref())
            .map(|live| live.count.load(Ordering::SeqCst).max(0) as usize)
    }

    pub fn live_count(&self) -> usize {
        let st = self.lock();
        st.slots.iter().filter(|s| s.live.is_some()).count()
    }

    pub fn get(&self, slot: usize, epoch: u32) -> Option<usize> {
        let st = self.lock();
        let s = st.slots.get(slot)?;
        match &s.live {
            Some(live) if s.epoch == epoch => Some(live.class),
            _ => None,
        }
    }

    pub fn fields(&self, slot: usize, epoch: u32) -> Option<Vec<Value>> {
        let st = self.lock();
        let s = st.slots.get(slot)?;
        match &s.live {
            Some(live) if s.epoch == epoch => Some(live.fields.clone()),
            _ => None,
        }
    }

    pub fn set_field(&mut self, slot: usize, epoch: u32, field: usize, value: Value) -> bool {
        let mut st = self.lock();
        match st.slots.get_mut(slot) {
            Some(s) => match &mut s.live {
                Some(live) if s.epoch == epoch && field < live.fields.len() => {
                    live.fields[field] = value;
                    true
                }
                _ => false,
            },
            None => false,
        }
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ArrayLive {
    pub count: AtomicI64,
    pub elems: Vec<Value>,
}

struct ArrayState {
    entries: Vec<Option<ArrayLive>>,
    free: Vec<usize>,
}

pub struct ArrayTable {
    state: Arc<Mutex<ArrayState>>,
}

impl ArrayTable {
    pub fn new() -> ArrayTable {
        ArrayTable {
            state: Arc::new(Mutex::new(ArrayState { entries: Vec::new(), free: Vec::new() })),
        }
    }

    pub fn share(&self) -> ArrayTable {
        ArrayTable { state: Arc::clone(&self.state) }
    }

    pub fn same(&self, other: &ArrayTable) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ArrayState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn alloc(&mut self, cap: usize) -> usize {
        let entry = ArrayLive {
            count: AtomicI64::new(1),
            elems: Vec::with_capacity(cap),
        };
        let mut st = self.lock();
        if let Some(id) = st.free.pop() {
            st.entries[id] = Some(entry);
            id
        } else {
            let id = st.entries.len();
            st.entries.push(Some(entry));
            id
        }
    }

    pub fn retain(&mut self, id: usize) {
        let st = self.lock();
        if let Some(live) = st.entries.get(id).and_then(|e| e.as_ref()) {
            live.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn release(&mut self, id: usize) -> Option<Vec<Value>> {
        let st = self.lock();
        let prev = match st.entries.get(id).and_then(|e| e.as_ref()) {
            Some(live) => live.count.fetch_sub(1, Ordering::SeqCst),
            None => return None,
        };
        if prev <= 0 {
            if let Some(live) = st.entries.get(id).and_then(|e| e.as_ref()) {
                live.count.fetch_add(1, Ordering::SeqCst);
            }
        }
        if prev > 1 {
            return None;
        }
        let mut st = st;
        let elems = st.entries.get_mut(id)?.take().map(|l| l.elems).unwrap_or_default();
        st.free.push(id);
        Some(elems)
    }

    pub fn count(&self, id: usize) -> Option<usize> {
        let st = self.lock();
        st.entries
            .get(id)
            .and_then(|e| e.as_ref())
            .map(|l| l.count.load(Ordering::SeqCst).max(0) as usize)
    }

    pub fn len(&self, id: usize) -> Option<usize> {
        let st = self.lock();
        st.entries.get(id).and_then(|e| e.as_ref()).map(|l| l.elems.len())
    }

    pub fn with_live<R>(&self, id: usize, f: impl FnOnce(&mut ArrayLive) -> R) -> Option<R> {
        let mut st = self.lock();
        let live = st.entries.get_mut(id)?.as_mut()?;
        Some(f(live))
    }

    pub fn live_count(&self) -> usize {
        let st = self.lock();
        st.entries.iter().filter(|e| e.is_some()).count()
    }
}

impl Default for ArrayTable {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum GMapKey {
    Int(i64),
    Float(u64),
    Bool(bool),
    Str(String),
    Obj(usize, u32),
}

#[derive(Clone, Debug)]
pub struct GMapEntry {
    pub key: GMapKey,
    pub val: Value,
    pub active: bool,
}

#[derive(Debug, Default)]
pub struct GenericMapState {
    pub entries: Vec<GMapEntry>,
    pub index: std::collections::HashMap<GMapKey, usize, crate::native::SplitMixBuild>,
    pub count: usize,
}

impl GenericMapState {
    pub fn new() -> GenericMapState {
        GenericMapState {
            entries: Vec::new(),
            index: std::collections::HashMap::with_hasher(crate::native::SplitMixBuild),
            count: 0,
        }
    }

    pub fn set(&mut self, key: GMapKey, val: Value) -> Option<Value> {
        if let Some(ix) = self.index.get(&key).copied() {
            return Some(std::mem::replace(&mut self.entries[ix].val, val));
        }
        let ix = self.entries.len();
        self.entries.push(GMapEntry { key: key.clone(), val, active: true });
        self.index.insert(key, ix);
        self.count += 1;
        None
    }

    pub fn get(&self, key: &GMapKey) -> Option<Value> {
        self.index.get(key).and_then(|ix| {
            let e = &self.entries[*ix];
            if e.active {
                Some(e.val.clone())
            } else {
                None
            }
        })
    }

    pub fn has(&self, key: &GMapKey) -> bool {
        self.index.get(key).is_some_and(|ix| self.entries[*ix].active)
    }

    pub fn delete(&mut self, key: &GMapKey) -> Option<Value> {
        match self.index.remove(key) {
            Some(ix) => {
                if self.entries[ix].active {
                    self.entries[ix].active = false;
                    self.count -= 1;
                    Some(std::mem::replace(&mut self.entries[ix].val, Value::Null))
                } else {
                    None
                }
            }
            None => None,
        }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn clear(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        for e in self.entries.iter_mut() {
            if e.active {
                e.active = false;
                out.push(std::mem::replace(&mut e.val, Value::Null));
            }
        }
        self.index.clear();
        self.count = 0;
        out
    }

    pub fn live_keys(&self) -> Vec<Value> {
        self.entries
            .iter()
            .filter(|e| e.active)
            .map(|e| match &e.key {
                GMapKey::Int(i) => Value::Int(*i),
                GMapKey::Float(b) => Value::Float(f64::from_bits(*b), lir::instr::FloatKind::Strict),
                GMapKey::Bool(b) => Value::Bool(*b),
                GMapKey::Str(s) => Value::Str(s.clone()),
                GMapKey::Obj(slot, epoch) => Value::Obj { slot: *slot, epoch: *epoch },
            })
            .collect()
    }

    pub fn live_vals(&self) -> Vec<Value> {
        self.entries.iter().filter(|e| e.active).map(|e| e.val.clone()).collect()
    }

    pub fn drain(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        for e in self.entries.drain(..) {
            if e.active {
                out.push(e.val);
            }
        }
        self.index.clear();
        self.count = 0;
        out
    }
}

#[derive(Debug, Default, Clone)]
pub struct MapTable {
    state: Arc<Mutex<MapState>>,
}

#[derive(Debug, Default)]
struct MapState {
    entries: Vec<Option<crate::native::NativeMapState>>,
    free: Vec<i64>,
    next: i64,
}

#[derive(Debug, Default, Clone)]
pub struct GenericMapTable {
    state: Arc<Mutex<GenericMapStateInner>>,
}

#[derive(Debug, Default)]
struct GenericMapStateInner {
    entries: Vec<Option<GenericMapState>>,
    free: Vec<i64>,
    next: i64,
}

impl GenericMapTable {
    pub fn share(&self) -> GenericMapTable {
        GenericMapTable { state: Arc::clone(&self.state) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, GenericMapStateInner> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn alloc(&self) -> i64 {
        let mut inner = self.lock();
        let state = GenericMapState::new();
        if let Some(id) = inner.free.pop() {
            inner.entries[id as usize] = Some(state);
            return id + 1;
        }
        let id = inner.next;
        inner.next += 1;
        let idx = inner.entries.len();
        inner.entries.push(Some(state));
        debug_assert_eq!(id as usize, idx);
        id + 1
    }

    pub fn with<R>(&self, id: i64, f: impl FnOnce(&GenericMapState) -> R) -> Option<R> {
        let inner = self.lock();
        let idx = (id - 1) as usize;
        inner.entries.get(idx).and_then(|e| e.as_ref()).map(f)
    }

    pub fn with_mut<R>(&self, id: i64, f: impl FnOnce(&mut GenericMapState) -> R) -> Option<R> {
        let mut inner = self.lock();
        let idx = (id - 1) as usize;
        inner.entries.get_mut(idx).and_then(|e| e.as_mut()).map(f)
    }

    pub fn remove(&self, id: i64) -> Option<GenericMapState> {
        let mut inner = self.lock();
        let idx = (id - 1) as usize;
        let st = inner.entries.get_mut(idx)?.take()?;
        inner.free.push(idx as i64);
        Some(st)
    }
}

impl MapTable {
    pub fn share(&self) -> MapTable {
        MapTable { state: Arc::clone(&self.state) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MapState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn alloc(&self) -> i64 {
        let mut inner = self.lock();
        let state = crate::native::NativeMapState::new();
        if let Some(id) = inner.free.pop() {
            inner.entries[id as usize] = Some(state);
            return id;
        }
        let id = inner.next;
        inner.next += 1;
        let idx = inner.entries.len();
        inner.entries.push(Some(state));
        debug_assert_eq!(id as usize, idx);
        id + 1
    }

    pub fn with<R>(
        &self,
        id: i64,
        f: impl FnOnce(&crate::native::NativeMapState) -> R,
    ) -> Option<R> {
        let inner = self.lock();
        let idx = (id - 1) as usize;
        inner.entries.get(idx).and_then(|e| e.as_ref()).map(f)
    }

    pub fn with_mut<R>(
        &self,
        id: i64,
        f: impl FnOnce(&mut crate::native::NativeMapState) -> R,
    ) -> Option<R> {
        let mut inner = self.lock();
        let idx = (id - 1) as usize;
        inner.entries.get_mut(idx).and_then(|e| e.as_mut()).map(f)
    }
}
