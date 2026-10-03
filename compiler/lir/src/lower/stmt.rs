use super::*;
use diagnostics::{Code, Diagnostic, Span};
use frontend::ast as A;
use std::collections::{BTreeSet};
impl<'a> Builder<'a> {
    pub(super) fn lower_fn_body(&mut self, f: &A::FnDecl) -> Result<(), Diagnostic> {
        if self.enclosing.is_some() {
            self.deny_tuple_sig()?;
        } else {
            for p in self.func.sig_params.clone() {
                Self::deny_nested_tuple(&p, crate::instr::UNKNOWN_SPAN)?;
            }
            let ret = self.func.ret.clone();
            Self::deny_nested_tuple(&ret, crate::instr::UNKNOWN_SPAN)?;
        }
        self.throws_fn = f.throws;
        if f.is_unsafe {
            self.unsafe_depth = 1;
        }
        self.bind_params(&f.params)?;
        match &f.body {
            A::FnBody::Block(b) => {
                self.lower_block(b)?;
                Ok(())
            }
            A::FnBody::Expr(e) => {
                let (v, _) = self.lower_expr(&e.node, e.span)?;
                let rt = self.func.ret.clone();
                let rvt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                let fresh = matches!(e.node, A::Expr::Array(_));
                self.check_array_view(&rvt, &rt, fresh, e.span, "return value")?;
                let vals = self.lower_return_value(v, e.span)?;
                self.emit_run_defers(0);
                self.set_term(Terminator::Ret(vals));
                Ok(())
            }
        }
    }

    pub(super) fn lower_init_body(&mut self, params: Vec<A::Param>, body: A::Block) -> Result<(), Diagnostic> {
        self.is_init = true;
        self.throws_fn = self.func.throws;
        self.bind_params(&params)?;
        if let Some(pid) = self.implicit_super {
            if let Some(slf) = self.self_local {
                self.emit_checked_call(pid, &[], &[slf], crate::instr::UNKNOWN_SPAN)?;
            }
        }
        self.lower_block(&body)?;
        if let Some(slf) = self.self_local {
            if !self.terminated {
                self.emit_run_defers(0);
                self.set_term(Terminator::Ret(vec![slf]));
            }
        }
        Ok(())
    }

    pub(super) fn lower_bare_body(&mut self, params: Vec<A::Param>, body: A::Block) -> Result<(), Diagnostic> {
        self.bind_params(&params)?;
        self.lower_block(&body)?;
        Ok(())
    }

    pub(super) fn bind_params(&mut self, params: &[A::Param]) -> Result<(), Diagnostic> {
        let skip = if self.self_local.is_some() { 1 } else { 0 };
        let mut idx = skip as Local;
        for p in params.iter() {
            let ty = p.ty.as_ref().map(|t| self.resolve_here(t)).unwrap_or(LirType::Any);
            let flat = crate::instr::flat_sig(&ty);
            if flat.len() == 1 {
                self.def(p.name.clone(), idx, simple_of(&flat[0]), flat[0].clone());
                if let Some(arg) = p.ty.as_ref().and_then(option_arg) {
                    let pt = self.resolve_here(arg);
                    self.precise.insert(idx, pt);
                }
                if p.ty.as_ref().is_some_and(|t| nub_ty(t)) {
                    self.nub_mark(idx);
                }
                idx += 1;
            } else if ty == LirType::Range {
                let lo = idx;
                let hi = idx + 1;
                let incl = idx + 2;
                let step = idx + 3;
                idx += 4;
                let marker = self.local(ty.clone());
                self.ranges.insert(marker, (lo, hi, incl, step));
                self.def(p.name.clone(), marker, Simple::Other, ty);
            } else {
                let mut elems = Vec::with_capacity(flat.len());
                for _ in &flat {
                    elems.push(idx);
                    idx += 1;
                }
                let marker = self.local(ty.clone());
                self.tuples.insert(marker, elems);
                self.def(p.name.clone(), marker, Simple::Other, ty);
            }
        }
        Ok(())
    }

    pub(super) fn emit_run_defers(&mut self, keep: usize) {
        if self.defer_count > keep {
            self.emit(Instr::RunDefers { span: crate::instr::UNKNOWN_SPAN, keep });
        }
    }

    pub(super) fn lower_block(&mut self, b: &A::Block) -> Result<(), Diagnostic> {
        self.push_scope();
        for st in &b.stmts {
            if self.terminated {
                return self.fail(self.err(Code::E108, "unreachable statement", st.span));
            }
            self.lower_stmt(&st.node, st.span)?;
            if self.failed {
                match self.diags.last().cloned() {
                    Some(d) => return Err(d),
                    None => return self.ice("lowering failed with no diagnostic recorded", Some(st.span)),
                }
            }
        }
        self.pop_scope();
        Ok(())
    }

    pub(super) fn lower_stmt(&mut self, st: &A::Stmt, span: Span) -> Result<(), Diagnostic> {
        match st {
            A::Stmt::Var {
                name, ty, value, ..
            }  => self.lower_stmt_var(name, ty, value, span),
            A::Stmt::DestructureTuple { names, ty, value }  => self.lower_stmt_destructure_tuple(names, ty, value),
            A::Stmt::DestructureRecord { fields, rest, value }  => self.lower_stmt_destructure_record(fields, rest, value),
            A::Stmt::DestructureArray { names, rest, value }  => self.lower_stmt_destructure_array(names, rest, value),
            A::Stmt::Assign { target, op, value }  => self.lower_stmt_assign(target, op, value, span),
            A::Stmt::Expr(e)  => self.lower_stmt_expr(e),
            A::Stmt::Assert(e)  => self.lower_stmt_assert(e, span),
            A::Stmt::If { cond, then, otherwise }  => self.lower_stmt_if(cond, then, otherwise, span),
            A::Stmt::While { cond, body }  => self.lower_stmt_while(cond, body, span),
            A::Stmt::DoWhile { body, cond }  => self.lower_stmt_do_while(body, cond, span),
            A::Stmt::For { binding, iter, body }  => self.lower_for(binding, iter, body, span),
            A::Stmt::Switch {
                scrutinee,
                cases,
                default,
            }  => self.lower_switch(scrutinee, cases, default),
            A::Stmt::Return(v)  => self.lower_stmt_return(v, span),
            A::Stmt::Break  => self.lower_stmt_break(span),
            A::Stmt::Continue  => self.lower_stmt_continue(span),
            A::Stmt::Pass => Self::lower_stmt_pass(),
            A::Stmt::Empty => Self::lower_stmt_empty(),
            A::Stmt::Fallthrough  => self.lower_stmt_fallthrough(span),
            A::Stmt::Defer(b)  => self.lower_stmt_defer(b, span),
            A::Stmt::Guard { name, value, otherwise }  => self.lower_stmt_guard(name, value, otherwise, span),
            A::Stmt::Try { body, catch, .. }  => self.lower_stmt_try(body, catch),
            A::Stmt::UnsafeBlock(b)  => self.lower_stmt_unsafe(b),
            A::Stmt::Throw(v)  => self.lower_stmt_throw(v, span),
        }
    }

    pub(super) fn lower_stmt_var(&mut self, name: &String, ty: &Option<A::Type>, value: &A::Spanned<A::Expr>, span: Span) -> Result<(), Diagnostic> {

        if let Some(t) = ty.as_ref() {
            if let LirType::Enum(ei) = self.resolve_here(t) {
                let implicit = match &value.node {
                    A::Expr::Call { callee, args, .. } => match &callee.node {
                        A::Expr::ImplicitMember(path) => Some((path.clone(), args.clone())),
                        _ => None,
                    },
                    A::Expr::ImplicitMember(path) => Some((path.clone(), Vec::new())),
                    _ => None,
                };
                if let Some((path, args)) = implicit {
                    let vname = path.last().cloned().unwrap_or_default();
                    let vi = self.module.enums[ei].variant_index.get(&vname).copied()
                        .ok_or_else(|| {
                            self.unknown_variant(&vname, value.span)
                        })?;
                    let (v, simple) = self.lower_enum_construct(ei, vi, &args, value.span)?;
                    let t = LirType::Enum(ei);
                    let s = simple_of(&t);
                    let dst = self.local(t.clone());
                    self.emit_copy(dst, v, value.span);
                    self.def(name.clone(), dst, if s == Simple::Other { simple } else { s }, t);
                    return Ok(());
                }
            }
        }
        let (v, simple) = self.lower_expr(&value.node, value.span)?;
        let t = ty.as_ref().map(|t| self.resolve_here(t)).unwrap_or_else(|| {
            // infer from value local type when known
            match self.func.locals[v as usize].clone() {
                // Uninitialized `let x;` (and explicit `let x = null;`)
                // is dynamically typed: an Any slot holding null keeps
                // later stores and null checks sound.
                LirType::Null => LirType::Any,
                other => other,
            }
        });
        let s = simple_of(&t);
        if let Some(elems) = self.tuples.get(&v) {
            let vt = LirType::Tuple(
                elems.iter().map(|e| self.func.locals[*e as usize].clone()).collect(),
            );
            if t != vt {
                return self.fail(self.err(
                    Code::E108,
                    "tuple value does not match the declared tuple type",
                    span,
                ));
            }
        }
        let dst = self.local(t.clone());
        if ty.as_ref().is_some() {
            let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
            let fresh = matches!(value.node, A::Expr::Array(_));
            self.check_array_view(&vt, &t, fresh, value.span, &format!("initializer of `{name}`"))?;
        }
        let v = if ty.as_ref().is_some() {
            let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
            if matches!(vt, LirType::Null)
                && matches!(t, LirType::I64 | LirType::Bool | LirType::F64(_))
                && !ty.as_ref().is_some_and(|at| nub_ty(at))
            {
                let lit = match &t {
                    LirType::Bool => Lit::Bool(false),
                    LirType::F64(k) => Lit::Float(0.0, *k),
                    _ => Lit::Int(0),
                };
                let zv = self.local(t.clone());
                self.emit(Instr::Const { span, dst: zv, lit });
                zv
            } else {
                v
            }
        } else {
            v
        };
        if ty.as_ref().is_some_and(|at| nub_ty(at)) {
            let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
            if self.nub_has(v) {
                self.nub_store(dst, v, span);
            } else if matches!(vt, LirType::I64 | LirType::Bool | LirType::F64(_)) {
                self.nub_box(dst, v, span);
            } else if matches!(vt, LirType::Null) {
                self.emit_copy(dst, v, span);
                self.nub_mark(dst);
            } else {
                let u = if vt == LirType::Any {
                    let tmp = self.local(t.clone());
                    self.emit_any_unbox(tmp, v, span);
                    tmp
                } else {
                    v
                };
                self.emit_copy(dst, u, span);
            }
        } else if self.nub_has(v)
            && matches!(t, LirType::I64 | LirType::Bool | LirType::F64(_))
        {
            self.nub_store(dst, v, span);
        } else {
            self.emit_copy(dst, v, span);
        }
        self.def(name.clone(), dst, if s == Simple::Other { simple } else { s }, t);
        if let Some(arg) = ty.as_ref().and_then(option_arg) {
            let pt = self.resolve_here(arg);
            self.precise.insert(dst, pt);
        } else if let Some(pt) = self.precise.get(&v).cloned() {
            self.precise.insert(dst, pt);
        }
        self.record_construct_targs(name, &value.node);
        Ok(())

    }

