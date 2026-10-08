use super::instr::{
    ArithOp, Block, CallTarget, ClassDesc, CmpOp, ConvertKind, EnumDesc, FloatKind, ForeignFn,
    Function, IfaceMethod, Instr, InterfaceDesc, IsDecision, LirType, Lit, MethodRef, Module,
    NamespaceDesc, NsKind, NumKind, SwitchPat, Terminator, VecKind, VecOp, VecUnaryOp,
};
use sha2::{Digest, Sha256};

struct H(Sha256);

impl H {
    fn u8(&mut self, v: u8) {
        self.0.update([v]);
    }
    fn u32(&mut self, v: u32) {
        self.0.update(v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.update(v.to_le_bytes());
    }
    fn usize_(&mut self, v: usize) {
        self.u64(v as u64);
    }
    fn i64(&mut self, v: i64) {
        self.0.update(v.to_le_bytes());
    }
    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    fn str(&mut self, s: &str) {
        let b = s.as_bytes();
        self.u64(b.len() as u64);
        self.0.update(b);
    }
    fn local(&mut self, l: u32) {
        self.u32(l);
    }
    fn ty(&mut self, t: &LirType) {
        match t {
            LirType::I64 => self.u8(1),
            LirType::I8 => self.u8(2),
            LirType::F64(k) => {
                self.u8(3);
                self.float_kind(k);
            }
            LirType::Bool => self.u8(4),
            LirType::Str => self.u8(5),
            LirType::Null => self.u8(6),
            LirType::Any => self.u8(7),
            LirType::Obj(n) => {
                self.u8(8);
                self.str(n);
            }
            LirType::Enum(id) => {
                self.u8(9);
                self.usize_(*id);
            }
            LirType::Pointer(inner) => {
                self.u8(10);
                self.ty(inner);
            }
            LirType::Pool => self.u8(11),
            LirType::Vec4f => self.u8(12),
            LirType::Vec4i => self.u8(13),
            LirType::Array(inner) => {
                self.u8(14);
                self.ty(inner);
            }
            LirType::Tuple(items) => {
                self.u8(15);
                self.usize_(items.len());
                for t in items {
                    self.ty(t);
                }
            }
            LirType::Closure => self.u8(16),
            LirType::GenRef(c) => {
                self.u8(17);
                match c {
                    Some(n) => {
                        self.u8(1);
                        self.str(n);
                    }
                    None => self.u8(0),
                }
            }
            LirType::Range => self.u8(18),
            LirType::Error => self.u8(19),
            LirType::Void => self.u8(20),
        }
    }
    fn float_kind(&mut self, k: &FloatKind) {
        match k {
            FloatKind::Strict => self.u8(0),
            FloatKind::Fast => self.u8(1),
        }
    }
    fn lit(&mut self, l: &Lit) {
        match l {
            Lit::Int(v) => {
                self.u8(1);
                self.i64(*v);
            }
            Lit::Float(v, k) => {
                self.u8(2);
                self.u64(v.to_bits());
                self.float_kind(k);
            }
            Lit::Bool(v) => {
                self.u8(3);
                self.bool(*v);
            }
            Lit::Str(s) => {
                self.u8(4);
                self.str(s);
            }
            Lit::Null => self.u8(5),
        }
    }
    fn span(&mut self, s: &diagnostics::Span) {
        self.u32(s.start);
        self.u32(s.end);
    }
    fn arith_op(&mut self, op: &ArithOp) {
        self.u8(match op {
            ArithOp::Add => 1,
            ArithOp::Sub => 2,
            ArithOp::Mul => 3,
            ArithOp::Div => 4,
            ArithOp::Mod => 5,
            ArithOp::BitAnd => 6,
            ArithOp::BitOr => 7,
            ArithOp::BitXor => 8,
            ArithOp::Shl => 9,
            ArithOp::Shr => 10,
            ArithOp::Zshr => 11,
        });
    }
    fn cmp_op(&mut self, op: &CmpOp) {
        self.u8(match op {
            CmpOp::Eq => 1,
            CmpOp::NotEq => 2,
            CmpOp::Lt => 3,
            CmpOp::LtEq => 4,
            CmpOp::Gt => 5,
            CmpOp::GtEq => 6,
        });
    }
    fn num_kind(&mut self, k: &NumKind) {
        match k {
            NumKind::Int => self.u8(0),
            NumKind::Float(f) => {
                self.u8(1);
                self.float_kind(f);
            }
        }
    }
    fn convert_kind(&mut self, k: &ConvertKind) {
        match k {
            ConvertKind::IntToFloat(f) => {
                self.u8(0);
                self.float_kind(f);
            }
            ConvertKind::FloatToInt => self.u8(1),
        }
    }
    fn vec_kind(&mut self, k: &VecKind) {
        match k {
            VecKind::F => self.u8(0),
            VecKind::I => self.u8(1),
        }
    }
    fn vec_op(&mut self, op: &VecOp) {
        self.u8(match op {
            VecOp::Add => 1,
            VecOp::Sub => 2,
            VecOp::Mul => 3,
            VecOp::Div => 4,
            VecOp::Min => 5,
            VecOp::Max => 6,
        });
    }
    fn target(&mut self, t: &CallTarget) {
        match t {
            CallTarget::Fn(id) => {
                self.u8(1);
                self.usize_(*id);
            }
            CallTarget::Method { class, method } => {
                self.u8(2);
                self.usize_(*class);
                self.usize_(*method);
            }
            CallTarget::Dyn { obj, method } => {
                self.u8(3);
                self.local(*obj);
                self.str(method);
            }
            CallTarget::Builtin(n) => {
                self.u8(4);
                self.str(n);
            }
            CallTarget::Value(l) => {
                self.u8(5);
                self.local(*l);
            }
            CallTarget::Foreign { lib, symbol } => {
                self.u8(6);
                self.str(lib);
                self.str(symbol);
            }
        }
    }
    fn is_decision(&mut self, d: &IsDecision) {
        match d {
            IsDecision::Const(b) => {
                self.u8(1);
                self.bool(*b);
            }
            IsDecision::Tag(t) => {
                self.u8(2);
                self.i64(*t);
            }
            IsDecision::Class(c) => {
                self.u8(3);
                self.usize_(*c);
            }
            IsDecision::Iface(i) => {
                self.u8(4);
                self.usize_(*i);
            }
        }
    }
    fn switch_pat(&mut self, p: &SwitchPat) {
        match p {
            SwitchPat::Int(n) => {
                self.u8(1);
                self.i64(*n);
            }
            SwitchPat::Is { tag, source, check } => {
                self.u8(2);
                self.str(tag);
                self.local(*source);
                self.is_decision(check);
            }
            SwitchPat::Range { lo, hi, inclusive } => {
                self.u8(3);
                self.i64(*lo);
                self.i64(*hi);
                self.bool(*inclusive);
            }
            SwitchPat::Enum { enu, variant } => {
                self.u8(4);
                self.usize_(*enu);
                self.usize_(*variant);
            }
        }
    }
    fn opt_local(&mut self, l: &Option<u32>) {
        match l {
            Some(v) => {
                self.u8(1);
                self.local(*v);
            }
            None => self.u8(0),
        }
    }
    fn instr(&mut self, ins: &Instr) {
        match ins {
            Instr::Const { span, dst, lit } => {
                self.u8(1);
                self.span(span);
                self.local(*dst);
                self.lit(lit);
            }
            Instr::Copy { span, dst, src } => {
                self.u8(2);
                self.span(span);
                self.local(*dst);
                self.local(*src);
            }
            Instr::Cast { span, dst, src } => {
                self.u8(3);
                self.span(span);
                self.local(*dst);
                self.local(*src);
            }
            Instr::Convert { span, dst, src, kind } => {
                self.u8(4);
                self.span(span);
                self.local(*dst);
                self.local(*src);
                self.convert_kind(kind);
            }
            Instr::Arith { span, op, kind, dst, lhs, rhs } => {
                self.u8(5);
                self.span(span);
                self.arith_op(op);
                self.num_kind(kind);
                self.local(*dst);
                self.local(*lhs);
                self.local(*rhs);
            }
            Instr::Fma { span, dst, a, b, c } => {
                self.u8(6);
                self.span(span);
                self.local(*dst);
                self.local(*a);
                self.local(*b);
                self.local(*c);
            }
            Instr::Cmp { span, op, kind, dst, lhs, rhs } => {
                self.u8(7);
                self.span(span);
                self.cmp_op(op);
                self.num_kind(kind);
                self.local(*dst);
                self.local(*lhs);
                self.local(*rhs);
            }
            Instr::Not { span, dst, src } => {
                self.u8(8);
                self.span(span);
                self.local(*dst);
                self.local(*src);
            }
            Instr::Neg { span, kind, dst, src } => {
                self.u8(9);
                self.span(span);
                self.num_kind(kind);
                self.local(*dst);
                self.local(*src);
            }
            Instr::Concat { span, dst, lhs, rhs } => {
                self.u8(10);
                self.span(span);
                self.local(*dst);
                self.local(*lhs);
                self.local(*rhs);
            }
            Instr::ToStr { span, dst, src } => {
                self.u8(11);
                self.span(span);
                self.local(*dst);
                self.local(*src);
            }
            Instr::Range { span, dst, lo, hi, inclusive } => {
                self.u8(12);
                self.span(span);
                self.local(*dst);
                self.local(*lo);
                self.local(*hi);
                self.bool(*inclusive);
            }
            Instr::Stride { span, dst, range, step } => {
                self.u8(13);
                self.span(span);
                self.local(*dst);
                self.local(*range);
                self.local(*step);
            }
            Instr::ArrayNew { span, dst, cap, elem_size } => {
                self.u8(14);
                self.span(span);
                self.local(*dst);
                self.usize_(*cap);
                self.usize_(*elem_size);
            }
            Instr::ArrayPush { span, arr, value, elem_size } => {
                self.u8(15);
                self.span(span);
                self.local(*arr);
                self.local(*value);
                self.usize_(*elem_size);
            }
            Instr::ArrayPop { span, dst, arr } => {
                self.u8(16);
                self.span(span);
                self.local(*dst);
                self.local(*arr);
            }
            Instr::ArrayLen { span, dst, arr } => {
                self.u8(17);
                self.span(span);
                self.local(*dst);
                self.local(*arr);
            }
            Instr::ArrayGet { span, dst, arr, index, elem_size, unchecked } => {
                self.u8(18);
                self.span(span);
                self.local(*dst);
                self.local(*arr);
                self.local(*index);
                self.usize_(*elem_size);
                self.bool(*unchecked);
            }
            Instr::ArraySet { span, arr, index, value, elem_size, unchecked } => {
                self.u8(19);
                self.span(span);
                self.local(*arr);
                self.local(*index);
                self.local(*value);
                self.usize_(*elem_size);
                self.bool(*unchecked);
            }
            Instr::ObjNew { span, dst, class, instance_size } => {
                self.u8(20);
                self.span(span);
                self.local(*dst);
                self.usize_(*class);
                self.usize_(*instance_size);
            }
            Instr::StackAlloc { span, dst, class, instance_size } => {
                self.u8(21);
                self.span(span);
                self.local(*dst);
                self.usize_(*class);
                self.usize_(*instance_size);
            }
            Instr::EnumNew { span, dst, enu, variant, payload } => {
                self.u8(22);
                self.span(span);
                self.local(*dst);
                self.usize_(*enu);
                self.usize_(*variant);
                self.usize_(payload.len());
                for p in payload {
                    self.local(*p);
                }
            }
            Instr::EnumPayload { span, dst, scrut, index } => {
                self.u8(23);
                self.span(span);
                self.local(*dst);
                self.local(*scrut);
                self.usize_(*index);
            }
            Instr::EnumTag { span, dst, scrut } => {
                self.u8(24);
                self.span(span);
                self.local(*dst);
                self.local(*scrut);
            }
            Instr::Extract { span, dst, base, index, field } => {
                self.u8(25);
                self.span(span);
                self.local(*dst);
                self.local(*base);
                self.usize_(*index);
                self.str(field);
            }
            Instr::AddrOf { span, dst, src } => {
                self.u8(26);
                self.span(span);
                self.local(*dst);
                self.local(*src);
            }
            Instr::PtrLoad { span, dst, ptr, volatile } => {
                self.u8(27);
                self.span(span);
                self.local(*dst);
                self.local(*ptr);
                self.bool(*volatile);
            }
            Instr::PtrStore { span, ptr, val, volatile } => {
                self.u8(28);
                self.span(span);
                self.local(*ptr);
                self.local(*val);
                self.bool(*volatile);
            }
            Instr::RangeLo { span, dst, range } => {
                self.u8(29);
                self.span(span);
                self.local(*dst);
                self.local(*range);
            }
            Instr::RangeHi { span, dst, range } => {
                self.u8(30);
                self.span(span);
                self.local(*dst);
                self.local(*range);
            }
            Instr::RangeStep { span, dst, range } => {
                self.u8(31);
                self.span(span);
                self.local(*dst);
                self.local(*range);
            }
            Instr::GetField { span, dst, obj, field } => {
                self.u8(32);
                self.span(span);
                self.local(*dst);
                self.local(*obj);
                self.usize_(*field);
            }
            Instr::GetFieldByName { span, dst, obj, field } => {
                self.u8(33);
                self.span(span);
                self.local(*dst);
                self.local(*obj);
                self.str(field);
            }
            Instr::SetField { span, obj, field, value } => {
                self.u8(34);
                self.span(span);
                self.local(*obj);
                self.usize_(*field);
                self.local(*value);
            }
            Instr::SetFieldByName { span, obj, field, value } => {
                self.u8(35);
                self.span(span);
                self.local(*obj);
                self.str(field);
                self.local(*value);
            }
            Instr::ClosureNew { span, dst, func, captures, decay, decay_this } => {
                self.u8(36);
                self.span(span);
                self.local(*dst);
                self.usize_(*func);
                self.usize_(captures.len());
                for c in captures {
                    self.local(*c);
                }
                self.bool(*decay);
                self.bool(*decay_this);
            }
            Instr::Call { span, dsts, err, target, args } => {
                self.u8(37);
                self.span(span);
                self.usize_(dsts.len());
                for d in dsts {
                    self.local(*d);
                }
                self.opt_local(err);
                self.target(target);
                self.usize_(args.len());
                for a in args {
                    self.local(*a);
                }
            }
            Instr::GenRefOf { span, dst, obj } => {
                self.u8(38);
                self.span(span);
                self.local(*dst);
                self.local(*obj);
            }
            Instr::GenRefEmpty { span, dst } => {
                self.u8(39);
                self.span(span);
                self.local(*dst);
            }
            Instr::GenRefGet { span, dst, gref } => {
                self.u8(40);
                self.span(span);
                self.local(*dst);
                self.local(*gref);
            }
            Instr::GenRefInvalidate { span, obj } => {
                self.u8(41);
                self.span(span);
                self.local(*obj);
            }
            Instr::ThreadSpawn { span, dst, func, closure, ret_tag } => {
                self.u8(42);
                self.span(span);
                self.local(*dst);
                self.usize_(*func);
                self.opt_local(closure);
                self.u32(*ret_tag);
            }
            Instr::ThreadJoin { span, dst, handle } => {
                self.u8(43);
                self.span(span);
                self.local(*dst);
                self.local(*handle);
            }
            Instr::PoolInit { span, dst, id, workers } => {
                self.u8(44);
                self.span(span);
                self.local(*dst);
                self.local(*id);
                self.local(*workers);
            }
            Instr::PoolSubmit { span, dst, pool, func, arg, closure, ret_tag } => {
                self.u8(45);
                self.span(span);
                self.local(*dst);
                self.local(*pool);
                self.usize_(*func);
                self.opt_local(arg);
                self.opt_local(closure);
                self.u32(*ret_tag);
            }
            Instr::PoolParallelFor { span, pool, start, end, chunk, func, closure } => {
                self.u8(46);
                self.span(span);
                self.local(*pool);
                self.local(*start);
                self.local(*end);
                self.local(*chunk);
                self.usize_(*func);
                self.opt_local(closure);
            }
            Instr::PoolJoin { span, pool } => {
                self.u8(47);
                self.span(span);
                self.local(*pool);
            }
            Instr::PoolShutdown { span, pool } => {
                self.u8(48);
                self.span(span);
                self.local(*pool);
            }
            Instr::VecNew { span, dst, kind, x, y, z, w } => {
                self.u8(49);
                self.span(span);
                self.local(*dst);
                self.vec_kind(kind);
                self.local(*x);
                self.local(*y);
                self.local(*z);
                self.local(*w);
            }
            Instr::VecSplat { span, dst, kind, val } => {
                self.u8(50);
                self.span(span);
                self.local(*dst);
                self.vec_kind(kind);
                self.local(*val);
            }
            Instr::VecExtract { span, dst, vec, lane } => {
                self.u8(51);
                self.span(span);
                self.local(*dst);
                self.local(*vec);
                self.local(*lane);
            }
            Instr::VecInsert { span, dst, vec, lane, val } => {
                self.u8(52);
                self.span(span);
                self.local(*dst);
                self.local(*vec);
                self.u8(*lane);
                self.local(*val);
            }
            Instr::VecArith { span, dst, op, kind, lhs, rhs } => {
                self.u8(53);
                self.span(span);
                self.local(*dst);
                self.vec_op(op);
                self.vec_kind(kind);
                self.local(*lhs);
                self.local(*rhs);
            }
            Instr::VecUnary { span, dst, op, src } => {
                self.u8(54);
                self.span(span);
                self.local(*dst);
                self.u8(match op {
                    VecUnaryOp::Sqrt => 1,
                });
                self.local(*src);
            }
            Instr::VecDot { span, dst, lhs, rhs } => {
                self.u8(55);
                self.span(span);
                self.local(*dst);
                self.local(*lhs);
                self.local(*rhs);
            }
            Instr::ReleaseField { span, obj, field } => {
                self.u8(56);
                self.span(span);
                self.local(*obj);
                self.usize_(*field);
            }
            Instr::Retain { span, obj } => {
                self.u8(57);
                self.span(span);
                self.local(*obj);
            }
            Instr::Release { span, obj } => {
                self.u8(58);
                self.span(span);
                self.local(*obj);
            }
            Instr::ReleaseAs { span, obj, class } => {
                self.u8(59);
                self.span(span);
                self.local(*obj);
                self.usize_(*class);
            }
            Instr::Defer { span, body } => {
                self.u8(60);
                self.span(span);
                self.usize_(body.len());
                for ins in body {
                    self.instr(ins);
                }
            }
            Instr::RunDefers { span, keep } => {
                self.u8(61);
                self.span(span);
                self.usize_(*keep);
            }
            Instr::Assert { span, cond, message } => {
                self.u8(62);
                self.span(span);
                self.local(*cond);
                self.local(*message);
            }
            Instr::Panic { span, message } => {
                self.u8(63);
                self.span(span);
                self.local(*message);
            }
        }
    }
    fn term(&mut self, t: &Terminator) {
        match t {
            Terminator::Ret(v) => {
                self.u8(1);
                self.usize_(v.len());
                for l in v {
                    self.local(*l);
                }
            }
            Terminator::Br(bb) => {
                self.u8(2);
                self.usize_(*bb);
            }
            Terminator::BrIf { span, cond, then_bb, else_bb } => {
                self.u8(3);
                self.span(span);
                self.local(*cond);
                self.usize_(*then_bb);
                self.usize_(*else_bb);
            }
            Terminator::BrErr { span, err, catch_bb, catch_bind, next_bb, depth } => {
                self.u8(4);
                self.span(span);
                self.local(*err);
                self.usize_(*catch_bb);
                self.local(*catch_bind);
                self.usize_(*next_bb);
                self.usize_(*depth);
            }
            Terminator::Switch { span, scrut, cases, default } => {
                self.u8(5);
                self.span(span);
                self.local(*scrut);
                self.usize_(cases.len());
                for (pat, bb) in cases {
                    self.switch_pat(pat);
                    self.usize_(*bb);
                }
                self.usize_(*default);
            }
            Terminator::Throw { span, src, catch } => {
                self.u8(6);
                self.span(span);
                self.local(*src);
                match catch {
                    Some((bb, bind, depth)) => {
                        self.u8(1);
                        self.usize_(*bb);
                        self.local(*bind);
                        self.usize_(*depth);
                    }
                    None => self.u8(0),
                }
            }
            Terminator::Rethrow { span, catch_bb, err, depth } => {
                self.u8(7);
                self.span(span);
                self.usize_(*catch_bb);
                self.local(*err);
                self.usize_(*depth);
            }
            Terminator::Unreachable { span } => {
                self.u8(8);
                self.span(span);
            }
        }
    }
    fn block(&mut self, b: &Block) {
        self.usize_(b.instrs.len());
        for ins in &b.instrs {
            self.instr(ins);
        }
        self.term(&b.term);
    }
    fn function(&mut self, f: &Function) {
        self.str(&f.name);
        self.usize_(f.params.len());
        for t in &f.params {
            self.ty(t);
        }
        self.usize_(f.sig_params.len());
        for t in &f.sig_params {
            self.ty(t);
        }
        self.ty(&f.ret);
        self.bool(f.throws);
        self.bool(f.is_unsafe);
        self.bool(f.method_self);
        self.bool(f.is_pub);
        self.bool(f.is_closure);
        self.usize_(f.locals.len());
        for t in &f.locals {
            self.ty(t);
        }
        self.usize_(f.blocks.len());
        for b in &f.blocks {
            self.block(b);
        }
    }
    fn method_ref(&mut self, m: &MethodRef) {
        self.usize_(m.id);
        self.bool(m.private);
        self.str(&m.owner);
    }
    fn class(&mut self, c: &ClassDesc) {
        self.str(&c.name);
        self.bool(c.is_struct);
        self.usize_(c.type_params.len());
        for t in &c.type_params {
            self.str(t);
        }
        self.usize_(c.fields.len());
        for f in &c.fields {
            self.str(&f.name);
            self.ty(&f.ty);
            self.bool(f.private);
            self.str(&f.owner);
        }
        self.usize_(c.field_index.len());
        for (k, v) in &c.field_index {
            self.str(k);
            self.usize_(*v);
        }
        self.usize_(c.methods.len());
        for (k, v) in &c.methods {
            self.str(k);
            self.method_ref(v);
        }
        match c.deinit {
            Some(v) => {
                self.u8(1);
                self.usize_(v);
            }
            None => self.u8(0),
        }
        match c.dtor {
            Some(v) => {
                self.u8(1);
                self.usize_(v);
            }
            None => self.u8(0),
        }
        self.usize_(c.ifaces.len());
        for i in &c.ifaces {
            self.usize_(*i);
        }
        match c.parent {
            Some(v) => {
                self.u8(1);
                self.usize_(v);
            }
            None => self.u8(0),
        }
    }
    fn iface_method(&mut self, m: &IfaceMethod) {
        self.str(&m.name);
        self.usize_(m.params.len());
        for t in &m.params {
            self.ty(t);
        }
        self.ty(&m.ret);
    }
    fn iface(&mut self, i: &InterfaceDesc) {
        self.str(&i.name);
        self.usize_(i.type_params.len());
        for t in &i.type_params {
            self.str(t);
        }
        self.usize_(i.methods.len());
        for m in &i.methods {
            self.iface_method(m);
        }
        self.usize_(i.method_index.len());
        for (k, v) in &i.method_index {
            self.str(k);
            self.usize_(*v);
        }
    }
    fn enu(&mut self, e: &EnumDesc) {
        self.str(&e.name);
        self.usize_(e.variants.len());
        for v in &e.variants {
            self.str(&v.name);
            self.usize_(v.payload.len());
            for t in &v.payload {
                self.ty(t);
            }
        }
        self.usize_(e.variant_index.len());
        for (k, v) in &e.variant_index {
            self.str(k);
            self.usize_(*v);
        }
        match e.dtor {
            Some(v) => {
                self.u8(1);
                self.usize_(v);
            }
            None => self.u8(0),
        }
    }
    fn foreign(&mut self, f: &ForeignFn) {
        self.str(&f.local);
        self.str(&f.lib);
        self.str(&f.symbol);
        self.usize_(f.params.len());
        for t in &f.params {
            self.ty(t);
        }
        self.ty(&f.ret);
    }
    fn ns(&mut self, n: &NamespaceDesc) {
        self.str(&n.alias);
        self.str(&n.key);
        self.usize_(n.exports.len());
        for e in &n.exports {
            self.str(&e.name);
            match &e.kind {
                NsKind::Function => self.u8(1),
                NsKind::Class => self.u8(2),
                NsKind::Enum => self.u8(3),
                NsKind::Const(l) => {
                    self.u8(4);
                    self.lit(l);
                }
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0xf) as u32, 16).unwrap_or('0'));
    }
    out
}

