use crate::value::{Arena, ArrayTable, ClosureVal, ErrorVal, GenericMapTable, MapTable, PrettyCtx, Value};
use diagnostics::Span;
use lir::instr::*;
use std::collections::BTreeSet;

#[derive(Debug, PartialEq)]
pub enum ExecError {
    Throw(Value),
    Fatal(String),
}

pub struct Machine<'a> {
    pub module: &'a Module,
    pub arena: Arena,
    pub arrays: ArrayTable,
    pub maps: MapTable,
    pub gmaps: GenericMapTable,
    pub output: Vec<String>,
    pub error_span: Option<Span>,
    pub error_func: Option<String>,
    ptr_seq: i64,
    json_maps: BTreeSet<i64>,
}

fn term_span(term: &Terminator) -> Option<Span> {
    match term {
        Terminator::BrIf { span, .. }
        | Terminator::BrErr { span, .. }
        | Terminator::Switch { span, .. }
        | Terminator::Throw { span, .. }
        | Terminator::Rethrow { span, .. }
        | Terminator::Unreachable { span } => Some(*span),
        Terminator::Ret(_) | Terminator::Br(_) => None,
    }
}

fn heap_alias(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array { id: x }, Value::Array { id: y }) => x == y,
        (
            Value::Obj { slot: s1, epoch: e1 },
            Value::Obj { slot: s2, epoch: e2 },
        ) => s1 == s2 && e1 == e2,
        (
            Value::Enum { enu: e1, variant: v1, payload: p1, .. },
            Value::Enum { enu: e2, variant: v2, payload: p2, .. },
        ) => {
            e1 == e2
                && v1 == v2
                && p1.len() == p2.len()
                && p1.iter().zip(p2.iter()).all(|(x, y)| heap_alias(x, y))
        }
        _ => false,
    }
}

struct Frame {
    func: usize,
    locals: Vec<Value>,
    defers: Vec<Vec<Instr>>,
    bb: BlockId,
    owned: BTreeSet<Local>,
    ever_owned: BTreeSet<Local>,
    nborrowed: usize,
}


mod builtin;
mod builtin_json;
mod exec;