    pub(super) fn lower_stmt_destructure_tuple(&mut self, names: &Vec<String>, ty: &Option<A::Type>, value: &A::Spanned<A::Expr>) -> Result<(), Diagnostic> {

        let (v, _) = self.lower_expr(&value.node, value.span)?;
        let elems = self.tuples.get(&v).cloned().ok_or_else(|| {
            self.err(Code::E108, "cannot destructure non-tuple value", value.span)
        })?;
        if elems.len() != names.len() {
            return self.fail(self.err(
                Code::E108,
                format!(
                    "tuple pattern binds {} names but value has {} elements",
                    names.len(),
                    elems.len()
                ),
                value.span,
            ));
        }
        let declared: Option<Vec<LirType>> = ty.as_ref().map(|t| match resolve_ty(self.module, t) {
            LirType::Tuple(items) => items,
            _ => Vec::new(),
        });
        if let Some(t) = ty {
            match resolve_ty(self.module, t) {
                LirType::Tuple(items) if items.len() == names.len() => {}
                _ => {
                    return self.fail(self.err(
                        Code::E108,
                        "tuple annotation must list one type per name",
                        value.span,
                    ));
                }
            }
        }
        let elem_tys = match self.func.locals.get(v as usize).cloned() {
            Some(LirType::Tuple(tys)) => tys,
            _ => vec![LirType::Any; names.len()],
        };
        for (i, (name, src)) in names.iter().zip(elems.iter()).enumerate() {
            let want = declared.as_ref().and_then(|d| d.get(i).cloned()).unwrap_or_else(|| {
                elem_tys.get(i).cloned().unwrap_or(LirType::Any)
            });
            let dst = self.local(want.clone());
            self.emit_copy(dst, *src, value.span);
            self.def(name.clone(), dst, simple_of(&want), want);
        }
        Ok(())

    }

