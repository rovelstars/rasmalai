use super::*;
use diagnostics::{Code, Diagnostic, Span};
use frontend::ast as A;
use std::collections::{BTreeMap, BTreeSet};
impl<'a> Builder<'a> {
    pub(super) fn narrow_for_is(
        &mut self,
        base: &A::Spanned<A::Expr>,
        target: &A::Type,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let A::Expr::Ident(name) = &base.node else {
            return Ok(());
        };
        let Some((l, _, cur)) = self.lookup(name) else {
            return Ok(());
        };
        let tt = resolve_ty(self.module, target);
        match (&cur, &tt) {
            (LirType::Any, t) if *t != LirType::Any => {
                let nl = self.coerce_to_slot(l, t, span);
                self.def(name.clone(), nl, simple_of(t), t.clone());
                if let LirType::Obj(n) = t {
                    if self.module.interface_index.contains_key(n) {
                        self.emit(Instr::Retain { span, obj: nl });
                    }
                }
            }
            (LirType::Error, LirType::Obj(_)) => {
                let nl = self.local(tt.clone());
                self.emit(Instr::Call {
                    span,
                    dsts: vec![nl],
                    err: None,
                    target: CallTarget::Builtin("__rnx_error_unbox".to_string()),
                    args: vec![l],
                });
                self.emit(Instr::Retain { span, obj: nl });
                self.def(name.clone(), nl, simple_of(&tt), tt.clone());
            }
            (LirType::Obj(_), LirType::Obj(_)) => {
                self.def(name.clone(), l, simple_of(&tt), tt.clone());
                if let LirType::Obj(n) = &tt {
                    if self.module.interface_index.contains_key(n) {
                        self.emit(Instr::Retain { span, obj: l });
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn lower_expr(&mut self, e: &A::Expr, span: Span) -> Result<(Local, Simple), Diagnostic> {
        match e {
            A::Expr::Ident(n)  => self.lower_expr_ident(n, span),
            A::Expr::This  => self.lower_expr_this(span),
            A::Expr::Super  => self.lower_expr_super(span),
            A::Expr::Bool(v)  => self.lower_expr_bool(v, span),
            A::Expr::Null  => self.lower_expr_null(span),
            A::Expr::Int(v)  => self.lower_expr_int(v, span),
            A::Expr::Float(v)  => self.lower_expr_float(v, span),
            A::Expr::Interp(parts)  => self.lower_expr_interp(parts, span),
            A::Expr::Array(items)  => self.lower_expr_array(items, span),
            A::Expr::UnsafeBlock(b)  => self.lower_expr_unsafe(b, span),
            A::Expr::Record(fields)  => self.lower_expr_record(fields),
            A::Expr::MapLiteral(entries)  => self.lower_map_literal(entries, span),
            A::Expr::Await(_)  => self.lower_expr_await(span),
            A::Expr::Propagate(inner)  => self.lower_propagate(inner, span),
            A::Expr::Coalesce { lhs, rhs }  => self.lower_coalesce(lhs, rhs, span),
            A::Expr::OptChain { base, field }  => self.lower_opt_access(base, field, &[], span),
            A::Expr::OptCall { base, field, args }  => self.lower_opt_access(base, field, args, span),
            A::Expr::Switch { scrutinee, cases, default }  => self.lower_expr_switch(scrutinee, cases, default, span),
            A::Expr::Tuple(items)  => self.lower_expr_tuple(items, span),
            A::Expr::TupleGet { base, index }  => self.lower_expr_tuple_get(base, index, span),
            A::Expr::Binary { op, lhs, rhs }  => self.lower_binary(*op, lhs, rhs, span),
            A::Expr::Unary { op, rhs } if matches!(op, A::UnOp::PreInc | A::UnOp::PreDec)  => self.lower_expr_preinc(op, rhs, span),
            A::Expr::Postfix { op, expr }  => self.lower_expr_postfix(op, expr, span),
            A::Expr::Unary { op, rhs }  => self.lower_expr_unary(op, rhs, span),
            A::Expr::Ternary { cond, then, otherwise }  => self.lower_expr_ternary(cond, then, otherwise, span),
            A::Expr::Range { lo, hi, inclusive }  => self.lower_expr_range(lo, hi, inclusive, span),
            A::Expr::Call {
                callee,
                type_args,
                args,
                trailing,
            }  => self.lower_expr_call(callee, type_args, args, trailing, span),
            A::Expr::New { target, args, .. }  => self.lower_new(target, args, span),
            A::Expr::Index { base, index }  => self.lower_expr_index(base, index, span),
            A::Expr::Member { base, field }  => self.lower_expr_member(base, field, span),
            A::Expr::ImplicitMember(path) => {
                let vname = path.last().cloned().unwrap_or_default();
                self.fail(self.err(
                    Code::E108,
                    format!("implicit member `.{vname}` needs an enum type context"),
                    span,
                ))
            }
            A::Expr::Cast { expr, ty }  => self.lower_expr_cast(expr, ty, span),
            A::Expr::Is { base, target }  => self.lower_expr_is(base, target, span),
            A::Expr::Macro { name, args, .. }  => self.lower_macro(name, args, span),
            A::Expr::Closure {
                decay,
                is_async,
                params,
                ret,
                throws,
                body,
            }  => self.lower_expr_closure(decay, is_async, params, ret, throws, body, span),
        }
    }

    pub(super) fn lower_expr_ident(&mut self, n: &String, span: Span) -> Result<(Local, Simple), Diagnostic> {
        match self.lookup(n) {
        Some((l, s, _)) => Ok((l, s)),
        None => match self.mod_const_emit(n, span) {
            Some(v) => Ok(v),
            None => match self.variant_of(n) {
            Some((enu, vi)) => {
                let arity = self.module.enums[enu].variants[vi].payload.len();
                if arity != 0 {
                    return self.fail(self.err(
                        Code::E108,
                        format!("variant `{n}` takes {arity} payload args, use `{n}(...)`"),
                        span,
                    ));
                }
                let dst = self.local(LirType::Enum(enu));
                self.emit(Instr::EnumNew { span, 
                    dst,
                    enu,
                    variant: vi,
                    payload: Vec::new(),
                });
                Ok((dst, Simple::Other))
            }
            None => Err(self.unknown_name(n, span)),
            },
        },
    }
    }

    pub(super) fn lower_expr_this(&mut self, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let slf = self.self_local;
        let alias = self.this_alias;
        slf.or(alias).map(|l| (l, Simple::Other)).ok_or_else(|| {
            self.err(Code::E108, "`this` outside method", span)
        })

    }

    pub(super) fn lower_expr_super(&mut self, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let slf = self.self_local;
        let alias = self.this_alias;
        slf.or(alias).map(|l| (l, Simple::Other)).ok_or_else(|| {
            self.err(Code::E108, "`super` used outside of a class method", span)
        })

    }

    pub(super) fn lower_expr_bool(&mut self, v: &bool, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let dst = self.local(LirType::Bool);
        self.emit(Instr::Const { span, 
            dst,
            lit: Lit::Bool(*v),
        });
        Ok((dst, Simple::Other))

    }

    pub(super) fn lower_expr_null(&mut self, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let dst = self.local(LirType::Null);
        self.emit(Instr::Const { span,  dst, lit: Lit::Null });
        Ok((dst, Simple::Other))

    }

    pub(super) fn lower_expr_int(&mut self, v: &i64, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let dst = self.local(LirType::I64);
        self.emit(Instr::Const { span, 
            dst,
            lit: Lit::Int(*v),
        });
        Ok((dst, Simple::Int))

    }

    pub(super) fn lower_expr_float(&mut self, v: &f64, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let dst = self.local(LirType::F64(FloatKind::Strict));
        self.emit(Instr::Const { span, 
            dst,
            lit: Lit::Float(*v, FloatKind::Strict),
        });
        Ok((dst, Simple::Strict))

    }

    pub(super) fn lower_expr_interp(&mut self, parts: &Vec<A::InterpPart>, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let mut acc: Option<Local> = None;
        for p in parts {
            let s = match p {
                A::InterpPart::Text(t) => {
                    let dst = self.local(LirType::Str);
                    self.emit(Instr::Const { span, 
                        dst,
                        lit: Lit::Str(t.clone()),
                    });
                    dst
                }
                A::InterpPart::Expr(ex) => {
                    let (v, _) = self.lower_expr(&ex.node, ex.span)?;
                    self.deny_tuple(v, "in string interpolation", ex.span)?;
                    let dst = self.local(LirType::Str);
                    self.emit_to_str(dst, v, span);
                    dst
                }
            };
            acc = Some(match acc {
                None => s,
                Some(a) => {
                    let dst = self.local(LirType::Str);
                    self.emit(Instr::Concat { span,  dst, lhs: a, rhs: s });
                    dst
                }
            });
        }
        let dst = self.local(LirType::Str);
        match acc {
            Some(a) => self.emit_copy(dst, a, span),
            None => self.emit(Instr::Const { span, 
                dst,
                lit: Lit::Str(String::new()),
            }),
        }
        Ok((dst, Simple::Other))

    }

    pub(super) fn lower_expr_array(&mut self, items: &Vec<A::ArrayElem>, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let mut elem_ty = LirType::Any;
        let mut lowered: Vec<(Option<Local>, Local, Span)> = Vec::with_capacity(items.len());
        for i in items {
            let (v, _) = self.lower_expr(&i.expr.node, i.expr.span)?;
            self.deny_tuple(v, "in an array literal", i.expr.span)?;
            if i.spread {
                match self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any) {
                    LirType::Array(inner) => {
                        if elem_ty == LirType::Any && *inner != LirType::Any {
                            elem_ty = (*inner).clone();
                        }
                        lowered.push((None, v, i.expr.span));
                    }
                    _ => {
                        return self.fail(self.err(
                            Code::E108,
                            "spread `...` needs an Array value",
                            i.expr.span,
                        ));
                    }
                }
            } else {
                let vt = self.func.locals[v as usize].clone();
                if elem_ty == LirType::Any && vt != LirType::Any {
                    elem_ty = vt;
                }
                lowered.push((Some(v), v, i.expr.span));
            }
        }
        let dst = self.local(LirType::Array(Box::new(elem_ty.clone())));
        self.fresh_arrays.insert(dst);
        let es = elem_size(&elem_ty);
        let cap = lowered.iter().filter(|(s, _, _)| s.is_some()).count();
        self.emit(Instr::ArrayNew { span, dst, cap, elem_size: es });
        for (slot, v, s) in lowered {
            match slot {
                Some(_) => {
                    let v = self.coerce_to_slot(v, &elem_ty, s);
                    self.emit(Instr::ArrayPush { span, arr: dst, value: v, elem_size: es });
                }
                None => {
                    let zero = self.const_int(0, s);
                    self.emit_array_extend(dst, v, &elem_ty, zero, s)?;
                }
            }
        }
        Ok((dst, Simple::Other))

    }

    pub(super) fn lower_expr_unsafe(&mut self, b: &A::Block, span: Span) -> Result<(Local, Simple), Diagnostic> {

        self.unsafe_depth += 1;
        let r = self.lower_block(b);
        self.unsafe_depth -= 1;
        r?;
        let dst = self.local(LirType::Null);
        self.emit(Instr::Const { span,  dst, lit: Lit::Null });
        Ok((dst, Simple::Other))

    }

    pub(super) fn lower_expr_record(&mut self, fields: &Vec<A::RecordEntry>) -> Result<(Local, Simple), Diagnostic> {

        let mut seen = BTreeSet::new();
        let mut tys = Vec::with_capacity(fields.len());
        let mut named: Vec<(String, Local)> = Vec::with_capacity(fields.len());
        for e in fields {
            let (name, v) = match e {
                A::RecordEntry::Field(n, v) => (n, v),
                A::RecordEntry::Spread(v) => {
                    return self.fail(self.err(
                        Code::E108,
                        "`...spread` in record literals is only supported in Project.config",
                        v.span,
                    ));
                }
            };
            if !seen.insert(name.clone()) {
                return self.fail(self.err(
                    Code::E108,
                    format!("duplicate field `{name}` in record literal"),
                    v.span,
                ));
            }
            let (e, _) = self.lower_expr(&v.node, v.span)?;
            if self.tuples.contains_key(&e) {
                return self.fail(self.err(
                    Code::E108,
                    "tuple value cannot be used in a record literal",
                    v.span,
                ));
            }
            tys.push(self.func.locals[e as usize].clone());
            named.push((name.clone(), e));
        }
        let marker = self.local(LirType::Tuple(tys));
        self.records.insert(marker, named);
        Ok((marker, Simple::Other))

    }

    pub(super) fn lower_expr_await(&mut self, span: Span) -> Result<(Local, Simple), Diagnostic> {

        self.fail(self.err(Code::E108, "stray `await` after async expansion", span))

    }

    pub(super) fn lower_expr_switch(&mut self, scrutinee: &Box<A::Spanned<A::Expr>>, cases: &Vec<A::SwitchExprCase>, default: &Option<A::SwitchExprBody>, span: Span) -> Result<(Local, Simple), Diagnostic> {

        self.lower_switch_expr(&scrutinee.node, scrutinee.span, cases, default, span)

    }

    pub(super) fn lower_expr_tuple(&mut self, items: &Vec<A::Spanned<A::Expr>>, span: Span) -> Result<(Local, Simple), Diagnostic> {

        if items.len() < 2 {
            return self.fail(self.err(
                Code::E108,
                "tuple needs at least 2 elements",
                span,
            ));
        }
        let mut elems = Vec::with_capacity(items.len());
        let mut tys = Vec::with_capacity(items.len());
        for it in items {
            let (v, _) = self.lower_expr(&it.node, it.span)?;
            self.deny_tuple(v, "tuple element", it.span)?;
            tys.push(self.func.locals[v as usize].clone());
            elems.push(v);
        }
        let marker = self.local(LirType::Tuple(tys));
        self.tuples.insert(marker, elems);
        Ok((marker, Simple::Other))

    }

    pub(super) fn lower_expr_tuple_get(&mut self, base: &Box<A::Spanned<A::Expr>>, index: &usize, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let (b, _) = self.lower_expr(&base.node, base.span)?;
        let elems = self.tuples.get(&b).cloned().ok_or_else(|| {
            self.err(Code::E108, "tuple index on non-tuple value", base.span)
        })?;
        let arity = elems.len();
        let elem_ty = match self.func.locals.get(b as usize).cloned() {
            Some(LirType::Tuple(tys)) => tys.get(*index).cloned().unwrap_or(LirType::Any),
            _ => LirType::Any,
        };
        let src = *elems.get(*index).ok_or_else(|| {
            self.err(
                Code::E108,
                format!("tuple index `{index}` out of bounds for {arity}-tuple"),
                base.span,
            )
        })?;
        let dst = self.local(elem_ty);
        self.emit_copy(dst, src, span);
        Ok((dst, Simple::Other))

    }

    pub(super) fn lower_expr_preinc(&mut self, op: &A::UnOp, rhs: &Box<A::Spanned<A::Expr>>, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let delta = if matches!(op, A::UnOp::PreInc) { 1 } else { -1 };
        self.lower_incdec(&rhs.node, rhs.span, delta, false, span)

    }

    pub(super) fn lower_expr_postfix(&mut self, op: &A::PostfixOp, expr: &Box<A::Spanned<A::Expr>>, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let delta = if matches!(op, A::PostfixOp::PostInc) { 1 } else { -1 };
        self.lower_incdec(&expr.node, expr.span, delta, true, span)

    }

    pub(super) fn lower_expr_unary(&mut self, op: &A::UnOp, rhs: &Box<A::Spanned<A::Expr>>, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let (v, s) = self.lower_expr(&rhs.node, rhs.span)?;
        self.deny_tuple(v, "in a unary operation", rhs.span)?;
        if op == &A::UnOp::Neg {
            let vty = self.func.locals[v as usize].clone();
            if matches!(vty, LirType::Obj(_)) {
                return self.lower_op_hook(v, "op_neg", "-", Vec::new(), Vec::new(), span);
            }
        }
        let dst = self.local(self.func.locals[v as usize].clone());
        match op {
            A::UnOp::Neg => {
                let kind = match s {
                    Simple::Int => NumKind::Int,
                    Simple::Strict => NumKind::Float(FloatKind::Strict),
                    Simple::Fast => NumKind::Float(FloatKind::Fast),
                    Simple::Other => NumKind::Int,
                };
                self.emit(Instr::Neg { span,  kind, dst, src: v });
            }
            A::UnOp::Not => self.emit(Instr::Not { span,  dst, src: v }),
            A::UnOp::BitNot => {
                let vty = self.func.locals[v as usize].clone();
                if vty != LirType::I64 {
                    return self.fail(self.err(
                        Code::E108,
                        "bitwise `~` needs an `Int` operand",
                        span,
                    ));
                }
                let one = self.const_int(-1, span);
                self.emit(Instr::Arith { span,
                    op: ArithOp::BitXor,
                    kind: NumKind::Int,
                    dst,
                    lhs: v,
                    rhs: one,
                });
            }
            A::UnOp::AddrOf => {
                if self.unsafe_depth == 0 {
                    return self.fail(self.err(
                        Code::E202,
                        "address-of needs `unsafe`",
                        span,
                    ));
                }
                if !matches!(&rhs.node, A::Expr::Ident(_)) {
                    return self.fail(self.err(
                        Code::E108,
                        "address-of needs a local",
                        rhs.span,
                    ));
                }
                let dst = self.local(LirType::Pointer(Box::new(LirType::Any)));
                self.emit(Instr::AddrOf { span,  dst, src: v });
                return Ok((dst, Simple::Other));
            }
            A::UnOp::Deref => {
                if self.unsafe_depth == 0 {
                    return self.fail(self.err(
                        Code::E202,
                        "pointer dereference requires unsafe block",
                        span,
                    ));
                }
                let pty = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                let scalar = self.pointer_scalar(&pty, span)?;
                let dst = self.local(scalar.clone());
                self.emit(Instr::PtrLoad { span, dst, ptr: v, volatile: false });
                return Ok((dst, simple_of(&scalar)));
            }
            A::UnOp::PreInc | A::UnOp::PreDec => {
                return self.fail(self.err(Code::E108, "unreachable prefix operator", span));
            }
        }
        Ok((dst, s))

    }

    pub(super) fn lower_expr_ternary(&mut self, cond: &Box<A::Spanned<A::Expr>>, then: &Box<A::Spanned<A::Expr>>, otherwise: &Box<A::Spanned<A::Expr>>, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let (c, _) = self.lower_expr(&cond.node, cond.span)?;
        self.deny_tuple(c, "as a ternary condition", cond.span)?;
        let tbb = self.new_block();
        let ebb = self.new_block();
        let merge = self.new_block();
        self.set_term(Terminator::BrIf { span, 
            cond: c,
            then_bb: tbb,
            else_bb: ebb,
        });
        self.set_current(tbb);
        let (t, _) = self.lower_expr(&then.node, then.span)?;
        self.deny_tuple(t, "as a ternary branch", then.span)?;
        let out_ty = self.func.locals[t as usize].clone();
        let dst = self.local(out_ty);
        self.emit_copy(dst, t, span);
        if !self.terminated {
            self.set_term(Terminator::Br(merge));
        }
        self.set_current(ebb);
        let (el, _) = self.lower_expr(&otherwise.node, otherwise.span)?;
        self.deny_tuple(el, "as a ternary branch", otherwise.span)?;
        self.emit_copy(dst, el, span);
        if !self.terminated {
            self.set_term(Terminator::Br(merge));
        }
        self.set_current(merge);
        Ok((dst, Simple::Other))

    }

    pub(super) fn lower_expr_range(&mut self, lo: &Box<A::Spanned<A::Expr>>, hi: &Box<A::Spanned<A::Expr>>, inclusive: &bool, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let (a, _) = self.lower_expr(&lo.node, lo.span)?;
        self.deny_tuple(a, "as a range bound", lo.span)?;
        let (b, _) = self.lower_expr(&hi.node, hi.span)?;
        self.deny_tuple(b, "as a range bound", hi.span)?;
        let ta = self.func.locals[a as usize].clone();
        let tb = self.func.locals[b as usize].clone();
        let ai = match ta {
            LirType::I64 => a,
            LirType::Any => self.unbox_to_int(a, lo.span),
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    "range bounds must be ints",
                    lo.span,
                ));
            }
        };
        let bi = match tb {
            LirType::I64 => b,
            LirType::Any => self.unbox_to_int(b, hi.span),
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    "range bounds must be ints",
                    hi.span,
                ));
            }
        };
        let incl = self.local(LirType::Bool);
        self.emit(Instr::Const { span, dst: incl, lit: Lit::Bool(*inclusive) });
        let step = self.const_int(1, span);
        let marker = self.local(LirType::Range);
        self.ranges.insert(marker, (ai, bi, incl, step));
        Ok((marker, Simple::Other))

    }

    pub(super) fn lower_expr_call(&mut self, callee: &Box<A::Spanned<A::Expr>>, type_args: &Vec<A::Type>, args: &Vec<A::CallArg>, trailing: &Option<A::Block>, span: Span) -> Result<(Local, Simple), Diagnostic> {

        if trailing.is_some() {
            return self.fail(self.err(Code::E108, "UI blocks pending", span));
        }
        self.lower_call(callee, type_args, args, span)

    }

    pub(super) fn lower_expr_index(&mut self, base: &Box<A::Spanned<A::Expr>>, index: &Box<A::Spanned<A::Expr>>, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let (arr, _) = self.lower_expr(&base.node, base.span)?;
        self.deny_tuple(arr, "as an index base", base.span)?;
        let (ix, _) = self.lower_expr(&index.node, index.span)?;
        if self.ranges.get(&ix).is_none() {
            self.deny_tuple(ix, "as an index", index.span)?;
        }
        let base_ty = self.func.locals.get(arr as usize).cloned().unwrap_or(LirType::Any);
        if matches!(base_ty, LirType::Obj(_)) && self.ranges.get(&ix).is_none() {
            return self.lower_op_hook(
                arr,
                "op_index",
                "[]",
                vec![(**index).clone()],
                vec![ix],
                span,
            );
        }
        if let Some((rlo, rhi, rincl, _)) = self.ranges.get(&ix).cloned() {
            let base_ty = self.func.locals.get(arr as usize).cloned().unwrap_or(LirType::Any);
            match base_ty {
                LirType::Array(inner) => {
                    let elem = (*inner).clone();
                    let es = elem_size(&elem);
                    let retain = matches!(elem, LirType::Str | LirType::Obj(_) | LirType::Array(_) | LirType::Enum(_) | LirType::Any) as i64;
                    let dst = self.local(LirType::Array(Box::new(elem)));
                    let incl_flag = self.local(LirType::I64);
                    let t_bb = self.new_block();
                    let f_bb = self.new_block();
                    let m_bb = self.new_block();
                    self.set_term(Terminator::BrIf { span, cond: rincl, then_bb: t_bb, else_bb: f_bb });
                    self.set_current(t_bb);
                    self.emit(Instr::Const { span, dst: incl_flag, lit: Lit::Int(1) });
                    self.set_term(Terminator::Br(m_bb));
                    self.set_current(f_bb);
                    self.emit(Instr::Const { span, dst: incl_flag, lit: Lit::Int(0) });
                    self.set_term(Terminator::Br(m_bb));
                    self.set_current(m_bb);
                    let es_arg = self.const_int(es as i64, span);
                    let retain_arg = self.const_int(retain, span);
                    self.emit(Instr::Call { span, dsts: vec![dst], err: None, target: CallTarget::Builtin("__rnx_array_slice".to_string()), args: vec![arr, rlo, rhi, incl_flag, es_arg, retain_arg] });
                    return Ok((dst, Simple::Other));
                }
                LirType::Str => {
                    let end = self.local(LirType::I64);
                    let one = self.const_int(1, span);
                    let end_plus = self.local(LirType::I64);
                    self.emit(Instr::Arith { span, op: ArithOp::Add, kind: NumKind::Int, dst: end_plus, lhs: rhi, rhs: one });
                    let t_bb = self.new_block();
                    let f_bb = self.new_block();
                    let m_bb = self.new_block();
                    self.set_term(Terminator::BrIf { span, cond: rincl, then_bb: t_bb, else_bb: f_bb });
                    self.set_current(t_bb);
                    self.emit_copy(end, end_plus, span);
                    self.set_term(Terminator::Br(m_bb));
                    self.set_current(f_bb);
                    self.emit_copy(end, rhi, span);
                    self.set_term(Terminator::Br(m_bb));
                    self.set_current(m_bb);
                    let dst = self.local(LirType::Str);
                    self.emit(Instr::Call { span, dsts: vec![dst], err: None, target: CallTarget::Builtin("__rnx_string_slice".to_string()), args: vec![arr, rlo, end] });
                    return Ok((dst, Simple::Other));
                }
                _ => {
                    return self.fail(self.err(
                        Code::E108,
                        "range index needs an Array or String target",
                        base.span,
                    ));
                }
            }
        }
        let es = self.elem_size_of(arr);
        let out_ty = match self.func.locals.get(arr as usize) {
            Some(LirType::Array(inner)) => (**inner).clone(),
            _ => LirType::Any,
        };
        let dst = self.local(out_ty);
        self.emit(Instr::ArrayGet { span,  dst, arr, index: ix, elem_size: es, unchecked: false });
        Ok((dst, Simple::Other))

    }

    pub(super) fn lower_expr_member(&mut self, base: &Box<A::Spanned<A::Expr>>, field: &String, span: Span) -> Result<(Local, Simple), Diagnostic> {

        if let A::Expr::Ident(root) = &base.node {
            if let Some(enu) = self.module.enum_index.get(root).copied() {
                let vi = *self.module.enums[enu].variant_index.get(field).ok_or_else(|| {
                    self.err(Code::E108, format!("unknown variant `{root}.{field}`"), span)
                })?;
                let arity = self.module.enums[enu].variants[vi].payload.len();
                if arity != 0 {
                    return self.fail(self.err(
                        Code::E108,
                        format!("variant `{root}.{field}` takes {arity} payload args"),
                        span,
                    ));
                }
                let dst = self.local(LirType::Enum(enu));
                self.emit(Instr::EnumNew { span, 
                    dst,
                    enu,
                    variant: vi,
                    payload: Vec::new(),
                });
                return Ok((dst, Simple::Other));
            }
        }
        let (obj, _) = self.lower_expr(&base.node, base.span)?;
        if let Some(rec) = self.records.get(&obj).cloned() {
            let src = rec
                .iter()
                .find(|(n, _)| n == field)
                .map(|(_, l)| *l)
                .ok_or_else(|| {
                    self.err(Code::E108, format!("unknown field `{field}`"), span)
                })?;
            let ty = self.func.locals[src as usize].clone();
            let dst = self.local(ty);
            self.emit_copy(dst, src, span);
            return Ok((dst, Simple::Other));
        }
        self.deny_tuple(obj, "in member access", base.span)?;
        let ty = self.func.locals[obj as usize].clone();
        if let Some(nskey) = super::ns_key_of_ty(&ty) {
            let mangled = format!("{nskey}.{field}");
            if let Some((lit, lt, s)) = self.mod_consts.get(&mangled).cloned() {
                let dst = self.local(lt);
                self.emit(Instr::Const { span, dst, lit });
                return Ok((dst, s));
            }
            return self.fail(self.err(
                Code::E108,
                format!("cannot read `{field}` from a module namespace as a value"),
                span,
            ));
        }
        if let LirType::Obj(name) = &ty {
            if let Some(ci) = self.module.class_index.get(name).copied() {
                if self.module.classes[ci].field_index.contains_key(field) {
                    let fi = self.check_field(ci, field, span)?;
                    let dst = self.local(self.module.classes[ci].fields[fi].ty.clone());
                    self.emit(Instr::GetField { span,
                        dst,
                        obj,
                        field: fi,
                    });
                    let key = format!("{name}.{field}");
                    if let Some(pt) = self.field_opt.get(&key).cloned() {
                        self.precise.insert(dst, pt);
                    }
                    return Ok((dst, Simple::Other));
                }
            }
        }
        if matches!(ty, LirType::Array(_)) && (field == "length" || field == "len") {
            let dst = self.local(LirType::I64);
            self.emit(Instr::ArrayLen { span,  dst, arr: obj });
            return Ok((dst, Simple::Other));
        }
        if matches!(ty, LirType::Str) && (field == "length" || field == "len") {
            let dst = self.local(LirType::I64);
            self.emit(Instr::Call { span,  dsts: vec![dst], err: None, target: CallTarget::Builtin("__rnx_string_len".to_string()), args: vec![obj] });
            return Ok((dst, Simple::Int));
        }
        let dst = self.local(LirType::Any);
        self.emit(Instr::GetFieldByName { span, 
            dst,
            obj,
            field: field.clone(),
        });
        Ok((dst, Simple::Other))

    }

    pub(super) fn lower_expr_cast(&mut self, expr: &Box<A::Spanned<A::Expr>>, ty: &A::Type, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let (v, _) = self.lower_expr(&expr.node, expr.span)?;
        self.deny_tuple(v, "in a cast", expr.span)?;
        let target = resolve_ty(self.module, ty);
        let owned = |t: &LirType| {
            matches!(
                t,
                LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_)
            )
        };
        let st = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
        if owned(&st) && !(owned(&target) || matches!(target, LirType::Any)) {
            return self.fail(self.err(
                Code::E108,
                "`as` cannot cast an object to a plain value type",
                span,
            ));
        }
        let fresh = matches!(expr.node, A::Expr::Array(_));
        self.check_array_view(&st, &target, fresh, expr.span, "cast value")?;
        let dst = self.local(target.clone());
        self.emit_cast(dst, v, span);
        Ok((dst, simple_of(&target)))

    }

    pub(super) fn lower_expr_is(&mut self, base: &Box<A::Spanned<A::Expr>>, target: &A::Type, span: Span) -> Result<(Local, Simple), Diagnostic> {

        let (v, _) = self.lower_expr(&base.node, base.span)?;
        if self.ranges.contains_key(&v) {
            let tt = resolve_ty(self.module, target);
            let dst = self.local(LirType::Bool);
            self.emit_const_bool(dst, tt == LirType::Range, span);
            return Ok((dst, Simple::Other));
        }
        self.deny_tuple(v, "in `is` check", base.span)?;
        let vt = self.func.locals[v as usize].clone();
        let tt = resolve_ty(self.module, target);
        let dst = self.local(LirType::Bool);
        match crate::instr::resolve_is(self.module, &vt, &tt) {
            Ok(crate::instr::IsDecision::Const(b)) => self.emit_const_bool(dst, b, span),
            Ok(crate::instr::IsDecision::Tag(pt)) => self.emit_tag_check(dst, v, pt, span),
            Ok(crate::instr::IsDecision::Class(ci)) => self.emit_class_check(dst, v, ci, span),
            Ok(crate::instr::IsDecision::Iface(ii)) => self.emit_iface_check(dst, v, ii, span),
            Err(msg) => {
                return self.fail(self.err(Code::E108, msg, base.span));
            }
        }
        Ok((dst, Simple::Other))

    }

    pub(super) fn lower_expr_closure(&mut self, decay: &bool, is_async: &bool, params: &Vec<A::Param>, ret: &Option<A::Type>, throws: &bool, body: &A::FnBody, span: Span) -> Result<(Local, Simple), Diagnostic> {

        if *is_async {
            return self.fail(self.err(
                Code::E108,
                "async closure reached lowering unexpanded",
                span,
            ));
        }
        self.lower_closure(*decay, params, ret.clone(), *throws, body, span)

    }


    pub(super) fn str_const(&mut self, s: &str, span: Span) -> Local {
        let dst = self.local(LirType::Str);
        self.emit(Instr::Const { span, dst, lit: Lit::Str(s.to_string()) });
        dst
    }

    pub(super) fn str_concat(&mut self, lhs: Local, rhs: Local, span: Span) -> Local {
        let dst = self.local(LirType::Str);
        self.emit(Instr::Concat { span, dst, lhs, rhs });
        dst
    }

    pub(super) fn record_to_str(&mut self, marker: Local, span: Span) -> Local {
        let fields = self.records.get(&marker).cloned().unwrap_or_default();
        let mut out = self.str_const("{ ", span);
        for (i, (name, fl)) in fields.iter().enumerate() {
            if i > 0 {
                let sep = self.str_const(", ", span);
                out = self.str_concat(out, sep, span);
            }
            let label = self.str_const(&format!("{name}: "), span);
            out = self.str_concat(out, label, span);
            let vs = self.record_field_str(*fl, span);
            out = self.str_concat(out, vs, span);
        }
        let close = self.str_const(" }", span);
        out = self.str_concat(out, close, span);
        out
    }

    pub(super) fn record_field_str(&mut self, fl: Local, span: Span) -> Local {
        let ty = self.func.locals[fl as usize].clone();
        match ty {
            LirType::Str => {
                let open = self.str_const("\"", span);
                let inner = self.str_concat(open, fl, span);
                let shut = self.str_const("\"", span);
                self.str_concat(inner, shut, span)
            }
            LirType::Bool | LirType::I64 | LirType::F64(_) => {
                let dst = self.local(LirType::Str);
                self.emit_to_str(dst, fl, span);
                dst
            }
            LirType::Closure => self.str_const("<fn>", span),
            LirType::Null => self.str_const("null", span),
            LirType::Tuple(_) => self.record_to_str(fl, span),
            _ => {
                let av = self.coerce_to_slot(fl, &LirType::Any, span);
                let dst = self.local(LirType::Str);
                self.emit(Instr::ToStr { span, dst, src: av });
                dst
            }
        }
    }

    pub(super) fn emit_to_str(&mut self, dst: Local, src: Local, span: Span) {
        let sty = self.func.locals[src as usize].clone();
        if sty == LirType::Str {
            self.emit_copy(dst, src, span);
            return;
        }
        if sty == LirType::Error {
            self.emit(Instr::Call {
                span,
                dsts: vec![dst],
                err: None,
                target: CallTarget::Builtin("__rnx_error_str".to_string()),
                args: vec![src],
            });
            return;
        }
        if sty != LirType::Bool {
            self.emit(Instr::ToStr { span, dst, src });
            return;
        }
        let t_bb = self.new_block();
        let f_bb = self.new_block();
        let merge = self.new_block();
        self.set_term(Terminator::BrIf { span, 
            cond: src,
            then_bb: t_bb,
            else_bb: f_bb,
        });
        self.set_current(t_bb);
        let t = self.local(LirType::Str);
        self.emit(Instr::Const {
            span,
            dst: t,
            lit: Lit::Str("true".to_string()),
        });
        self.emit_copy(dst, t, span);
        self.set_term(Terminator::Br(merge));
        self.set_current(f_bb);
        let f = self.local(LirType::Str);
        self.emit(Instr::Const {
            span,
            dst: f,
            lit: Lit::Str("false".to_string()),
        });
        self.emit_copy(dst, f, span);
        self.set_term(Terminator::Br(merge));
        self.set_current(merge);
    }

    pub(super) fn lower_op_hook(
        &mut self,
        obj: Local,
        op_name: &str,
        op_sym: &str,
        arg_exprs: Vec<A::Spanned<A::Expr>>,
        arg_locals: Vec<Local>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let ty = self.func.locals[obj as usize].clone();
        let cname = match &ty {
            LirType::Obj(n) => n.clone(),
            other => {
                let disp = match other {
                    LirType::I64 => "Int",
                    LirType::F64(_) => "Float",
                    LirType::Bool => "Bool",
                    LirType::Str => "String",
                    LirType::Array(_) => "Array",
                    LirType::Range => "Range",
                    _ => "value",
                };
                return self.fail(self.err(
                    Code::E108,
                    format!("operator `{op_sym}` is not defined for type `{disp}`"),
                    span,
                ));
            }
        };
        let short = short_name(&cname).to_string();
        let check_arity = |this: &Self, mid: usize| -> Result<(), Diagnostic> {
            let want = arg_locals.len() + 1;
            if this.module.functions[mid].sig_params.len() != want {
                return Err(this.err(
                    Code::E108,
                    format!("operator `{op_sym}` needs {want} parameters"),
                    span,
                ));
            }
            let mname = this.module.functions[mid].name.clone();
            if this.param_info.get(&mname).map(|p| p.iter().any(|(_, d)| d.is_some())).unwrap_or(false) {
                return Err(this.err(
                    Code::E108,
                    format!("operator `{op_sym}` cannot have default parameters"),
                    span,
                ));
            }
            Ok(())
        };
        if let Some(ci) = self.module.class_index.get(&cname).copied() {
            if self.module.classes[ci].methods.contains_key(op_name) {
                let mid = self.check_method(ci, op_name, span)?;
                check_arity(self, mid)?;
                let mut full = vec![obj];
                full.extend(arg_locals.iter().copied());
                let slf0 = full.first().copied();
                return self.call_fn_ex(mid, &[], full, &BTreeMap::new(), span, slf0);
            }
        }
        if let Some(eid) = self.find_extension(&ty, op_name) {
            check_arity(self, eid)?;
            let args: Vec<A::CallArg> =
                arg_exprs.into_iter().map(|value| A::CallArg { name: None, value }).collect();
            return self.lower_ext_call(eid, obj, &args, &arg_locals, &[], span);
        }
        self.fail(self.err(
            Code::E108,
            format!("operator `{op_sym}` is not defined for type `{short}`"),
            span,
        ))
    }

    pub(super) fn compound_kind(&mut self, lt: &LirType, rt: &LirType, span: Span) -> Result<NumKind, Diagnostic> {
        let lk = match lt {
            LirType::F64(k) => Some(*k),
            _ => None,
        };
        let rk = match rt {
            LirType::F64(k) => Some(*k),
            _ => None,
        };
        match (lk, rk) {
            (Some(FloatKind::Fast), Some(FloatKind::Strict))
            | (Some(FloatKind::Strict), Some(FloatKind::Fast)) => {
                self.fail(self.err(Code::E305, "implicit Float/FastFloat mix; use `.asFast()`/`.asStrict()`", span))
            }
            (Some(k), _) | (_, Some(k)) => Ok(NumKind::Float(k)),
            _ => Ok(NumKind::Int),
        }
    }

    pub(super) fn incdec_step(&mut self, cur: Local, want: &LirType, delta: i64, span: Span) -> Result<(Local, Simple), Diagnostic> {
        let cty = self.func.locals.get(cur as usize).cloned().unwrap_or(LirType::Any);
        let eff = match want {
            LirType::F64(_) | LirType::I64 | LirType::I8 | LirType::Pointer(_) => want.clone(),
            _ => cty.clone(),
        };
        let arith = if delta >= 0 { ArithOp::Add } else { ArithOp::Sub };
        match &eff {
            LirType::Pointer(inner) => {
                if self.unsafe_depth == 0 {
                    return self.fail(self.err(Code::E202, "pointer arithmetic needs `unsafe`", span));
                }
                let curi = self.unbox_operand(cur, span);
                let one = self.const_int(1, span);
                let dst = self.local(LirType::Pointer(inner.clone()));
                self.emit(Instr::Arith { span, op: arith, kind: NumKind::Int, dst, lhs: curi, rhs: one });
                Ok((dst, Simple::Other))
            }
            LirType::F64(k) => {
                let k = *k;
                let curf = self.local(LirType::F64(k));
                if cty == LirType::Any {
                    self.emit_any_unbox(curf, cur, span);
                } else {
                    self.emit_copy(curf, cur, span);
                }
                let one = self.local(LirType::F64(k));
                self.emit(Instr::Const { span, dst: one, lit: Lit::Float(1.0, k) });
                let dst = self.local(LirType::F64(k));
                self.emit(Instr::Arith { span, op: arith, kind: NumKind::Float(k), dst, lhs: curf, rhs: one });
                Ok((dst, simple_of(&LirType::F64(k))))
            }
            LirType::I64 | LirType::I8 | LirType::Any => {
                let curi = self.unbox_operand(cur, span);
                let one = self.const_int(1, span);
                let dst = self.local(LirType::I64);
                self.emit(Instr::Arith { span, op: arith, kind: NumKind::Int, dst, lhs: curi, rhs: one });
                Ok((dst, Simple::Int))
            }
            _ => self.fail(self.err(Code::E108, "increment/decrement needs an `Int`, `Float`, or `Pointer` operand", span)),
        }
    }

    pub(super) fn lower_incdec(
        &mut self,
        target: &A::Expr,
        tsp: Span,
        delta: i64,
        yield_old: bool,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        match target {
            A::Expr::Ident(n) => {
                let (dst, _, _) = self
                    .lookup(n)
                    .ok_or_else(|| self.err(Code::E108, format!("unknown `{n}`"), tsp))?;
                let dt = self.func.locals.get(dst as usize).cloned().unwrap_or(LirType::Any);
                let (newv, s) = self.incdec_step(dst, &dt, delta, span)?;
                let out = if yield_old {
                    let old = self.local(dt.clone());
                    self.emit_copy(old, dst, span);
                    let nv = self.coerce_to_slot(newv, &dt, span);
                    self.emit_copy(dst, nv, span);
                    old
                } else {
                    let nv = self.coerce_to_slot(newv, &dt, span);
                    self.emit_copy(dst, nv, span);
                    dst
                };
                Ok((out, s))
            }
            A::Expr::Member { base, field } => {
                let (obj, _) = self.lower_expr(&base.node, base.span)?;
                let oty = self.func.locals.get(obj as usize).cloned().unwrap_or(LirType::Any);
                let fty = match &oty {
                    LirType::Obj(name) => {
                        let ci = self.module.class_index.get(name.as_str()).copied();
                        ci.and_then(|c| {
                            let fi = self.module.classes[c].field_index.get(field).copied();
                            fi.map(|f| self.module.classes[c].fields[f].ty.clone())
                        })
                    }
                    _ => None,
                };
                let want = fty.clone().unwrap_or(LirType::Any);
                let cur = self.local(LirType::Any);
                self.emit(Instr::GetFieldByName { span, dst: cur, obj, field: field.clone() });
                let (newv, s) = self.incdec_step(cur, &want, delta, span)?;
                self.emit(Instr::SetFieldByName { span, obj, field: field.clone(), value: newv });
                if yield_old {
                    Ok((cur, s))
                } else {
                    Ok((newv, s))
                }
            }
            A::Expr::Index { base, index } => {
                let (arr, _) = self.lower_expr(&base.node, base.span)?;
                let (ix, _) = self.lower_expr(&index.node, index.span)?;
                let arr_ty = self.func.locals.get(arr as usize).cloned().unwrap_or(LirType::Any);
                if matches!(arr_ty, LirType::Obj(_)) {
                    return self.fail(self.err(Code::E108, "increment/decrement on an overloaded `[]` target is not supported", span));
                }
                let es = self.elem_size_of(arr);
                let slot = match self.func.locals.get(arr as usize) {
                    Some(LirType::Array(inner)) => (**inner).clone(),
                    _ => LirType::Any,
                };
                let cur = self.local(slot.clone());
                self.emit(Instr::ArrayGet { span, dst: cur, arr, index: ix, elem_size: es, unchecked: false });
                let (newv, s) = self.incdec_step(cur, &slot, delta, span)?;
                let nv = self.coerce_to_slot(newv, &slot, span);
                self.emit(Instr::ArraySet { span, arr, index: ix, value: nv, elem_size: es, unchecked: false });
                if yield_old {
                    Ok((cur, s))
                } else {
                    Ok((newv, s))
                }
            }
            A::Expr::Unary { op, rhs } if matches!(op, A::UnOp::Deref) => {
                if self.unsafe_depth == 0 {
                    return self.fail(self.err(Code::E202, "pointer dereference requires unsafe block", span));
                }
                let (p, _) = self.lower_expr(&rhs.node, rhs.span)?;
                let pty = self.func.locals.get(p as usize).cloned().unwrap_or(LirType::Any);
                let scalar = self.pointer_scalar(&pty, tsp)?;
                let cur = self.local(scalar.clone());
                self.emit(Instr::PtrLoad { span, dst: cur, ptr: p, volatile: false });
                let (newv, s) = self.incdec_step(cur, &scalar, delta, span)?;
                let nv = self.coerce_to_slot(newv, &scalar, span);
                self.emit(Instr::PtrStore { span, ptr: p, val: nv, volatile: false });
                if yield_old {
                    Ok((cur, s))
                } else {
                    Ok((newv, s))
                }
            }
            _ => self.fail(self.err(Code::E108, "invalid target for increment/decrement operator", tsp)),
        }
    }

    pub(super) fn lower_binary(
        &mut self,
        op: A::BinOp,
        lhs: &A::Spanned<A::Expr>,
        rhs: &A::Spanned<A::Expr>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        if op == A::BinOp::And || op == A::BinOp::Or {
            return self.lower_binary_logic(op, lhs, rhs, span);
        }
        if matches!(
            op,
            A::BinOp::BitAnd | A::BinOp::BitOr | A::BinOp::BitXor | A::BinOp::Shl | A::BinOp::Shr | A::BinOp::Zshr
        ) {
            return self.lower_binary_bitwise(op, lhs, rhs, span);
        }
        let (l, ls) = self.lower_expr(&lhs.node, Self::eff_span(lhs.span, span))?;
        let (r, rs) = self.lower_expr(&rhs.node, Self::eff_span(rhs.span, span))?;
        self.deny_tuple(l, "in a binary operation", span)?;
        self.deny_tuple(r, "in a binary operation", span)?;
        let lty = self.func.locals[l as usize].clone();
        let rty = self.func.locals[r as usize].clone();
        let ls = match ls {
            Simple::Other => simple_of(&lty),
            _ => ls,
        };
        let rs = match rs {
            Simple::Other => simple_of(&rty),
            _ => rs,
        };
        if matches!(lty, LirType::Pointer(_)) || matches!(rty, LirType::Pointer(_)) {
            return self.lower_binary_pointer(op, l, r, lty.clone(), rty.clone(), span);
        }
        let is_cmp = !matches!(op, A::BinOp::Add | A::BinOp::Sub | A::BinOp::Mul | A::BinOp::Div | A::BinOp::Mod);
        if matches!((ls, rs), (Simple::Strict, Simple::Fast) | (Simple::Fast, Simple::Strict)) {
            return self.fail(self.err(
                Code::E305,
                "implicit Float/FastFloat mix; use `.asFast()`/`.asStrict()`",
                span,
            ));
        }
        self.deny_tuple(l, "in a binary operation", span)?;
        self.deny_tuple(r, "in a binary operation", span)?;
        let lty = self.func.locals[l as usize].clone();
        let rty = self.func.locals[r as usize].clone();
        if matches!(lty, LirType::Vec4f | LirType::Vec4i)
            || matches!(rty, LirType::Vec4f | LirType::Vec4i) {
            return self.lower_binary_vec(op, l, r, lty.clone(), rty.clone(), span);
        }
        if !is_cmp && (lty == LirType::Str || rty == LirType::Str || lty == LirType::Error || rty == LirType::Error) {
            return self.lower_binary_string(op, l, r, lty.clone(), rty.clone(), span);
        }
        if matches!(lty, LirType::Obj(_)) {
            let op_name = match op {
                A::BinOp::Add => Some(("op_add", "+")),
                A::BinOp::Sub => Some(("op_sub", "-")),
                A::BinOp::Mul => Some(("op_mul", "*")),
                A::BinOp::Div => Some(("op_div", "/")),
                _ => None,
            };
            if let Some((name, sym)) = op_name {
                return self.lower_op_hook(l, name, sym, vec![rhs.clone()], vec![r], span);
            }
        }
        let kind = if ls == Simple::Fast || rs == Simple::Fast {
            NumKind::Float(FloatKind::Fast)
        } else if ls == Simple::Strict || rs == Simple::Strict {
            NumKind::Float(FloatKind::Strict)
        } else {
            NumKind::Int
        };
        if is_cmp && (op == A::BinOp::Eq || op == A::BinOp::NotEq)
            && ((lty == LirType::Str && rty == LirType::Any)
                || (lty == LirType::Any && rty == LirType::Str)) {
            return self.lower_binary_any_str(op, l, r, lty.clone(), span);
        }
        if is_cmp && (op == A::BinOp::Eq || op == A::BinOp::NotEq)
            && lty == LirType::Any
            && rty == LirType::Any {
            return self.lower_binary_any_any(op, l, r, span);
        }
        let null_cmp = is_cmp
            && (op == A::BinOp::Eq || op == A::BinOp::NotEq)
            && (lty == LirType::Null || rty == LirType::Null);
        let l = if null_cmp {
            l
        } else {
            let u = self.unbox_operand(l, span);
            self.nub_use(u, span)
        };
        let r = if null_cmp {
            r
        } else {
            let u = self.unbox_operand(r, span);
            self.nub_use(u, span)
        };
        if is_cmp {
            return self.lower_binary_compare(op, l, r, lty.clone(), rty.clone(), kind, span);
        }
        let out_ty = match kind {
            NumKind::Int => LirType::I64,
            NumKind::Float(k) => LirType::F64(k),
        };
        let dst = self.local(out_ty);
        let op = match op {
            A::BinOp::Add => ArithOp::Add,
            A::BinOp::Sub => ArithOp::Sub,
            A::BinOp::Mul => ArithOp::Mul,
            A::BinOp::Div => ArithOp::Div,
            A::BinOp::Mod => ArithOp::Mod,
            _ => {
                return self.ice("unhandled arithmetic operator", Some(span));
            }
        };
        self.emit(Instr::Arith { span,  op, kind, dst, lhs: l, rhs: r });
        let s = match kind {
            NumKind::Int => Simple::Int,
            NumKind::Float(FloatKind::Strict) => Simple::Strict,
            NumKind::Float(FloatKind::Fast) => Simple::Fast,
        };
        Ok((dst, s))
    }

    pub(super) fn lower_binary_compare(&mut self, op: A::BinOp, l: Local, r: Local, lty: LirType, rty: LirType, kind: NumKind, span: Span) -> Result<(Local, Simple), Diagnostic> {

    if (op == A::BinOp::Eq || op == A::BinOp::NotEq)
        && lty == LirType::Str
        && rty == LirType::Str
    {
        let dst = self.local(LirType::Bool);
        self.emit(Instr::Call { span, 
            dsts: vec![dst],
            err: None,
            target: CallTarget::Builtin("streq".to_string()),
            args: vec![l, r],
        });
        if op == A::BinOp::NotEq {
            let tmp = self.local(LirType::Bool);
            self.emit_copy(tmp, dst, span);
            self.emit(Instr::Not { span,  dst, src: tmp });
        }
        return Ok((dst, Simple::Other));
    }
    if matches!(
        op,
        A::BinOp::Lt | A::BinOp::LtEq | A::BinOp::Gt | A::BinOp::GtEq
    ) && lty == LirType::Str
        && rty == LirType::Str
    {
        let cv = self.local(LirType::I64);
        self.emit(Instr::Call { span,
            dsts: vec![cv],
            err: None,
            target: CallTarget::Builtin("strcmp".to_string()),
            args: vec![l, r],
        });
        let zero = self.local(LirType::I64);
        self.emit(Instr::Const { span,  dst: zero, lit: Lit::Int(0) });
        let dst = self.local(LirType::Bool);
        let cop = match op {
            A::BinOp::Lt => CmpOp::Lt,
            A::BinOp::LtEq => CmpOp::LtEq,
            A::BinOp::Gt => CmpOp::Gt,
            A::BinOp::GtEq => CmpOp::GtEq,
            _ => {
                return self.ice("unhandled string comparison operator", Some(span));
            }
        };
        self.emit(Instr::Cmp { span,  op: cop, kind: NumKind::Int, dst, lhs: cv, rhs: zero });
        return Ok((dst, Simple::Other));
    }
    if (op == A::BinOp::Eq || op == A::BinOp::NotEq)
        && matches!(lty, LirType::Enum(_))
        && matches!(rty, LirType::Enum(_))
    {
        let same = lty == rty;
        let dst = self.local(LirType::Bool);
        if same {
            let lt = self.local(LirType::I64);
            let rt = self.local(LirType::I64);
            self.emit(Instr::EnumTag { span,  dst: lt, scrut: l });
            self.emit(Instr::EnumTag { span,  dst: rt, scrut: r });
            let op = if op == A::BinOp::Eq { CmpOp::Eq } else { CmpOp::NotEq };
            self.emit(Instr::Cmp { span,  op, kind: NumKind::Int, dst, lhs: lt, rhs: rt });
        } else {
            let c = self.local(LirType::Bool);
            self.emit(Instr::Const { span,  dst: c, lit: Lit::Bool(false) });
            if op == A::BinOp::Eq {
                self.emit_copy(dst, c, span);
            } else {
                self.emit(Instr::Not { span,  dst, src: c });
            }
        }
        return Ok((dst, Simple::Other));
    }
    let dst = self.local(LirType::Bool);
    let op = match op {
        A::BinOp::Eq => CmpOp::Eq,
        A::BinOp::NotEq => CmpOp::NotEq,
        A::BinOp::Lt => CmpOp::Lt,
        A::BinOp::LtEq => CmpOp::LtEq,
        A::BinOp::Gt => CmpOp::Gt,
        A::BinOp::GtEq => CmpOp::GtEq,
        _ => {
            return self.ice("unhandled comparison operator", Some(span));
        }
    };
    self.emit(Instr::Cmp { span,  op, kind, dst, lhs: l, rhs: r });
    return Ok((dst, Simple::Other));

    }

    pub(super) fn lower_binary_any_any(&mut self, op: A::BinOp, l: Local, r: Local, span: Span) -> Result<(Local, Simple), Diagnostic> {

    let dst = self.local(LirType::Bool);
    self.emit(Instr::Call { span,
        dsts: vec![dst],
        err: None,
        target: CallTarget::Builtin("__rnx_eq_any".to_string()),
        args: vec![l, r],
    });
    if op == A::BinOp::NotEq {
        let tmp = self.local(LirType::Bool);
        self.emit_copy(tmp, dst, span);
        self.emit(Instr::Not { span,  dst, src: tmp });
    }
    return Ok((dst, Simple::Other));

    }

    pub(super) fn lower_binary_any_str(&mut self, op: A::BinOp, l: Local, r: Local, lty: LirType, span: Span) -> Result<(Local, Simple), Diagnostic> {

    let (anyv, strv) = if lty == LirType::Any { (l, r) } else { (r, l) };
    let dst = self.local(LirType::Bool);
    self.emit(Instr::Call { span,
        dsts: vec![dst],
        err: None,
        target: CallTarget::Builtin("__rnx_eq_any_str".to_string()),
        args: vec![anyv, strv],
    });
    if op == A::BinOp::NotEq {
        let tmp = self.local(LirType::Bool);
        self.emit_copy(tmp, dst, span);
        self.emit(Instr::Not { span,  dst, src: tmp });
    }
    return Ok((dst, Simple::Other));

    }

    pub(super) fn lower_binary_string(&mut self, op: A::BinOp, l: Local, r: Local, lty: LirType, rty: LirType, span: Span) -> Result<(Local, Simple), Diagnostic> {

    if op != A::BinOp::Add {
        return self.fail(self.err(Code::E108, "strings support `+` only", span));
    }
    let dst = self.local(LirType::Str);
    let ll = if lty == LirType::Str {
        l
    } else if lty == LirType::Error {
        let t = self.local(LirType::Str);
        self.emit(Instr::Call {
            span,
            dsts: vec![t],
            err: None,
            target: CallTarget::Builtin("__rnx_error_str".to_string()),
            args: vec![l],
        });
        t
    } else {
        let t = self.local(LirType::Str);
        self.emit_to_str(t, l, span);
        t
    };
    let rr = if rty == LirType::Str {
        r
    } else if rty == LirType::Error {
        let t = self.local(LirType::Str);
        self.emit(Instr::Call {
            span,
            dsts: vec![t],
            err: None,
            target: CallTarget::Builtin("__rnx_error_str".to_string()),
            args: vec![r],
        });
        t
    } else {
        let t = self.local(LirType::Str);
        self.emit_to_str(t, r, span);
        t
    };
    self.emit(Instr::Concat { span,  dst, lhs: ll, rhs: rr });
    return Ok((dst, Simple::Other));

    }

    pub(super) fn lower_binary_vec(&mut self, op: A::BinOp, l: Local, r: Local, lty: LirType, rty: LirType, span: Span) -> Result<(Local, Simple), Diagnostic> {

    let kind = match (&lty, &rty) {
        (LirType::Vec4f, LirType::Vec4f) => VecKind::F,
        (LirType::Vec4i, LirType::Vec4i) => VecKind::I,
        _ => {
            return self.fail(self.err(
                Code::E108,
                "vector operands must both be Vec4f or both Vec4i",
                span,
            ));
        }
    };
    let op = match op {
        A::BinOp::Add => VecOp::Add,
        A::BinOp::Sub => VecOp::Sub,
        A::BinOp::Mul => VecOp::Mul,
        A::BinOp::Div => VecOp::Div,
        _ => {
            return self.fail(self.err(
                Code::E108,
                "vectors support `+`, `-`, `*`, `/` only",
                span,
            ));
        }
    };
    let dst = self.local(if kind == VecKind::F { LirType::Vec4f } else { LirType::Vec4i });
    self.emit(Instr::VecArith { span,  dst, op, kind, lhs: l, rhs: r });
    return Ok((dst, Simple::Other));

    }

    pub(super) fn lower_binary_pointer(&mut self, op: A::BinOp, l: Local, r: Local, lty: LirType, rty: LirType, span: Span) -> Result<(Local, Simple), Diagnostic> {

    if self.unsafe_depth == 0 {
        return self.fail(self.err(
            Code::E202,
            "pointer arithmetic needs `unsafe`",
            span,
        ));
    }
    let inner = match &lty {
        LirType::Pointer(inner) => inner.as_ref().clone(),
        _ => match &rty {
            LirType::Pointer(inner) => inner.as_ref().clone(),
            _ => LirType::Any,
        },
    };
    let dst = self.local(LirType::Pointer(Box::new(inner)));
    let li = self.local(LirType::I64);
    let ri = self.local(LirType::I64);
    self.emit_copy(li, l, span);
    self.emit_copy(ri, r, span);
    let arith = match op {
        A::BinOp::Add => ArithOp::Add,
        A::BinOp::Sub => ArithOp::Sub,
        _ => {
            return self.fail(self.err(
                Code::E108,
                "pointer arithmetic supports `+` and `-` only",
                span,
            ));
        }
    };
    self.emit(Instr::Arith { span, 
        op: arith,
        kind: NumKind::Int,
        dst,
        lhs: li,
        rhs: ri,
    });
    return Ok((dst, Simple::Int));

    }

    pub(super) fn lower_binary_bitwise(&mut self, op: A::BinOp, lhs: &A::Spanned<A::Expr>, rhs: &A::Spanned<A::Expr>, span: Span) -> Result<(Local, Simple), Diagnostic> {

    let (l, _) = self.lower_expr(&lhs.node, Self::eff_span(lhs.span, span))?;
    let (r, _) = self.lower_expr(&rhs.node, Self::eff_span(rhs.span, span))?;
    self.deny_tuple(l, "in a bitwise operation", span)?;
    self.deny_tuple(r, "in a bitwise operation", span)?;
    let lty = self.func.locals[l as usize].clone();
    let rty = self.func.locals[r as usize].clone();
    if lty != LirType::I64 || rty != LirType::I64 {
        return self.fail(self.err(
            Code::E108,
            "bitwise operations need `Int` operands",
            span,
        ));
    }
    let arith = match op {
        A::BinOp::BitAnd => ArithOp::BitAnd,
        A::BinOp::BitOr => ArithOp::BitOr,
        A::BinOp::BitXor => ArithOp::BitXor,
        A::BinOp::Shl => ArithOp::Shl,
        A::BinOp::Shr => ArithOp::Shr,
        _ => ArithOp::Zshr,
    };
    let dst = self.local(LirType::I64);
    self.emit(Instr::Arith { span,
        op: arith,
        kind: NumKind::Int,
        dst,
        lhs: l,
        rhs: r,
    });
    return Ok((dst, Simple::Int));

    }

    pub(super) fn lower_binary_logic(&mut self, op: A::BinOp, lhs: &A::Spanned<A::Expr>, rhs: &A::Spanned<A::Expr>, span: Span) -> Result<(Local, Simple), Diagnostic> {

    let (l, _) = self.lower_expr(&lhs.node, Self::eff_span(lhs.span, span))?;
    let l = self.nub_cond(l, Self::eff_span(lhs.span, span));
    let dst = self.local(LirType::Bool);
    let rhs_bb = self.new_block();
    let merge = self.new_block();
    if op == A::BinOp::And {
        let skip = self.new_block();
        self.set_term(Terminator::BrIf { span, 
            cond: l,
            then_bb: rhs_bb,
            else_bb: skip,
        });
        self.set_current(skip);
        let f = self.local(LirType::Bool);
        self.emit(Instr::Const { span,  dst: f, lit: Lit::Bool(false) });
        self.emit_copy(dst, f, span);
        self.set_term(Terminator::Br(merge));
    } else {
        let skip = self.new_block();
        self.set_term(Terminator::BrIf { span, 
            cond: l,
            then_bb: skip,
            else_bb: rhs_bb,
        });
        self.set_current(skip);
        let t = self.local(LirType::Bool);
        self.emit(Instr::Const { span,  dst: t, lit: Lit::Bool(true) });
        self.emit_copy(dst, t, span);
        self.set_term(Terminator::Br(merge));
    }
    self.set_current(rhs_bb);
    let (r, _) = self.lower_expr(&rhs.node, Self::eff_span(rhs.span, span))?;
    let r = self.nub_use(r, Self::eff_span(rhs.span, span));
    self.emit_copy(dst, r, span);
    if !self.terminated {
        self.set_term(Terminator::Br(merge));
    }
    self.set_current(merge);
    return Ok((dst, Simple::Other));

    }


    pub(super) fn lower_propagate(
        &mut self,
        inner: &A::Spanned<A::Expr>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let (v, _) = self.lower_expr(&inner.node, inner.span)?;
        let vt = self.func.locals[v as usize].clone();
        let ei = match vt {
            LirType::Enum(ei) => ei,
            _ => {
                let mut d = self.err(
                    Code::E108,
                    "`?` needs a `Result` value",
                    inner.span,
                );
                if matches!(inner.node, A::Expr::Call { .. }) {
                    d = d.with_hint(
                        "a `throws` call propagates to the enclosing `catch` automatically; remove the `?`",
                    );
                }
                return self.fail(d);
            }
        };
        let ename = self.module.enums[ei].name.clone();
        let (ok_name, err_name) = match ename.as_str() {
            "std.prelude.Result" => ("Ok", "Err"),
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    format!("`?` needs `Result`, got `{ename}`"),
                    inner.span,
                ));
            }
        };
        if self.func.ret != LirType::Enum(ei) {
            return self.fail(self.err(
                Code::E108,
                format!("`?` on `{ename}` needs an enclosing function returning `{ename}`"),
                span,
            ));
        }
        let ok_vi = *self.module.enums[ei].variant_index.get(ok_name).ok_or_else(|| {
            self.err(Code::E108, format!("missing `{ename}::{ok_name}`"), span)
        })?;
        let err_vi = *self.module.enums[ei].variant_index.get(err_name).ok_or_else(|| {
            self.err(Code::E108, format!("missing `{ename}::{err_name}`"), span)
        })?;
        let payload_ty = self.module.enums[ei].variants[ok_vi].payload.first().cloned().unwrap_or(LirType::Any);
        let tag = self.local(LirType::I64);
        self.emit(Instr::EnumTag { span, dst: tag, scrut: v });
        let want = self.local(LirType::I64);
        self.emit(Instr::Const { span, dst: want, lit: Lit::Int(ok_vi as i64) });
        let is_ok = self.local(LirType::Bool);
        self.emit(Instr::Cmp { span, op: CmpOp::Eq, kind: NumKind::Int, dst: is_ok, lhs: tag, rhs: want });
        let some_bb = self.new_block();
        let none_bb = self.new_block();
        let merge = self.new_block();
        self.set_term(Terminator::BrIf { span, cond: is_ok, then_bb: some_bb, else_bb: none_bb });
        let inner_ty = self.precise.get(&v).cloned().unwrap_or_else(|| payload_ty.clone());
        let dst = self.local(inner_ty.clone());
        self.set_current(some_bb);
        let boxable = matches!(inner_ty, LirType::I64 | LirType::Bool | LirType::F64(_) | LirType::Str);
        if boxable && inner_ty != payload_ty {
            let tmp = self.local(payload_ty.clone());
            self.emit(Instr::EnumPayload { span, dst: tmp, scrut: v, index: 0 });
            self.emit_any_unbox(dst, tmp, span);
        } else {
            self.emit(Instr::EnumPayload { span, dst, scrut: v, index: 0 });
        }
        self.set_term(Terminator::Br(merge));
        self.set_current(none_bb);
        if ename.as_str() == "std.prelude.Result" {
            let err_ty = self.module.enums[ei].variants[err_vi].payload.first().cloned().unwrap_or(LirType::Any);
            let e = self.local(err_ty);
            self.emit(Instr::EnumPayload { span, dst: e, scrut: v, index: 0 });
            let ev = self.local(LirType::Enum(ei));
            self.emit(Instr::EnumNew { span, dst: ev, enu: ei, variant: err_vi, payload: vec![e] });
            self.emit_run_defers(0);
            self.set_term(Terminator::Ret(vec![ev]));
        } else {
            let nv = self.local(LirType::Enum(ei));
            self.emit(Instr::EnumNew { span, dst: nv, enu: ei, variant: err_vi, payload: Vec::new() });
            self.emit_run_defers(0);
            self.set_term(Terminator::Ret(vec![nv]));
        }
        self.set_current(merge);
        self.precise.insert(dst, inner_ty.clone());
        Ok((dst, simple_of(&inner_ty)))
    }

    pub(super) fn lower_coalesce(
        &mut self,
        lhs: &A::Spanned<A::Expr>,
        rhs: &A::Spanned<A::Expr>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let (v, _) = self.lower_expr(&lhs.node, lhs.span)?;
        let vt = self.func.locals[v as usize].clone();
        let null = self.local(LirType::Null);
        self.emit(Instr::Const { span, dst: null, lit: Lit::Null });
        let is_null = self.local(LirType::Bool);
        self.emit(Instr::Cmp {
            span,
            op: CmpOp::Eq,
            kind: NumKind::Int,
            dst: is_null,
            lhs: v,
            rhs: null,
        });
        let keep_bb = self.new_block();
        let else_bb = self.new_block();
        let merge = self.new_block();
        self.set_term(Terminator::BrIf { span, cond: is_null, then_bb: else_bb, else_bb: keep_bb });
        self.set_current(else_bb);
        let (r, _) = self.lower_expr(&rhs.node, rhs.span)?;
        self.deny_tuple(r, "as a `??` fallback", rhs.span)?;
        let rt = self.func.locals[r as usize].clone();
        let lt = self.precise.get(&v).cloned().unwrap_or_else(|| vt.clone());
        if !matches!(lt, LirType::Any)
            && !matches!(rt, LirType::Any)
            && !matches!(lt, LirType::Null)
            && !matches!(rt, LirType::Null)
            && rt != lt
        {
            return self.fail(self.err(
                Code::E108,
                "`??` fallback must match the value type",
                rhs.span,
            ));
        }
        let result_ty = if matches!(lt, LirType::Null) {
            rt.clone()
        } else if matches!(lt, LirType::Any) && matches!(rt, LirType::Obj(_)) {
            rt.clone()
        } else {
            lt.clone()
        };
        let dst = self.local(result_ty.clone());
        let v_marked = self.nub_has(v);
        let r_marked = self.nub_has(r);
        let vt_scalar = matches!(vt, LirType::I64 | LirType::Bool | LirType::F64(_));
        let rt_scalar = matches!(rt, LirType::I64 | LirType::Bool | LirType::F64(_));
        if v_marked || r_marked {
            self.nub_mark(dst);
        }
        if r_marked {
            self.nub_store(dst, r, span);
        } else if self.nub_has(dst) && rt_scalar {
            self.nub_box(dst, r, span);
        } else if result_ty == LirType::Any {
            let c = self.coerce_to_slot(r, &LirType::Any, span);
            self.emit_copy(dst, c, span);
        } else {
            self.emit_copy(dst, r, span);
        }
        if !self.terminated {
            self.set_term(Terminator::Br(merge));
        }
        self.set_current(keep_bb);
        if v_marked {
            self.nub_store(dst, v, span);
        } else if vt == LirType::Any && result_ty == LirType::Any {
            self.emit_copy(dst, v, span);
        } else if vt == LirType::Any {
            let ku = self.local(result_ty.clone());
            self.emit_any_unbox(ku, v, span);
            self.emit_copy(dst, ku, span);
        } else if self.nub_has(dst) && vt_scalar {
            self.nub_box(dst, v, span);
        } else {
            self.emit_copy(dst, v, span);
        }
        if let Some(pt) = self.precise.get(&v).cloned() {
            self.precise.insert(dst, pt);
        }
        if !self.terminated {
            self.set_term(Terminator::Br(merge));
        }
        self.set_current(merge);
        Ok((dst, simple_of(&result_ty)))
    }

    pub(super) fn lower_opt_access(
        &mut self,
        base: &A::Spanned<A::Expr>,
        field: &str,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let (v, _) = self.lower_expr(&base.node, base.span)?;
        let null = self.local(LirType::Null);
        self.emit(Instr::Const { span, dst: null, lit: Lit::Null });
        let is_null = self.local(LirType::Bool);
        self.emit(Instr::Cmp {
            span,
            op: CmpOp::Eq,
            kind: NumKind::Int,
            dst: is_null,
            lhs: v,
            rhs: null,
        });
        let some_bb = self.new_block();
        let none_bb = self.new_block();
        let merge = self.new_block();
        self.set_term(Terminator::BrIf { span, cond: is_null, then_bb: none_bb, else_bb: some_bb });
        self.set_current(some_bb);
        let vt = self.func.locals[v as usize].clone();
        let inner_ty = self.precise.get(&v).cloned().unwrap_or(vt);
        let synth = format!("$opt{}", self.synth_seq);
        self.synth_seq += 1;
        self.push_scope();
        self.def(synth.clone(), v, simple_of(&inner_ty), inner_ty.clone());
        let access: A::Expr = if args.is_empty() {
            A::Expr::Member {
                base: Box::new(A::Spanned { node: A::Expr::Ident(synth.clone()), span }),
                field: field.to_string(),
            }
        } else {
            A::Expr::Call {
                callee: Box::new(A::Spanned {
                    node: A::Expr::Member {
                        base: Box::new(A::Spanned { node: A::Expr::Ident(synth.clone()), span }),
                        field: field.to_string(),
                    },
                    span,
                }),
                type_args: Vec::new(),
                args: args.to_vec(),
                trailing: None,
            }
        };
        let lowered = self.lower_expr(&access, span);
        self.scopes.pop();
        let (got, _) = lowered?;
        self.deny_tuple(got, "as a `?.` result", span)?;
        let gt = self.func.locals[got as usize].clone();
        let dst = self.local(gt.clone());
        if self.nub_has(got) {
            self.nub_store(dst, got, span);
        } else if matches!(gt, LirType::I64 | LirType::Bool | LirType::F64(_)) {
            self.nub_box(dst, got, span);
        } else {
            self.emit_copy(dst, got, span);
        }
        if let Some(pt) = self.precise.get(&got).cloned() {
            self.precise.insert(dst, pt);
        }
        if !self.terminated {
            self.set_term(Terminator::Br(merge));
        }
        self.set_current(none_bb);
        self.emit(Instr::Const { span, dst, lit: Lit::Null });
        self.set_term(Terminator::Br(merge));
        self.set_current(merge);
        Ok((dst, simple_of(&gt)))
    }

    pub(super) fn lower_handle_result(
        &mut self,
        handle: Local,
        val_builtin: &str,
        err_builtin: &str,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let res_ei = self.module.enum_index.get("std.prelude.Result").copied().ok_or_else(|| {
            self.err(Code::E108, "missing `std.prelude.Result`", span)
        })?;
        let ok_vi = *self.module.enums[res_ei].variant_index.get("Ok").ok_or_else(|| {
            self.err(Code::E108, "missing `Result::Ok`", span)
        })?;
        let err_vi = *self.module.enums[res_ei].variant_index.get("Err").ok_or_else(|| {
            self.err(Code::E108, "missing `Result::Err`", span)
        })?;
        let val = self.local(LirType::Any);
        self.emit(Instr::Call { span,  dsts: vec![val], err: None, target: CallTarget::Builtin(val_builtin.to_string()), args: vec![handle] });
        let emsg = self.local(LirType::Str);
        self.emit(Instr::Call { span,  dsts: vec![emsg], err: None, target: CallTarget::Builtin(err_builtin.to_string()), args: vec![handle] });
        let n = self.local(LirType::I64);
        self.emit(Instr::Call { span,  dsts: vec![n], err: None, target: CallTarget::Builtin("__rnx_string_len".to_string()), args: vec![emsg] });
        let zero = self.local(LirType::I64);
        self.emit(Instr::Const { span,  dst: zero, lit: Lit::Int(0) });
        let ok = self.local(LirType::Bool);
        self.emit(Instr::Cmp { span,  op: CmpOp::Eq, kind: NumKind::Int, dst: ok, lhs: n, rhs: zero });
        let ok_bb = self.new_block();
        let err_bb = self.new_block();
        let merge = self.new_block();
        self.set_term(Terminator::BrIf { span,  cond: ok, then_bb: ok_bb, else_bb: err_bb });
        let dst = self.local(LirType::Enum(res_ei));
        self.set_current(ok_bb);
        self.emit(Instr::EnumNew { span,  dst, enu: res_ei, variant: ok_vi, payload: vec![val] });
        self.set_term(Terminator::Br(merge));
        self.set_current(err_bb);
        self.emit(Instr::EnumNew { span,  dst, enu: res_ei, variant: err_vi, payload: vec![emsg] });
        self.set_term(Terminator::Br(merge));
        self.set_current(merge);
        Ok((dst, Simple::Other))
    }

}