impl<'a> Machine<'a> {
    pub fn new(module: &'a Module) -> Machine<'a> {
        Machine {
            module,
            arena: Arena::new(),
            arrays: ArrayTable::default(),
            maps: MapTable::default(),
            gmaps: GenericMapTable::default(),
            output: Vec::new(),
            error_span: None,
            error_func: None,
            ptr_seq: 0,
            json_maps: BTreeSet::new(),
        }
    }


    fn note_error(&mut self, span: Span) {
        if self.error_span.is_none() && span != UNKNOWN_SPAN {
            self.error_span = Some(span);
        }
    }


    fn native_value_error(&mut self, span: Span) -> ExecError {
        self.note_error(span);
        ExecError::Fatal("got null or a value of the wrong type".to_string())
    }


    fn native_arity_error(&mut self, span: Span) -> ExecError {
        self.note_error(span);
        ExecError::Fatal("takes no arguments".to_string())
    }


    fn note_func(&mut self, fr: &Frame) {
        if self.error_func.is_none() {
            self.error_func = Some(self.module.functions[fr.func].name.clone());
        }
    }


    pub fn new_array(&mut self, elems: Vec<Value>) -> Value {
        let id = self.arrays.alloc(elems.len());
        for v in elems {
            let sv = self.shared(v);
            self.arrays.with_live(id, |live| live.elems.push(sv));
        }
        Value::Array { id }
    }


    fn strings_of(&self, id: usize) -> Result<Vec<String>, ExecError> {
        match self.arrays.with_live(id, |live| {
            live.elems
                .iter()
                .map(|e| match e {
                    Value::Str(s) => Some(s.clone()),
                    _ => None,
                })
                .collect::<Option<Vec<String>>>()
        }) {
            Some(Some(items)) => Ok(items),
            _ => Err(ExecError::Fatal("expected Array<String>".to_string())),
        }
    }


    pub fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, ExecError> {
        crate::guard::guard_install();
        #[cfg(target_arch = "wasm32")]
        {
            return self.call_inner(name, args);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            const INTERP_STACK: usize = 64 * 1024 * 1024;
            std::thread::scope(|s| {
                std::thread::Builder::new()
                    .name("rnx-interp".to_string())
                    .stack_size(INTERP_STACK)
                    .spawn_scoped(s, || {
                            crate::guard::guard_thread_init();
                        self.call_inner(name, args)
                    })
                    .map(|h| h.join().unwrap_or(Err(ExecError::Fatal("interpreter thread failed".to_string()))))
                    .unwrap_or_else(|_| Err(ExecError::Fatal("interpreter thread failed to spawn".to_string())))
            })
        }
    }


    fn call_inner(&mut self, name: &str, args: Vec<Value>) -> Result<Value, ExecError> {
        let id = self.module.fn_id(name).ok_or_else(|| {
            ExecError::Fatal(format!("unknown function `{name}`"))
        })?;
        let vals = self.run_fn_public(id, args)?;
        Ok(vals.into_iter().next().unwrap_or(Value::Null))
    }


    pub fn run_fn_public(&mut self, fi: usize, args: Vec<Value>) -> Result<Vec<Value>, ExecError> {
        self.run_fn(fi, args)
    }


    fn run_fn(&mut self, fi: usize, args: Vec<Value>) -> Result<Vec<Value>, ExecError> {
        if unsafe { crate::native::rnx_stack_check() }
            < crate::native::STACK_RESERVE_BYTES as i64
        {
            return Err(ExecError::Fatal(
                crate::native::STACK_EXHAUSTED_MSG.to_string(),
            ));
        }
        let nlocals = self.module.functions[fi].locals.len();
        let mut locals = vec![Value::Null; nlocals];
        for (i, a) in args.into_iter().enumerate() {
            if i < nlocals {
                locals[i] = a;
            }
        }
        let nborrowed = self.module.functions[fi].params.len();
        let mut fr = Frame {
            func: fi,
            locals,
            defers: Vec::new(),
            bb: 0,
            owned: BTreeSet::new(),
            ever_owned: BTreeSet::new(),
            nborrowed,
        };
        loop {
            let term = self.run_block(&mut fr)?;
            match term {
                Flow::Return { values, locals } => {
                    self.drain_defers(&mut fr, 0)?;
                    self.drop_frame_roots(&mut fr, &locals)?;
                    return Ok(values);
                }
                Flow::Goto(bb) => {
                    fr.bb = bb;
                }
            }
        }
    }


    fn run_block(&mut self, fr: &mut Frame) -> Result<Flow, ExecError> {
        let instrs = self.module.functions[fr.func].blocks[fr.bb].instrs.clone();
        for ins in &instrs {
            if let Err(e) = self.exec(fr, ins) {
                self.note_error(ins.span());
                self.note_func(fr);
                return Err(e);
            }
        }
        let term = self.module.functions[fr.func].blocks[fr.bb].term.clone();
        match self.exec_term(fr, term.clone()) {
            Err(e) => {
                if let Some(span) = term_span(&term) {
                    self.note_error(span);
                }
                self.note_func(fr);
                Err(e)
            }
            ok => ok,
        }
    }


    fn drain_defers(&mut self, fr: &mut Frame, keep: usize) -> Result<(), ExecError> {
        while fr.defers.len() > keep {
            let body = fr.defers.pop().unwrap_or_default();
            for ins in &body.clone() {
                if let Err(e) = self.exec(fr, ins) {
                    self.note_error(ins.span());
                    self.note_func(fr);
                    return Err(e);
                }
            }
        }
        Ok(())
    }


    fn heaps(&self) -> crate::ichan::Heaps {
        crate::ichan::Heaps {
            objs: self.arena.share(),
            arrs: self.arrays.share(),
            maps: self.maps.share(),
            gmaps: self.gmaps.share(),
        }
    }


    fn pool_int(&self, fr: &Frame, l: Local, what: &str) -> Result<i64, ExecError> {
        match &fr.locals[l as usize] {
            Value::Int(v) => Ok(*v),
            o => Err(ExecError::Fatal(format!("pool {what} needs Int, got {}", o.display()))),
        }
    }


    fn vec_float(&self, fr: &Frame, l: Local) -> Result<f64, ExecError> {
        match &fr.locals[l as usize] {
            Value::Float(v, _) => Ok(*v),
            Value::Int(v) => Ok(*v as f64),
            o => Err(ExecError::Fatal(format!("vector lane needs Float, got {}", o.display()))),
        }
    }


    fn vec_f32(&self, fr: &Frame, l: Local) -> Result<[f32; 4], ExecError> {
        match &fr.locals[l as usize] {
            Value::Vec4f(v) => Ok(*v),
            o => Err(ExecError::Fatal(format!("Vec4f needed, got {}", o.display()))),
        }
    }


    fn vec_i32(&self, fr: &Frame, l: Local) -> Result<[i32; 4], ExecError> {
        match &fr.locals[l as usize] {
            Value::Vec4i(v) => Ok(*v),
            o => Err(ExecError::Fatal(format!("Vec4i needed, got {}", o.display()))),
        }
    }


    fn pool_handle(&self, fr: &Frame, l: Local) -> Result<i64, ExecError> {
        match &fr.locals[l as usize] {
            Value::Pointer(t) => Ok(*t),
            o => Err(ExecError::Fatal(format!("pool handle needed, got {}", o.display()))),
        }
    }


    fn local_ty(&self, fr: &Frame, l: Local) -> LirType {
        self.module.functions[fr.func]
            .locals
            .get(l as usize)
            .cloned()
            .unwrap_or(LirType::Any)
    }


    fn own(&self, fr: &mut Frame, l: Local) {
        if (l as usize) < fr.nborrowed {
            return;
        }
        fr.owned.insert(l);
        fr.ever_owned.insert(l);
    }


    fn drop_value(&mut self, v: Value) -> Result<(), ExecError> {
        if let Value::Array { id } = v {
            return self.drop_array(id);
        }
        if let Value::Enum { payload, .. } = v {
            for p in payload {
                self.drop_value(p)?;
            }
            return Ok(());
        }
        let (slot, epoch) = match v {
            Value::Obj { slot, epoch } => (slot, epoch),
            _ => return Ok(()),
        };
        let class = match self.arena.get(slot, epoch) {
            Some(c) => c,
            None => return Ok(()),
        };
        let count = match self.arena.count(slot) {
            Some(c) => c,
            None => return Ok(()),
        };

        if count > 1 {
            self.arena.release(slot);
            return Ok(());
        }
        if let Some(deinit) = self.module.classes.get(class).and_then(|c| c.deinit) {
            let o = Value::Obj { slot, epoch };
            let _ = self.run_fn(deinit, vec![o]);
        }
        if let Some(dtor) = self.module.classes.get(class).and_then(|c| c.dtor) {
            let o = Value::Obj { slot, epoch };
            let _ = self.run_fn(dtor, vec![o]);
        }
        self.arena.release(slot);
        Ok(())
    }


    fn drop_array(&mut self, id: usize) -> Result<(), ExecError> {
        let elems = match self.arrays.count(id) {
            None => return Ok(()),
            Some(c) if c > 1 => {
                self.arrays.release(id);
                return Ok(());
            }
            Some(_) => match self.arrays.release(id) {
                Some(elems) => elems,
                None => return Ok(()),
            },
        };
        for e in elems {
            self.drop_value(e)?;
        }
        Ok(())
    }


    fn drop_local(&mut self, fr: &mut Frame, l: Local) -> Result<(), ExecError> {
        fr.owned.remove(&l);
        fr.ever_owned.remove(&l);
        let v = fr.locals[l as usize].clone();
        self.drop_value(v)
    }


    fn drop_frame_roots(&mut self, fr: &mut Frame, ret: &[Local]) -> Result<(), ExecError> {
        let live: Vec<Local> = fr.ever_owned.iter().copied().collect();
        for l in live {
            if ret.contains(&l) {
                continue;
            }
            self.drop_local(fr, l)?;
        }
        Ok(())
    }

}

enum Flow {
    Goto(BlockId),
    Return { values: Vec<Value>, locals: Vec<Local> },
}

fn any_tag_of(v: &Value) -> Option<i64> {
    match v {
        Value::Int(_) => Some(0),
        Value::Bool(_) => Some(1),
        Value::Float(_, _) => Some(2),
        Value::Str(_) => Some(3),
        _ => None,
    }
}

fn obj_class_idx(_module: &Module, arena: &Arena, v: &Value) -> Option<usize> {
    match v {
        Value::Obj { slot, epoch } => arena.get(*slot, *epoch),
        Value::Struct { class, .. } => Some(*class),
        _ => None,
    }
}

fn pat_matches(module: &Module, arena: &Arena, v: &Value, pat: &SwitchPat) -> bool {
    match pat {
        SwitchPat::Int(i) => matches!(v, Value::Int(x) if x == i),
        SwitchPat::Enum { enu, variant } => matches!(
            v,
            Value::Enum { enu: e, variant: vt, .. } if e == enu && vt == variant
        ),
        SwitchPat::Is { tag, check, .. } => {
            if let Value::Error(e) = v {
                return &e.tag == tag;
            }
            match check {
                lir::instr::IsDecision::Const(b) => *b,
                lir::instr::IsDecision::Tag(pt) => any_tag_of(v) == Some(*pt),
                lir::instr::IsDecision::Class(ci) => match obj_class_idx(module, arena, v) {
                    Some(rt) => lir::instr::is_subclass_of(module, rt, *ci),
                    None => false,
                },
                lir::instr::IsDecision::Iface(ii) => match obj_class_idx(module, arena, v) {
                    Some(ci) => module
                        .classes
                        .get(ci)
                        .map(|c| c.ifaces.contains(ii))
                        .unwrap_or(false),
                    None => false,
                },
            }
        }
        SwitchPat::Range { lo, hi, inclusive } => match v {
            Value::Int(x) => {
                if *inclusive {
                    *x >= *lo && *x <= *hi
                } else {
                    *x >= *lo && *x < *hi
                }
            }
            _ => false,
        },
    }
}

impl<'a> Machine<'a> {
    pub fn pretty_ctx(&self, color: bool) -> PrettyCtx<'_> {
        PrettyCtx {
            module: self.module,
            arena: &self.arena,
            arrays: &self.arrays,
            gmaps: &self.gmaps,
            color,
        }
    }

