use diagnostics::Span;
use std::collections::BTreeMap;

pub const UNKNOWN_SPAN: Span = Span { start: 0, end: 0 };

pub type Local = u32;
pub type BlockId = usize;

pub const HEADER_SIZE: usize = 16;
pub const SLOT_SIZE: usize = 8;

pub fn field_offset(index: usize) -> usize {
    HEADER_SIZE + index * SLOT_SIZE
}

pub fn instance_size(nfields: usize) -> usize {
    HEADER_SIZE + nfields * SLOT_SIZE
}

pub fn enum_max_payloads(desc: &EnumDesc) -> usize {
    desc.variants.iter().map(|v| v.payload.len()).max().unwrap_or(0)
}

pub fn enum_instance_size(desc: &EnumDesc) -> usize {
    instance_size(1 + enum_max_payloads(desc))
}

pub fn flat_sig(ty: &LirType) -> Vec<LirType> {
    match ty {
        LirType::Tuple(items) => items.iter().flat_map(flat_sig).collect(),
        LirType::Range => vec![LirType::I64, LirType::I64, LirType::Bool, LirType::I64],
        other => vec![other.clone()],
    }
}

pub fn walk_instrs(instrs: &[Instr], f: &mut impl FnMut(&Instr)) {
    for ins in instrs {
        f(ins);
        if let Instr::Defer { body, .. } = ins {
            walk_instrs(body, f);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsDecision {
    Const(bool),
    Tag(i64),
    Class(usize),
    Iface(usize),
}

fn prim_tag_of(ty: &LirType) -> Option<i64> {
    match ty {
        LirType::I64 => Some(0),
        LirType::Bool => Some(1),
        LirType::F64(_) => Some(2),
        LirType::Str => Some(3),
        _ => None,
    }
}

pub fn class_idx_by_name(module: &Module, name: &str) -> Result<usize, String> {
    if let Some(ci) = module.class_index.get(name) {
        return Ok(*ci);
    }
    let short = name.rsplit('.').next().unwrap_or(name);
    let mut found = None;
    for (ci, c) in module.classes.iter().enumerate() {
        if c.name.rsplit('.').next().unwrap_or(&c.name) == short {
            if found.is_some() {
                return Err(format!("ambiguous type `{name}`"));
            }
            found = Some(ci);
        }
    }
    found.ok_or_else(|| format!("unknown type `{name}`"))
}

pub fn iface_idx_by_name(module: &Module, name: &str) -> Result<usize, String> {    if let Some(ii) = module.interface_index.get(name) {
        return Ok(*ii);
    }
    let short = name.rsplit('.').next().unwrap_or(name);
    let mut found = None;
    for (ii, it) in module.interfaces.iter().enumerate() {
        if it.name.rsplit('.').next().unwrap_or(&it.name) == short {
            if found.is_some() {
                return Err(format!("ambiguous type `{name}`"));
            }
            found = Some(ii);
        }
    }
    found.ok_or_else(|| format!("unknown type `{name}`"))
}

pub fn ancestors_of(module: &Module, mut ci: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut guard = 0usize;
    while let Some(p) = module.classes.get(ci).and_then(|c| c.parent) {
        if guard > module.classes.len() {
            break;
        }
        guard += 1;
        if out.contains(&p) {
            break;
        }
        out.push(p);
        ci = p;
    }
    out
}

pub fn is_subclass_of(module: &Module, child: usize, ancestor: usize) -> bool {
    child == ancestor || ancestors_of(module, child).contains(&ancestor)
}

pub fn subclasses_of(module: &Module, ancestor: usize) -> Vec<usize> {
    module
        .classes
        .iter()
        .enumerate()
        .filter(|(ci, _)| *ci != ancestor && is_subclass_of(module, *ci, ancestor))
        .map(|(ci, _)| ci)
        .collect()
}

pub fn resolve_is(module: &Module, scrut_ty: &LirType, target: &LirType) -> Result<IsDecision, String> {
    if *target == LirType::Any {
        return Ok(IsDecision::Const(true));
    }
    if let Some(pt) = prim_tag_of(target) {
        return match prim_tag_of(scrut_ty) {
            Some(t2) => Ok(IsDecision::Const(t2 == pt)),
            None if *scrut_ty == LirType::Any => Ok(IsDecision::Tag(pt)),
            None => Ok(IsDecision::Const(false)),
        };
    }
    match target {
        LirType::Obj(name) => {
            if let Ok(ii) = iface_idx_by_name(module, name) {
                return match scrut_ty {
                    LirType::Obj(_) | LirType::Any | LirType::Error => Ok(IsDecision::Iface(ii)),
                    _ => Ok(IsDecision::Const(false)),
                };
            }
            let ci = class_idx_by_name(module, name)?;
            match scrut_ty {
                LirType::Obj(_) | LirType::Any | LirType::Error => Ok(IsDecision::Class(ci)),
                _ => Ok(IsDecision::Const(false)),
            }
        }
        LirType::Array(_) => match scrut_ty {
            LirType::Array(_) => Ok(IsDecision::Const(true)),
            LirType::Any => Err("cannot test `is Array` on `Any`; bind to a typed local first".to_string()),
            _ => Ok(IsDecision::Const(false)),
        },
        LirType::Enum(ei) => match scrut_ty {
            LirType::Enum(ei2) => Ok(IsDecision::Const(*ei == *ei2)),
            LirType::Any => Err("cannot test `is Enum` on `Any`; bind to a typed local first".to_string()),
            _ => Ok(IsDecision::Const(false)),
        },
        LirType::Tuple(items) => match scrut_ty {
            LirType::Tuple(vtys) => Ok(IsDecision::Const(vtys == items)),
            LirType::Any => Err("cannot test `is Tuple` on `Any`; bind to a typed local first".to_string()),
            _ => Ok(IsDecision::Const(false)),
        },
        LirType::Range => match scrut_ty {
            LirType::Range => Ok(IsDecision::Const(true)),
            LirType::Any => Err("cannot test `is Range` on `Any`; bind to a typed local first".to_string()),
            _ => Ok(IsDecision::Const(false)),
        },
        _ => {
            if scrut_ty == target {
                Ok(IsDecision::Const(true))
            } else if *scrut_ty == LirType::Any {
                Err("cannot test that type on `Any`; bind to a typed local first".to_string())
            } else {
                Ok(IsDecision::Const(false))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatKind {
    Strict,
    Fast,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LirType {
    I64,
    I8,
    F64(FloatKind),
    Bool,
    Str,
    Null,
    Any,
    Obj(String),
    Enum(usize),
    Pointer(Box<LirType>),
    Pool,
    Vec4f,
    Vec4i,
    Array(Box<LirType>),
    Tuple(Vec<LirType>),
    Closure,
    GenRef(Option<String>),
    Range,
    Error,
    Void,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Int(i64),
    Float(f64, FloatKind),
    Bool(bool),
    Str(String),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Zshr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumKind {
    Int,
    Float(FloatKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertKind {
    IntToFloat(FloatKind),
    FloatToInt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecKind {
    F,
    I,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecOp {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecUnaryOp {
    Sqrt,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Instr {
    Const {
        span: Span,
        dst: Local,
        lit: Lit,
    },
    Copy {
        span: Span,
        dst: Local,
        src: Local,
    },
    Cast {
        span: Span,
        dst: Local,
        src: Local,
    },
    Convert {
        span: Span,
        dst: Local,
        src: Local,
        kind: ConvertKind,
    },
    Arith {
        span: Span,
        op: ArithOp,
        kind: NumKind,
        dst: Local,
        lhs: Local,
        rhs: Local,
    },
    Fma {
        span: Span,
        dst: Local,
        a: Local,
        b: Local,
        c: Local,
    },
    Cmp {
        span: Span,
        op: CmpOp,
        kind: NumKind,
        dst: Local,
        lhs: Local,
        rhs: Local,
    },
    Not {
        span: Span,
        dst: Local,
        src: Local,
    },
    Neg {
        span: Span,
        kind: NumKind,
        dst: Local,
        src: Local,
    },
    Concat {
        span: Span,
        dst: Local,
        lhs: Local,
        rhs: Local,
    },
    ToStr {
        span: Span,
        dst: Local,
        src: Local,
    },
    Range {
        span: Span,
        dst: Local,
        lo: Local,
        hi: Local,
        inclusive: bool,
    },
    Stride {
        span: Span,
        dst: Local,
        range: Local,
        step: Local,
    },
    ArrayNew {
        span: Span,
        dst: Local,
        cap: usize,
        elem_size: usize,
    },
    ArrayPush {
        span: Span,
        arr: Local,
        value: Local,
        elem_size: usize,
    },
    ArrayPop {
        span: Span,
        dst: Local,
        arr: Local,
    },
    ArrayLen {
        span: Span,
        dst: Local,
        arr: Local,
    },
    ArrayGet {
        span: Span,
        dst: Local,
        arr: Local,
        index: Local,
        elem_size: usize,
        unchecked: bool,
    },
    ArraySet {
        span: Span,
        arr: Local,
        index: Local,
        value: Local,
        elem_size: usize,
        unchecked: bool,
    },
    ObjNew {
        span: Span,
        dst: Local,
        class: usize,
        instance_size: usize,
    },
    StackAlloc {
        span: Span,
        dst: Local,
        class: usize,
        instance_size: usize,
    },
    EnumNew {
        span: Span,
        dst: Local,
        enu: usize,
        variant: usize,
        payload: Vec<Local>,
    },
    EnumPayload {
        span: Span,
        dst: Local,
        scrut: Local,
        index: usize,
    },
    EnumTag {
        span: Span,
        dst: Local,
        scrut: Local,
    },
    Extract {
        span: Span,
        dst: Local,
        base: Local,
        index: usize,
        field: String,
    },
    AddrOf {
        span: Span,
        dst: Local,
        src: Local,
    },
    PtrLoad {
        span: Span,
        dst: Local,
        ptr: Local,
        volatile: bool,
    },
    PtrStore {
        span: Span,
        ptr: Local,
        val: Local,
        volatile: bool,
    },
    RangeLo {
        span: Span,
        dst: Local,
        range: Local,
    },
    RangeHi {
        span: Span,
        dst: Local,
        range: Local,
    },
    RangeStep {
        span: Span,
        dst: Local,
        range: Local,
    },
    GetField {
        span: Span,
        dst: Local,
        obj: Local,
        field: usize,
    },
    GetFieldByName {
        span: Span,
        dst: Local,
        obj: Local,
        field: String,
    },
    SetField {
        span: Span,
        obj: Local,
        field: usize,
        value: Local,
    },
    SetFieldByName {
        span: Span,
        obj: Local,
        field: String,
        value: Local,
    },
    ClosureNew {
        span: Span,
        dst: Local,
        func: usize,
        captures: Vec<Local>,
        decay: bool,
        decay_this: bool,
    },
    Call {
        span: Span,
        dsts: Vec<Local>,
        err: Option<Local>,
        target: CallTarget,
        args: Vec<Local>,
    },
    GenRefOf {
        span: Span,
        dst: Local,
        obj: Local,
    },
    GenRefEmpty {
        span: Span,
        dst: Local,
    },
    GenRefGet {
        span: Span,
        dst: Local,
        gref: Local,
    },
    GenRefInvalidate {
        span: Span,
        obj: Local,
    },
    ThreadSpawn {
        span: Span,
        dst: Local,
        func: usize,
        closure: Option<Local>,
        ret_tag: u32,
    },
    ThreadJoin {
        span: Span,
        dst: Local,
        handle: Local,
    },
    PoolInit {
        span: Span,
        dst: Local,
        id: Local,
        workers: Local,
    },
    PoolSubmit {
        span: Span,
        dst: Local,
        pool: Local,
        func: usize,
        arg: Option<Local>,
        closure: Option<Local>,
        ret_tag: u32,
    },
    PoolParallelFor {
        span: Span,
        pool: Local,
        start: Local,
        end: Local,
        chunk: Local,
        func: usize,
        closure: Option<Local>,
    },
    PoolJoin {
        span: Span,
        pool: Local,
    },
    PoolShutdown {
        span: Span,
        pool: Local,
    },
    VecNew {
        span: Span,
        dst: Local,
        kind: VecKind,
        x: Local,
        y: Local,
        z: Local,
        w: Local,
    },
    VecSplat {
        span: Span,
        dst: Local,
        kind: VecKind,
        val: Local,
    },
    VecExtract {
        span: Span,
        dst: Local,
        vec: Local,
        lane: Local,
    },
    VecInsert {
        span: Span,
        dst: Local,
        vec: Local,
        lane: u8,
        val: Local,
    },
    VecArith {
        span: Span,
        dst: Local,
        op: VecOp,
        kind: VecKind,
        lhs: Local,
        rhs: Local,
    },
    VecUnary {
        span: Span,
        dst: Local,
        op: VecUnaryOp,
        src: Local,
    },
    VecDot {
        span: Span,
        dst: Local,
        lhs: Local,
        rhs: Local,
    },
    ReleaseField {
        span: Span,
        obj: Local,
        field: usize,
    },
    Retain {
        span: Span,
        obj: Local,
    },
    Release {
        span: Span,
        obj: Local,
    },
    ReleaseAs {
        span: Span,
        obj: Local,
        class: usize,
    },
    Defer {
        span: Span,
        body: Vec<Instr>,
    },
    RunDefers {
        span: Span,
        keep: usize,
    },
    Assert {
        span: Span,
        cond: Local,
        message: Local,
    },
    Panic {
        span: Span,
        message: Local,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CallTarget {
    Fn(usize),
    Method {
        class: usize,
        method: usize,
    },
    Dyn {
        obj: Local,
        method: String,
    },
    Builtin(String),
    Value(Local),
    Foreign {
        lib: String,
        symbol: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum SwitchPat {
    Int(i64),
    Is { tag: String, source: Local, check: IsDecision },
    Range { lo: i64, hi: i64, inclusive: bool },
    Enum { enu: usize, variant: usize },
}

#[derive(Clone, Debug)]
pub enum Terminator {
    Ret(Vec<Local>),
    Br(BlockId),
    BrIf {
        span: Span,
        cond: Local,
        then_bb: BlockId,
        else_bb: BlockId,
    },
    BrErr {
        span: Span,
        err: Local,
        catch_bb: BlockId,
        catch_bind: Local,
        next_bb: BlockId,
        depth: usize,
    },
    Switch {
        span: Span,
        scrut: Local,
        cases: Vec<(SwitchPat, BlockId)>,
        default: BlockId,
    },
    Throw {
        span: Span,
        src: Local,
        catch: Option<(BlockId, Local, usize)>,
    },
    Rethrow {
        span: Span,
        catch_bb: BlockId,
        err: Local,
        depth: usize,
    },
    Unreachable {
        span: Span,
    },
}

#[derive(Clone, Debug)]
pub struct Block {
    pub instrs: Vec<Instr>,
    pub term: Terminator,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<LirType>,
    pub sig_params: Vec<LirType>,
    pub ret: LirType,
    pub throws: bool,
    pub is_unsafe: bool,
    pub method_self: bool,
    pub is_pub: bool,
    pub is_closure: bool,
    pub locals: Vec<LirType>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug)]
pub struct FieldDesc {
    pub name: String,
    pub ty: LirType,
    pub private: bool,
    pub owner: String,
}

#[derive(Clone, Debug)]
pub struct ClassDesc {
    pub name: String,
    pub is_struct: bool,
    pub type_params: Vec<String>,
    pub fields: Vec<FieldDesc>,
    pub field_index: BTreeMap<String, usize>,
    pub methods: BTreeMap<String, MethodRef>,
    pub deinit: Option<usize>,
    pub dtor: Option<usize>,
    pub ifaces: Vec<usize>,
    pub parent: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct IfaceMethod {
    pub name: String,
    pub params: Vec<LirType>,
    pub ret: LirType,
}

#[derive(Clone, Debug)]
pub struct InterfaceDesc {
    pub name: String,
    pub type_params: Vec<String>,
    pub methods: Vec<IfaceMethod>,
    pub method_index: BTreeMap<String, usize>,
}

#[derive(Clone, Debug)]
pub struct MethodRef {
    pub id: usize,
    pub private: bool,
    pub owner: String,
}

#[derive(Clone, Debug)]
pub struct VariantDesc {
    pub name: String,
    pub payload: Vec<LirType>,
}

#[derive(Clone, Debug)]
pub struct EnumDesc {
    pub name: String,
    pub variants: Vec<VariantDesc>,
    pub variant_index: BTreeMap<String, usize>,
    pub dtor: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct NsExport {
    pub name: String,
    pub kind: NsKind,
}

#[derive(Clone, Debug)]
pub enum NsKind {
    Function,
    Class,
    Enum,
    Const(Lit),
}

#[derive(Clone, Debug)]
pub struct NamespaceDesc {
    pub alias: String,
    pub key: String,
    pub exports: Vec<NsExport>,
}

#[derive(Clone, Debug, Default)]
pub struct Module {
    pub classes: Vec<ClassDesc>,
    pub class_index: BTreeMap<String, usize>,
    pub interfaces: Vec<InterfaceDesc>,
    pub interface_index: BTreeMap<String, usize>,
    pub enums: Vec<EnumDesc>,
    pub enum_index: BTreeMap<String, usize>,
    pub functions: Vec<Function>,
    pub fn_index: BTreeMap<String, usize>,
    pub array_dtors: BTreeMap<String, usize>,
    pub foreign: Vec<ForeignFn>,
    pub foreign_index: BTreeMap<String, usize>,
    pub namespaces: BTreeMap<String, NamespaceDesc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignFn {
    pub local: String,
    pub lib: String,
    pub symbol: String,
    pub params: Vec<LirType>,
    pub ret: LirType,
}

impl Module {
    pub fn fn_id(&self, name: &str) -> Option<usize> {
        self.fn_index.get(name).copied()
    }
}

pub fn native_libs(module: &Module) -> Vec<String> {
    let mut libs: Vec<String> = module.foreign.iter().map(|f| f.lib.clone()).collect();
    libs.sort();
    libs.dedup();
    libs.retain(|l| l != "c");
    libs
}

pub fn clean_native_lib(lib: &str) -> String {
    let s = lib.strip_prefix("local:").unwrap_or(lib);
    let s = s.strip_prefix("lib").unwrap_or(s);
    s.to_string()
}

pub fn is_c_abi(ty: &LirType) -> bool {
    match ty {
        LirType::I64 | LirType::I8 | LirType::F64(_) | LirType::Bool | LirType::Pointer(_) => true,
        _ => false,
    }
}

pub fn elem_size(ty: &LirType) -> usize {
    match ty {
        LirType::Array(inner) => elem_size(inner),
        _ => 8,
    }
}

/// Compact type tag for the `@std/io` pretty-printer: how one 8-byte slot
/// reads. `array` elements resolve through the array-kind registry;
/// `enum:N` resolves payloads through the enum schema registry.
pub fn pretty_kind(ty: &LirType) -> std::borrow::Cow<'static, str> {
    match ty {
        LirType::I64 | LirType::I8 => std::borrow::Cow::Borrowed("int"),
        LirType::Bool => std::borrow::Cow::Borrowed("bool"),
        LirType::F64(_) => std::borrow::Cow::Borrowed("float"),
        LirType::Str => std::borrow::Cow::Borrowed("str"),
        LirType::Any => std::borrow::Cow::Borrowed("any"),
        LirType::Array(_) => std::borrow::Cow::Borrowed("array"),
        LirType::Obj(_) => std::borrow::Cow::Borrowed("obj"),
        LirType::Enum(id) => std::borrow::Cow::Owned(format!("enum:{id}")),
        _ => std::borrow::Cow::Borrowed("other"),
    }
}

/// Numeric twin of `pretty_kind` for array element notes, which cross the
/// codegen boundary as integers: 1 int, 2 bool, 3 float, 4 str, 5 any,
/// 6 array, 7 obj, 8 enum (companion holds the enum index), else 9.
pub fn pretty_kind_code(ty: &LirType) -> (u64, u64) {
    match ty {
        LirType::I64 | LirType::I8 => (1, 0),
        LirType::Bool => (2, 0),
        LirType::F64(_) => (3, 0),
        LirType::Str => (4, 0),
        LirType::Any => (5, 0),
        LirType::Array(_) => (6, 0),
        LirType::Obj(_) => (7, 0),
        LirType::Enum(id) => (8, *id as u64),
        _ => (9, 0),
    }
}

/// `name:kind,...` in declaration order for one class.
pub fn pretty_field_desc(class: &ClassDesc) -> String {
    class
        .fields
        .iter()
        .map(|f| format!("{}:{}", f.name, pretty_kind(&f.ty)))
        .collect::<Vec<_>>()
        .join(",")
}

/// `Variant:kind,...` for one enum variant.
pub fn pretty_variant_desc(variant: &VariantDesc) -> String {
    let kinds = variant
        .payload
        .iter()
        .map(pretty_kind)
        .collect::<Vec<_>>()
        .join(",");
    if kinds.is_empty() {
        variant.name.clone()
    } else {
        format!("{}:{kinds}", variant.name)
    }
}

fn ns_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '|' => out.push_str("\\p"),
            ';' => out.push_str("\\s"),
            ':' => out.push_str("\\c"),
            ',' => out.push_str("\\m"),
            '=' => out.push_str("\\e"),
            _ => out.push(c),
        }
    }
    out
}

fn ns_unescape(s: &str) -> String {
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

fn ns_split(s: &str, sep: char) -> Vec<String> {
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

/// `alias|name:kind,...` for one namespace, crossing to native pretty.
pub fn pretty_namespace_desc(ns: &NamespaceDesc) -> String {
    let mut parts = Vec::with_capacity(ns.exports.len());
    for e in &ns.exports {
        let kind = match &e.kind {
            NsKind::Function => "fn".to_string(),
            NsKind::Class => "class".to_string(),
            NsKind::Enum => "enum".to_string(),
            NsKind::Const(lit) => match lit {
                Lit::Int(v) => format!("int={v}"),
                Lit::Bool(v) => format!("bool={v}"),
                Lit::Float(v, _) => format!("float={}", v.to_bits()),
                Lit::Str(v) => format!("str={}", ns_escape(v)),
                Lit::Null => "null".to_string(),
            },
        };
        parts.push(format!("{}:{}", ns_escape(&e.name), kind));
    }
    format!("{}|{}", ns_escape(&ns.alias), parts.join(","))
}

pub fn parse_namespace_desc(text: &str) -> Option<(String, Vec<(String, NsKind)>)> {
    let (alias, rest) = text.split_once('|')?;
    let alias = ns_unescape(alias);
    let mut out = Vec::new();
    if rest.trim().is_empty() {
        return Some((alias, out));
    }
    for part in ns_split(rest, ',') {
        if part.trim().is_empty() {
            continue;
        }
        let (name, kind) = part.split_once(':')?;
        let name = ns_unescape(name);
        let kind = match kind {
            "fn" => NsKind::Function,
            "class" => NsKind::Class,
            "enum" => NsKind::Enum,
            "null" => NsKind::Const(Lit::Null),
            k if k.starts_with("int=") => k[4..].parse::<i64>().ok().map(Lit::Int).map(NsKind::Const)?,
            k if k.starts_with("bool=") => k[5..].parse::<bool>().ok().map(Lit::Bool).map(NsKind::Const)?,
            k if k.starts_with("float=") => k[6..].parse::<u64>().ok().map(|b| Lit::Float(f64::from_bits(b), FloatKind::Strict)).map(NsKind::Const)?,
            k if k.starts_with("str=") => NsKind::Const(Lit::Str(ns_unescape(&k[4..]))),
            _ => continue,
        };
        out.push((name, kind));
    }
    Some((alias, out))
}

pub fn type_key(ty: &LirType) -> String {
    match ty {
        LirType::I64 => "i64".to_string(),
        LirType::I8 => "i8".to_string(),
        LirType::F64(k) => format!("f64:{k:?}"),
        LirType::Bool => "bool".to_string(),
        LirType::Str => "str".to_string(),
        LirType::Null => "null".to_string(),
        LirType::Any => "any".to_string(),
        LirType::Obj(n) => format!("obj:{n}"),
        LirType::Enum(id) => format!("enum:{id}"),
        LirType::Pointer(_) => "ptr".to_string(),
        LirType::Pool => "pool".to_string(),
        LirType::Vec4f => "vec4f".to_string(),
        LirType::Vec4i => "vec4i".to_string(),
        LirType::Array(inner) => format!("array<{}>", type_key(inner)),
        LirType::Tuple(items) => format!("tuple<{}>", items.iter().map(type_key).collect::<Vec<_>>().join(",")),
        LirType::Closure => "closure".to_string(),
        LirType::GenRef(c) => format!("genref:{}", c.as_deref().unwrap_or("any")),
        LirType::Range => "range".to_string(),
        LirType::Error => "error".to_string(),
        LirType::Void => "void".to_string(),
    }
}

impl Instr {
    pub fn span(&self) -> Span {
        match self {
            Instr::Const { span, .. }
            | Instr::Copy { span, .. }
            | Instr::Cast { span, .. }
            | Instr::Convert { span, .. }
            | Instr::Arith { span, .. }
            | Instr::Cmp { span, .. }
            | Instr::Not { span, .. }
            | Instr::Fma { span, .. }
            | Instr::Neg { span, .. }
            | Instr::Concat { span, .. }
            | Instr::ToStr { span, .. }
            | Instr::Range { span, .. }
            | Instr::Stride { span, .. }
            | Instr::ArrayNew { span, .. }
            | Instr::ArrayPush { span, .. }
            | Instr::ArrayPop { span, .. }
            | Instr::ArrayLen { span, .. }
            | Instr::ArrayGet { span, .. }
            | Instr::ArraySet { span, .. }
            | Instr::ObjNew { span, .. }
            | Instr::StackAlloc { span, .. }
            | Instr::EnumNew { span, .. }
            | Instr::EnumPayload { span, .. }
            | Instr::EnumTag { span, .. }
            | Instr::Extract { span, .. }
            | Instr::AddrOf { span, .. }
            | Instr::PtrLoad { span, .. }
            | Instr::PtrStore { span, .. }
            | Instr::RangeLo { span, .. }
            | Instr::RangeHi { span, .. }
            | Instr::RangeStep { span, .. }
            | Instr::GetField { span, .. }
            | Instr::GetFieldByName { span, .. }
            | Instr::SetField { span, .. }
            | Instr::SetFieldByName { span, .. }
            | Instr::ClosureNew { span, .. }
            | Instr::Call { span, .. }
            | Instr::GenRefOf { span, .. }
            | Instr::GenRefEmpty { span, .. }
            | Instr::GenRefGet { span, .. }
            | Instr::GenRefInvalidate { span, .. }
            | Instr::ThreadSpawn { span, .. }
            | Instr::ThreadJoin { span, .. }
            | Instr::PoolInit { span, .. }
            | Instr::PoolSubmit { span, .. }
            | Instr::PoolParallelFor { span, .. }
            | Instr::PoolJoin { span, .. }
            | Instr::PoolShutdown { span, .. }
            | Instr::VecNew { span, .. }
            | Instr::VecSplat { span, .. }
            | Instr::VecExtract { span, .. }
            | Instr::VecInsert { span, .. }
            | Instr::VecArith { span, .. }
            | Instr::VecUnary { span, .. }
            | Instr::VecDot { span, .. }
            | Instr::ReleaseField { span, .. }
            | Instr::Retain { span, .. }
            | Instr::Release { span, .. }
            | Instr::ReleaseAs { span, .. }
            | Instr::Defer { span, .. }
            | Instr::RunDefers { span, .. }
            | Instr::Assert { span, .. }
            | Instr::Panic { span, .. } => *span,
        }
    }
}