    pub(super) fn lower_stmt_destructure_record(&mut self, fields: &Vec<(String, String)>, rest: &Option<String>, value: &A::Spanned<A::Expr>) -> Result<(), Diagnostic> {

        let (v, _) = self.lower_expr(&value.node, value.span)?;
        if let Some(rec) = self.records.get(&v).cloned() {
            for (field, local) in fields {
                let src = rec
                    .iter()
                    .find(|(n, _)| n == field)
                    .map(|(_, l)| *l)
                    .ok_or_else(|| {
                        self.err(
                            Code::E108,
                            format!("unknown field `{field}`"),
                            value.span,
                        )
                    })?;
                let fty = self.func.locals[src as usize].clone();
                let dst = self.local(fty.clone());
                self.emit_copy(dst, src, value.span);
                self.def(local.clone(), dst, simple_of(&fty), fty);
            }
            if let Some(r) = rest {
                let taken: BTreeSet<&String> =
                    fields.iter().map(|(f, _)| f).collect();
                let mut tys = Vec::new();
                let mut named = Vec::new();
                for (name, src) in &rec {
                    if taken.contains(name) {
                        continue;
                    }
                    tys.push(self.func.locals[*src as usize].clone());
                    named.push((name.clone(), *src));
                }
                let rty = LirType::Tuple(tys);
                let marker = self.local(rty.clone());
                self.records.insert(marker, named);
                self.def(r.clone(), marker, Simple::Other, rty);
            }
            return Ok(());
        }
        self.deny_tuple(v, "as a record", value.span)?;
        let class_name = match self.func.locals.get(v as usize).cloned() {
            Some(LirType::Obj(name)) => name,
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    "cannot destructure non-object value",
                    value.span,
                ));
            }
        };
        let ci = self.module.class_index.get(&class_name).copied().ok_or_else(|| {
            self.err(Code::E108, format!("unknown class `{class_name}`"), value.span)
        })?;
        for (field, local) in fields {
            let fi = match self.module.classes[ci].field_index.get(field) {
                Some(fi) => {
                    self.check_field(ci, field, value.span)?;
                    *fi
                }
                None => {
                    return self.fail(self.err(
                        Code::E108,
                        format!("unknown field `{field}` on `{class_name}`"),
                        value.span,
                    ));
                }
            };
            let fty = self.module.classes[ci].fields[fi].ty.clone();
            let dst = self.local(fty.clone());
            self.emit(Instr::GetField { span: value.span, dst, obj: v, field: fi });
            self.def(local.clone(), dst, simple_of(&fty), fty);
        }
        if let Some(r) = rest {
            let taken: BTreeSet<&String> =
                fields.iter().map(|(f, _)| f).collect();
            let remaining: Vec<(usize, String, LirType)> = self.module.classes[ci]
                .fields
                .iter()
                .enumerate()
                .filter(|(_, fld)| !taken.contains(&fld.name))
                .map(|(fi, fld)| (fi, fld.name.clone(), fld.ty.clone()))
                .collect();
            let mut tys = Vec::with_capacity(remaining.len());
            let mut named = Vec::with_capacity(remaining.len());
            for (fi, name, fty) in remaining {
                self.check_field(ci, &name, value.span)?;
                let dst = self.local(fty.clone());
                self.emit(Instr::GetField { span: value.span, dst, obj: v, field: fi });
                tys.push(fty);
                named.push((name, dst));
            }
            let rty = LirType::Tuple(tys);
            let marker = self.local(rty.clone());
            self.records.insert(marker, named);
            self.def(r.clone(), marker, Simple::Other, rty);
        }
        Ok(())

    }

    pub(super) fn lower_stmt_destructure_array(&mut self, names: &Vec<String>, rest: &Option<String>, value: &A::Spanned<A::Expr>) -> Result<(), Diagnostic> {

        let (v, _) = self.lower_expr(&value.node, value.span)?;
        self.deny_tuple(v, "as an array", value.span)?;
        let elem_ty = match self.func.locals.get(v as usize).cloned() {
            Some(LirType::Array(inner)) => (*inner).clone(),
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    "cannot destructure non-array value",
                    value.span,
                ));
            }
        };
        let es = self.elem_size_of(v);
        let len = self.local(LirType::I64);
        self.emit(Instr::ArrayLen { span: value.span, dst: len, arr: v });
        for (i, name) in names.iter().enumerate() {
            let idx = self.const_int(i as i64, value.span);
            let inb = self.local(LirType::Bool);
            self.emit(Instr::Cmp { span: value.span, op: CmpOp::Lt, kind: NumKind::Int, dst: inb, lhs: idx, rhs: len });
            let get_bb = self.new_block();
            let else_bb = self.new_block();
            let merge = self.new_block();
            self.set_term(Terminator::BrIf { span: value.span, cond: inb, then_bb: get_bb, else_bb });
            let dst = self.local(elem_ty.clone());
            self.set_current(get_bb);
            self.emit(Instr::ArrayGet { span: value.span, dst, arr: v, index: idx, elem_size: es, unchecked: false });
            self.set_term(Terminator::Br(merge));
            self.set_current(else_bb);
            let null_lit = match &elem_ty {
                LirType::I64 => Lit::Int(0),
                LirType::Bool => Lit::Bool(false),
                LirType::F64(k) => Lit::Float(0.0, *k),
                LirType::Str => Lit::Str(String::new()),
                _ => Lit::Null,
            };
            self.emit(Instr::Const { span: value.span, dst, lit: null_lit });
            self.set_term(Terminator::Br(merge));
            self.set_current(merge);
            self.def(name.clone(), dst, simple_of(&elem_ty), elem_ty.clone());
        }
        if let Some(r) = rest {
            let rdst = self.local(LirType::Array(Box::new(elem_ty.clone())));
            let res = elem_size(&elem_ty);
            self.emit(Instr::ArrayNew { span: value.span, dst: rdst, cap: 0, elem_size: res });
            let from = self.const_int(names.len() as i64, value.span);
            self.emit_array_extend(rdst, v, &elem_ty, from, value.span)?;
            self.def(r.clone(), rdst, Simple::Other, LirType::Array(Box::new(elem_ty)));
        }
        Ok(())

    }

    pub(super) fn lower_stmt_assign(&mut self, target: &A::Spanned<A::Expr>, op: &A::AssignOp, value: &A::Spanned<A::Expr>, span: Span) -> Result<(), Diagnostic> {

        let (v, _) = self.lower_expr(&value.node, value.span)?;
        match &target.node {
            A::Expr::Ident(n) => {
                let Some((dst, _, _)) = self.lookup(n) else {
                    if self.module.fn_index.contains_key(n) {
                        return self.fail(self.err(
                            Code::E108,
                            format!("cannot assign to function `{n}`"),
                            target.span,
                        ));
                    }
                    return self.fail(self.err(Code::E108, format!("unknown `{n}`"), target.span));
                };
                if *op == A::AssignOp::Eq {
                    let dt = self.func.locals.get(dst as usize).cloned().unwrap_or(LirType::Any);
                    let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                    let fresh = matches!(value.node, A::Expr::Array(_));
                    self.check_array_view(&vt, &dt, fresh, value.span, &format!("value assigned to `{n}`"))?;
                    if self.nub_has(dst) {
                        let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                        if self.nub_has(v) {
                            if v != dst {
                                self.nub_release(dst, span);
                                self.nub_store(dst, v, span);
                            }
                        } else if matches!(vt, LirType::I64 | LirType::Bool | LirType::F64(_)) {
                            let tmp = self.local(vt);
                            self.nub_box(tmp, v, span);
                            self.nub_release(dst, span);
                            self.emit_copy(dst, tmp, span);
                        } else if matches!(vt, LirType::Null) {
                            self.nub_release(dst, span);
                            self.emit_copy(dst, v, span);
                        } else {
                            let u = if vt == LirType::Any {
                                let tmp = self.local(LirType::I64);
                                self.emit_any_unbox(tmp, v, span);
                                tmp
                            } else {
                                v
                            };
                            self.nub_release(dst, span);
                            self.emit_copy(dst, u, span);
                            self.nub_unmark(dst);
                        }
                    } else {
                        let v = self.nub_use(v, span);
                        self.emit_copy(dst, v, span);
                    }
                    self.record_construct_targs(n, &value.node);
                } else {
                    let arith = match op {
                        A::AssignOp::PlusEq => ArithOp::Add,
                        A::AssignOp::MinusEq => ArithOp::Sub,
                        A::AssignOp::StarEq => ArithOp::Mul,
                        A::AssignOp::SlashEq => ArithOp::Div,
                        A::AssignOp::PercentEq => ArithOp::Mod,
                        A::AssignOp::ShlEq => ArithOp::Shl,
                        A::AssignOp::ShrEq => ArithOp::Shr,
                        A::AssignOp::ZshrEq => ArithOp::Zshr,
                        _ => {
                            return self.ice(
                                "unhandled compound assignment operator",
                                Some(span),
                            );
                        }
                    };
                    let v = self.unbox_operand(v, span);
                    let dt = self.func.locals.get(dst as usize).cloned().unwrap_or(LirType::Any);
                    let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                    if dt == LirType::Any {
                        let ldu = self.unbox_operand(dst, span);
                        let tmp = self.local(LirType::I64);
                        self.emit(Instr::Arith { span,
                            op: arith,
                            kind: NumKind::Int,
                            dst: tmp,
                            lhs: ldu,
                            rhs: v,
                        });
                        self.emit_copy(dst, tmp, span);
                    } else {
                        if matches!(dt, LirType::Pointer(_)) && self.unsafe_depth == 0 {
                            return self.fail(self.err(Code::E202, "pointer arithmetic needs `unsafe`", span));
                        }
                        let kind = self.compound_kind(&dt, &vt, span)?;
                        if matches!(dt, LirType::Pointer(_)) && !matches!(kind, NumKind::Int) {
                            return self.fail(self.err(Code::E108, "pointer arithmetic supports `+` and `-` only", span));
                        }
                        self.emit(Instr::Arith { span,
                            op: arith,
                            kind,
                            dst,
                            lhs: dst,
                            rhs: v,
                        });
                    }
                }
                Ok(())
            }
            A::Expr::Member { base, field } => {
                let (obj, _) = self.lower_expr(&base.node, base.span)?;
                let oty = self.func.locals[obj as usize].clone();
                if super::ns_key_of_ty(&oty).is_some() {
                    return self.fail(self.err(
                        Code::E108,
                        "cannot assign to a module namespace field",
                        span,
                    ));
                }
                let mut fty: Option<LirType> = None;
                if let LirType::Obj(name) = &oty {
                    if let Some(ci) = self.module.class_index.get(name.as_str()).copied() {
                        if self.module.classes[ci].field_index.contains_key(field) {
                            self.check_field(ci, field, span)?;
                            if let Some(fi) = self.module.classes[ci].field_index.get(field) {
                                fty = Some(self.module.classes[ci].fields[*fi].ty.clone());
                            }
                        }
                    }
                }
                if *op == A::AssignOp::Eq {
                    let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                    if let Some(ft) = fty.clone() {
                        let fresh = matches!(value.node, A::Expr::Array(_));
                        self.check_array_view(&vt, &ft, fresh, value.span, &format!("value assigned to field `{field}`"))?;
                    }
                    let fkey = match &oty {
                        LirType::Obj(name) => Some(format!("{name}.{field}")),
                        _ => None,
                    };
                    if fkey.as_ref().is_some_and(|k| self.nub_fields.contains(k)) {
                        let old = self.local(LirType::Any);
                        self.emit(Instr::GetFieldByName { span,
                            dst: old,
                            obj,
                            field: field.clone(),
                        });
                        self.nub_release(old, span);
                        let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                        let v = if self.nub_has(v) {
                            if self.nub_scope_has(v) {
                                self.nub_retain(v, span);
                                v
                            } else {
                                self.nub_unmark(v);
                                v
                            }
                        } else if matches!(vt, LirType::I64 | LirType::Bool | LirType::F64(_)) {
                            let d = self.local(vt);
                            self.nub_box(d, v, span);
                            d
                        } else if vt == LirType::Any {
                            let d = self.local(LirType::I64);
                            self.emit_any_unbox(d, v, span);
                            d
                        } else {
                            v
                        };
                        self.emit(Instr::SetFieldByName { span,
                            obj,
                            field: field.clone(),
                            value: v,
                        });
                    } else {
                        let v = if self.nub_has(v) && fty == Some(LirType::Any) {
                            if self.nub_scope_has(v) {
                                self.nub_retain(v, span);
                            } else {
                                self.nub_unmark(v);
                            }
                            v
                        } else {
                            let v = self.nub_use(v, span);
                            match fty {
                                Some(t) => self.coerce_to_slot(v, &t, span),
                                None => v,
                            }
                        };
                        self.emit(Instr::SetFieldByName { span,
                            obj,
                            field: field.clone(),
                            value: v,
                        });
                    }
                } else {
                    let arith = match op {
                        A::AssignOp::PlusEq => ArithOp::Add,
                        A::AssignOp::MinusEq => ArithOp::Sub,
                        A::AssignOp::StarEq => ArithOp::Mul,
                        A::AssignOp::SlashEq => ArithOp::Div,
                        A::AssignOp::PercentEq => ArithOp::Mod,
                        A::AssignOp::ShlEq => ArithOp::Shl,
                        A::AssignOp::ShrEq => ArithOp::Shr,
                        A::AssignOp::ZshrEq => ArithOp::Zshr,
                        A::AssignOp::Eq => {
                            return self.fail(self.err(
                                Code::E108,
                                "bad assign",
                                span,
                            ));
                        }
                    };
                    let cur = self.local(LirType::Any);
                    self.emit(Instr::GetFieldByName { span, 
                        dst: cur,
                        obj,
                        field: field.clone(),
                    });
                    let fkey = match &oty {
                        LirType::Obj(name) => Some(format!("{name}.{field}")),
                        _ => None,
                    };
                    if fkey.as_ref().is_some_and(|k| self.nub_fields.contains(k)) {
                        let fts = fty.clone().unwrap_or(LirType::I64);
                        let cur2 = self.local(fts.clone());
                        self.emit_any_unbox(cur2, cur, span);
                        let v = self.nub_use(v, span);
                        let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                        let kind = self.compound_kind(&fts, &vt, span)?;
                        let nv = self.local(fts.clone());
                        self.emit(Instr::Arith { span,
                            op: arith,
                            kind,
                            dst: nv,
                            lhs: cur2,
                            rhs: v,
                        });
                        let tag = Self::any_box_tag(&fts).unwrap_or(0);
                        let nb = self.local(fts);
                        self.emit_any_box(nb, tag, nv, span);
                        self.nub_release(cur, span);
                        self.emit(Instr::SetFieldByName { span,
                            obj,
                            field: field.clone(),
                            value: nb,
                        });
                        return Ok(());
                    }
                    if let Some(ft) = fty.clone() {
                        if matches!(ft, LirType::I64 | LirType::I8 | LirType::F64(_)) {
                            let cur2 = self.local(ft.clone());
                            self.emit_any_unbox(cur2, cur, span);
                            let v = self.unbox_operand(v, span);
                            let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                            let kind = self.compound_kind(&ft, &vt, span)?;
                            let nv = self.local(ft);
                            self.emit(Instr::Arith { span,
                                op: arith,
                                kind,
                                dst: nv,
                                lhs: cur2,
                                rhs: v,
                            });
                            self.emit(Instr::SetFieldByName { span,
                                obj,
                                field: field.clone(),
                                value: nv,
                            });
                            return Ok(());
                        }
                    }
                    let nv = self.local(LirType::Any);
                    self.emit(Instr::Arith { span, 
                        op: arith,
                        kind: NumKind::Int,
                        dst: nv,
                        lhs: cur,
                        rhs: v,
                    });
                    self.emit(Instr::SetFieldByName { span, 
                        obj,
                        field: field.clone(),
                        value: nv,
                    });
                }
                Ok(())
            }
            A::Expr::Unary { op: uop, rhs } if matches!(uop, A::UnOp::Deref) => {
                if *op != A::AssignOp::Eq {
                    return self.fail(self.err(
                        Code::E108,
                        "compound assignment to a pointer dereference is not supported",
                        span,
                    ));
                }
                if self.unsafe_depth == 0 {
                    return self.fail(self.err(
                        Code::E202,
                        "pointer dereference requires unsafe block",
                        span,
                    ));
                }
                let (p, _) = self.lower_expr(&rhs.node, rhs.span)?;
                let pty = self.func.locals.get(p as usize).cloned().unwrap_or(LirType::Any);
                let scalar = self.pointer_scalar(&pty, target.span)?;
                self.deny_tuple(v, "in a pointer store", value.span)?;
                let v = self.coerce_to_slot(v, &scalar, value.span);
                self.emit(Instr::PtrStore { span, ptr: p, val: v, volatile: false });
                Ok(())
            }
            A::Expr::Index { base, index } => {
                let (arr, _) = self.lower_expr(&base.node, base.span)?;
                let (ix, _) = self.lower_expr(&index.node, index.span)?;
                if *op == A::AssignOp::Eq {
                    let arr_ty = self.func.locals.get(arr as usize).cloned().unwrap_or(LirType::Any);
                    if matches!(arr_ty, LirType::Obj(_)) {
                        self.deny_tuple(v, "in an index store", value.span)?;
                        self.lower_op_hook(
                            arr,
                            "op_index_set",
                            "[]=",
                            vec![(**index).clone(), (*value).clone()],
                            vec![ix, v],
                            span,
                        )?;
                        return Ok(());
                    }
                    let es = self.elem_size_of(arr);
                    let slot = match self.func.locals.get(arr as usize) {
                        Some(LirType::Array(inner)) => (**inner).clone(),
                        _ => LirType::Any,
                    };
                    self.deny_tuple(v, "in an array store", value.span)?;
                    let v = self.coerce_to_slot(v, &slot, value.span);
                    let needs_drop = matches!(
                        self.func.locals.get(arr as usize),
                        Some(LirType::Array(inner))
                            if matches!(**inner, LirType::Obj(_) | LirType::Str | LirType::Array(_))
                    );
                    let old = if needs_drop {
                        let inner = match self.func.locals.get(arr as usize) {
                            Some(LirType::Array(inner)) => (**inner).clone(),
                            _ => LirType::Any,
                        };
                        let tmp = self.local(inner);
                        self.emit(Instr::ArrayGet { span,  dst: tmp, arr, index: ix, elem_size: es, unchecked: false });
                        Some(tmp)
                    } else {
                        None
                    };
                    self.emit(Instr::ArraySet { span, 
                        arr,
                        index: ix,
                        value: v,
                        elem_size: es,
                        unchecked: false,
                    });
                    if let Some(tmp) = old {
                        self.emit(Instr::Release { span,  obj: tmp });
                    }
                } else {
                    let arith = match op {
                        A::AssignOp::PlusEq => ArithOp::Add,
                        A::AssignOp::MinusEq => ArithOp::Sub,
                        A::AssignOp::StarEq => ArithOp::Mul,
                        A::AssignOp::SlashEq => ArithOp::Div,
                        A::AssignOp::PercentEq => ArithOp::Mod,
                        A::AssignOp::ShlEq => ArithOp::Shl,
                        A::AssignOp::ShrEq => ArithOp::Shr,
                        A::AssignOp::ZshrEq => ArithOp::Zshr,
                        A::AssignOp::Eq => {
                            return self.fail(self.err(Code::E108, "bad assign", span));
                        }
                    };
                    let arr_ty = self.func.locals.get(arr as usize).cloned().unwrap_or(LirType::Any);
                    if matches!(arr_ty, LirType::Obj(_)) {
                        return self.fail(self.err(Code::E108, "compound assignment on an overloaded `[]` target is not supported", span));
                    }
                    let es = self.elem_size_of(arr);
                    let slot = match self.func.locals.get(arr as usize) {
                        Some(LirType::Array(inner)) => (**inner).clone(),
                        _ => LirType::Any,
                    };
                    let cur = self.local(slot.clone());
                    self.emit(Instr::ArrayGet { span, dst: cur, arr, index: ix, elem_size: es, unchecked: false });
                    let curu = self.unbox_operand(cur, span);
                    let v = self.unbox_operand(v, span);
                    let lt = self.func.locals.get(curu as usize).cloned().unwrap_or(LirType::Any);
                    let rt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                    let kind = self.compound_kind(&lt, &rt, span)?;
                    let nv = self.local(lt);
                    self.emit(Instr::Arith { span, op: arith, kind, dst: nv, lhs: curu, rhs: v });
                    let nv = self.coerce_to_slot(nv, &slot, span);
                    self.emit(Instr::ArraySet { span, arr, index: ix, value: nv, elem_size: es, unchecked: false });
                }
                Ok(())
            }
            _ => self.fail(self.err(Code::E108, "bad assign target", target.span)),
        }

    }

    pub(super) fn lower_stmt_expr(&mut self, e: &A::Spanned<A::Expr>) -> Result<(), Diagnostic> {

        self.lower_expr(&e.node, e.span)?;
        Ok(())

    }

    pub(super) fn lower_stmt_assert(&mut self, e: &A::Spanned<A::Expr>, span: Span) -> Result<(), Diagnostic> {

        let (c, _) = self.lower_expr(&e.node, e.span)?;
        self.deny_tuple(c, "as an assertion", e.span)?;
        let msg = self.local(LirType::Str);
        self.emit(Instr::Const { span, 
            dst: msg,
            lit: Lit::Str("assertion failed".to_string()),
        });
        self.emit(Instr::Assert { span,  cond: c, message: msg });
        Ok(())

    }

    pub(super) fn lower_stmt_if(&mut self, cond: &A::IfCond, then: &A::Block, otherwise: &Option<A::Else>, span: Span) -> Result<(), Diagnostic> {

        let merge = self.new_block();
        match cond {
            A::IfCond::Expr(e) => {
                let (c, _) = self.lower_expr(&e.node, e.span)?;
                self.deny_tuple(c, "as a condition", e.span)?;
                let c = self.nub_cond(c, e.span);
                let then_bb = self.new_block();
                let else_bb = self.new_block();
                self.set_term(Terminator::BrIf { span, 
                    cond: c,
                    then_bb,
                    else_bb,
                });
                self.set_current(then_bb);
                let narrowed = matches!(&e.node, A::Expr::Is { .. });
                if narrowed {
                    self.push_scope();
                }
                if let A::Expr::Is { base, target } = &e.node {
                    self.narrow_for_is(base, target, e.span)?;
                }
                self.lower_block(then)?;
                if narrowed {
                    self.pop_scope();
                }
                if !self.terminated {
                    self.set_term(Terminator::Br(merge));
                }
                self.set_current(else_bb);
                match otherwise {
                    None => self.set_term(Terminator::Br(merge)),
                    Some(A::Else::Block(b)) => {
                        self.lower_block(b)?;
                        if !self.terminated {
                            self.set_term(Terminator::Br(merge));
                        }
                    }
                    Some(A::Else::If(st)) => {
                        self.lower_stmt(&st.node, st.span)?;
                        if !self.terminated {
                            self.set_term(Terminator::Br(merge));
                        }
                    }
                }
                self.set_current(merge);
                Ok(())
            }
            A::IfCond::Let { name, value } => {
                let (v, _) = self.lower_expr(&value.node, value.span)?;
                let is_some = self.local(LirType::Bool);
                let null = self.local(LirType::Null);
                self.emit(Instr::Const { span,  dst: null, lit: Lit::Null });
                self.emit(Instr::Cmp { span, 
                    op: CmpOp::NotEq,
                    kind: NumKind::Int,
                    dst: is_some,
                    lhs: v,
                    rhs: null,
                });
                let then_bb = self.new_block();
                let else_bb = self.new_block();
                self.set_term(Terminator::BrIf { span, 
                    cond: is_some,
                    then_bb,
                    else_bb,
                });
                self.set_current(then_bb);
                self.push_scope();
                let t = self.func.locals[v as usize].clone();
                self.def(name.clone(), v, simple_of(&t), t);
                self.lower_block(then)?;
                self.pop_scope();
                if !self.terminated {
                    self.set_term(Terminator::Br(merge));
                }
                self.set_current(else_bb);
                match otherwise {
                    None => self.set_term(Terminator::Br(merge)),
                    Some(A::Else::Block(b)) => {
                        self.lower_block(b)?;
                        if !self.terminated {
                            self.set_term(Terminator::Br(merge));
                        }
                    }
                    Some(A::Else::If(st)) => {
                        self.lower_stmt(&st.node, st.span)?;
                        if !self.terminated {
                            self.set_term(Terminator::Br(merge));
                        }
                    }
                }
                self.set_current(merge);
                Ok(())
            }
        }

    }

    pub(super) fn lower_stmt_while(&mut self, cond: &A::Spanned<A::Expr>, body: &A::Block, span: Span) -> Result<(), Diagnostic> {

        let header = self.new_block();
        let lbody = self.new_block();
        let merge = self.new_block();
        self.set_term(Terminator::Br(header));
        self.set_current(header);
        let (c, _) = self.lower_expr(&cond.node, cond.span)?;
        let c = self.nub_cond(c, cond.span);
        self.set_term(Terminator::BrIf { span, 
            cond: c,
            then_bb: lbody,
            else_bb: merge,
        });
        let depth = self.defer_count;
        self.loop_stack.push(LoopCtx {
            break_bb: merge,
            cont_bb: header,
            defer_depth: depth,
        });
        self.set_current(lbody);
        self.lower_block(body)?;
        if !self.terminated {
            self.set_term(Terminator::Br(header));
        }
        self.loop_stack.pop();
        self.set_current(merge);
        Ok(())

    }

    pub(super) fn lower_stmt_do_while(&mut self, body: &A::Block, cond: &A::Spanned<A::Expr>, span: Span) -> Result<(), Diagnostic> {

        let lbody = self.new_block();
        let header = self.new_block();
        let merge = self.new_block();
        self.set_term(Terminator::Br(lbody));
        let depth = self.defer_count;
        self.loop_stack.push(LoopCtx {
            break_bb: merge,
            cont_bb: header,
            defer_depth: depth,
        });
        self.set_current(lbody);
        self.lower_block(body)?;
        if !self.terminated {
            self.set_term(Terminator::Br(header));
        }
        self.set_current(header);
        let (c, _) = self.lower_expr(&cond.node, cond.span)?;
        let c = self.nub_cond(c, cond.span);
        self.set_term(Terminator::BrIf { span, 
            cond: c,
            then_bb: lbody,
            else_bb: merge,
        });
        self.loop_stack.pop();
        self.set_current(merge);
        Ok(())

    }

    pub(super) fn lower_stmt_return(&mut self, v: &Option<A::Spanned<A::Expr>>, span: Span) -> Result<(), Diagnostic> {

        let vals = match v {
            Some(e) => {
                let (r, _) = self.lower_expr(&e.node, e.span)?;
                let rt = self.func.ret.clone();
                let rvt = self.func.locals.get(r as usize).cloned().unwrap_or(LirType::Any);
                let fresh = matches!(e.node, A::Expr::Array(_));
                self.check_array_view(&rvt, &rt, fresh, e.span, "return value")?;
                self.lower_return_value(r, e.span)?
            }
            None => Vec::new(),
        };
        self.emit_run_defers(0);
        let mut ifaces: Vec<(Local, usize)> = Vec::new();
        let mut seen: BTreeSet<Local> = BTreeSet::new();
        let mut nub_dead: Vec<Local> = Vec::new();
        for scope in self.scopes.iter() {
            for (_, (l, _, t)) in scope.iter() {
                if vals.contains(l) || self.moved.contains(l) || !seen.insert(*l) {
                    continue;
                }
                if self.nub.contains(l) {
                    nub_dead.push(*l);
                }
                if (*l as usize) < self.func.params.len() {
                    continue;
                }
                if let LirType::Obj(name) = t {
                    if self.module.class_index.get(name).is_none() {
                        if let Some(ii) = self.module.interface_index.get(name).copied() {
                            ifaces.push((*l, ii));
                        }
                    }
                }
            }
        }
        for (l, ii) in ifaces {
            self.emit_iface_release(l, ii, span);
            self.moved.insert(l);
        }
        for l in nub_dead {
            self.nub_release(l, span);
        }
        if self.is_init {
            let slf = self.self_local.ok_or_else(|| {
                self.err(Code::E108, "init without self", span)
            })?;
            self.set_term(Terminator::Ret(vec![slf]));
        } else {
            self.set_term(Terminator::Ret(vals));
        }
        Ok(())

    }

    pub(super) fn lower_stmt_break(&mut self, span: Span) -> Result<(), Diagnostic> {

        let (bb, depth) = match self.loop_stack.last() {
            Some(c) => (c.break_bb, c.defer_depth),
            None => return self.fail(self.err(Code::E108, "`break` outside loop", span)),
        };
        self.emit_run_defers(depth);
        self.set_term(Terminator::Br(bb));
        Ok(())

    }

    pub(super) fn lower_stmt_continue(&mut self, span: Span) -> Result<(), Diagnostic> {

        let (bb, depth) = match self.loop_stack.last() {
            Some(c) => (c.cont_bb, c.defer_depth),
            None => {
                return self.fail(self.err(Code::E108, "`continue` outside loop", span));
            }
        };
        self.emit_run_defers(depth);
        self.set_term(Terminator::Br(bb));
        Ok(())

    }

    pub(super) fn lower_stmt_pass() -> Result<(), Diagnostic> {
        Ok(())
    }

    pub(super) fn lower_stmt_empty() -> Result<(), Diagnostic> {
        Ok(())
    }

    pub(super) fn lower_stmt_fallthrough(&mut self, span: Span) -> Result<(), Diagnostic> {

        let (targets, pos) = self.fallthrough.as_ref().ok_or_else(|| {
            self.err(Code::E108, "`fallthrough` outside `switch`", span)
        })?;
        let next = *targets.get(pos + 1).ok_or_else(|| {
            self.err(Code::E108, "`fallthrough` on last case", span)
        })?;
        self.set_term(Terminator::Br(next));
        Ok(())

    }

    pub(super) fn lower_stmt_defer(&mut self, b: &A::Block, span: Span) -> Result<(), Diagnostic> {

        let save_cur = self.current;
        let save_term = self.terminated;
        let tmp = self.new_block();
        self.set_current(tmp);
        self.terminated = false;
        self.push_scope();
        for st in &b.stmts {
            self.lower_stmt(&st.node, st.span)?;
        }
        self.pop_scope();
        if self.terminated {
            return self.fail(self.err(
                Code::E108,
                "return/throw/branch inside `defer` pending",
                span,
            ));
        }
        let body = std::mem::take(&mut self.blocks[tmp]);
        self.blocks.pop();
        self.terms.pop();
        self.set_current(save_cur);
        self.terminated = save_term;
        self.emit(Instr::Defer { span,  body });
        self.defer_count += 1;
        Ok(())

    }

    pub(super) fn lower_stmt_guard(&mut self, name: &String, value: &A::Spanned<A::Expr>, otherwise: &A::Block, span: Span) -> Result<(), Diagnostic> {

        let (v, _) = self.lower_expr(&value.node, value.span)?;
        let is_some = self.local(LirType::Bool);
        let null = self.local(LirType::Null);
        self.emit(Instr::Const { span,  dst: null, lit: Lit::Null });
        self.emit(Instr::Cmp { span, 
            op: CmpOp::NotEq,
            kind: NumKind::Int,
            dst: is_some,
            lhs: v,
            rhs: null,
        });
        let cont = self.new_block();
        let else_bb = self.new_block();
        self.set_term(Terminator::BrIf { span, 
            cond: is_some,
            then_bb: cont,
            else_bb,
        });
        self.set_current(else_bb);
        self.lower_block(otherwise)?;
        if !self.terminated {
            return self.fail(self.err(
                Code::E108,
                "`guard else` must diverge",
                otherwise.stmts.last().map(|s| s.span).unwrap_or(span),
            ));
        }
        self.set_current(cont);
        let t = self.func.locals[v as usize].clone();
        self.def(name.clone(), v, simple_of(&t), t);
        Ok(())

    }

    pub(super) fn lower_stmt_try(&mut self, body: &A::Block, catch: &Option<(String, A::Block)>) -> Result<(), Diagnostic> {

        let merge = self.new_block();
        match catch {
            None => {
                self.lower_block(body)?;
                if !self.terminated {
                    self.set_term(Terminator::Br(merge));
                }
                self.set_current(merge);
                Ok(())
            }
            Some((id, cbody)) => {
                let catch_bb = self.new_block();
                let err_local = self.local(LirType::Error);
                let depth = self.defer_count;
                self.catch_stack.push(CatchCtx {
                    catch_bb: Some(catch_bb),
                    err_local,
                    defer_depth: depth,
                });
                self.lower_block(body)?;
                if !self.terminated {
                    self.set_term(Terminator::Br(merge));
                }
                self.catch_stack.pop();
                self.set_current(catch_bb);
                self.push_scope();
                self.def(id.clone(), err_local, Simple::Other, LirType::Error);
                self.catch_stack.push(CatchCtx {
                    catch_bb: None,
                    err_local,
                    defer_depth: depth,
                });
                self.lower_block(cbody)?;
                self.catch_stack.pop();
                self.pop_scope();
                if !self.terminated {
                    self.set_term(Terminator::Br(merge));
                }
                self.set_current(merge);
                Ok(())
            }
        }

    }

    pub(super) fn lower_stmt_unsafe(&mut self, b: &A::Block) -> Result<(), Diagnostic> {

        self.unsafe_depth += 1;
        let r = self.lower_block(b);
        self.unsafe_depth -= 1;
        r

    }

    pub(super) fn lower_stmt_throw(&mut self, v: &Option<A::Spanned<A::Expr>>, span: Span) -> Result<(), Diagnostic> {

        match v {
            Some(e) => {
                let (err, _) = self.lower_expr(&e.node, e.span)?;
                let catch = self
                    .catch_stack
                    .iter()
                    .rev()
                    .find_map(|c| c.catch_bb.map(|bb| (bb, c.err_local, c.defer_depth)));
                let depth = self
                    .catch_stack
                    .last()
                    .map(|c| c.defer_depth)
                    .unwrap_or(0);
                self.emit_run_defers(depth);
                self.set_term(Terminator::Throw { span,  src: err, catch });
                Ok(())
            }
            None => {
                let top = self.catch_stack.last();
                match top {
                    Some(c) if c.catch_bb.is_none() => {
                        let (err, depth) = (c.err_local, c.defer_depth);
                        let catch = self.catch_stack[..self.catch_stack.len() - 1]
                            .iter()
                            .rev()
                            .find_map(|o| {
                                o.catch_bb.map(|bb| (bb, o.err_local, o.defer_depth))
                            });
                        let _ = c;
                        self.emit_run_defers(depth);
                        self.set_term(Terminator::Throw { span,  src: err, catch });
                        Ok(())
                    }
                    _ => self.fail(self.err(
                        Code::E108,
                        "bare `throw` only inside `catch`",
                        span,
                    )),
                }
            }
        }

    }


    pub(super) fn lower_for_iter(
        &mut self,
        bound: &str,
        it: Local,
        it_ty: &LirType,
        body: &A::Block,
        _span: Span,
        iter_span: Span,
    ) -> Result<(), Diagnostic> {
        let obj_name = match it_ty {
            LirType::Obj(n) => n.clone(),
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    "`for` needs a range, array, or iterable",
                    iter_span,
                ));
            }
        };
        let has_iterator = self
            .module
            .class_index
            .get(&obj_name)
            .is_some_and(|ci| self.module.classes[*ci].methods.contains_key("iterator"))
            || self.find_extension(it_ty, "iterator").is_some();
        let has_next = self
            .module
            .class_index
            .get(&obj_name)
            .is_some_and(|ci| self.module.classes[*ci].methods.contains_key("next"))
            || self.find_extension(it_ty, "next").is_some();
        if !has_iterator && !has_next {
            return self.fail(self.err(
                Code::E108,
                format!(
                    "`for` target of type `{obj_name}` must implement `Iterable` (missing `.iterator()`)",
                ),
                iter_span,
            ));
        }
        let seq = self.synth_seq;
        self.synth_seq += 1;
        let iter_var = format!("$for_iter{seq}");
        let next_var = format!("$for_next{seq}");
        let zspan = crate::instr::UNKNOWN_SPAN;
        let ident = |n: &str| A::Spanned { node: A::Expr::Ident(n.to_string()), span: zspan };
        let method_call = |base: A::Expr, field: &str| A::Expr::Call {
            callee: Box::new(A::Spanned {
                node: A::Expr::Member {
                    base: Box::new(A::Spanned { node: base, span: zspan }),
                    field: field.to_string(),
                },
                span: zspan,
            }),
            type_args: Vec::new(),
            args: Vec::new(),
            trailing: None,
        };
        let it_simple = simple_of(it_ty);
        self.def(iter_var.clone(), it, it_simple, it_ty.clone());
        let (iter_l, _) = if has_iterator {
            self.lower_expr(&method_call(A::Expr::Ident(iter_var.clone()), "iterator"), zspan)?
        } else {
            (it, Simple::Other)
        };
        let iter_lty = self.func.locals[iter_l as usize].clone();
        if !matches!(iter_lty, LirType::Obj(_)) {
            return self.fail(self.err(
                Code::E108,
                "`for` target `.iterator()` must return an object",
                iter_span,
            ));
        }
        self.def(iter_var.clone(), iter_l, simple_of(&iter_lty), iter_lty);
        let merge = self.new_block();
        let header = self.new_block();
        self.set_term(Terminator::Br(header));
        self.set_current(header);
        let depth = self.defer_count;
        self.loop_stack.push(LoopCtx {
            break_bb: merge,
            cont_bb: header,
            defer_depth: depth,
        });
        let (nx, _) = self.lower_expr(&method_call(A::Expr::Ident(iter_var.clone()), "next"), zspan)?;
        let nx_ty2 = self.func.locals[nx as usize].clone();
        self.def(next_var.clone(), nx, simple_of(&nx_ty2), nx_ty2);
        let null = self.local(LirType::Null);
        self.emit(Instr::Const { span: zspan, dst: null, lit: Lit::Null });
        let is_done = self.local(LirType::Bool);
        self.emit(Instr::Cmp {
            span: zspan,
            op: CmpOp::Eq,
            kind: NumKind::Int,
            dst: is_done,
            lhs: nx,
            rhs: null,
        });
        let body_bb = self.new_block();
        self.set_term(Terminator::BrIf { span: zspan, cond: is_done, then_bb: merge, else_bb: body_bb });
        self.set_current(body_bb);
        let bound_ty = self.func.locals[nx as usize].clone();
        self.def(bound.to_string(), nx, simple_of(&bound_ty), bound_ty);
        for st in &body.stmts {
            self.lower_stmt(&st.node, st.span)?;
            if self.terminated {
                break;
            }
        }
        if !self.terminated {
            self.set_term(Terminator::Br(header));
        }
        self.loop_stack.pop();
        self.set_current(merge);
        Ok(())
    }

    pub(super) fn lower_for(
        &mut self,
        binding: &A::ForBinding,
        iter: &A::Spanned<A::Expr>,
        body: &A::Block,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let (it, _) = self.lower_expr(&iter.node, iter.span)?;
        let it_ty = self.func.locals[it as usize].clone();
        let bound_names: Vec<String> = match binding {
            A::ForBinding::One(n) => vec![n.clone()],
            A::ForBinding::Many(ns) => {
                if ns.is_empty() {
                    return self.fail(self.err(Code::E108, "`for ()` binds nothing", span));
                }
                ns.clone()
            }
        };
        let is_tuple = bound_names.len() > 1;
        match it_ty {
            LirType::Range | LirType::Array(_) | LirType::Any => {}
            LirType::Obj(_) => {
                if is_tuple {
                    return self.fail(self.err(
                        Code::E108,
                        "`for (a, b)` over an iterable binds a single item",
                        span,
                    ));
                }
                return self.lower_for_iter(&bound_names[0], it, &it_ty, body, span, iter.span);
            }
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    "`for` needs a range, array, or iterable",
                    iter.span,
                ));
            }
        }
        if is_tuple && !matches!(it_ty, LirType::Array(_) | LirType::Any) {
            return self.fail(self.err(
                Code::E108,
                "`for (a, b)` needs an array of pairs or records",
                iter.span,
            ));
        }
        let is_range = !matches!(it_ty, LirType::Array(_));
        let lo = self.local(LirType::I64);
        let hi = self.local(LirType::I64);
        let step = self.local(LirType::I64);
        if is_range {
            if let Some((rlo, rhi, rincl, rstep)) = self.ranges.get(&it).cloned() {
                self.emit_copy(lo, rlo, span);
                let one = self.const_int(1, span);
                let hi_plus = self.local(LirType::I64);
                self.emit(Instr::Arith { span, op: ArithOp::Add, kind: NumKind::Int, dst: hi_plus, lhs: rhi, rhs: one });
                let hi_plain = self.local(LirType::I64);
                self.emit_copy(hi_plain, rhi, span);
                let incl_set = self.local(LirType::Bool);
                self.emit_copy(incl_set, rincl, span);
                let incl_bb = self.new_block();
                let excl_bb = self.new_block();
                let join_bb = self.new_block();
                self.set_term(Terminator::BrIf { span, cond: incl_set, then_bb: incl_bb, else_bb: excl_bb });
                self.set_current(incl_bb);
                self.emit_copy(hi, hi_plus, span);
                self.set_term(Terminator::Br(join_bb));
                self.set_current(excl_bb);
                self.emit_copy(hi, hi_plain, span);
                self.set_term(Terminator::Br(join_bb));
                self.set_current(join_bb);
                self.emit_copy(step, rstep, span);
            } else {
                self.emit(Instr::RangeLo { span,  dst: lo, range: it });
                self.emit(Instr::RangeHi { span,  dst: hi, range: it });
                self.emit(Instr::RangeStep { span,  dst: step, range: it });
            }
        } else {
            let zero = self.local(LirType::I64);
            let one = self.local(LirType::I64);
            self.emit(Instr::Const { span,  dst: zero, lit: Lit::Int(0) });
            self.emit(Instr::Const { span,  dst: one, lit: Lit::Int(1) });
            self.emit_copy(lo, zero, span);
            self.emit(Instr::ArrayLen { span,  dst: hi, arr: it });
            self.emit_copy(step, one, span);
        }
        let idx = self.local(LirType::I64);
        self.emit_copy(idx, lo, span);
        let header = self.new_block();
        let lbody = self.new_block();
        let merge = self.new_block();
        self.set_term(Terminator::Br(header));
        self.set_current(header);
        let cmp = self.local(LirType::Bool);
        self.emit(Instr::Cmp { span, 
            op: CmpOp::Lt,
            kind: NumKind::Int,
            dst: cmp,
            lhs: idx,
            rhs: hi,
        });
        self.set_term(Terminator::BrIf { span, 
            cond: cmp,
            then_bb: lbody,
            else_bb: merge,
        });
        let depth = self.defer_count;
        self.loop_stack.push(LoopCtx {
            break_bb: merge,
            cont_bb: header,
            defer_depth: depth,
        });
        // continue must step before jumping: use step block
        let step_bb = self.new_block();
        match self.loop_stack.last_mut() {
            Some(ctx) => ctx.cont_bb = step_bb,
            None => return self.ice("`for` loop lowered with no loop context", Some(span)),
        }
        self.set_current(lbody);
        self.push_scope();
        let elem_ty = match &it_ty {
            LirType::Array(inner) => (**inner).clone(),
            LirType::Range => LirType::I64,
            _ => LirType::Any,
        };
        let bind = self.local(elem_ty);
        if is_range {
            self.emit_copy(bind, idx, span);
        } else {
            let es = self.elem_size_of(it);
            self.emit(Instr::ArrayGet { span,  dst: bind, arr: it, index: idx, elem_size: es, unchecked: false });
        }
        if is_tuple {
            for (i, n) in bound_names.iter().enumerate() {
                let dst = self.local(LirType::Any);
                self.emit(Instr::Extract { span, 
                    dst,
                    base: bind,
                    index: i,
                    field: n.clone(),
                });
                self.def(n.clone(), dst, Simple::Other, LirType::Any);
            }
        } else {
            let bt = self.func.locals[bind as usize].clone();
            self.def(bound_names[0].clone(), bind, Simple::Other, bt);
        }
        self.lower_block(body)?;
        self.pop_scope();
        if !self.terminated {
            self.set_term(Terminator::Br(step_bb));
        }
        self.set_current(step_bb);
        self.emit(Instr::Arith { span, 
            op: ArithOp::Add,
            kind: NumKind::Int,
            dst: idx,
            lhs: idx,
            rhs: step,
        });
        self.set_term(Terminator::Br(header));
        self.loop_stack.pop();
        self.set_current(merge);
        Ok(())
    }

    pub(super) fn lower_switch(
        &mut self,
        scrut: &A::Spanned<A::Expr>,
        cases: &[A::SwitchCase],
        default: &Option<Vec<A::Spanned<A::Stmt>>>,
    ) -> Result<(), Diagnostic> {
        let (sv, _) = self.lower_expr(&scrut.node, scrut.span)?;
        self.deny_tuple(sv, "as a switch scrutinee", scrut.span)?;
        let scrut_ty = self.func.locals[sv as usize].clone();
        let orig_sv = sv;
        let sv = if scrut_ty == LirType::Any {
            self.unbox_operand(sv, scrut.span)
        } else {
            sv
        };
        let mut case_bbs = Vec::with_capacity(cases.len());
        let mut pats = Vec::with_capacity(cases.len());
        for c in cases {
            case_bbs.push(self.new_block());
            pats.push(self.lower_pattern(&c.pattern, orig_sv, &scrut_ty)?);
        }
        let mut default_dead = false;
        if default.is_none() {
            if let LirType::Enum(enu) = scrut_ty {
                let ename = self.module.enums[enu].name.clone();
                let mut covered = vec![false; self.module.enums[enu].variants.len()];
                for (c, p) in cases.iter().zip(pats.iter()) {
                    if c.guard.is_none() {
                        if let SwitchPat::Enum { enu: pe, variant } = p {
                            if *pe == enu {
                                covered[*variant] = true;
                            }
                        }
                    }
                }
                let missing: Vec<String> = self.module.enums[enu]
                    .variants
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !covered[*i])
                    .map(|(_, v)| v.name.clone())
                    .collect();
                if !missing.is_empty() {
                    return self.fail(self.err(
                        Code::E108,
                        format!(
                            "non-exhaustive switch over `{ename}`; missing: {}",
                            missing.join(", ")
                        ),
                        scrut.span,
                    ));
                }
                if cases.last().map(|c| c.guard.is_none()).unwrap_or(true) {
                    default_dead = true;
                }
            }
        }
        let merge = self.new_block();
        let default_bb = self.new_block();
        self.set_term(Terminator::Switch {
            span: scrut.span,
            scrut: sv,
            cases: pats.into_iter().zip(case_bbs.iter().copied()).collect(),
            default: default_bb,
        });
        let saved_fallthrough = self.fallthrough.take();
        self.fallthrough = Some((case_bbs.clone(), 0));
        for (i, c) in cases.iter().enumerate() {
            if let Some(f) = self.fallthrough.as_mut() {
                f.1 = i;
            }
            self.set_current(case_bbs[i]);
            self.push_scope();
            if let A::Pattern::Is(t) = &c.pattern {
                if let A::Expr::Ident(_) = &scrut.node {
                    let base = scrut.clone();
                    self.narrow_for_is(&base, t, scrut.span)?;
                }
            }
            if let A::Pattern::Enum { path, args } = &c.pattern {
                let payload_tys = self
                    .variant_of_path(path)
                    .map(|(enu, vi)| self.module.enums[enu].variants[vi].payload.clone())
                    .unwrap_or_default();
                for (i, a) in args.iter().enumerate() {
                    if let A::Pattern::Literal(e) = a {
                        if let A::Expr::Ident(name) = &e.node {
                            let ty = payload_tys.get(i).cloned().unwrap_or(LirType::Any);
                            let dst = self.local(ty.clone());
                            self.emit(Instr::EnumPayload { span: scrut.span, dst, scrut: sv, index: i });
                            self.def(name.clone(), dst, simple_of(&ty), ty);
                        }
                    }
                }
            }
            if let Some(g) = &c.guard {
                let (gv, _) = self.lower_expr(&g.node, g.span)?;
                let gbody = self.new_block();
                let fail_bb = self.new_block();
                let gnext = if i + 1 < case_bbs.len() {
                    case_bbs[i + 1]
                } else {
                    default_bb
                };
                self.set_term(Terminator::BrIf {
                    span: g.span,
                    cond: gv,
                    then_bb: gbody,
                    else_bb: fail_bb,
                });
                self.set_current(fail_bb);
                self.emit_scope_releases();
                self.set_term(Terminator::Br(gnext));
                self.set_current(gbody);
            }
            for st in &c.body {
                if self.terminated {
                    break;
                }
                self.lower_stmt(&st.node, st.span)?;
            }
            self.pop_scope();
            if !self.terminated {
                self.set_term(Terminator::Br(merge));
            }
        }
        self.fallthrough = saved_fallthrough;
        self.set_current(default_bb);
        if let Some(d) = default {
            self.push_scope();
            for st in d {
                if self.terminated {
                    break;
                }
                self.lower_stmt(&st.node, st.span)?;
            }
            self.pop_scope();
        }
        if !self.terminated {
            if default_dead {
                self.set_term(Terminator::Ret(vec![]));
            } else {
                self.set_term(Terminator::Br(merge));
            }
        }
        self.set_current(merge);
        Ok(())
    }

    pub(super) fn lower_switch_arm_value(
        &mut self,
        scrut: &A::Expr,
        pattern: Option<&A::Pattern>,
        guard: Option<&A::Spanned<A::Expr>>,
        body: &A::SwitchExprBody,
        sv: Local,
        scrut_span: Span,
        case_bb: BlockId,
        gnext: BlockId,
        merge: BlockId,
        dst: &mut Option<Local>,
        dst_ty: &mut Option<LirType>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        self.set_current(case_bb);
        self.push_scope();
        if let Some(A::Pattern::Is(t)) = pattern {
            if let A::Expr::Ident(_) = scrut {
                let base = A::sp(scrut.clone(), scrut_span);
                self.narrow_for_is(&base, t, scrut_span)?;
            }
        }
        if let Some(A::Pattern::Enum { path, args }) = pattern {
            let payload_tys = self
                .variant_of_path(path)
                .map(|(enu, vi)| self.module.enums[enu].variants[vi].payload.clone())
                .unwrap_or_default();
            for (i, a) in args.iter().enumerate() {
                if let A::Pattern::Literal(e) = a {
                    if let A::Expr::Ident(name) = &e.node {
                        let ty = payload_tys.get(i).cloned().unwrap_or(LirType::Any);
                        let bound = self.local(ty.clone());
                        self.emit(Instr::EnumPayload { span: scrut_span, dst: bound, scrut: sv, index: i });
                        self.def(name.clone(), bound, simple_of(&ty), ty);
                    }
                }
            }
        }
        if let Some(g) = guard {
            let (gv, _) = self.lower_expr(&g.node, g.span)?;
            let gbody = self.new_block();
            let fail_bb = self.new_block();
            self.set_term(Terminator::BrIf { span: g.span, cond: gv, then_bb: gbody, else_bb: fail_bb });
            self.set_current(fail_bb);
            self.emit_scope_releases();
            self.set_term(Terminator::Br(gnext));
            self.set_current(gbody);
        }
        let (tv, _) = match body {
            A::SwitchExprBody::Expr(e) => self.lower_expr(&e.node, e.span)?,
            A::SwitchExprBody::Block(b) => {
                let (init, last) = b.stmts.split_at(b.stmts.len().saturating_sub(1));
                for st in init {
                    self.lower_stmt(&st.node, st.span)?;
                    if self.failed {
                        match self.diags.last().cloned() {
                            Some(d) => return Err(d),
                            None => return self.ice("lowering failed with no diagnostic recorded", Some(st.span)),
                        }
                    }
                }
                match last.first() {
                    Some(st) if matches!(st.node, A::Stmt::Expr(_)) => {
                        if let A::Stmt::Expr(e) = &st.node {
                            self.lower_expr(&e.node, e.span)?
                        } else {
                            return self.ice("`switch` case block pattern mismatch", Some(span));
                        }
                    }
                    _ => {
                        return self.fail(self.err(
                            Code::E108,
                            "case block must end with a value expression",
                            span,
                        ));
                    }
                }
            }
        };
        let tv_ty = self.func.locals[tv as usize].clone();
        match (dst_ty.clone(), *dst) {
            (None, _) => {
                let d = self.local(tv_ty.clone());
                *dst = Some(d);
                *dst_ty = Some(tv_ty.clone());
                self.emit_copy(d, tv, span);
            }
            (Some(t), Some(d)) if t == tv_ty => {
                self.emit_copy(d, tv, span);
            }
            (Some(t), _) => {
                return self.fail(self.err(
                    Code::E108,
                    format!("`switch` arms yield `{}` and `{}`", switch_ty_name(&t), switch_ty_name(&tv_ty)),
                    span,
                ));
            }
        }
        self.pop_scope();
        if !self.terminated {
            self.set_term(Terminator::Br(merge));
        }
        Ok(())
    }

    pub(super) fn lower_switch_expr(
        &mut self,
        scrut: &A::Expr,
        scrut_span: Span,
        cases: &[A::SwitchExprCase],
        default: &Option<A::SwitchExprBody>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let (sv, _) = self.lower_expr(scrut, scrut_span)?;
        self.deny_tuple(sv, "as a switch scrutinee", scrut_span)?;
        let scrut_ty = self.func.locals[sv as usize].clone();
        let orig_sv = sv;
        let sv = if scrut_ty == LirType::Any {
            self.unbox_operand(sv, scrut_span)
        } else {
            sv
        };
        let mut case_bbs = Vec::with_capacity(cases.len());
        let mut pats = Vec::with_capacity(cases.len());
        for c in cases {
            case_bbs.push(self.new_block());
            pats.push(self.lower_pattern(&c.pattern, orig_sv, &scrut_ty)?);
        }
        if default.is_none() {
            if let LirType::Enum(enu) = &scrut_ty {
                let ename = self.module.enums[*enu].name.clone();
                let mut covered = vec![false; self.module.enums[*enu].variants.len()];
                for (c, p) in cases.iter().zip(pats.iter()) {
                    if c.guard.is_none() {
                        if let SwitchPat::Enum { enu: pe, variant } = p {
                            if pe == enu {
                                covered[*variant] = true;
                            }
                        }
                    }
                }
                let missing: Vec<String> = self.module.enums[*enu]
                    .variants
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !covered[*i])
                    .map(|(_, v)| v.name.clone())
                    .collect();
                if !missing.is_empty() {
                    return self.fail(self.err(
                        Code::E108,
                        format!(
                            "non-exhaustive expression `switch` over `{ename}`; missing: {}",
                            missing.join(", ")
                        ),
                        scrut_span,
                    ));
                }
            } else {
                return self.fail(self.err(
                    Code::E108,
                    "expression `switch` must be exhaustive or have a `default` arm",
                    scrut_span,
                ));
            }
        }
        let merge = self.new_block();
        let default_bb = self.new_block();
        self.set_term(Terminator::Switch {
            span: scrut_span,
            scrut: sv,
            cases: pats.into_iter().zip(case_bbs.iter().copied()).collect(),
            default: default_bb,
        });
        let mut dst: Option<Local> = None;
        let mut dst_ty: Option<LirType> = None;
        for (i, c) in cases.iter().enumerate() {
            let gnext = if i + 1 < case_bbs.len() { case_bbs[i + 1] } else { default_bb };
            self.lower_switch_arm_value(
                scrut,
                Some(&c.pattern),
                c.guard.as_ref(),
                &c.body,
                sv,
                scrut_span,
                case_bbs[i],
                gnext,
                merge,
                &mut dst,
                &mut dst_ty,
                span,
            )?;
            if self.failed {
                match self.diags.last().cloned() {
                    Some(d) => return Err(d),
                    None => return self.ice("lowering failed with no diagnostic recorded", Some(span)),
                }
            }
        }
        if let Some(d) = default {
            self.lower_switch_arm_value(
                scrut, None, None, d, sv, scrut_span, default_bb, default_bb, merge,
                &mut dst, &mut dst_ty, span,
            )?;
            if self.failed {
                match self.diags.last().cloned() {
                    Some(d) => return Err(d),
                    None => return self.ice("lowering failed with no diagnostic recorded", Some(span)),
                }
            }
        } else {
            self.set_current(default_bb);
            self.set_term(Terminator::Br(merge));
        }
        self.set_current(merge);
        match (dst, dst_ty) {
            (Some(d), Some(t)) => Ok((d, simple_of(&t))),
            _ => self.fail(self.err(Code::E108, "empty `switch` yields no value", span)),
        }
    }

    pub(super) fn lower_pattern(
        &mut self,
        p: &A::Pattern,
        scrut_source: Local,
        scrut_ty: &LirType,
    ) -> Result<SwitchPat, Diagnostic> {
        match p {
            A::Pattern::Literal(e) => match &e.node {
                A::Expr::Int(v) => Ok(SwitchPat::Int(*v)),
                _ => Err(self.err(Code::E108, "non-int `case` patterns pending", e.span)),
            },
            A::Pattern::Range { lo, hi, inclusive } => {
                let (a, b) = match (&lo.node, &hi.node) {
                    (A::Expr::Int(a), A::Expr::Int(b)) => (*a, *b),
                    _ => {
                        return Err(self.err(
                            Code::E108,
                            "range `case` needs int bounds",
                            lo.span,
                        ));
                    }
                };
                Ok(SwitchPat::Range {
                    lo: a,
                    hi: b,
                    inclusive: *inclusive,
                })
            }
            A::Pattern::Is(t) => {
                let tag = t
                    .path
                    .last()
                    .map(|s| s.rsplit('.').next().unwrap_or(s).to_string())
                    .unwrap_or_default();
                let target = A::Type {
                    path: vec![tag.clone()],
                    args: Vec::new(),
                    nullable: false,
                    fn_sig: None,
                    tuple: Vec::new(),
                };
                let target = resolve_ty(self.module, &target);
                match crate::instr::resolve_is(self.module, scrut_ty, &target) {
                    Ok(check) => Ok(SwitchPat::Is { tag, source: scrut_source, check }),
                    Err(msg) => Err(self.err(Code::E108, msg, Span { start: 0, end: 0 })),
                }
            }
            A::Pattern::Enum { path, args } => {
                let vname = path.last().ok_or_else(|| {
                    self.err(Code::E108, "empty enum pattern", Span { start: 0, end: 0 })
                })?;
                let (enu, vi) = self.variant_of_path(path).ok_or_else(|| {
                    if path.len() > 1 {
                        if let Some((oenu, _)) = self.variant_of(vname) {
                            let owner = short_name(&self.module.enums[oenu].name).to_string();
                            return self.err(
                                Code::E108,
                                format!(
                                    "variant `{vname}` belongs to `{owner}`, not `{}`",
                                    path[path.len() - 2]
                                ),
                                Span { start: 0, end: 0 },
                            );
                        }
                    }
                    self.unknown_variant(&vname, Span { start: 0, end: 0 })
                })?;
                let want = self.module.enums[enu].variants[vi].payload.len();
                if args.len() != want {
                    return Err(self.err(
                        Code::E108,
                        format!("variant `{vname}` binds {want} payloads, got {}", args.len()),
                        Span { start: 0, end: 0 },
                    ));
                }
                for a in args {
                    match a {
                        A::Pattern::Literal(e) => match &e.node {
                            A::Expr::Ident(_) => {}
                            _ => {
                                return Err(self.err(
                                    Code::E108,
                                    "nested enum patterns pending",
                                    e.span,
                                ));
                            }
                        },
                        _ => {
                            return Err(self.err(
                                Code::E108,
                                "nested enum patterns pending",
                                Span { start: 0, end: 0 },
                            ));
                        }
                    }
                }
                Ok(SwitchPat::Enum { enu, variant: vi })
            }
            A::Pattern::Wildcard => Err(self.err(Code::E108, "wildcard patterns pending", Span {
                start: 0,
                end: 0,
            })),
        }
    }

}