    pub fn to_pretty(&self, v: &Value) -> String {
        let ctx = self.pretty_ctx(false);
        let mut out = String::new();
        v.pretty(&ctx, 0, &mut Vec::new(), &mut out);
        out
    }

    pub fn to_pretty_colored(&self, v: &Value, fd: i64) -> String {
        let ctx = self.pretty_ctx(crate::native::pretty::pretty_color_on(fd));
        let mut out = String::new();
        v.pretty(&ctx, 0, &mut Vec::new(), &mut out);
        out
    }
}

fn arith(op: ArithOp, a: &Value, b: &Value) -> Result<Value, ExecError> {
    match (a, b) {
        (Value::Pointer(x), Value::Int(y)) => match op {
            ArithOp::Add => Ok(Value::Pointer(x.wrapping_add(*y))),
            ArithOp::Sub => Ok(Value::Pointer(x.wrapping_sub(*y))),
            _ => Err(ExecError::Fatal("pointer arithmetic supports `+` and `-` only".to_string())),
        },
        (Value::Int(x), Value::Pointer(y)) => match op {
            ArithOp::Add => Ok(Value::Pointer(x.wrapping_add(*y))),
            _ => Err(ExecError::Fatal("pointer arithmetic supports `+` and `-` only".to_string())),
        },
        (Value::Pointer(x), Value::Pointer(y)) => match op {
            ArithOp::Sub => Ok(Value::Int(x.wrapping_sub(*y))),
            _ => Err(ExecError::Fatal("pointer arithmetic supports `+` and `-` only".to_string())),
        },
        (Value::StackToken(x), Value::Int(y)) => match op {
            ArithOp::Add => Ok(Value::StackToken(x.wrapping_add(*y))),
            ArithOp::Sub => Ok(Value::StackToken(x.wrapping_sub(*y))),
            _ => Err(ExecError::Fatal("pointer arithmetic supports `+` and `-` only".to_string())),
        },
        (Value::Int(x), Value::StackToken(y)) => match op {
            ArithOp::Add => Ok(Value::StackToken(x.wrapping_add(*y))),
            _ => Err(ExecError::Fatal("pointer arithmetic supports `+` and `-` only".to_string())),
        },
        (Value::StackToken(x), Value::StackToken(y)) => match op {
            ArithOp::Sub => Ok(Value::Int(x.wrapping_sub(*y))),
            _ => Err(ExecError::Fatal("pointer arithmetic supports `+` and `-` only".to_string())),
        },
        (Value::Int(x), Value::Int(y)) => {
            let r = match op {
                ArithOp::Add => x.wrapping_add(*y),
                ArithOp::Sub => x.wrapping_sub(*y),
                ArithOp::Mul => x.wrapping_mul(*y),
                ArithOp::Div => {
                    if *y == 0 {
                        return Err(ExecError::Fatal("division by zero".to_string()));
                    }
                    x.wrapping_div(*y)
                }
                ArithOp::Mod => {
                    if *y == 0 {
                        return Err(ExecError::Fatal("division by zero".to_string()));
                    }
                    x.wrapping_rem(*y)
                }
                ArithOp::BitAnd => x & y,
                ArithOp::BitOr => x | y,
                ArithOp::BitXor => x ^ y,
                ArithOp::Shl => x.wrapping_shl((*y as u64 & 63) as u32),
                ArithOp::Shr => x.wrapping_shr((*y as u64 & 63) as u32),
                ArithOp::Zshr => ((*x as u64).wrapping_shr((*y as u64 & 63) as u32)) as i64,
            };
            Ok(Value::Int(r))
        }
        _ => {
            let (x, xk) = num(a)?;
            let (y, yk) = num(b)?;
            let fast = xk == FloatKind::Fast || yk == FloatKind::Fast;
            let k = if fast { FloatKind::Fast } else { FloatKind::Strict };
            let r = match op {
                ArithOp::Add => x + y,
                ArithOp::Sub => x - y,
                ArithOp::Mul => x * y,
                ArithOp::Div => x / y,
                ArithOp::Mod => x % y,
                ArithOp::BitAnd | ArithOp::BitOr | ArithOp::BitXor | ArithOp::Shl | ArithOp::Shr | ArithOp::Zshr => {
                    return Err(ExecError::Fatal("bitwise operations need `Int` operands".to_string()));
                }
            };
            Ok(Value::Float(r, k))
        }
    }
}

