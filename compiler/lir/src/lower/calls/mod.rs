use diagnostics::{Code, Diagnostic, Span};
use frontend::ast as A;
use std::collections::{BTreeMap};
pub(super) use super::*;

mod intrinsics;

impl<'a> Builder<'a> {
    pub(super) fn lower_ext_call(
        &mut self,
        eid: usize,
        obj: Local,
        args: &[A::CallArg],
        argv: &[Local],
        targs: &[A::Type],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let mname = self.module.functions[eid].name.clone();
        let needs_resolve = args.iter().any(|a| a.name.is_some())
            || self.param_info.get(&mname).map(|p| p.iter().any(|(_, d)| d.is_some())).unwrap_or(false);
        let mut full = vec![obj];
        let mut seed = BTreeMap::new();
        if needs_resolve {
            let (rv, rs) = self.resolve_call_args(eid, args, span)?;
            full.extend(rv);
            seed = rs;
        } else {
            full.extend(argv.iter().copied());
        }
        let slf0 = full.first().copied();
        self.call_fn_ex(eid, targs, full, &seed, span, slf0)
    }
    pub(super) fn lower_iface_call(
        &mut self,
        obj: Local,
        ii: usize,
        method: &str,
        args: &[A::CallArg],
        targs: &[A::Type],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let iface = self.module.interfaces[ii].clone();
        let mi = iface.method_index.get(method).copied().ok_or_else(|| {
            self.err(Code::E108, format!("unknown method `{method}`"), span)
        })?;
        let mdesc = iface.methods[mi].clone();
        let cis = self.implementors(ii);
        if cis.is_empty() {
            return self.fail(self.err(
                Code::E108,
                format!("interface `{}` has no implementations", iface.name),
                span,
            ));
        }
        let first_mid = self.module.classes[cis[0]].methods.get(method).cloned().map(|r| r.id);
        let (resolved, iseed) = match first_mid {
            Some(mid) => self.resolve_call_args(mid, args, span)?,
            None => {
                let mut argv = Vec::with_capacity(args.len());
                for a in args {
                    argv.push(self.lower_expr(&a.value.node, a.value.span)?.0);
                }
                (argv, BTreeMap::new())
            }
        };
        let is_tuple = matches!(mdesc.ret, LirType::Tuple(_));
        let is_range = matches!(mdesc.ret, LirType::Range);
        let shared: Local = if is_tuple || is_range {
            let elems = crate::instr::flat_sig(&mdesc.ret)
                .iter()
                .map(|t| self.local(t.clone()))
                .collect::<Vec<_>>();
            let marker = self.local(mdesc.ret.clone());
            if is_tuple {
                self.tuples.insert(marker, elems);
            } else {
                let (lo, hi, incl, step) = (elems[0], elems[1], elems[2], elems[3]);
                self.ranges.insert(marker, (lo, hi, incl, step));
            }
            marker
        } else {
            self.local(mdesc.ret.clone())
        };
        let cls = self.local(LirType::I64);
        self.emit(Instr::Call { span, dsts: vec![cls], err: None, target: CallTarget::Builtin("__rnx_obj_class".to_string()), args: vec![obj] });
        let merge = self.new_block();
        let mut cases = Vec::with_capacity(cis.len());
        let mut arms = Vec::with_capacity(cis.len());
        for ci in cis {
            let arm = self.new_block();
            cases.push((crate::instr::SwitchPat::Int(ci as i64 + 1), arm));
            arms.push((arm, ci));
        }
        let default = self.new_block();
        self.set_term(crate::instr::Terminator::Switch { span, scrut: cls, cases, default });
        for (arm, ci) in arms {
            self.set_current(arm);
            let mid = self.module.classes[ci].methods.get(method).cloned().map(|r| r.id).ok_or_else(|| {
                self.err(Code::E108, format!("implementation lost method `{method}`"), span)
            })?;
            let mut full = vec![obj];
            full.append(&mut resolved.clone());
            let slf0 = full.first().copied();
            let (got, _) = self.call_fn_ex(mid, targs, full, &iseed, span, slf0)?;
            if is_tuple || is_range {
                let dsts = self.expand_value(shared, span)?;
                let srcs = self.expand_value(got, span)?;
                for (d, s) in dsts.into_iter().zip(srcs.into_iter()) {
                    self.emit_copy(d, s, span);
                }
            } else if self.nub_has(got) {
                self.nub_unmark(got);
                self.emit(Instr::Copy { span, dst: shared, src: got });
            } else {
                self.emit_copy(shared, got, span);
            }
            if !self.terminated {
                self.set_term(crate::instr::Terminator::Br(merge));
            }
        }
        self.set_current(default);
        let msg = self.local(LirType::Str);
        self.emit(Instr::Const { span, dst: msg, lit: Lit::Str("dead object".to_string()) });
        self.emit_run_defers(0);
        self.set_term(crate::instr::Terminator::Throw { span, src: msg, catch: None });
        self.set_current(merge);
        Ok((shared, simple_of(&mdesc.ret)))
    }
    pub(super) fn lower_call(
        &mut self,
        callee: &A::Spanned<A::Expr>,
        targs: &[A::Type],
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        match &callee.node {
            A::Expr::Super => {
                return self.lower_super_init(args, span);
            }
            A::Expr::Ident(n) => {
                if let Some((l, _, _)) = self.lookup(n) {
                    if args.iter().any(|a| a.name.is_some()) {
                        return self.fail(self.err(
                            Code::E108,
                            "named arguments need a directly named function or method",
                            span,
                        ));
                    }
                    let mut argv = Vec::with_capacity(args.len());
                    for a in args {
                        argv.push(self.lower_expr(&a.value.node, a.value.span)?.0);
                    }
                    let dst = self.local(LirType::Any);
                    self.emit(Instr::Call { span, 
                        dsts: vec![dst],
                        err: None,
                        target: CallTarget::Value(l),
                        args: argv,
                    });
                    return Ok((dst, Simple::Other));
                }
                if let Some(kind) = vec_name(n) {
                    if self.module.class_index.contains_key(n) {
                        return self.lower_vec_new(kind, args, span);
                    }
                }
                if let Some(ci) = self.module.class_index.get(n) {
                    let short = short_name(n);
                    let primitive_stub = n.starts_with("std.prelude.")
                        && matches!(short, "Int" | "Float" | "FastFloat" | "Bool" | "String" | "Char" | "Array" | "GenRef");
                    if !primitive_stub {
                        // Direct class calls are rejected with E204 in
                        // semantic analysis; lowering keeps the legacy
                        // construction path for direct-lower users.
                        return self.lower_construct(*ci, args, span);
                    }
                }
                if self.module.interface_index.contains_key(n)
                    || self.module.interface_index.contains_key(short_name(n))
                {
                    return self.fail(self.err(
                        Code::E108,
                        format!("cannot construct interface `{n}`"),
                        span,
                    ));
                }
                if let Some((enu, vi)) = self.variant_of(n) {
                    return self.lower_enum_construct(enu, vi, args, span);
                }
                if short_name(n) == "GenRef" {
                    if args.len() != 1 {
                        return self.fail(self.err(Code::E108, "`GenRef` takes 1 arg", span));
                    }
                    let (o, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
                    let dst = self.local(LirType::GenRef(None));
                    self.emit(Instr::GenRefOf { span,  dst, obj: o });
                    return Ok((dst, Simple::Other));
                }
                if n == "std.testing.blackBox" {
                    if args.len() != 1 {
                        return self.fail(self.err(Code::E108, "`blackBox` takes 1 arg", span));
                    }
                    let (v, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
                    self.deny_tuple(v, "in `blackBox`", args[0].value.span)?;
                    let ty = self.func.locals[v as usize].clone();
                    let dst = self.local(ty);
                    self.emit(Instr::Call { span, 
                        dsts: vec![dst],
                        err: None,
                        target: CallTarget::Builtin("__rnx_black_box".to_string()),
                        args: vec![v],
                    });
                    return Ok((dst, Simple::Other));
                }
                if n == "typeOf" || short_name(n) == "typeOf" {
                    if args.len() != 1 {
                        return self.fail(self.err(Code::E108, "`typeOf` takes 1 arg", span));
                    }
                    let (v, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
                    if self.ranges.contains_key(&v) {
                        let dst = self.local(LirType::Str);
                        self.emit(Instr::Const { span, dst, lit: Lit::Str("Range".to_string()) });
                        return Ok((dst, Simple::Other));
                    }
                    self.deny_tuple(v, "in `typeOf`", args[0].value.span)?;
                    let vt = self.func.locals[v as usize].clone();
                    let dst = self.local(LirType::Str);
                    let dynamic = matches!(&vt, LirType::Obj(name) if self.module.interface_index.contains_key(name));
                    match Self::static_type_name(self.module, &vt) {
                        Some(name) if !dynamic => {
                            self.emit(Instr::Const { span, dst, lit: Lit::Str(name) });
                        }
                        _ => {
                            self.emit(Instr::Call { span, 
                                dsts: vec![dst],
                                err: None,
                                target: CallTarget::Builtin("__rnx_typeof_any".to_string()),
                                args: vec![v],
                            });
                        }
                    }
                    return Ok((dst, Simple::Other));
                }
                if short_name(n) == "Float" || short_name(n) == "FastFloat" || short_name(n) == "Int" {
                    if args.len() != 1 {
                        return self.fail(self.err(Code::E108, format!("`{n}` takes 1 arg"), span));
                    }
                    let (v, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
                    let sty = self.func.locals[v as usize].clone();
                    let want_float = short_name(n) != "Int";
                    let want_kind = if short_name(n) == "FastFloat" {
                        FloatKind::Fast
                    } else {
                        FloatKind::Strict
                    };
                    match (&sty, want_float) {
                        (LirType::I64, true) => {
                            let dst = self.local(LirType::F64(want_kind));
                            self.emit(Instr::Convert { span, 
                                dst,
                                src: v,
                                kind: ConvertKind::IntToFloat(want_kind),
                            });
                            let s = if want_kind == FloatKind::Fast {
                                Simple::Fast
                            } else {
                                Simple::Strict
                            };
                            return Ok((dst, s));
                        }
                        (LirType::F64(_), false) => {
                            let dst = self.local(LirType::I64);
                            self.emit(Instr::Convert { span, 
                                dst,
                                src: v,
                                kind: ConvertKind::FloatToInt,
                            });
                            return Ok((dst, Simple::Int));
                        }
                        (LirType::I64, false) => {
                            let dst = self.local(LirType::I64);
                            self.emit_copy(dst, v, span);
                            return Ok((dst, Simple::Int));
                        }
                        (LirType::F64(_), true) => {
                            let dst = self.local(LirType::F64(want_kind));
                            self.emit_copy(dst, v, span);
                            let s = if want_kind == FloatKind::Fast {
                                Simple::Fast
                            } else {
                                Simple::Strict
                            };
                            return Ok((dst, s));
                        }
                        _ => {
                            return self.fail(self.err(
                                Code::E108,
                                format!("`{n}` needs a numeric argument"),
                                span,
                            ));
                        }
                    }
                }
                if let Some(id) = self.module.fn_index.get(n) {
                    let id = *id;
                    let (argv, seed) = self.resolve_call_args(id, args, span)?;
                    return self.call_fn(id, targs, argv, &seed, span);
                }
                if let Some(fid) = self.module.foreign_index.get(n).copied() {
                    return self.call_foreign(fid, args, span);
                }
                if n == "print" {
                    let mut argv = Vec::with_capacity(args.len());
                    for a in args {
                        let v = self.lower_expr(&a.value.node, a.value.span)?.0;
                        if self.records.contains_key(&v) {
                            argv.push(self.record_to_str(v, a.value.span));
                        } else if self.nub_has(v) {
                            let pa = self.local(LirType::Any);
                            self.emit(Instr::Copy { span: a.value.span, dst: pa, src: v });
                            argv.push(pa);
                        } else {
                            argv.push(v);
                        }
                    }
                    if let Some(id) = self.module.fn_index.get("std.io.write").copied() {
                        let mut joined: Option<Local> = None;
                        for v in argv {
                            let av = self.coerce_to_slot(v, &LirType::Any, span);
                            let fd = self.const_int(1, span);
                            let s = self.local(LirType::Str);
                            self.emit(Instr::Call { span,
                                dsts: vec![s],
                                err: None,
                                target: CallTarget::Builtin("__rnx_io_pretty".to_string()),
                                args: vec![av, fd],
                            });
                            joined = Some(match joined {
                                None => s,
                                Some(acc) => {
                                    let gap = self.local(LirType::Str);
                                    self.emit(Instr::Const { span,  dst: gap, lit: Lit::Str(" ".to_string()) });
                                    let pair = self.local(LirType::Str);
                                    self.emit(Instr::Concat { span,  dst: pair, lhs: acc, rhs: gap });
                                    let both = self.local(LirType::Str);
                                    self.emit(Instr::Concat { span,  dst: both, lhs: pair, rhs: s });
                                    both
                                }
                            });
                        }
                        let text = match joined {
                            Some(j) => j,
                            None => {
                                let e = self.local(LirType::Str);
                                self.emit(Instr::Const { span,  dst: e, lit: Lit::Str(String::new()) });
                                e
                            }
                        };
                        return self.call_fn(id, targs, vec![text], &BTreeMap::new(), span);
                    }
                    self.emit(Instr::Call { span, 
                        dsts: vec![],
                        err: None,
                        target: CallTarget::Builtin("print".to_string()),
                        args: argv,
                    });
                    let dst = self.local(LirType::Null);
                    self.emit(Instr::Const { span,  dst, lit: Lit::Null });
                    return Ok((dst, Simple::Other));
                }
                if n == "assert" {
                    if args.len() != 2 {
                        return self.fail(self.err(
                            Code::E108,
                            "`assert` takes a condition and a message",
                            span,
                        ));
                    }
                    let (c, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
                    if self.func.locals[c as usize] != LirType::Bool {
                        return self.fail(self.err(
                            Code::E108,
                            "`assert` condition must be a Bool",
                            args[0].value.span,
                        ));
                    }
                    let (m, _) = self.lower_expr(&args[1].value.node, args[1].value.span)?;
                    if self.func.locals[m as usize] != LirType::Str {
                        return self.fail(self.err(
                            Code::E108,
                            "`assert` message must be a Str",
                            args[1].value.span,
                        ));
                    }
                    self.emit(Instr::Call { span, 
                        dsts: vec![],
                        err: None,
                        target: CallTarget::Builtin("assert".to_string()),
                        args: vec![c, m],
                    });
                    let dst = self.local(LirType::Null);
                    self.emit(Instr::Const { span,  dst, lit: Lit::Null });
                    return Ok((dst, Simple::Other));
                }
                if n == "__testCheck" {
                    if !args.is_empty() {
                        return self.fail(self.err(
                            Code::E108,
                            "`__testCheck` takes no args",
                            span,
                        ));
                    }
                    let dst = self.local(LirType::Bool);
                    self.emit(Instr::Call { span, 
                        dsts: vec![dst],
                        err: None,
                        target: CallTarget::Builtin("__testCheck".to_string()),
                        args: Vec::new(),
                    });
                    return Ok((dst, Simple::Other));
                }
                if n == "Thread" || n == "std.sync.Thread" {
                    return self.fail(self.err(
                        Code::E108,
                        "use `Thread.spawn(fn_name)`",
                        callee.span,
                    ));
                }
                if n == "__rnx_clock_mono" {
                    if !args.is_empty() {
                        return self.fail(self.err(
                            Code::E108,
                            "`__rnx_clock_mono` takes no args",
                            span,
                        ));
                    }
                    let dst = self.local(LirType::I64);
                    self.emit(Instr::Call { span, 
                        dsts: vec![dst],
                        err: None,
                        target: CallTarget::Builtin("__rnx_clock_mono".to_string()),
                        args: Vec::new(),
                    });
                    return Ok((dst, Simple::Int));
                }
                if let Some(arity) = math_arity(n) {
                    if args.len() != arity {
                        return self.fail(self.err(
                            Code::E108,
                            format!("`{n}` takes {arity} args"),
                            span,
                        ));
                    }
                    let mut argv = Vec::with_capacity(arity);
                    for a in args {
                        let v = self.lower_expr(&a.value.node, a.value.span)?.0;
                        match self.func.locals[v as usize].clone() {
                            LirType::I64 => {
                                let dst = self.local(LirType::F64(FloatKind::Strict));
                                self.emit(Instr::Convert { span, 
                                    dst,
                                    src: v,
                                    kind: ConvertKind::IntToFloat(FloatKind::Strict),
                                });
                                argv.push(dst);
                            }
                            LirType::F64(_) => argv.push(v),
                            _ => {
                                return self.fail(self.err(
                                    Code::E108,
                                    format!("`{n}` needs float args"),
                                    a.value.span,
                                ));
                            }
                        }
                    }
                    let dst = self.local(LirType::F64(FloatKind::Strict));
                    self.emit(Instr::Call { span, 
                        dsts: vec![dst],
                        err: None,
                        target: CallTarget::Builtin(n.to_string()),
                        args: argv,
                    });
                    return Ok((dst, Simple::Strict));
                }
                if let Some((arity, ret, simple)) = intrin_sig(n) {
                    if args.len() != arity {
                        return self.fail(self.err(
                            Code::E108,
                            format!("`{n}` takes {arity} args"),
                            span,
                        ));
                    }
                    let mut argv = Vec::with_capacity(arity);
                    for a in args {
                        argv.push(self.lower_expr(&a.value.node, a.value.span)?.0);
                    }
                    if ret == LirType::Null {
                        self.emit(Instr::Call { span, 
                            dsts: vec![],
                            err: None,
                            target: CallTarget::Builtin(n.to_string()),
                            args: argv,
                        });
                        let dst = self.local(LirType::Null);
                        self.emit(Instr::Const { span,  dst, lit: Lit::Null });
                        return Ok((dst, Simple::Other));
                    }

                    let dst = self.local(ret.clone());
                    self.emit(Instr::Call { span, 
                        dsts: vec![dst],
                        err: None,
                        target: CallTarget::Builtin(n.to_string()),
                        args: argv,
                    });
                    return Ok((dst, simple));
                }
                self.fail(self.err(Code::E108, format!("unknown `{n}`"), callee.span))
            }
            A::Expr::Member { base, field } => {
                if matches!(&base.node, A::Expr::Super) {
                    return self.lower_super_call(field, args, targs, span);
                }
                if let A::Expr::Ident(root) = &base.node {
                    if root == "GenRef" || root.ends_with(".GenRef") {
                        return self.lower_genref(field, args, span);
                    }
                    if root == "Thread" || root == "std.sync.Thread" {
                        return self.lower_thread(field, args, span, root != "Thread");
                    }
                    if root == "ThreadPool" {
                        return self.lower_pool_init(field, args, span);
                    }
                    if root == "Pointer" || root.ends_with(".Pointer") {
                        return self.lower_pointer_from_address(field, targs, args, span);
                    }
                    if let Some(kind) = vec_name(root) {
                        return self.lower_vec_static(kind, field, args, span);
                    }
                    if root == "Float" || root.ends_with(".Float") {
                        return self.lower_float_static(field, args, span);
                    }
                    if self.module.class_index.contains_key(root) {
                        if (field == "parse" || field == "decode")
                            && !targs.is_empty()
                            && short_name(root) == "JSON"
                        {
                            return self.lower_json_parse_typed(args, targs, span);
                        }
                        let q = format!("{root}.{field}");
                        if let Some(id) = self.module.fn_index.get(&q).copied() {
                            let needs_resolve = args.iter().any(|a| a.name.is_some())
                                || self.param_info.get(&q).map(|p| p.iter().any(|(_, d)| d.is_some())).unwrap_or(false);
                            if needs_resolve {
                                let (rv, seed) = self.resolve_call_args(id, args, span)?;
                                return self.call_fn(id, targs, rv, &seed, span);
                            }
                            let ptys = self.param_tys.get(&q).cloned().unwrap_or_default();
                            let mut argv = Vec::with_capacity(args.len());
                            for (i, a) in args.iter().enumerate() {
                                let decl = ptys.get(i).map(|(_, t)| t.clone());
                                match decl {
                                    Some(d) => argv.push(self.lower_call_arg(Some(&d), a)?.0),
                                    None => argv.push(self.lower_expr(&a.value.node, a.value.span)?.0),
                                }
                            }
                            return self.call_fn(id, targs, argv, &BTreeMap::new(), span);
                        }
                    }
                    if let Some(enu) = self.module.enum_index.get(root).copied() {
                        let vi = *self.module.enums[enu].variant_index.get(field).ok_or_else(|| {
                            self.err(Code::E108, format!("unknown variant `{root}.{field}`"), span)
                        })?;
                        return self.lower_enum_construct(enu, vi, args, span);
                    }
                }
                let (obj, _) = self.lower_expr(&base.node, base.span)?;
                let ty = self.func.locals[obj as usize].clone();
                if let Some(nskey) = super::ns_key_of_ty(&ty) {
                    let q = format!("{nskey}.{field}");
                    if let Some(id) = self.module.fn_index.get(&q).copied() {
                        let needs_resolve = args.iter().any(|a| a.name.is_some())
                            || self.param_info.get(&q).map(|p| p.iter().any(|(_, d)| d.is_some())).unwrap_or(false);
                        if needs_resolve {
                            let (rv, seed) = self.resolve_call_args(id, args, span)?;
                            return self.call_fn(id, targs, rv, &seed, span);
                        }
                        let ptys = self.param_tys.get(&q).cloned().unwrap_or_default();
                        let mut argv = Vec::with_capacity(args.len());
                        for (i, a) in args.iter().enumerate() {
                            match ptys.get(i) {
                                Some((_, d)) => argv.push(self.lower_call_arg(Some(d), a)?.0),
                                None => argv.push(self.lower_expr(&a.value.node, a.value.span)?.0),
                            }
                        }
                        return self.call_fn(id, targs, argv, &BTreeMap::new(), span);
                    }
                    return self.fail(self.err(
                        Code::E108,
                        format!("unknown member `{field}` on module namespace"),
                        span,
                    ));
                }
                if let Some(rec) = self.records.get(&obj).cloned() {
                    if let Some((_, fl)) = rec.iter().find(|(n, _)| n == field) {
                        let fl = *fl;
                        if matches!(self.func.locals[fl as usize], LirType::Closure) {
                            if args.iter().any(|a| a.name.is_some()) {
                                return self.fail(self.err(
                                    Code::E108,
                                    "named arguments need a directly named function or method",
                                    span,
                                ));
                            }
                            let mut argv = Vec::with_capacity(args.len());
                            for a in args {
                                argv.push(self.lower_expr(&a.value.node, a.value.span)?.0);
                            }
                            let dst = self.local(LirType::Any);
                            self.emit(Instr::Call { span, 
                                dsts: vec![dst],
                                err: None,
                                target: CallTarget::Value(fl),
                                args: argv,
                            });
                            return Ok((dst, Simple::Other));
                        }
                    }
                }
                if matches!(ty, LirType::Pool) {
                    return self.lower_pool_method(obj, field, args, span);
                }
                if let LirType::Pointer(inner) = &ty {
                    if matches!(
                        field.as_str(),
                        "read" | "readVolatile" | "write" | "writeVolatile"
                    ) {
                        return self.lower_pointer_method(
                            obj,
                            inner.as_ref().clone(),
                            field,
                            args,
                            span,
                        );
                    }
                }
                let hof_hint = match &ty {
                    LirType::Array(elem) if (field == "map" || field == "filter") && args.len() == 1 => {
                        match &args[0].value.node {
                            A::Expr::Closure { params, .. } if params.iter().all(|p| p.ty.is_none()) => {
                                Some(vec![elem.as_ref().clone()])
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let pushed = hof_hint.is_some();
                if let Some(h) = hof_hint {
                    self.closure_hints.push(h);
                }
                let ext_hint = self.ext_hint_target(&ty, field);
                let positional_ext = ext_hint.as_ref().is_some_and(|(mname, _)| {
                    args.iter().all(|a| a.name.is_none())
                        && !self
                            .param_info
                            .get(mname)
                            .is_some_and(|p| p.iter().any(|(_, d)| d.is_some()))
                });
                let mut prefix: Vec<(A::Type, LirType)> = Vec::new();
                if positional_ext {
                    if let Some((mname, _)) = ext_hint.as_ref() {
                        if let Some(self_ty) = self.ext_self.get(mname) {
                            prefix.push((self_ty.clone(), ty.clone()));
                        }
                    }
                }
                let mut argv = Vec::with_capacity(args.len());
                let method_tys: Vec<A::Type> = if args.iter().any(|a| a.name.is_some()) {
                    Vec::new()
                } else {
                    match &ty {
                    LirType::Obj(name) => self
                        .module
                        .class_index
                        .get(name)
                        .and_then(|ci| self.module.classes[*ci].methods.get(field))
                        .map(|r| {
                            let mname = self.module.functions[r.id].name.clone();
                            self.param_tys
                                .get(&mname)
                                .map(|v| v.iter().map(|(_, t)| t.clone()).collect())
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    _ => Vec::new(),
                    }
                };
                for (i, a) in args.iter().enumerate() {
                    let mut hint: Option<Vec<LirType>> = None;
                    if positional_ext {
                        if let Some((mname, _)) = ext_hint.as_ref() {
                            if let A::Expr::Closure { params, .. } = &a.value.node {
                                if params.iter().all(|p| p.ty.is_none()) {
                                    hint = self.ext_closure_hint(mname, i, &prefix);
                                }
                            }
                        }
                    }
                    let pushed_hint = hint.is_some();
                    if let Some(h) = hint {
                        self.closure_hints.push(h);
                    }
                    let v = match method_tys.get(i) {
                        Some(d) => self.lower_call_arg(Some(d), a)?.0,
                        None => self.lower_expr(&a.value.node, a.value.span)?.0,
                    };
                    if pushed_hint {
                        self.closure_hints.pop();
                    }
                    if positional_ext {
                        if let Some((mname, _)) = ext_hint.as_ref() {
                            let actual =
                                self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                            let decl = self
                                .param_tys
                                .get(mname)
                                .and_then(|tys| tys.get(i))
                                .map(|(_, t)| t.clone())
                                .unwrap_or_else(|| A::Type::named("Any"));
                            prefix.push((decl, actual));
                        }
                    }
                    argv.push(v);
                }
                if pushed {
                    self.closure_hints.pop();
                }
                if let LirType::Obj(name) = &ty {
                    if name.rsplit('.').next() == Some("Channel") && field == "send" {
                        return self.lower_channel_send(obj, argv, span, name.clone());
                    }
                }
                if matches!(ty, LirType::Vec4f | LirType::Vec4i) {
                    return self.lower_vec_method(obj, ty, field, argv, span);
                }
                if let LirType::Obj(name) = &ty {
                    if let Some(ci) = self.module.class_index.get(name) {
                        let ci = *ci;
                        if self.module.classes[ci].methods.contains_key(field) {
                            let mid = self.check_method(ci, field, span)?;
                            let mname = self.module.functions[mid].name.clone();
                            let needs_resolve = args.iter().any(|a| a.name.is_some())
                                || self.param_info.get(&mname).map(|p| p.iter().any(|(_, d)| d.is_some())).unwrap_or(false);
                            let mut full = vec![obj];
                            let mut seed = BTreeMap::new();
                            if needs_resolve {
                                let (rv, rs) = self.resolve_call_args(mid, args, span)?;
                                full.extend(rv);
                                seed = rs;
                            } else {
                                full.extend(argv);
                            }
                            let recv = match &base.node {
                                A::Expr::Ident(n) => Some(n.clone()),
                                _ => None,
                            };
                            let cls = self.module.classes[ci].name.clone();
                            let slf0 = full.first().copied();
                            let (got, s) = self.call_fn_ex(mid, targs, full, &seed, span, slf0)?;
                            if let Some(n) = recv {
                                if let Some(pt) = self.recv_opt_payload(&n, &mname, &cls) {
                                    self.precise.insert(got, pt);
                                }
                            }
                            return Ok((got, s));
                        }
                        if self.module.classes[ci].field_index.contains_key(field)
                            && matches!(
                                self.module.classes[ci].fields
                                    [self.module.classes[ci].field_index[field]]
                                .ty,
                                LirType::Closure
                            )
                        {
                            if args.iter().any(|a| a.name.is_some()) {
                                return self.fail(self.err(
                                    Code::E108,
                                    "named arguments need a directly named function or method",
                                    span,
                                ));
                            }
                            let fi = self.check_field(ci, field, span)?;
                            let fl = self.local(LirType::Closure);
                            self.emit(Instr::GetField { span,
                                dst: fl,
                                obj,
                                field: fi,
                            });
                            let dst = self.local(LirType::Any);
                            self.emit(Instr::Call { span, 
                                dsts: vec![dst],
                                err: None,
                                target: CallTarget::Value(fl),
                                args: argv,
                            });
                            return Ok((dst, Simple::Other));
                        }
                    }
                    if let Some(ii) = self.module.interface_index.get(name).copied() {
                        return self.lower_iface_call(obj, ii, field, args, targs, span);
                    }
                }
                if let LirType::Enum(ei) = &ty {
                    let ename = self.module.enums.get(*ei).map(|d| d.name.as_str()).unwrap_or("");
                    if ename == "std.prelude.Result" {
                        return self.lower_prelude_enum_method(obj, *ei, ename, field, args, argv, span);
                    }
                }
                if matches!(ty, LirType::Any) {
                    if matches!(field.as_str(), "isOk" | "isErr" | "unwrap" | "unwrapOr" | "map") {
                        if let Some(ei) = self.module.enum_index.get("std.prelude.Result").copied() {
                            let ename = self.module.enums.get(ei).map(|d| d.name.clone()).unwrap_or_default();
                            return self.lower_prelude_enum_method(obj, ei, &ename, field, args, argv, span);
                        }
                    }
                }
                if let LirType::GenRef(class) = &ty {
                    if field == "get" {
                        let out = class.clone().map(LirType::Obj).unwrap_or(LirType::Any);
                        let dst = self.local(out);
                        self.emit(Instr::GenRefGet { span,  dst, gref: obj });
                        return Ok((dst, Simple::Other));
                    }
                }
                if field == "asFast" || field == "asStrict" {
                    if !args.is_empty() {
                        return self.fail(self.err(
                            Code::E108,
                            format!("`{field}` takes no args"),
                            span,
                        ));
                    }
                    let want = if field == "asFast" {
                        FloatKind::Fast
                    } else {
                        FloatKind::Strict
                    };
                    match &ty {
                        LirType::F64(_) => {
                            let dst = self.local(LirType::F64(want));
                            self.emit_copy(dst, obj, span);
                            let s = if want == FloatKind::Fast {
                                Simple::Fast
                            } else {
                                Simple::Strict
                            };
                            return Ok((dst, s));
                        }
                        _ => {
                            return self.fail(self.err(
                                Code::E108,
                                format!("`{field}` needs a Float receiver"),
                                span,
                            ));
                        }
                    }
                }
                if field == "stride" {
                    if let Some((rlo, rhi, rincl, _)) = self.ranges.get(&obj).cloned() {
                        if args.len() != 1 {
                            return self.fail(self.err(Code::E108, "`stride` takes 1 arg", span));
                        }
                        let sv = argv.first().copied().ok_or_else(|| {
                            self.err(Code::E108, "`stride` takes 1 arg", span)
                        })?;
                        self.deny_tuple(sv, "in `stride`", args[0].value.span)?;
                        let st = self.func.locals[sv as usize].clone();
                        let step = match st {
                            LirType::I64 => sv,
                            LirType::Any => self.unbox_to_int(sv, args[0].value.span),
                            _ => {
                                return self.fail(self.err(
                                    Code::E108,
                                    "`stride` needs an Int step",
                                    args[0].value.span,
                                ));
                            }
                        };
                        let marker = self.local(LirType::Range);
                        self.ranges.insert(marker, (rlo, rhi, rincl, step));
                        return Ok((marker, Simple::Other));
                    }
                }
                if matches!(ty, LirType::Array(_)) && field == "push" {                    if args.len() != 1 {
                        return self.fail(self.err(Code::E108, "`push` takes 1 arg", span));
                    }
                    let v = argv.first().copied().ok_or_else(|| {
                        self.err(Code::E108, "`push` takes 1 arg", span)
                    })?;
                    let es = self.elem_size_of(obj);
                    let slot = match self.func.locals.get(obj as usize) {
                        Some(LirType::Array(inner)) => (**inner).clone(),
                        _ => LirType::Any,
                    };
                    self.deny_tuple(v, "in an array push", args[0].value.span)?;
                    let v = self.coerce_to_slot(v, &slot, args[0].value.span);
                    self.emit(Instr::ArrayPush { span,  arr: obj, value: v, elem_size: es });
                    let dst = self.local(LirType::Null);
                    self.emit(Instr::Const { span,  dst, lit: Lit::Null });
                    return Ok((dst, Simple::Other));
                }
                if matches!(ty, LirType::Str) {
                    if let Some(eid) = self.find_extension(&ty, field) {
                        return self.lower_ext_call(eid, obj, args, &argv, targs, span);
                    }
                    return self.lower_string_method(obj, field, argv, span);
                }
                if let LirType::Array(elem) = &ty {
                    if let Some(eid) = self.find_extension(&ty, field) {
                        return self.lower_ext_call(eid, obj, args, &argv, targs, span);
                    }
                    return self.lower_array_method(obj, elem.as_ref().clone(), field, args, argv, span);
                }
                if matches!(ty, LirType::I64 | LirType::F64(_) | LirType::Bool) {
                    if let Some(eid) = self.find_extension(&ty, field) {
                        return self.lower_ext_call(eid, obj, args, &argv, targs, span);
                    }
                    if matches!(ty, LirType::I64) && field == "join" {
                        if !args.is_empty() {
                            return self.fail(self.err(Code::E108, "`join` takes no args", span));
                        }
                        return self.lower_handle_result(obj, "__rnx_thread_join_val", "__rnx_thread_join_err", span);
                    }
                    return self.lower_prim_method(obj, &ty.clone(), field, argv, span);
                }
                if matches!(ty, LirType::Pointer(_)) && field == "join" {
                    if !args.is_empty() {
                        return self.fail(self.err(Code::E108, "`join` takes no args", span));
                    }
                    return self.lower_handle_result(obj, "__rnx_task_await_val", "__rnx_task_await_err", span);
                }
                if matches!(ty, LirType::Pointer(_)) && field == "await" {
                    return self.fail(self.err(
                        Code::E108,
                        "`task.await()` was renamed to `task.join()`; `await` is only a keyword for `Promise<T>`",
                        span,
                    ));
                }
                if let Some(eid) = self.find_extension(&ty, field) {
                    return self.lower_ext_call(eid, obj, args, &argv, targs, span);
                }
                let dst = self.local(LirType::Any);
                self.emit(Instr::Call { span, 
                    dsts: vec![dst],
                    err: None,
                    target: CallTarget::Dyn { obj, method: field.clone() },
                    args: argv,
                });
                Ok((dst, Simple::Other))
            }
            _ => {
                let (v, _) = self.lower_expr(&callee.node, callee.span)?;
                let mut argv = Vec::with_capacity(args.len());
                for a in args {
                    argv.push(self.lower_expr(&a.value.node, a.value.span)?.0);
                }
                let dst = self.local(LirType::Any);
                self.emit(Instr::Call { span, 
                    dsts: vec![dst],
                    err: None,
                    target: CallTarget::Value(v),
                    args: argv,
                });
                Ok((dst, Simple::Other))
            }
        }
    }
    pub(super) fn try_lower_implicit(
        &mut self,
        ei: usize,
        arg: &A::CallArg,
    ) -> Result<Option<(Local, Simple)>, Diagnostic> {
        match &arg.value.node {
            A::Expr::ImplicitMember(path) => {
                let vname = path.last().cloned().unwrap_or_default();
                let vi = self.module.enums[ei]
                    .variant_index
                    .get(&vname)
                    .copied()
                    .ok_or_else(|| self.unknown_variant(&vname, arg.value.span))?;
                let arity = self.module.enums[ei].variants[vi].payload.len();
                if arity != 0 {
                    return self.fail(self.err(
                        Code::E108,
                        format!("variant `.{vname}` takes {arity} payload args, use `.{vname}(...)`"),
                        arg.value.span,
                    ));
                }
                let dst = self.local(LirType::Enum(ei));
                self.emit(Instr::EnumNew { span: arg.value.span,
                    dst,
                    enu: ei,
                    variant: vi,
                    payload: Vec::new(),
                });
                Ok(Some((dst, Simple::Other)))
            }
            A::Expr::Call { callee, args: pargs, trailing: None, .. }
                if matches!(&callee.node, A::Expr::ImplicitMember(_)) =>
            {
                let vname = match &callee.node {
                    A::Expr::ImplicitMember(p) => p.last().cloned().unwrap_or_default(),
                    _ => String::new(),
                };
                let vi = self.module.enums[ei]
                    .variant_index
                    .get(&vname)
                    .copied()
                    .ok_or_else(|| self.unknown_variant(&vname, arg.value.span))?;
                self.lower_enum_construct(ei, vi, pargs, arg.value.span).map(Some)
            }
            _ => Ok(None),
        }
    }

    pub(super) fn lower_arg_with_slot(
        &mut self,
        slot: &LirType,
        arg: &A::CallArg,
    ) -> Result<(Local, Simple), Diagnostic> {
        if let LirType::Enum(ei) = slot {
            if let Some(v) = self.try_lower_implicit(*ei, arg)? {
                return Ok(v);
            }
        }
        self.lower_expr(&arg.value.node, arg.value.span)
    }

    pub(super) fn lower_call_arg(
        &mut self,
        declared: Option<&A::Type>,
        arg: &A::CallArg,
    ) -> Result<(Local, Simple), Diagnostic> {
        match declared {
            Some(d) => {
                let slot = self.resolve_here(d);
                self.lower_arg_with_slot(&slot, arg)
            }
            None => self.lower_expr(&arg.value.node, arg.value.span),
        }
    }

    pub(super) fn resolve_call_args(
        &mut self,
        id: usize,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Vec<Local>, BTreeMap<String, LirType>), Diagnostic> {
        let fname = self.module.functions[id].name.clone();
        let info = self.param_info.get(&fname).cloned().unwrap_or_default();
        if args.iter().all(|a| a.name.is_none()) && info.iter().all(|(_, d)| d.is_none()) {
            let mut argv = Vec::with_capacity(args.len());
            let mut prefix: Vec<(A::Type, LirType)> = Vec::new();
            let mut seed: BTreeMap<String, LirType> = BTreeMap::new();
            let ptys = self.param_tys.get(&fname).cloned().unwrap_or_default();
            for (i, a) in args.iter().enumerate() {
                let hint = match &a.value.node {
                    A::Expr::Closure { params, body, .. } => {
                        let h = self.closure_hint_for(&fname, i, &prefix);
                        if let Some(ref h) = h {
                            for (k, v) in self.closure_ret_seed(&fname, params, body, h, &prefix) {
                                seed.insert(k, v);
                            }
                        }
                        h
                    }
                    _ => None,
                };
                let pushed = hint.is_some();
                if let Some(h) = hint {
                    self.closure_hints.push(h);
                }
                let decl = ptys.get(i).map(|(_, t)| t.clone());
                let v = match decl {
                    Some(d) => self.lower_call_arg(Some(&d), a)?.0,
                    None => self.lower_expr(&a.value.node, a.value.span)?.0,
                };
                if pushed {
                    self.closure_hints.pop();
                }
                let actual = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                let decl = ptys.get(i).map(|(_, t)| t.clone()).unwrap_or_else(|| A::Type::named("Any"));
                prefix.push((decl, actual));
                argv.push(v);
            }
            return Ok((argv, seed));
        }
        let n = info.len();
        let ptys = self.param_tys.get(&fname).cloned().unwrap_or_default();
        let mut seed: BTreeMap<String, LirType> = BTreeMap::new();
        let mut slots: Vec<Option<Local>> = vec![None; n];
        let mut pos = 0usize;
        let mut named_started = false;
        let mut given = 0usize;
        for a in args {
            given += 1;
            if let Some(nm) = &a.name {
                named_started = true;
                let idx = info.iter().position(|(pn, _)| pn == nm).ok_or_else(|| {
                    self.err(Code::E108, format!("unknown parameter `{nm}`"), a.value.span)
                })?;
                if slots[idx].is_some() {
                    return self.fail(self.err(
                        Code::E108,
                        format!("duplicate argument for parameter `{nm}`"),
                        a.value.span,
                    ));
                }
                let hint = match &a.value.node {
                    A::Expr::Closure { params, body, .. } => {
                        let h = self.closure_hint_for(&fname, idx, &[]);
                        if let Some(ref h) = h {
                            for (k, v) in self.closure_ret_seed(&fname, params, body, h, &[]) {
                                seed.insert(k, v);
                            }
                        }
                        h
                    }
                    _ => None,
                };
                let pushed = hint.is_some();
                if let Some(h) = hint {
                    self.closure_hints.push(h);
                }
                let decl = ptys.get(idx).map(|(_, t)| t.clone());
                let (v, _) = match decl {
                    Some(d) => self.lower_call_arg(Some(&d), a)?,
                    None => self.lower_expr(&a.value.node, a.value.span)?,
                };
                if pushed {
                    self.closure_hints.pop();
                }
                slots[idx] = Some(v);
            } else {
                if named_started {
                    return self.fail(self.err(
                        Code::E108,
                        "positional argument follows a named argument",
                        a.value.span,
                    ));
                }
                while pos < n && slots[pos].is_some() {
                    pos += 1;
                }
                if pos >= n {
                    return self.fail(self.err(
                        Code::E108,
                        format!("call to `{fname}` takes {n} arguments but {given} were given"),
                        span,
                    ));
                }
                let hint = match &a.value.node {
                    A::Expr::Closure { params, body, .. } => {
                        let h = self.closure_hint_for(&fname, pos, &[]);
                        if let Some(ref h) = h {
                            for (k, v) in self.closure_ret_seed(&fname, params, body, h, &[]) {
                                seed.insert(k, v);
                            }
                        }
                        h
                    }
                    _ => None,
                };
                let pushed = hint.is_some();
                if let Some(h) = hint {
                    self.closure_hints.push(h);
                }
                let decl = ptys.get(pos).map(|(_, t)| t.clone());
                let (v, _) = match decl {
                    Some(d) => self.lower_call_arg(Some(&d), a)?,
                    None => self.lower_expr(&a.value.node, a.value.span)?,
                };
                if pushed {
                    self.closure_hints.pop();
                }
                slots[pos] = Some(v);
                pos += 1;
            }
        }
        let mut argv = Vec::with_capacity(n);
        for (i, (pn, dflt)) in info.iter().enumerate() {
            match slots[i] {
                Some(v) => argv.push(v),
                None => match dflt {
                    Some(d) => {
                        let (v, _) = self.lower_expr(&d.node, d.span)?;
                        argv.push(v);
                    }
                    None => {
                        return self.fail(self.err(
                            Code::E108,
                            format!("missing argument for parameter `{pn}`"),
                            span,
                        ));
                    }
                },
            }
        }
        Ok((argv, seed))
    }
    pub(super) fn unify_generic(
        &self,
        tparams: &[String],
        declared: &A::Type,
        actual: &LirType,
        subst: &mut BTreeMap<String, LirType>,
    ) -> Result<(), String> {
        if declared.fn_sig.is_none()
            && declared.args.is_empty()
            && declared.tuple.is_empty()
            && declared.path.len() == 1
        {
            let n = &declared.path[0];
            if tparams.iter().any(|p| p == n) {
                match subst.get(n) {
                    Some(prev) if prev != actual => {
                        if matches!(actual, LirType::Any | LirType::Null) {
                            return Ok(());
                        }
                        if matches!(prev, LirType::Null) {
                            subst.insert(n.clone(), actual.clone());
                            return Ok(());
                        }
                        return Err(format!("conflicting types for `{n}`"));
                    }
                    Some(_) => return Ok(()),
                    None => {
                        subst.insert(n.clone(), actual.clone());
                        return Ok(());
                    }
                }
            }
        }
        if !declared.tuple.is_empty() {
            if let LirType::Tuple(tys) = actual {
                if tys.len() != declared.tuple.len() {
                    return Err("tuple arity does not match".to_string());
                }
                for (dt, at) in declared.tuple.iter().zip(tys.iter()) {
                    self.unify_generic(tparams, dt, at, subst)?;
                }
                return Ok(());
            }
            return Ok(());
        }
        if let Some(first) = declared.path.first() {
            if (first == "Array" || first.ends_with(".Array")) && !declared.args.is_empty() {
                if let LirType::Array(elem) = actual {
                    return self.unify_generic(tparams, &declared.args[0], elem, subst);
                }
                return Ok(());
            }
        }
        let d = resolve_ty(self.module, declared);
        if d == *actual || *actual == LirType::Any {
            return Ok(());
        }
        Err("argument type does not match parameter type".to_string())
    }
    pub(super) fn call_fn_ex(
        &mut self,
        id: usize,
        targs: &[A::Type],
        argv: Vec<Local>,
        seed: &BTreeMap<String, LirType>,
        span: Span,
        self_arg: Option<Local>,
    ) -> Result<(Local, Simple), Diagnostic> {
        self.solve_generic(id, targs, &argv, seed, span)?;
        if self.module.functions[id].is_unsafe && self.unsafe_depth == 0 {
            return self.fail(self.err(
                Code::E201,
                format!(
                    "call to unsafe function `{}` needs `unsafe`",
                    self.module.functions[id].name
                ),
                span,
            ));
        }
        let throws = self.module.functions[id].throws;
        let ret = self.module.functions[id].ret.clone();
        let sig_params = self.module.functions[id].sig_params.clone();
        let params = self.module.functions[id].params.clone();
        let mut saw_tuple = matches!(ret, LirType::Tuple(_))
            || matches!(ret, LirType::Range)
            || sig_params.iter().any(|p| matches!(p, LirType::Tuple(_) | LirType::Range));
        let mut grouped: Vec<Local> = Vec::with_capacity(argv.len());
        if argv.len() != sig_params.len() && saw_tuple {
            return self.fail(self.err(
                Code::E108,
                format!(
                    "call to `{}` takes {} arguments but {} were given",
                    self.module.functions[id].name,
                    sig_params.len(),
                    argv.len()
                ),
                span,
            ));
        }
        if argv.len() == sig_params.len() {
            for (a, pty) in argv.iter().zip(sig_params.iter()) {
                if matches!(pty, LirType::Tuple(_) | LirType::Range) {
                    saw_tuple = true;
                    let want_range = matches!(pty, LirType::Range);
                    let has_it = if want_range {
                        self.ranges.contains_key(a)
                    } else {
                        self.tuples.contains_key(a)
                    };
                    if !has_it {
                        return self.fail(self.err(
                            Code::E108,
                            if want_range {
                                "range parameter needs a range argument"
                            } else {
                                "tuple parameter needs a tuple argument"
                            },
                            span,
                        ));
                    }
                    let elems = self.expand_value(*a, span)?;
                    let slots = crate::instr::flat_sig(pty);
                    if elems.len() != slots.len() {
                        return self.fail(self.err(
                            Code::E108,
                            format!(
                                "tuple argument has {} values but the parameter needs {}",
                                elems.len(),
                                slots.len()
                            ),
                            span,
                        ));
                    }
                    for (el, st) in elems.into_iter().zip(slots.iter()) {
                        grouped.push(self.coerce_to_slot(el, st, span));
                    }
                } else {
                    self.deny_tuple(*a, "as a call argument", span)?;
                    if Some(*a) != self_arg && !self.fresh_arrays.contains(a) {
                        let aty = self.func.locals.get(*a as usize).cloned().unwrap_or(LirType::Any);
                        if let (LirType::Array(_), LirType::Array(_)) = (&aty, pty) {
                            let name = self.module.functions[id].name.clone();
                            self.check_array_view(&aty, pty, false, span, &format!("argument {} of `{name}`", grouped.len() + 1))?;
                        }
                    }
                    grouped.push(*a);
                }
            }
        } else {
            for a in argv.iter() {
                self.deny_tuple(*a, "as a call argument", span)?;
            }
            grouped = argv;
        }
        if saw_tuple && grouped.len() != params.len() {
            return self.fail(self.err(
                Code::E108,
                format!(
                    "call to `{}` passes {} values but the signature needs {}",
                    self.module.functions[id].name,
                    grouped.len(),
                    params.len()
                ),
                span,
            ));
        }
        let fname = self.module.functions[id].name.clone();
        let ptys = self.param_tys.get(&fname).cloned().unwrap_or_default();
        let argv = grouped
            .into_iter()
            .enumerate()
            .map(|(i, a)| {
                if ptys.get(i).is_some_and(|(_, t)| nub_ty(t)) {
                    let slot = params.get(i).cloned().unwrap_or(LirType::Any);
                    self.nub_arg(a, &slot, span)
                } else {
                    let slot = params.get(i).cloned().unwrap_or(LirType::Any);
                    if self.nub_has(a) && slot == LirType::Any {
                        if self.nub_scope_has(a) {
                            self.nub_retain(a, span);
                        } else {
                            self.nub_unmark(a);
                        }
                        a
                    } else {
                        let a = self.nub_use(a, span);
                        self.coerce_to_slot(a, &slot, span)
                    }
                }
            })
            .collect::<Vec<_>>();
        if ret == LirType::Void && !throws {
            self.emit(Instr::Call { span, 
                dsts: vec![],
                err: None,
                target: CallTarget::Fn(id),
                args: argv,
            });
            let dst = self.local(LirType::Null);
            self.emit(Instr::Const { span,  dst, lit: Lit::Null });
            return Ok((dst, Simple::Other));
        }
        let is_tuple_ret = matches!(ret, LirType::Tuple(_));
        let is_range_ret = matches!(ret, LirType::Range);
        let dsts: Vec<Local> = if is_tuple_ret || is_range_ret {
            crate::instr::flat_sig(&ret)
                .iter()
                .map(|t| self.local(t.clone()))
                .collect()
        } else {
            vec![self.local(ret.clone())]
        };
        if !is_tuple_ret
            && !is_range_ret
            && self.ret_tys.get(&fname).is_some_and(|t| nub_ty(t))
        {
            for d in dsts.iter() {
                self.nub_mark(*d);
            }
        }
        self.emit_checked_call(id, &dsts, &argv, span)?;
        if is_tuple_ret {
            let marker = self.local(ret.clone());
            self.tuples.insert(marker, dsts);
            return Ok((marker, Simple::Other));
        }
        if !is_range_ret {
            let fname = self.module.functions[id].name.clone();
            let rt_opt = self.ret_tys.get(&fname).cloned().or_else(|| {
                let short = short_name(&fname).to_string();
                self.ret_tys.get(&short).cloned()
            });
            if let Some(rt) = rt_opt {
                if let Some(arg) = option_arg(&rt) {
                    let bare_param = arg.fn_sig.is_none()
                        && arg.args.is_empty()
                        && arg.tuple.is_empty()
                        && arg.path.len() == 1
                        && self.generic_fns.get(&fname).map(|g| g.tparams.iter().any(|p| p == &arg.path[0])).or_else(|| {
                            let short = short_name(&fname).to_string();
                            self.generic_fns.get(&short).map(|g| g.tparams.iter().any(|p| p == &arg.path[0]))
                        }).unwrap_or(false);
                    if !bare_param {
                        let pt = self.resolve_here(arg);
                        if !matches!(pt, LirType::Any) && type_resolved(self.module, &pt) {
                            for d in &dsts {
                                self.precise.insert(*d, pt.clone());
                            }
                        }
                    }
                }
            }
        }
        if is_range_ret {
            let marker = self.local(LirType::Range);
            if let [lo, hi, incl, step] = dsts.as_slice() {
                self.ranges.insert(marker, (*lo, *hi, *incl, *step));
            }
            return Ok((marker, Simple::Other));
        }
        Ok((dsts[0], simple_of(&ret)))
    }
    pub(super) fn emit_checked_call(
        &mut self,
        id: usize,
        dsts: &[Local],
        argv: &[Local],
        span: Span,
    ) -> Result<(), Diagnostic> {
        let throws = self.module.functions[id].throws;
        if throws {
            let err = self.local(LirType::Error);
            let target = self.catch_stack.iter().rev().find_map(|c| {
                c.catch_bb.map(|bb| (bb, c.err_local, c.defer_depth))
            });
            if let Some((catch_bb, catch_bind, depth)) = target {
                let next = self.new_block();
                self.emit(Instr::Call { span, 
                    dsts: dsts.to_vec(),
                    err: Some(err),
                    target: CallTarget::Fn(id),
                    args: argv.to_vec(),
                });
                self.emit_run_defers(depth);
                let cur = self.current;
                self.set_term(Terminator::BrErr { span, 
                    err,
                    catch_bb,
                    catch_bind,
                    next_bb: next,
                    depth,
                });
                let _ = (cur, span);
                self.set_current(next);
            } else if self.throws_fn {
                let next = self.new_block();
                let prop = self.new_block();
                self.emit(Instr::Call { span, 
                    dsts: dsts.to_vec(),
                    err: Some(err),
                    target: CallTarget::Fn(id),
                    args: argv.to_vec(),
                });
                let bind = self.local(LirType::Error);
                self.set_term(Terminator::BrErr { span, 
                    err,
                    catch_bb: prop,
                    catch_bind: bind,
                    next_bb: next,
                    depth: self.defer_count,
                });
                self.set_current(prop);
                self.emit_run_defers(0);
                self.set_term(Terminator::Throw { span,  src: bind, catch: None });
                self.set_current(next);
            } else {
                self.emit(Instr::Call { span, 
                    dsts: dsts.to_vec(),
                    err: Some(err),
                    target: CallTarget::Fn(id),
                    args: argv.to_vec(),
                });
                let merge = self.new_block();
                let trap = self.new_block();
                let bind = self.local(LirType::Error);
                self.set_term(Terminator::BrErr { span, 
                    err,
                    catch_bb: trap,
                    catch_bind: bind,
                    next_bb: merge,
                    depth: self.defer_count,
                });
                self.set_current(trap);
                self.emit_run_defers(0);
                self.set_term(Terminator::Throw { span,  src: bind, catch: None });
                self.set_current(merge);
            }
        } else {
            self.emit(Instr::Call { span, 
                dsts: dsts.to_vec(),
                err: None,
                target: CallTarget::Fn(id),
                args: argv.to_vec(),
            });
        }
        Ok(())
    }
    pub(super) fn lower_map_literal(
        &mut self,
        entries: &[A::MapEntry],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let ci = match self
            .module
            .class_index
            .iter()
            .filter(|(n, _)| short_name(n) == "Map")
            .map(|(_, ci)| *ci)
            .find(|ci| self.module.classes[*ci].field_index.contains_key("handle"))
        {
            Some(ci) => ci,
            None => {
                return self.fail(self.err(
                    Code::E108,
                    "map literals need `import { Map } from \"@std/collections\"`",
                    span,
                ));
            }
        };
        let (cname, handle_field) = {
            let class = &self.module.classes[ci];
            (class.name.clone(), class.field_index["handle"])
        };
        let obj = self.local(LirType::Obj(cname));
        self.emit(Instr::ObjNew {
            span,
            dst: obj,
            class: ci,
            instance_size: instance_size(self.module.classes[ci].fields.len()),
        });
        let handle = self.local(LirType::I64);
        self.emit(Instr::Call {
            span,
            dsts: vec![handle],
            err: None,
            target: CallTarget::Builtin("__rnx_gmap_new".to_string()),
            args: vec![],
        });
        self.emit(Instr::SetField { span, obj, field: handle_field, value: handle });
        for e in entries {
            let (key, v) = match e {
                A::MapEntry::Field(k, v) => (k, v),
                A::MapEntry::Spread(v) => {
                    return self.fail(self.err(
                        Code::E108,
                        "`...spread` in map literals is only supported in Project.config",
                        v.span,
                    ));
                }
            };
            let kl = self.local(LirType::Str);
            self.emit(Instr::Const { span, dst: kl, lit: Lit::Str(key.clone()) });
            let kl = self.coerce_to_slot(kl, &LirType::Any, span);
            let (e, _) = self.lower_expr(&v.node, v.span)?;
            let e = self.coerce_to_slot(e, &LirType::Any, v.span);
            self.emit(Instr::Call {
                span,
                dsts: vec![],
                err: None,
                target: CallTarget::Builtin("__rnx_gmap_set".to_string()),
                args: vec![handle, kl, e],
            });
        }
        Ok((obj, Simple::Other))
    }
    pub(super) fn lower_new(
        &mut self,
        target: &str,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        if let Some(kind) = vec_name(target) {
            if self.module.class_index.contains_key(target) {
                return self.lower_vec_new(kind, args, span);
            }
        }
        if let Some(ci) = self.module.class_index.get(target).copied() {
            if self.module.classes[ci].is_struct {
                return self.fail(
                    self.err(
                        Code::E108,
                        format!("`{target}` is a struct and cannot be built with `new`"),
                        span,
                    )
                    .with_hint(format!("construct it with `{target}(...)` directly")),
                );
            }
            return self.lower_construct(ci, args, span);
        }
        if self.module.interface_index.contains_key(target)
            || self.module.interface_index.contains_key(short_name(target))
        {
            return self.fail(self.err(
                Code::E108,
                format!("cannot construct interface `{target}`"),
                span,
            ));
        }
        if self.variant_of(target).is_some() || self.module.enum_index.contains_key(target) {
            return self.fail(self.err(
                Code::E108,
                format!("cannot use `new` on enum `{target}`"),
                span,
            ));
        }
        self.fail(self.err(Code::E108, format!("unknown class `{target}`"), span))
    }
    pub(super) fn lower_enum_construct(
        &mut self,
        enu: usize,
        variant: usize,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let want = self.module.enums[enu].variants[variant].payload.len();
        let vname = self.module.enums[enu].variants[variant].name.clone();
        if args.len() != want {
            return self.fail(self.err(
                Code::E108,
                format!("variant `{vname}` takes {want} payload args, got {}", args.len()),
                span,
            ));
        }
        let mut payload = Vec::with_capacity(args.len());
        let slots = self.module.enums[enu].variants[variant].payload.clone();
        let mut raw_tys: Vec<LirType> = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let v = self.lower_expr(&a.value.node, a.value.span)?.0;
            self.deny_tuple(v, "as an enum payload", a.value.span)?;
            raw_tys.push(self.func.locals[v as usize].clone());
            let slot = slots.get(i).cloned().unwrap_or(LirType::Any);
            payload.push(self.coerce_to_slot(v, &slot, a.value.span));
        }
        let dst = self.local(LirType::Enum(enu));
        self.emit(Instr::EnumNew { span,
            dst,
            enu,
            variant,
            payload,
        });
        let ename = self.module.enums[enu].name.clone();
        if ename == "std.prelude.Result" && raw_tys.len() == 1 {
            if let Some(rt) = raw_tys.first().cloned() {
                self.precise.insert(dst, rt);
            }
        }
        Ok((dst, Simple::Other))
    }
    pub(super) fn pointer_scalar(&mut self, pty: &LirType, span: Span) -> Result<LirType, Diagnostic> {
        match pty {
            LirType::Pointer(inner) => match inner.as_ref() {
                LirType::I64 | LirType::I8 | LirType::F64(_) | LirType::Bool => Ok(inner.as_ref().clone()),
                LirType::Any => self.fail(self.err(
                    Code::E108,
                    "pointer dereference needs a `Pointer<Int>`, `Pointer<Float>`, `Pointer<Byte>`, or `Pointer<Bool>`",
                    span,
                ).with_hint("annotate the pointer type or use `Pointer.fromAddress<T>(addr)`")),
                _ => self.fail(self.err(
                    Code::E108,
                    "pointer dereference needs an Int, Float, Byte, or Bool pointee",
                    span,
                )),
            },
            _ => self.fail(self.err(
                Code::E108,
                "dereference needs a `Pointer` operand",
                span,
            )),
        }
    }
    pub(super) fn lower_macro(
        &mut self,
        name: &str,
        args: &[A::Spanned<A::Expr>],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        match name {
            "Assert" => {
                if args.is_empty() {
                    return self.fail(self.err(Code::E108, "`Assert!` needs a condition", span));
                }
                let (c, _) = self.lower_expr(&args[0].node, args[0].span)?;
                let msg = if args.len() > 1 {
                    self.lower_expr(&args[1].node, args[1].span)?.0
                } else {
                    let m = self.local(LirType::Str);
                    self.emit(Instr::Const { span, 
                        dst: m,
                        lit: Lit::Str("assertion failed".to_string()),
                    });
                    m
                };
                self.emit(Instr::Assert { span,  cond: c, message: msg });
                let dst = self.local(LirType::Null);
                self.emit(Instr::Const { span,  dst, lit: Lit::Null });
                Ok((dst, Simple::Other))
            }
            "StaticAssert" => {
                let dst = self.local(LirType::Null);
                self.emit(Instr::Const { span,  dst, lit: Lit::Null });
                Ok((dst, Simple::Other))
            }
            "PanicIf" => {
                if args.is_empty() {
                    return self.fail(self.err(Code::E108, "`PanicIf!` needs a condition", span));
                }
                let (c, _) = self.lower_expr(&args[0].node, args[0].span)?;
                let msg = if args.len() > 1 {
                    self.lower_expr(&args[1].node, args[1].span)?.0
                } else {
                    let m = self.local(LirType::Str);
                    self.emit(Instr::Const { span, 
                        dst: m,
                        lit: Lit::Str("panic".to_string()),
                    });
                    m
                };
                let panic_bb = self.new_block();
                let cont = self.new_block();
                self.set_term(Terminator::BrIf { span, 
                    cond: c,
                    then_bb: panic_bb,
                    else_bb: cont,
                });
                self.set_current(panic_bb);
                let catch = self.catch_stack.iter().rev().find_map(|c| {
                    c.catch_bb.map(|bb| (bb, c.err_local, c.defer_depth))
                });
                self.set_term(Terminator::Throw { span,  src: msg, catch });
                self.set_current(cont);
                let dst = self.local(LirType::Null);
                self.emit(Instr::Const { span,  dst, lit: Lit::Null });
                Ok((dst, Simple::Other))
            }
            "vec" => {
                let dst = self.local(LirType::Array(Box::new(LirType::Any)));
                self.emit(Instr::ArrayNew { span,  dst, cap: 0, elem_size: 8 });
                for a in args {
                    let (v, _) = self.lower_expr(&a.node, a.span)?;
                    self.deny_tuple(v, "in an array literal", a.span)?;
                    let v = self.coerce_to_slot(v, &LirType::Any, a.span);
                    self.emit(Instr::ArrayPush { span,  arr: dst, value: v, elem_size: 8 });
                }
                Ok((dst, Simple::Other))
            }
            "t" => {
                if args.is_empty() {
                    return self.fail(self.err(Code::E108, "`t!` needs a string", span));
                }
                self.lower_expr(&args[0].node, args[0].span)
            }
            _ => self.fail(self.err(Code::E108, format!("macro `{name}!` pending"), span)),
        }
    }
}
