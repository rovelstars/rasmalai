use super::*;
use diagnostics::{Code, Diagnostic, Span};
use frontend::ast as A;
use std::collections::{BTreeMap};

impl<'a> Builder<'a> {
    pub(super) fn closure_ret_seed(
        &self,
        fname: &str,
        params: &[A::Param],
        body: &A::FnBody,
        hints: &[LirType],
        prefix: &[(A::Type, LirType)],
    ) -> BTreeMap<String, LirType> {
        let mut seed = BTreeMap::new();
        let g = match self.generic_fns.get(fname).cloned() {
            Some(g) => g,
            None => return seed,
        };
        if g.tparams.is_empty() {
            return seed;
        }
        for (decl, actual) in prefix.iter() {
            let _ = self.unify_generic(&g.tparams, decl, actual, &mut seed);
        }
        let r = match infer_closure_ret(body, params, hints, &|n| scope_ty_of(&self.scopes, n)) {
            Some(r) => r,
            None => return seed,
        };
        let _ = self.unify_generic(&g.tparams, &g.ret, &r, &mut seed);
        if g.ret.args.len() == 1
            && g.ret.fn_sig.is_none()
            && g.ret.tuple.is_empty()
            && g.ret.args[0].fn_sig.is_none()
            && g.ret.args[0].args.is_empty()
            && g.ret.args[0].tuple.is_empty()
            && g.ret.args[0].path.len() == 1
            && g.tparams.iter().any(|p| p == &g.ret.args[0].path[0])
            && !matches!(r, LirType::Any | LirType::Array(_) | LirType::Tuple(_) | LirType::Enum(_))
            && seed.get(&g.ret.args[0].path[0]).is_none()
        {
            seed.insert(g.ret.args[0].path[0].clone(), r);
        }
        seed
    }
    pub(super) fn solve_generic(
        &self,
        id: usize,
        targs: &[A::Type],
        argv: &[Local],
        seed: &BTreeMap<String, LirType>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let fname = self.module.functions[id].name.clone();
        let g = match self.generic_fns.get(&fname).cloned() {
            Some(g) => g,
            None => {
                if !targs.is_empty() {
                    return Err(self.err(
                        Code::E108,
                        format!("`{fname}` is not generic and takes no type arguments"),
                        span,
                    ));
                }
                return Ok(());
            }
        };
        let mut subst: BTreeMap<String, LirType> = seed.clone();
        if !targs.is_empty() {
            if targs.len() != g.tparams.len() {
                return Err(self.err(
                    Code::E108,
                    format!(
                        "`{fname}` takes {} type arguments but {} were given",
                        g.tparams.len(),
                        targs.len()
                    ),
                    span,
                ));
            }
            for (tp, ta) in g.tparams.iter().zip(targs.iter()) {
                let rt = self.resolve_explicit(ta, span)?;
                subst.insert(tp.clone(), rt);
            }
        }
        let off = if self.module.functions[id].method_self && argv.len() == g.params.len() + 1 {
            1
        } else {
            0
        };
        for (i, (_, pt)) in g.params.iter().enumerate() {
            let Some(a) = argv.get(i + off) else { continue };
            let actual = self.func.locals.get(*a as usize).cloned().unwrap_or(LirType::Any);
            if let Err(msg) = self.unify_generic(&g.tparams, pt, &actual, &mut subst) {
                return Err(self.err(Code::E108, msg, span));
            }
        }
        for tp in &g.tparams {
            if !subst.contains_key(tp) {
                return Err(self.err(
                    Code::E108,
                    format!("cannot infer type parameter `{tp}` for `{fname}`; pass explicit type arguments"),
                    span,
                ));
            }
        }
        Ok(())
    }
    pub(super) fn resolve_explicit(&self, t: &A::Type, span: Span) -> Result<LirType, Diagnostic> {
        if !t.tuple.is_empty() {
            let mut items = Vec::with_capacity(t.tuple.len());
            for it in &t.tuple {
                items.push(self.resolve_explicit(it, span)?);
            }
            return Ok(LirType::Tuple(items));
        }
        if t.fn_sig.is_some() {
            return Ok(LirType::Closure);
        }
        if t.path.len() == 1 {
            let n = &t.path[0];
            let short = n.rsplit('.').next().unwrap_or(n);
            let known_prim = matches!(
                short,
                "Int" | "Int64" | "Int32" | "Int16" | "Int8" | "UInt" | "UInt64" | "UInt32"
                    | "UInt16" | "UInt8" | "Byte" | "Short" | "Float" | "Float32" | "FastFloat"
                    | "FastFloat32" | "Bool" | "String" | "Char" | "Array" | "Range" | "Any"
                    | "Pointer" | "Address" | "Void" | "Vec4f" | "Vec4i"
            );
            let known = known_prim
                || self.module.class_index.contains_key(n)
                || self.module.enum_index.contains_key(n)
                || self.module.interface_index.contains_key(n)
                || self.fn_tparams.iter().any(|p| p == n || p == short);
            if !known && t.args.is_empty() {
                let mut found = false;
                for (k, _) in self.module.class_index.iter() {
                    if k.rsplit('.').next().unwrap_or(k) == short {
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(self.err(
                        Code::E108,
                        format!("unknown type `{n}` in type arguments"),
                        span,
                    ));
                }
            }
        }
        if let Some(first) = t.path.first() {
            if first == "Array" || first.ends_with(".Array") {
                if let Some(a) = t.args.first() {
                    let inner = self.resolve_explicit(a, span)?;
                    return Ok(LirType::Array(Box::new(inner)));
                }
            }
        }
        Ok(resolve_ty(self.module, t))
    }
    pub(super) fn call_fn(
        &mut self,
        id: usize,
        targs: &[A::Type],
        argv: Vec<Local>,
        seed: &BTreeMap<String, LirType>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        self.call_fn_ex(id, targs, argv, seed, span, None)
    }
    pub(super) fn call_foreign(
        &mut self,
        fid: usize,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let f = self.module.foreign[fid].clone();
        if args.len() != f.params.len() {
            return self.fail(self.err(
                Code::E108,
                format!(
                    "foreign call to `{}` takes {} arguments but {} were given",
                    f.symbol,
                    f.params.len(),
                    args.len()
                ),
                span,
            ));
        }
        if args.iter().any(|a| a.name.is_some()) {
            return self.fail(self.err(
                Code::E108,
                "foreign calls take positional arguments only",
                span,
            ));
        }
        let mut argv = Vec::with_capacity(args.len());
        for (a, want) in args.iter().zip(f.params.iter()) {
            let (v, _) = self.lower_expr(&a.value.node, a.value.span)?;
            self.deny_tuple(v, "as a foreign call argument", a.value.span)?;
            let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
            match &vt {
                LirType::I64 | LirType::I8 | LirType::F64(_) | LirType::Bool | LirType::Pointer(_) => {}
                _ => {
                    return self.fail(self.err(
                        Code::E108,
                        "foreign calls need Int, Float, Byte, Bool, or Pointer arguments, not managed heap objects",
                        a.value.span,
                    ).with_hint("pass raw addresses or primitives, not managed heap objects"));
                }
            }
            argv.push(self.coerce_to_slot(v, want, a.value.span));
        }
        if f.ret == LirType::Void {
            self.emit(Instr::Call { span,
                dsts: vec![],
                err: None,
                target: CallTarget::Foreign { lib: f.lib.clone(), symbol: f.symbol.clone() },
                args: argv,
            });
            let dst = self.local(LirType::Null);
            self.emit(Instr::Const { span,  dst, lit: Lit::Null });
            return Ok((dst, Simple::Other));
        }
        let dst = self.local(f.ret.clone());
        self.emit(Instr::Call { span,
            dsts: vec![dst],
            err: None,
            target: CallTarget::Foreign { lib: f.lib.clone(), symbol: f.symbol.clone() },
            args: argv,
        });
        Ok((dst, simple_of(&f.ret)))
    }
    pub(super) fn lower_construct(
        &mut self,
        ci: usize,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {        let obj = self.local(LirType::Obj(self.module.classes[ci].name.clone()));
        self.emit(Instr::ObjNew { span, 
                    dst: obj,
                    class: ci,
                    instance_size: instance_size(self.module.classes[ci].fields.len()),
                });
        let cname = self.module.classes[ci].name.clone();
        let save_self = self.self_local;
        self.self_local = Some(obj);
        let mut r = Ok(());
        if self.module.fn_index.contains_key(&format!("{cname}.init")) {
            r = self.lower_field_defaults(&cname, obj);
        } else {
            let mut parent_call: Option<usize> = None;
            if let Some(pi) = self.module.classes[ci].parent {
                let pname = self.module.classes[pi].name.clone();
                if let Some(pid) = self.module.fn_index.get(&format!("{pname}.init")).copied() {
                    let n = self.module.functions[pid].sig_params.len().saturating_sub(1);
                    if n > 0 {
                        r = Err(self.err(Code::E108, format!("`{cname}` must define `init` calling `super(...)`: base `{pname}.init` requires arguments"), span));
                    } else {
                        parent_call = Some(pid);
                    }
                }
            }
            let nfields = self.module.classes[ci].fields.len();
            if args.len() == nfields && nfields > 0 {
                self.self_local = save_self;
                for (i, a) in args.iter().enumerate() {
                    let slot = self.module.classes[ci].fields.get(i).map(|f| f.ty.clone()).unwrap_or(LirType::Any);
                    match self.lower_arg_with_slot(&slot, a) {
                        Ok((v, _)) => {
                            let v = self.coerce_to_slot(v, &slot, a.value.span);
                            self.emit(Instr::SetField { span,  obj, field: i, value: v })
                        }
                        Err(e) => {
                            r = Err(e);
                            break;
                        }
                    }
                }
                self.self_local = Some(obj);
            } else if !args.is_empty() {
                r = Err(self.err(Code::E108, format!("`{cname}` takes {nfields} arguments but {} were given", args.len()), span));
            } else {
                r = self.lower_field_defaults(&cname, obj);
            }
            if r.is_ok() {
                if let Some(pid) = parent_call {
                    if let Err(e) = self.emit_checked_call(pid, &[], &[obj], span) {
                        r = Err(e);
                    }
                }
            }
        }
        self.self_local = save_self;
        r?;
        if let Some(id) = self.module.fn_index.get(&format!("{cname}.init")).copied() {
            let mname = format!("{cname}.init");
            let needs_resolve = args.iter().any(|a| a.name.is_some())
                || self.param_info.get(&mname).map(|p| p.iter().any(|(_, d)| d.is_some())).unwrap_or(false);
            let mut argv = vec![obj];
            if needs_resolve {
                let (rv, _) = self.resolve_call_args(id, args, span)?;
                argv.extend(rv);
            } else {
                let ptys = self.param_tys.get(&mname).cloned().unwrap_or_default();
                for (i, a) in args.iter().enumerate() {
                    match ptys.get(i) {
                        Some((_, d)) => argv.push(self.lower_call_arg(Some(d), a)?.0),
                        None => argv.push(self.lower_expr(&a.value.node, a.value.span)?.0),
                    }
                }
            }
            self.emit_checked_call(id, &[], &argv, span)?;
        }
        Ok((obj, Simple::Other))
    }
    pub(super) fn lower_json_parse_typed(
        &mut self,
        args: &[A::CallArg],
        targs: &[A::Type],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        if targs.len() != 1 {
            return self.fail(self.err(
                Code::E108,
                "`JSON.parse` takes exactly 1 type argument (the Record or Class to decode)",
                span,
            ));
        }
        if args.len() != 1 || args.iter().any(|a| a.name.is_some()) {
            return self.fail(self.err(
                Code::E108,
                "`JSON.parse<T>` takes exactly 1 argument (the JSON text)",
                span,
            ));
        }
        let ty = self.resolve_explicit(&targs[0], span)?;
        let short = match &ty {
            LirType::Obj(n) => n.clone(),
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    "`JSON.parse<T>` needs a Record or Class type argument",
                    span,
                ));
            }
        };
        let mut hit = None;
        for (name, id) in self.module.class_index.iter() {
            if short_name(name) == short.as_str() {
                if hit.is_some() {
                    return self.fail(self.err(
                        Code::E108,
                        format!("ambiguous type argument `{short}` for `JSON.parse<T>`"),
                        span,
                    ));
                }
                hit = Some(*id);
            }
        }
        let ci = match hit {
            Some(ci) => ci,
            None => {
                return self.fail(self.err(
                    Code::E108,
                    format!("unknown type `{short}` in `JSON.parse<T>`"),
                    span,
                ));
            }
        };
        let (cname, fields) = {
            let class = &self.module.classes[ci];
            if !class.type_params.is_empty() {
                return self.fail(self.err(
                    Code::E108,
                    "`JSON.parse<T>` needs a concrete Record or Class (no type parameters)",
                    span,
                ));
            }
            (class.name.clone(), class.fields.clone())
        };
        if self.module.fn_index.contains_key(&format!("{cname}.init")) {
            return self.fail(self.err(
                Code::E108,
                format!("`JSON.parse<{short}>` needs a Record or Class without a custom `init`; decode with `JSON.parse` and construct manually"),
                span,
            ));
        }
        if fields.is_empty() {
            return self.fail(self.err(
                Code::E108,
                format!("`JSON.parse<{short}>` needs at least one field to decode into"),
                span,
            ));
        }
        let mut desc = String::with_capacity(fields.len() * 8);
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                desc.push(';');
            }
            for c in f.name.chars() {
                if matches!(c, '\\' | ';' | ':') {
                    desc.push('\\');
                }
                desc.push(c);
            }
            desc.push(':');
            desc.push(match &f.ty {
                LirType::I64 => 'i',
                LirType::F64(_) => 'f',
                LirType::Bool => 'b',
                LirType::Str => 's',
                LirType::Array(_) => 'a',
                _ => 'v',
            });
        }
        let (textv, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
        let descv = self.local(LirType::Str);
        self.emit(Instr::Const {
            span,
            dst: descv,
            lit: Lit::Str(desc),
        });
        let arrv = self.local(LirType::Array(Box::new(LirType::Any)));
        self.emit(Instr::Call {
            span,
            dsts: vec![arrv],
            err: None,
            target: CallTarget::Builtin("__rnx_json_parse_typed".to_string()),
            args: vec![textv, descv],
        });
        let obj = self.local(LirType::Obj(cname));
        self.emit(Instr::ObjNew {
            span,
            dst: obj,
            class: ci,
            instance_size: instance_size(fields.len()),
        });
        for (i, f) in fields.iter().enumerate() {
            let idx = self.const_int(i as i64, span);
            let ev = self.local(LirType::Any);
            self.emit(Instr::ArrayGet {
                span,
                dst: ev,
                arr: arrv,
                index: idx,
                elem_size: 8,
                unchecked: false,
            });
            let cv = self.coerce_to_slot(ev, &f.ty, span);
            self.emit(Instr::SetField {
                span,
                obj,
                field: i,
                value: cv,
            });
        }
        Ok((obj, Simple::Other))
    }
    pub(super) fn lower_field_defaults(&mut self, cname: &str, obj: Local) -> Result<(), Diagnostic> {
        let defaults = self.defaults.get(cname).cloned().unwrap_or_default();
        for (field, expr) in defaults {
            let (v, _) = self.lower_expr(&expr.node, expr.span)?;
            let (fi, slot) = self.module.classes
                .iter()
                .find(|c| c.name == cname)
                .and_then(|c| c.field_index.get(&field).map(|fi| (*fi, c.fields[*fi].ty.clone())))
                .ok_or_else(|| self.err(Code::E108, format!("lost field `{field}`"), expr.span))?;
            let v = self.coerce_to_slot(v, &slot, expr.span);
            self.emit(Instr::SetField { span: expr.span, obj, field: fi, value: v });
        }
        Ok(())
    }
    pub(super) fn lower_genref(
        &mut self,
        field: &str,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        match field {
            "of" => {
                if args.len() != 1 {
                    return self.fail(self.err(Code::E108, "`GenRef.of` takes 1 arg", span));
                }
                let (o, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
                let dst = self.local(LirType::GenRef(None));
                self.emit(Instr::GenRefOf { span,  dst, obj: o });
                Ok((dst, Simple::Other))
            }
            "empty" => {
                let dst = self.local(LirType::GenRef(None));
                self.emit(Instr::GenRefEmpty { span,  dst });
                Ok((dst, Simple::Other))
            }
            _ => self.fail(self.err(Code::E108, format!("unknown `GenRef.{field}`"), span)),
        }
    }
    pub(super) fn lower_vec_new(
        &mut self,
        kind: VecKind,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let name = if kind == VecKind::F { "Vec4f" } else { "Vec4i" };
        if args.len() != 4 {
            return self.fail(self.err(Code::E108, format!("`{name}` takes 4 lanes"), span));
        }
        let mut lanes = Vec::with_capacity(4);
        for a in args {
            lanes.push(self.lower_expr(&a.value.node, a.value.span)?.0);
        }
        for (i, l) in lanes.iter().enumerate() {
            let want = if kind == VecKind::F { LirType::F64(FloatKind::Strict) } else { LirType::I64 };
            let got = self.func.locals[*l as usize].clone();
            let ok = match (&want, &got) {
                (LirType::F64(_), LirType::F64(_)) => true,
                (LirType::I64, LirType::I64) => true,
                _ => false,
            };
            if !ok {
                return self.fail(self.err(
                    Code::E108,
                    format!("`{name}` lane {} needs {}, got {}", i, lane_ty_name(kind), ty_name(&got)),
                    args[i].value.span,
                ));
            }
        }
        let dst = self.local(if kind == VecKind::F { LirType::Vec4f } else { LirType::Vec4i });
        self.emit(Instr::VecNew { span, 
            dst,
            kind,
            x: lanes[0],
            y: lanes[1],
            z: lanes[2],
            w: lanes[3],
        });
        Ok((dst, Simple::Other))
    }
    pub(super) fn lower_float_static(
        &mut self,
        field: &str,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let float_arg = |this: &mut Self, a: &A::CallArg| -> Result<Local, Diagnostic> {
            let (v, _) = this.lower_expr(&a.value.node, a.value.span)?;
            if !matches!(this.func.locals[v as usize], LirType::F64(_)) {
                return this.fail(this.err(
                    Code::E108,
                    format!("`Float.{field}` needs a Float argument"),
                    a.value.span,
                ));
            }
            Ok(v)
        };
        let builtin = |this: &mut Self, name: &str, argv: Vec<Local>| {
            let dst = this.local(LirType::F64(FloatKind::Strict));
            this.emit(Instr::Call { span,  dsts: vec![dst], err: None, target: CallTarget::Builtin(name.to_string()), args: argv });
            (dst, Simple::Strict)
        };
        match field {
            "isNaN" => {
                if args.len() != 1 {
                    return self.fail(self.err(Code::E108, "`Float.isNaN` takes 1 arg", span));
                }
                let v = float_arg(self, &args[0])?;
                let dst = self.local(LirType::Bool);
                self.emit(Instr::Cmp { span,  op: CmpOp::NotEq, kind: NumKind::Float(FloatKind::Strict), dst, lhs: v, rhs: v });
                Ok((dst, Simple::Other))
            }
            "nan" => {
                if !args.is_empty() {
                    return self.fail(self.err(Code::E108, "`Float.nan` takes no args", span));
                }
                Ok(builtin(self, "__rnx_float_nan", Vec::new()))
            }
            "fma" => {
                if args.len() != 3 {
                    return self.fail(self.err(Code::E108, "`Float.fma` takes 3 args", span));
                }
                let mut argv = Vec::with_capacity(3);
                for a in args {
                    argv.push(float_arg(self, a)?);
                }
                Ok(builtin(self, "__rnx_float_fma", argv))
            }
            "fromBits" => {
                if args.len() != 1 {
                    return self.fail(self.err(Code::E108, "`Float.fromBits` takes 1 arg", span));
                }
                let (v, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
                if self.func.locals[v as usize] != LirType::I64 {
                    return self.fail(self.err(
                        Code::E108,
                        "`Float.fromBits` needs an Int argument",
                        args[0].value.span,
                    ));
                }
                Ok(builtin(self, "__rnx_float_from_bits", vec![v]))
            }
            _ => self.fail(self.err(Code::E108, format!("unknown static `Float.{field}`"), span)),
        }
    }
    pub(super) fn lower_vec_static(
        &mut self,
        kind: VecKind,
        field: &str,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let name = if kind == VecKind::F { "Vec4f" } else { "Vec4i" };
        if field != "splat" {
            return self.fail(self.err(Code::E108, format!("unknown `{name}.{field}`"), span));
        }
        if args.len() != 1 {
            return self.fail(self.err(Code::E108, format!("`{name}.splat` takes 1 arg"), span));
        }
        let (v, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
        let got = self.func.locals[v as usize].clone();
        let ok = match kind {
            VecKind::F => matches!(got, LirType::F64(_)),
            VecKind::I => got == LirType::I64,
        };
        if !ok {
            return self.fail(self.err(
                Code::E108,
                format!("`{name}.splat` needs {}, got {}", lane_ty_name(kind), ty_name(&got)),
                args[0].value.span,
            ));
        }
        let dst = self.local(if kind == VecKind::F { LirType::Vec4f } else { LirType::Vec4i });
        self.emit(Instr::VecSplat { span,  dst, kind, val: v });
        Ok((dst, Simple::Other))
    }
    pub(super) fn lower_string_method(
        &mut self,
        obj: Local,
        field: &str,
        argv: Vec<Local>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let builtin = |this: &mut Self, name: &str, args: Vec<Local>, ret: LirType, simple: Simple, span: Span| {
            let dst = this.local(ret);
            this.emit(Instr::Call { span,  dsts: vec![dst], err: None, target: CallTarget::Builtin(name.to_string()), args });
            Ok((dst, simple))
        };
        match field {
            "length" | "len" => {
                if !argv.is_empty() {
                    return self.fail(self.err(Code::E108, format!("`{field}` takes no args"), span));
                }
                builtin(self, "__rnx_string_len", vec![obj], LirType::I64, Simple::Int, span)
            }
            "slice" => {
                if argv.len() != 2 {
                    return self.fail(self.err(Code::E108, "`slice` takes 2 args", span));
                }
                for a in &argv {
                    if self.func.locals[*a as usize] != LirType::I64 {
                        return self.fail(self.err(Code::E108, "`slice` needs Int bounds", span));
                    }
                }
                builtin(self, "__rnx_string_slice", vec![obj, argv[0], argv[1]], LirType::Str, Simple::Other, span)
            }
            "indexOf" => {
                if argv.len() != 1 && argv.len() != 2 {
                    return self.fail(self.err(Code::E108, "`indexOf` takes 1..2 args", span));
                }
                if self.func.locals[argv[0] as usize] != LirType::Str {
                    return self.fail(self.err(Code::E108, "`indexOf` needs a String needle", span));
                }
                if argv.len() == 2 {
                    if self.func.locals[argv[1] as usize] != LirType::I64 {
                        return self.fail(self.err(Code::E108, "`indexOf` needs an Int start index", span));
                    }
                    return builtin(self, "__rnx_string_index_of_from", vec![obj, argv[0], argv[1]], LirType::I64, Simple::Int, span);
                }
                builtin(self, "__rnx_string_index_of", vec![obj, argv[0]], LirType::I64, Simple::Int, span)
            }
            "trim" => {
                if !argv.is_empty() {
                    return self.fail(self.err(Code::E108, "`trim` takes no args", span));
                }
                builtin(self, "__rnx_string_trim", vec![obj], LirType::Str, Simple::Other, span)
            }
            "concat" => {
                if argv.len() != 1 {
                    return self.fail(self.err(Code::E108, "`concat` takes 1 arg", span));
                }
                if self.func.locals[argv[0] as usize] != LirType::Str {
                    return self.fail(self.err(Code::E108, "`concat` needs a String argument", span));
                }
                builtin(self, "__rnx_string_concat", vec![obj, argv[0]], LirType::Str, Simple::Other, span)
            }
            "charCodeAt" => {
                if argv.len() != 1 {
                    return self.fail(self.err(Code::E108, "`charCodeAt` takes 1 arg", span));
                }
                if self.func.locals[argv[0] as usize] != LirType::I64 {
                    return self.fail(self.err(Code::E108, "`charCodeAt` needs an Int index", span));
                }
                builtin(self, "__rnx_string_char_code_at", vec![obj, argv[0]], LirType::I64, Simple::Int, span)
            }
            _ => self.fail(self.err(Code::E108, format!("unknown method `{field}` on `String`"), span)),
        }
    }
    pub(super) fn lower_pointer_method(
        &mut self,
        obj: Local,
        pointee: LirType,
        field: &str,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        if self.unsafe_depth == 0 {
            return self.fail(self.err(
                Code::E202,
                "pointer dereference requires unsafe block",
                span,
            ));
        }
        let scalar = self.pointer_scalar(&LirType::Pointer(Box::new(pointee)), span)?;
        match field {
            "read" | "readVolatile" => {
                if !args.is_empty() {
                    return self.fail(self.err(
                        Code::E108,
                        format!("`{field}` takes no args"),
                        span,
                    ));
                }
                let dst = self.local(scalar.clone());
                self.emit(Instr::PtrLoad {
                    span,
                    dst,
                    ptr: obj,
                    volatile: field == "readVolatile",
                });
                Ok((dst, simple_of(&scalar)))
            }
            "write" | "writeVolatile" => {
                if args.len() != 1 {
                    return self.fail(self.err(
                        Code::E108,
                        format!("`{field}` takes 1 arg"),
                        span,
                    ));
                }
                let (v, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
                self.deny_tuple(v, "in a pointer write", args[0].value.span)?;
                let v = self.coerce_to_slot(v, &scalar, args[0].value.span);
                self.emit(Instr::PtrStore {
                    span,
                    ptr: obj,
                    val: v,
                    volatile: field == "writeVolatile",
                });
                let dst = self.local(LirType::Null);
                self.emit(Instr::Const { span, dst, lit: Lit::Null });
                Ok((dst, Simple::Other))
            }
            _ => self.fail(self.err(
                Code::E108,
                format!("unknown method `{field}` on `Pointer`"),
                span,
            )),
        }
    }
    pub(super) fn lower_pointer_from_address(
        &mut self,
        field: &str,
        targs: &[A::Type],
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        if field != "fromAddress" {
            return self.fail(self.err(
                Code::E108,
                format!("unknown static `{field}` on `Pointer`"),
                span,
            ));
        }
        if args.len() != 1 {
            return self.fail(self.err(
                Code::E108,
                "`fromAddress` takes 1 arg",
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
        let (a, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
        self.deny_tuple(a, "in `fromAddress`", args[0].value.span)?;
        let at = self.func.locals.get(a as usize).cloned().unwrap_or(LirType::Any);
        let bits = match at {
            LirType::I64 => a,
            LirType::Any => self.unbox_to_int(a, args[0].value.span),
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    "`fromAddress` takes an Int address",
                    args[0].value.span,
                ));
            }
        };
        let inner = targs.first().map(map_ty).unwrap_or(LirType::Any);
        let dst = self.local(LirType::Pointer(Box::new(inner)));
        self.emit_copy(dst, bits, span);
        Ok((dst, Simple::Other))
    }
    pub(super) fn lower_prim_method(
        &mut self,
        obj: Local,
        ty: &LirType,
        field: &str,
        argv: Vec<Local>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        if field == "toBits" {
            if !matches!(ty, LirType::F64(_)) {
                return self.fail(self.err(Code::E108, format!("unknown method `{field}`"), span));
            }
            if !argv.is_empty() {
                return self.fail(self.err(Code::E108, "`toBits` takes no args", span));
            }
            let dst = self.local(LirType::I64);
            self.emit(Instr::Call { span,  dsts: vec![dst], err: None, target: CallTarget::Builtin("__rnx_float_to_bits".to_string()), args: vec![obj] });
            return Ok((dst, Simple::Other));
        }
        if field != "toString" {
            return self.fail(self.err(Code::E108, format!("unknown method `{field}`"), span));
        }
        if !argv.is_empty() {
            return self.fail(self.err(Code::E108, "`toString` takes no args", span));
        }
        let name = match ty {
            LirType::I64 => "__rnx_int_to_str",
            LirType::F64(_) => "__rnx_float_to_str",
            LirType::Bool => "__rnx_bool_to_str",
            _ => return self.fail(self.err(Code::E108, format!("unknown method `{field}`"), span)),
        };
        let dst = self.local(LirType::Str);
        self.emit(Instr::Call { span,  dsts: vec![dst], err: None, target: CallTarget::Builtin(name.to_string()), args: vec![obj] });
        Ok((dst, Simple::Other))
    }
    pub(super) fn lower_array_method(
        &mut self,
        obj: Local,
        elem: LirType,
        field: &str,
        args: &[A::CallArg],
        argv: Vec<Local>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        match field {
            "length" | "len" => {
                if !argv.is_empty() {
                    return self.fail(self.err(Code::E108, format!("`{field}` takes no args"), span));
                }
                let dst = self.local(LirType::I64);
                self.emit(Instr::ArrayLen { span,  dst, arr: obj });
                Ok((dst, Simple::Int))
            }
            "isEmpty" => {
                if !argv.is_empty() {
                    return self.fail(self.err(Code::E108, "`isEmpty` takes no args", span));
                }
                let len = self.local(LirType::I64);
                self.emit(Instr::ArrayLen { span,  dst: len, arr: obj });
                let zero = self.local(LirType::I64);
                self.emit(Instr::Const { span,  dst: zero, lit: Lit::Int(0) });
                let dst = self.local(LirType::Bool);
                self.emit(Instr::Cmp { span,  op: CmpOp::Eq, kind: NumKind::Int, dst, lhs: len, rhs: zero });
                Ok((dst, Simple::Other))
            }
            "pop" => {
                if !argv.is_empty() {
                    return self.fail(self.err(Code::E108, "`pop` takes no args", span));
                }
                let es = self.elem_size_of(obj);
                let len = self.local(LirType::I64);
                self.emit(Instr::ArrayLen { span,  dst: len, arr: obj });
                let zero = self.local(LirType::I64);
                self.emit(Instr::Const { span,  dst: zero, lit: Lit::Int(0) });
                let empty = self.local(LirType::Bool);
                self.emit(Instr::Cmp { span,  op: CmpOp::Eq, kind: NumKind::Int, dst: empty, lhs: len, rhs: zero });
                let empty_bb = self.new_block();
                let full_bb = self.new_block();
                let merge = self.new_block();
                self.set_term(Terminator::BrIf { span,  cond: empty, then_bb: empty_bb, else_bb: full_bb });
                let dst = self.local(elem.clone());
                if matches!(elem, LirType::I64 | LirType::Bool | LirType::F64(_)) {
                    self.nub_mark(dst);
                }
                self.set_current(empty_bb);
                let null = self.local(LirType::Null);
                self.emit(Instr::Const { span,  dst: null, lit: Lit::Null });
                self.emit_copy(dst, null, span);
                self.set_term(Terminator::Br(merge));
                self.set_current(full_bb);
                let esc = self.local(LirType::I64);
                self.emit(Instr::Const { span,  dst: esc, lit: Lit::Int(es as i64) });
                let got = self.local(LirType::Any);
                self.emit(Instr::Call { span,  dsts: vec![got], err: None, target: CallTarget::Builtin("__rnx_array_pop".to_string()), args: vec![obj, esc] });
                if let Some(tag) = Self::any_box_tag(&elem) {
                    if matches!(elem, LirType::I64 | LirType::Bool | LirType::F64(_)) {
                        self.emit_any_box(dst, tag, got, span);
                        self.nub_mark(dst);
                    } else {
                        let boxed = self.local(LirType::Any);
                        self.emit_any_box(boxed, tag, got, span);
                        self.emit_copy(dst, boxed, span);
                    }
                } else {
                    self.emit_copy(dst, got, span);
                }
                self.set_term(Terminator::Br(merge));
                self.set_current(merge);
                Ok((dst, Simple::Other))
            }
            "map" | "filter" => {
                if argv.len() != 1 {
                    return self.fail(self.err(Code::E108, format!("`{field}` takes 1 arg"), span));
                }
                if self.func.locals.get(argv[0] as usize) != Some(&LirType::Closure) {
                    return self.fail(self.err(Code::E108, format!("`Array.{field}` needs a closure"), span));
                }
                let is_map = field == "map";
                let out_elem = if is_map {
                    match args.first().map(|a| &a.value.node) {
                        Some(A::Expr::Closure { ret, body, params, .. }) => ret
                            .as_ref()
                            .map(|t| resolve_ty(self.module, t))
                            .or_else(|| {
                                let scopes = &self.scopes;
                                let hint = if params.iter().all(|p| p.ty.is_none()) {
                                    vec![elem.clone()]
                                } else {
                                    Vec::new()
                                };
                                infer_closure_ret(body, params, &hint, &|n| {
                                    scope_ty_of(scopes, n)
                                })
                            })
                            .unwrap_or(LirType::Any),
                        _ => LirType::Any,
                    }
                } else {
                    elem.clone()
                };
                let es_in = elem_size(&elem);
                let es_out = elem_size(&out_elem);
                let len = self.local(LirType::I64);
                self.emit(Instr::ArrayLen { span,  dst: len, arr: obj });
                let out = self.local(LirType::Array(Box::new(out_elem.clone())));
                self.emit(Instr::ArrayNew { span,  dst: out, cap: 0, elem_size: es_out });
                let idx = self.local(LirType::I64);
                let zero = self.local(LirType::I64);
                self.emit(Instr::Const { span,  dst: zero, lit: Lit::Int(0) });
                self.emit_copy(idx, zero, span);
                let one = self.local(LirType::I64);
                self.emit(Instr::Const { span,  dst: one, lit: Lit::Int(1) });
                let header = self.new_block();
                let lbody = self.new_block();
                let merge = self.new_block();
                self.set_term(Terminator::Br(header));
                self.set_current(header);
                let cmp = self.local(LirType::Bool);
                self.emit(Instr::Cmp { span,  op: CmpOp::Lt, kind: NumKind::Int, dst: cmp, lhs: idx, rhs: len });
                self.set_term(Terminator::BrIf { span,  cond: cmp, then_bb: lbody, else_bb: merge });
                self.set_current(lbody);
                let item = self.local(elem.clone());
                self.emit(Instr::ArrayGet { span,  dst: item, arr: obj, index: idx, elem_size: es_in, unchecked: true });
                if is_map {
                    let res = self.local(out_elem.clone());
                    if matches!(out_elem, LirType::I64 | LirType::Bool | LirType::F64(_) | LirType::Str) {
                        let tmp = self.local(LirType::Any);
                        self.emit(Instr::Call { span,  dsts: vec![tmp], err: None, target: CallTarget::Value(argv[0]), args: vec![item] });
                        self.emit_any_unbox(res, tmp, span);
                    } else if let Some(st) = self.closure_call_ret(argv[0]) {
                        let tmp = self.local(st);
                        self.emit(Instr::Call { span,  dsts: vec![tmp], err: None, target: CallTarget::Value(argv[0]), args: vec![item] });
                        let boxed = self.coerce_to_slot(tmp, &out_elem, span);
                        self.emit_copy(res, boxed, span);
                    } else {
                        self.emit(Instr::Call { span,  dsts: vec![res], err: None, target: CallTarget::Value(argv[0]), args: vec![item] });
                    }
                    self.emit(Instr::ArrayPush { span,  arr: out, value: res, elem_size: es_out });
                } else {
                    let keep = self.local(LirType::Bool);
                    let tmp = self.local(LirType::Any);
                    self.emit(Instr::Call { span,  dsts: vec![tmp], err: None, target: CallTarget::Value(argv[0]), args: vec![item] });
                    self.emit_any_unbox(keep, tmp, span);
                    let push_bb = self.new_block();
                    let next_bb = self.new_block();
                    self.set_term(Terminator::BrIf { span,  cond: keep, then_bb: push_bb, else_bb: next_bb });
                    self.set_current(push_bb);
                    self.emit(Instr::ArrayPush { span,  arr: out, value: item, elem_size: es_out });
                    self.set_term(Terminator::Br(next_bb));
                    self.set_current(next_bb);
                }
                self.emit(Instr::Arith { span,  op: ArithOp::Add, kind: NumKind::Int, dst: idx, lhs: idx, rhs: one });
                self.set_term(Terminator::Br(header));
                self.set_current(merge);
                Ok((out, Simple::Other))
            }
            _ => self.fail(self.err(Code::E108, format!("unknown method `{field}` on `Array`"), span)),
        }
    }
    pub(super) fn lower_prelude_enum_method(
        &mut self,
        obj: Local,
        ei: usize,
        ename: &str,
        field: &str,
        args: &[A::CallArg],
        argv: Vec<Local>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let (some_name, none_name) = ("Ok", "Err");
        let desc = self.module.enums.get(ei).ok_or_else(|| {
            self.err(Code::E108, format!("unknown enum `{ename}`"), span)
        })?;
        let some_vi = *desc.variant_index.get(some_name).ok_or_else(|| {
            self.err(Code::E108, format!("`{ename}` has no `{some_name}`"), span)
        })?;
        let none_vi = *desc.variant_index.get(none_name).ok_or_else(|| {
            self.err(Code::E108, format!("`{ename}` has no `{none_name}`"), span)
        })?;
        let type_name = "Result";
        match field {
            "isOk" | "isErr" => {
                let expect_some = field == "isOk";
                if !argv.is_empty() {
                    return self.fail(self.err(Code::E108, format!("`{field}` takes no args"), span));
                }
                let tag = self.local(LirType::I64);
                self.emit(Instr::EnumTag { span,  dst: tag, scrut: obj });
                let want = self.local(LirType::I64);
                let vi = if expect_some { some_vi } else { none_vi };
                self.emit(Instr::Const { span,  dst: want, lit: Lit::Int(vi as i64) });
                let dst = self.local(LirType::Bool);
                self.emit(Instr::Cmp { span,  op: CmpOp::Eq, kind: NumKind::Int, dst, lhs: tag, rhs: want });
                Ok((dst, Simple::Other))
            }
            "unwrap" => {
                if !argv.is_empty() {
                    return self.fail(self.err(Code::E108, "`unwrap` takes no args", span));
                }
                let payload_ty = self.module.enums[ei].variants[some_vi].payload.first().cloned().unwrap_or(LirType::Any);
                let tag = self.local(LirType::I64);
                self.emit(Instr::EnumTag { span,  dst: tag, scrut: obj });
                let want = self.local(LirType::I64);
                self.emit(Instr::Const { span,  dst: want, lit: Lit::Int(some_vi as i64) });
                let is_some = self.local(LirType::Bool);
                self.emit(Instr::Cmp { span,  op: CmpOp::Eq, kind: NumKind::Int, dst: is_some, lhs: tag, rhs: want });
                let panic_bb = self.new_block();
                let cont = self.new_block();
                self.set_term(Terminator::BrIf { span,  cond: is_some, then_bb: cont, else_bb: panic_bb });
                self.set_current(panic_bb);
                let msg = self.local(LirType::Str);
                let prefix = self.local(LirType::Str);
                self.emit(Instr::Const { span,  dst: prefix, lit: Lit::Str("unwrap of Err: ".to_string()) });
                let reason_ty = self.module.enums[ei].variants[none_vi]
                    .payload
                    .first()
                    .cloned()
                    .unwrap_or(LirType::Str);
                let reason_raw = self.local(reason_ty);
                self.emit(Instr::EnumPayload { span,  dst: reason_raw, scrut: obj, index: 0 });
                let reason = self.local(LirType::Str);
                self.emit_to_str(reason, reason_raw, span);
                self.emit(Instr::Concat { span,  dst: msg, lhs: prefix, rhs: reason });
                let catch = self.catch_stack.iter().rev().find_map(|c| {
                    c.catch_bb.map(|bb| (bb, c.err_local, c.defer_depth))
                });
                self.set_term(Terminator::Throw { span,  src: msg, catch });
                self.set_current(cont);
                let inner_ty = self.precise.get(&obj).cloned().unwrap_or_else(|| payload_ty.clone());
                let dst = self.local(inner_ty.clone());
                let boxable = matches!(inner_ty, LirType::I64 | LirType::Bool | LirType::F64(_) | LirType::Str);
                if boxable && inner_ty != payload_ty {
                    let tmp = self.local(payload_ty.clone());
                    self.emit(Instr::EnumPayload { span,  dst: tmp, scrut: obj, index: 0 });
                    self.emit_any_unbox(dst, tmp, span);
                } else {
                    self.emit(Instr::EnumPayload { span,  dst, scrut: obj, index: 0 });
                }
                self.emit_retain(dst, span);
                self.precise.insert(dst, inner_ty.clone());
                Ok((dst, simple_of(&inner_ty)))
            }
            "unwrapOr" => {
                if argv.len() != 1 {
                    return self.fail(self.err(Code::E108, "`unwrapOr` takes 1 arg", span));
                }
                let payload_ty = self.module.enums[ei].variants[some_vi].payload.first().cloned().unwrap_or(LirType::Any);
                let tag = self.local(LirType::I64);
                self.emit(Instr::EnumTag { span,  dst: tag, scrut: obj });
                let want = self.local(LirType::I64);
                self.emit(Instr::Const { span,  dst: want, lit: Lit::Int(some_vi as i64) });
                let is_some = self.local(LirType::Bool);
                self.emit(Instr::Cmp { span,  op: CmpOp::Eq, kind: NumKind::Int, dst: is_some, lhs: tag, rhs: want });
                let some_bb = self.new_block();
                let none_bb = self.new_block();
                let merge = self.new_block();
                self.set_term(Terminator::BrIf { span,  cond: is_some, then_bb: some_bb, else_bb: none_bb });
                self.set_current(some_bb);
                let inner_ty = self.precise.get(&obj).cloned().unwrap_or_else(|| payload_ty.clone());
                let got = self.local(inner_ty.clone());
                let boxable = matches!(inner_ty, LirType::I64 | LirType::Bool | LirType::F64(_) | LirType::Str);
                if boxable && inner_ty != payload_ty {
                    let tmp = self.local(payload_ty.clone());
                    self.emit(Instr::EnumPayload { span,  dst: tmp, scrut: obj, index: 0 });
                    self.emit_any_unbox(got, tmp, span);
                } else {
                    self.emit(Instr::EnumPayload { span,  dst: got, scrut: obj, index: 0 });
                }
                self.set_term(Terminator::Br(merge));
                self.set_current(none_bb);
                let fb = self.coerce_to_slot(argv[0], &inner_ty, span);
                self.emit_copy(got, fb, span);
                self.set_term(Terminator::Br(merge));
                self.set_current(merge);
                self.precise.insert(got, inner_ty.clone());
                Ok((got, simple_of(&inner_ty)))
            }
            "map" => {
                if argv.len() != 1 {
                    return self.fail(self.err(Code::E108, format!("`{type_name}.map` takes 1 arg"), span));
                }
                if self.func.locals.get(argv[0] as usize) != Some(&LirType::Closure) {
                    return self.fail(self.err(Code::E108, format!("`{type_name}.map` needs a closure"), span));
                }
                let cb = match args.first().map(|a| &a.value.node) {
                    Some(A::Expr::Closure { params, .. }) if params.len() == 1 => argv[0],
                    _ => {
                        return self.fail(self.err(Code::E108, format!("`{type_name}.map` needs a one-parameter closure"), span));
                    }
                };
                let tag = self.local(LirType::I64);
                self.emit(Instr::EnumTag { span,  dst: tag, scrut: obj });
                let want = self.local(LirType::I64);
                self.emit(Instr::Const { span,  dst: want, lit: Lit::Int(some_vi as i64) });
                let is_some = self.local(LirType::Bool);
                self.emit(Instr::Cmp { span,  op: CmpOp::Eq, kind: NumKind::Int, dst: is_some, lhs: tag, rhs: want });
                let some_bb = self.new_block();
                let none_bb = self.new_block();
                let merge = self.new_block();
                self.set_term(Terminator::BrIf { span,  cond: is_some, then_bb: some_bb, else_bb: none_bb });
                self.set_current(some_bb);
                let inner = self.local(LirType::Any);
                self.emit(Instr::EnumPayload { span,  dst: inner, scrut: obj, index: 0 });
                let mapped = self.emit_value_call(cb, vec![inner], span);
                let dst = self.local(LirType::Enum(ei));
                self.emit(Instr::EnumNew { span,  dst, enu: ei, variant: some_vi, payload: vec![mapped] });
                self.set_term(Terminator::Br(merge));
                self.set_current(none_bb);
                let kept = self.local(LirType::Any);
                self.emit(Instr::EnumPayload { span,  dst: kept, scrut: obj, index: 0 });
                self.emit(Instr::EnumNew { span,  dst, enu: ei, variant: none_vi, payload: vec![kept] });
                self.set_term(Terminator::Br(merge));
                self.set_current(merge);
                Ok((dst, Simple::Other))
            }
            _ => self.fail(self.err(Code::E108, format!("unknown method `{field}` on `{type_name}`"), span)),
        }
    }
    pub(super) fn thread_wrap_class(&self) -> Option<(usize, String)> {
        self.module
            .class_index
            .get("std.sync.Thread")
            .copied()
            .map(|id| (id, "std.sync.Thread".to_string()))
    }
    pub(super) fn wrap_thread_handle(&mut self, handle: Local, span: Span) -> (Local, Simple) {
        let Some((ci, cname)) = self.thread_wrap_class() else {
            return (handle, Simple::Other);
        };
        let Ok(fi) = self.check_field(ci, "id", span) else {
            return (handle, Simple::Other);
        };
        let nfields = self.module.classes.get(ci).map(|c| c.fields.len()).unwrap_or(1);
        let obj = self.local(LirType::Obj(cname));
        self.emit(Instr::ObjNew { span,
            dst: obj,
            class: ci,
            instance_size: instance_size(nfields),
        });
        self.emit(Instr::SetField { span, obj, field: fi, value: handle });
        (obj, Simple::Other)
    }
    pub(super) fn lower_thread(
        &mut self,
        field: &str,
        args: &[A::CallArg],
        span: Span,
        wrap: bool,
    ) -> Result<(Local, Simple), Diagnostic> {
        if field != "spawn" {
            return self.fail(self.err(Code::E108, format!("unknown `Thread.{field}`"), span));
        }
        if args.len() != 1 {
            return self.fail(self.err(Code::E108, "`Thread.spawn` takes 1 arg", span));
        }
        if let A::Expr::Ident(n) = &args[0].value.node {
            let name = n.clone();
            let id = self.module.fn_index.get(&name).copied().ok_or_else(|| {
                self.err(Code::E108, format!("unknown function `{name}`"), args[0].value.span)
            })?;
        let target = &self.module.functions[id];
        if !target.params.is_empty() || target.throws {
            return self.fail(self.err(
                Code::E108,
                "`Thread.spawn` needs a zero-arg non-throwing function",
                args[0].value.span,
            ));
        }
        let ret_tag = result_tag(&target.ret).ok_or_else(|| {
            self.err(Code::E108, "spawned functions must return Int/Bool/Float/String/Null", args[0].value.span)
        })?;
        let dst = self.local(LirType::I64);
        self.emit(Instr::ThreadSpawn { span,  dst, func: id, closure: None, ret_tag });
        if !wrap {
            return Ok((dst, Simple::Other));
        }
        let (out, s) = self.wrap_thread_handle(dst, span);
        return Ok((out, s));
        }
        let (cb, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
        if self.func.locals.get(cb as usize) != Some(&LirType::Closure) {
            return self.fail(self.err(
                Code::E108,
                "`Thread.spawn` needs a function name or closure",
                args[0].value.span,
            ));
        }
        let ret_ty = match &args[0].value.node {
            A::Expr::Closure { ret, body, params, .. } => {
                if !params.is_empty() {
                    return self.fail(self.err(
                        Code::E108,
                        "`Thread.spawn` needs a zero-arg closure",
                        args[0].value.span,
                    ));
                }
                match ret.as_ref().map(|t| resolve_ty(self.module, t)) {
                    Some(ty) => ty,
                    None => {
                        let scopes = &self.scopes;
                        infer_closure_ret(body, params, &[], &|n| scope_ty_of(scopes, n)).ok_or_else(|| {
                            self.err(Code::E108, "annotate the spawned closure return type", args[0].value.span)
                        })?
                    }
                }
            }
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    "`Thread.spawn` needs a function name or closure literal",
                    args[0].value.span,
                ));
            }
        };
        let ret_tag = result_tag(&ret_ty).ok_or_else(|| {
            self.err(Code::E108, "spawned closures must return Int/Bool/Float/String/Null", args[0].value.span)
        })?;
        let dst = self.local(LirType::I64);
        self.emit(Instr::ThreadSpawn { span,  dst, func: usize::MAX, closure: Some(cb), ret_tag });
        if !wrap {
            return Ok((dst, Simple::Other));
        }
        let (out, s) = self.wrap_thread_handle(dst, span);
        Ok((out, s))
    }
    pub(super) fn lower_pool_init(
        &mut self,
        field: &str,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        if field == "new" {
            if args.len() != 1 {
                return self.fail(self.err(Code::E108, "`ThreadPool.new` takes 1 arg", span));
            }
            let (workers, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
            let dst = self.local(LirType::Pool);
            self.emit(Instr::Call { span,  dsts: vec![dst], err: None, target: CallTarget::Builtin("__rnx_pool_new".to_string()), args: vec![workers] });
            return Ok((dst, Simple::Other));
        }
        if field != "byId" {
            return self.fail(self.err(Code::E108, format!("unknown `ThreadPool.{field}`"), span));
        }
        if args.len() != 2 {
            return self.fail(self.err(Code::E108, "`ThreadPool.byId` takes 2 args", span));
        }
        let (id, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
        let (workers, _) = self.lower_expr(&args[1].value.node, args[1].value.span)?;
        let dst = self.local(LirType::Pool);
        self.emit(Instr::PoolInit { span,  dst, id, workers });
        Ok((dst, Simple::Other))
    }
    pub(super) fn pool_task_target(
        &mut self,
        arg: &A::CallArg,
        what: &str,
        need_params: usize,
        need_tag: bool,
    ) -> Result<(usize, Option<Local>, u32), Diagnostic> {        if let A::Expr::Ident(n) = &arg.value.node {
            let name = n.clone();
            let id = self.module.fn_index.get(&name).copied().ok_or_else(|| {
                self.err(Code::E108, format!("unknown function `{name}`"), arg.value.span)
            })?;
            let target = &self.module.functions[id];
            if target.throws {
                return self.fail(self.err(Code::E108, format!("`{what}` needs a non-throwing function"), arg.value.span));
            }
            if target.params.len() != need_params {
                return self.fail(self.err(Code::E108, format!("`{what}` needs a {need_params}-arg function"), arg.value.span));
            }
            if need_params == 1 && target.params[0] != LirType::I64 {
                return self.fail(self.err(Code::E108, format!("`{what}` needs a one-Int-arg function"), arg.value.span));
            }
            let tag = match result_tag(&target.ret) {
                Some(tag) => tag,
                None if need_tag => {
                    return self.fail(self.err(Code::E108, "tasks must return Int/Bool/Float/String/Null", arg.value.span));
                }
                None => 6,
            };
            return Ok((id, None, tag));
        }
        let (cb, _) = self.lower_expr(&arg.value.node, arg.value.span)?;
        if self.func.locals.get(cb as usize) != Some(&LirType::Closure) {
            return self.fail(self.err(Code::E108, format!("`{what}` needs a function name or closure literal"), arg.value.span));
        }
        let ret_ty = match &arg.value.node {
            A::Expr::Closure { ret, body, params, .. } => {
                if params.len() != need_params {
                    return self.fail(self.err(Code::E108, format!("`{what}` needs a {need_params}-arg closure"), arg.value.span));
                }
                if !need_tag {
                    LirType::Null
                } else {
                    match ret.as_ref().map(|t| resolve_ty(self.module, t)) {
                        Some(ty) => ty,
                        None => {
                            let scopes = &self.scopes;
                            infer_closure_ret(body, params, &[], &|n| scope_ty_of(scopes, n)).ok_or_else(|| {
                                self.err(Code::E108, "annotate the task closure return type", arg.value.span)
                            })?
                        }
                    }
                }
            }
            _ => {
                return self.fail(self.err(Code::E108, format!("`{what}` needs a function name or closure literal"), arg.value.span));
            }
        };
        let tag = match result_tag(&ret_ty) {
            Some(tag) => tag,
            None if need_tag => {
                return self.fail(self.err(Code::E108, "tasks must return Int/Bool/Float/String/Null", arg.value.span));
            }
            None => 6,
        };
        Ok((usize::MAX, Some(cb), tag))
    }
    pub(super) fn lower_pool_method(
        &mut self,
        obj: Local,
        field: &str,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        match field {
            "submit" => {
                if args.len() != 1 {
                    return self.fail(self.err(Code::E108, "`submit` takes 1 arg", span));
                }
                let (func, closure, tag) = self.pool_task_target(&args[0], "submit", 0, true)?;
                let dst = self.local(LirType::Pointer(Box::new(LirType::Any)));
                self.emit(Instr::PoolSubmit { span,  dst, pool: obj, func, arg: None, closure, ret_tag: tag });
                return Ok((dst, Simple::Other));
            }
            "submitArg" => {
                if args.len() != 2 {
                    return self.fail(self.err(Code::E108, "`submitArg` takes 2 args", span));
                }
                let (func, closure, tag) = self.pool_task_target(&args[0], "submitArg", 1, true)?;
                let (av, _) = self.lower_expr(&args[1].value.node, args[1].value.span)?;
                let dst = self.local(LirType::Pointer(Box::new(LirType::Any)));
                self.emit(Instr::PoolSubmit { span,  dst, pool: obj, func, arg: Some(av), closure, ret_tag: tag });
                return Ok((dst, Simple::Other));
            }
            "parallelFor" => {
                if args.len() != 4 {
                    return self.fail(self.err(Code::E108, "`parallelFor` takes 4 args", span));
                }
                let (func, closure, _) = self.pool_task_target(&args[3], "parallelFor", 1, false)?;
                let (start, _) = self.lower_expr(&args[0].value.node, args[0].value.span)?;
                let (end, _) = self.lower_expr(&args[1].value.node, args[1].value.span)?;
                let (chunk, _) = self.lower_expr(&args[2].value.node, args[2].value.span)?;
                self.emit(Instr::PoolParallelFor { span,  pool: obj, start, end, chunk, func, closure });
            }
            "join" => {
                if !args.is_empty() {
                    return self.fail(self.err(Code::E108, "`join` takes no args", span));
                }
                self.emit(Instr::PoolJoin { span,  pool: obj });
            }
            "shutdown" => {
                if !args.is_empty() {
                    return self.fail(self.err(Code::E108, "`shutdown` takes no args", span));
                }
                self.emit(Instr::PoolShutdown { span,  pool: obj });
            }
            _ => {
                return self.fail(self.err(Code::E108, format!("unknown pool method `{field}`"), span));
            }
        }
        let dst = self.local(LirType::Null);
        self.emit(Instr::Const { span,  dst, lit: Lit::Null });
        Ok((dst, Simple::Other))
    }
    pub(super) fn lower_channel_send(
        &mut self,
        obj: Local,
        argv: Vec<Local>,
        span: Span,
        class: String,
    ) -> Result<(Local, Simple), Diagnostic> {
        if argv.len() != 1 {
            return self.fail(self.err(Code::E108, "`Channel.send` takes 1 arg", span));
        }
        let v = argv[0];
        self.deny_tuple(v, "in a channel send", span)?;
        let vty = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
        let builtin = match &vty {
            LirType::I64 | LirType::Bool | LirType::F64(_) | LirType::Null | LirType::Pointer(_) => {
                "__rnx_sync_channel_send"
            }
            LirType::Str => "__rnx_sync_channel_send_str",
            LirType::Obj(_) => "__rnx_sync_channel_send_obj",
            LirType::Array(_) => "__rnx_sync_channel_send_array",
            _ => {
                return self.fail(self.err(
                    Code::E108,
                    "channel send needs Int, Float, String, Array, or Object; cast with `as` first",
                    span,
                ));
            }
        };
        let ci = match self.module.class_index.get(&class).copied() {
            Some(ci) => ci,
            None => {
                return self.fail(self.err(Code::E108, format!("unknown class `{class}`"), span));
            }
        };
        let fi = self.check_field(ci, "id", span)?;
        let idl = self.local(LirType::I64);
        self.emit(Instr::GetField { span,  dst: idl, obj, field: fi });
        let v = if builtin == "__rnx_sync_channel_send" {
            self.coerce_to_slot(v, &LirType::Any, span)
        } else {
            v
        };
        self.emit(Instr::Call { span, 
            dsts: vec![],
            err: None,
            target: CallTarget::Builtin(builtin.to_string()),
            args: vec![idl, v],
        });
        let dst = self.local(LirType::Null);
        self.emit(Instr::Const { span,  dst, lit: Lit::Null });
        Ok((dst, Simple::Other))
    }
}