fn num(v: &Value) -> Result<(f64, FloatKind), ExecError> {
    match v {
        Value::Int(i) => Ok((*i as f64, FloatKind::Strict)),
        Value::Float(f, k) => Ok((*f, *k)),
        _ => Err(ExecError::Fatal(format!("not a number: {}", v.display()))),
    }
}

fn cmp(op: CmpOp, a: &Value, b: &Value) -> Result<bool, ExecError> {
    match op {
        CmpOp::Eq => Ok(eq(a, b)),
        CmpOp::NotEq => Ok(!eq(a, b)),
        CmpOp::Lt | CmpOp::LtEq | CmpOp::Gt | CmpOp::GtEq => {
            if let (Value::Str(x), Value::Str(y)) = (a, b) {
                return Ok(match op {
                    CmpOp::Lt => x < y,
                    CmpOp::LtEq => x <= y,
                    CmpOp::Gt => x > y,
                    CmpOp::GtEq => x >= y,
                    _ => false,
                });
            }
            let (x, _) = num(a)?;
            let (y, _) = num(b)?;
            Ok(match op {
                CmpOp::Lt => x < y,
                CmpOp::LtEq => x <= y,
                CmpOp::Gt => x > y,
                CmpOp::GtEq => x >= y,
                _ => false,
            })
        }
    }
}

fn eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x, _), Value::Float(y, _)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Null, Value::Null) => true,
        (Value::Obj { slot: a, epoch: x }, Value::Obj { slot: b, epoch: y }) => a == b && x == y,
        _ => a == b,
    }
}