// Every field of Function is hashed, including spans: trap paths embed
// span.start/end as fatal_span constants, so spans affect object bytes.
// Index operands (callee ids, class ids, closure targets) are hashed raw;
// their meaning (index assignment, schemas) is covered by module_salt,
// which is always part of the cache key alongside this hash.
pub fn fn_hash(f: &Function) -> String {
    let mut h = H(Sha256::new());
    h.u8(1);
    h.function(f);
    hex(&h.0.finalize())
}

// Salt over everything outside a single Function that can leak into a
// unit's object bytes: declaration order and signatures (every unit
// declares every function), global-index closure tags, closure sets and
// arities for indirect-call dispatch, class/enum/interface schemas and
// their order (sizes, offsets, dtors, subclass walks), foreign decls,
// namespace pretty tables, and the index maps. Function bodies and locals
// are excluded: bodies are hashed per member, and no unit reads another
// function's locals.
pub fn module_salt(m: &Module) -> String {
    let mut h = H(Sha256::new());
    h.u8(2);
    h.usize_(m.functions.len());
    for f in &m.functions {
        h.str(&f.name);
        h.usize_(f.params.len());
        for t in &f.params {
            h.ty(t);
        }
        h.usize_(f.sig_params.len());
        for t in &f.sig_params {
            h.ty(t);
        }
        h.ty(&f.ret);
        h.bool(f.is_closure);
    }
    h.usize_(m.classes.len());
    for c in &m.classes {
        h.class(c);
    }
    h.usize_(m.interfaces.len());
    for i in &m.interfaces {
        h.iface(i);
    }
    h.usize_(m.enums.len());
    for e in &m.enums {
        h.enu(e);
    }
    h.usize_(m.foreign.len());
    for f in &m.foreign {
        h.foreign(f);
    }
    h.usize_(m.namespaces.len());
    for (k, v) in &m.namespaces {
        h.str(k);
        h.ns(v);
    }
    h.usize_(m.array_dtors.len());
    for (k, v) in &m.array_dtors {
        h.str(k);
        h.usize_(*v);
    }
    h.usize_(m.class_index.len());
    for (k, v) in &m.class_index {
        h.str(k);
        h.usize_(*v);
    }
    h.usize_(m.interface_index.len());
    for (k, v) in &m.interface_index {
        h.str(k);
        h.usize_(*v);
    }
    h.usize_(m.enum_index.len());
    for (k, v) in &m.enum_index {
        h.str(k);
        h.usize_(*v);
    }
    h.usize_(m.fn_index.len());
    for (k, v) in &m.fn_index {
        h.str(k);
        h.usize_(*v);
    }
    h.usize_(m.foreign_index.len());
    for (k, v) in &m.foreign_index {
        h.str(k);
        h.usize_(*v);
    }
    hex(&h.0.finalize())
}

#[cfg(test)]
mod tests {
    use super::super::instr::{Block, LirType, Module, Terminator};
    use super::*;

    fn one_fn() -> Function {
        Function {
            name: "Main".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: Vec::new(),
            blocks: vec![Block { instrs: Vec::new(), term: Terminator::Ret(Vec::new()) }],
        }
    }

    #[test]
    fn hash_is_stable_and_hex() {
        let f = one_fn();
        let a = fn_hash(&f);
        assert_eq!(a.len(), 64);
        assert_eq!(a, fn_hash(&f));
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_covers_name_sig_locals_and_flags() {
        let base = fn_hash(&one_fn());
        let mut f = one_fn();
        f.name = "Other".to_string();
        assert_ne!(base, fn_hash(&f));
        let mut f = one_fn();
        f.params.push(LirType::I64);
        assert_ne!(base, fn_hash(&f));
        let mut f = one_fn();
        f.ret = LirType::Bool;
        assert_ne!(base, fn_hash(&f));
        let mut f = one_fn();
        f.locals.push(LirType::Str);
        assert_ne!(base, fn_hash(&f));
        let mut f = one_fn();
        f.is_closure = true;
        assert_ne!(base, fn_hash(&f));
        let mut f = one_fn();
        f.throws = true;
        assert_ne!(base, fn_hash(&f));
    }

    #[test]
    fn hash_covers_spans() {
        let base = fn_hash(&one_fn());
        let mut f = one_fn();
        f.blocks[0].instrs.push(Instr::Panic {
            span: diagnostics::Span { start: 4, end: 9 },
            message: 0,
        });
        let moved = fn_hash(&f);
        assert_ne!(base, moved);
        f.blocks[0].instrs[0] = Instr::Panic {
            span: diagnostics::Span { start: 40, end: 90 },
            message: 0,
        };
        assert_ne!(moved, fn_hash(&f));
    }

    #[test]
    fn salt_covers_order_and_schema() {
        let mut m = Module::default();
        m.functions.push(one_fn());
        let mut other = one_fn();
        other.name = "Help".to_string();
        m.functions.push(other);
        let base = module_salt(&m);
        assert_eq!(base.len(), 64);
        m.functions.swap(0, 1);
        assert_ne!(base, module_salt(&m));
        m.functions.swap(0, 1);
        assert_eq!(base, module_salt(&m));
        m.functions[0].is_closure = true;
        assert_ne!(base, module_salt(&m));
    }
}
