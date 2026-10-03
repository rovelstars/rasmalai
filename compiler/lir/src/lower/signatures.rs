use super::*;

impl Lower {
    pub(super) fn iface_desc(
        &mut self,
        name: &str,
        type_params: &[String],
        members: &[A::Spanned<A::ClassMember>],
        span: Span,
    ) -> Result<(), Diagnostic> {
        if self.out.interface_index.contains_key(name) {
            return Err(Lower::derr(Code::E108, format!("duplicate interface `{name}`"), span));
        }
        let mut desc = InterfaceDesc {
            name: name.to_string(),
            type_params: type_params.to_vec(),
            methods: Vec::new(),
            method_index: BTreeMap::new(),
        };
        for mem in members {
            match &mem.node {
                A::ClassMember::Method(f) => {
                    if f.is_static {
                        return Err(Lower::derr(Code::E108, "interface methods cannot be static", mem.span));
                    }
                    let norm = |t: &A::Type| {
                        let r = resolve_ty(&self.out, t);
                        match &r {
                            LirType::Obj(n) => {
                                let short = n.rsplit('.').next().unwrap_or(n);
                                let known = self.out.class_index.contains_key(n)
                                    || self.out.enum_index.contains_key(n)
                                    || self.out.interface_index.contains_key(n)
                                    || self.out.classes.iter().any(|c| short_name(&c.name) == short)
                                    || self.out.enums.iter().any(|e| short_name(&e.name) == short);
                                if known {
                                    r
                                } else {
                                    LirType::Any
                                }
                            }
                            _ => r,
                        }
                    };
                    let params = f
                        .params
                        .iter()
                        .map(|p| {
                            p.ty.as_ref().map(|t| norm(t)).unwrap_or(LirType::Any)
                        })
                        .collect::<Vec<_>>();
                    let ret = f.ret.as_ref().map(|t| norm(t)).unwrap_or(LirType::Any);
                    desc.method_index.insert(f.name.clone(), desc.methods.len());
                    desc.methods.push(IfaceMethod { name: f.name.clone(), params, ret });
                }
                _ => {
                    return Err(Lower::derr(Code::E108, "interfaces hold method signatures only", mem.span));
                }
            }
        }
        self.out.interface_index.insert(name.to_string(), self.out.interfaces.len());
        self.out.interfaces.push(desc);
        Ok(())
    }

    pub(super) fn signatures(&mut self, m: &A::Module) -> Result<(), Diagnostic> {
        for decl in &m.decls {
            match &decl.node {
                A::Decl::Fn(f) => {
                    let id = self.out.functions.len();
                    self.out.functions.push(shell(&self.out, &f.name, &f.params, f.ret.as_ref(), f.throws, f.is_unsafe, None, &f.type_params));
                    if f.access == A::Access::Export {
                        self.out.functions[id].is_pub = true;
                    }
                    self.out.fn_index.insert(f.name.clone(), id);
                    self.ret_tys.insert(f.name.clone(), f.ret.clone().unwrap_or_else(|| A::Type::named("Any")));
                    if let Err(d) = self.record_params(&f.name, &f.params) {
                        self.diags.push(d);
                    }
                    if !f.type_params.is_empty() {
                        let params = f.params.iter().map(|p| {
                            (p.name.clone(), p.ty.clone().unwrap_or_else(|| A::Type::named("Any")))
                        }).collect();
                        self.generic_fns.insert(f.name.clone(), GenericSig {
                            tparams: f.type_params.clone(),
                            params,
                            ret: f.ret.clone().unwrap_or_else(|| A::Type::named("Any")),
                        });
                    }
                }
                A::Decl::Class { name, members, .. }
                | A::Decl::Struct { name, members, .. } => {
                    let ci = self.out.class_index[name];
                    for mem in members {
                        match &mem.node {
                            A::ClassMember::Method(f) => {
                                let q = format!("{}.{}", name, f.name);
                                let id = self.out.functions.len();
                                self.out.functions.push(shell(
                                    &self.out,
                                    &q,
                                    &f.params,
                                    f.ret.as_ref(),
                                    f.throws,
                                    f.is_unsafe,
                                    Some(name.clone()),
                                    &f.type_params,
                                ));
                                self.out.fn_index.insert(q.clone(), id);
                                self.ret_tys.insert(q.clone(), f.ret.clone().unwrap_or_else(|| A::Type::named("Any")));
                                if let Err(d) = self.record_params(&q, &f.params) {
                                    self.diags.push(d);
                                }
                                if !f.type_params.is_empty() {
                                    let params = f.params.iter().map(|p| {
                                        (p.name.clone(), p.ty.clone().unwrap_or_else(|| A::Type::named("Any")))
                                    }).collect();
                                    self.generic_fns.insert(q.clone(), GenericSig {
                                        tparams: f.type_params.clone(),
                                        params,
                                        ret: f.ret.clone().unwrap_or_else(|| A::Type::named("Any")),
                                    });
                                }
                                self.out.classes[ci].methods.insert(
                                    f.name.clone(),
                                    MethodRef {
                                        id,
                                        private: f.access == A::Access::Private,
                                        owner: name.clone(),
                                    },
                                );
                            }
                            A::ClassMember::Init { params, body } => {
                                let q = format!("{name}.init");
                                if self.out.fn_index.contains_key(&q) {
                                    self.diags.push(
                                        Diagnostic::new(Code::E108, format!("`{name}` declares more than one constructor"))
                                            .with_span(mem.span),
                                    );
                                    continue;
                                }
                                let id = self.out.functions.len();
                                let throws = body_can_throw(&A::FnBody::Block(body.clone()));
                                self.out.functions.push(shell(&self.out, &q, params, None, throws, false, Some(name.clone()), &[]));
                                self.out.fn_index.insert(q.clone(), id);
                                if let Err(d) = self.record_params(&q, params) {
                                    self.diags.push(d);
                                }
                            }
                            A::ClassMember::Deinit(_) => {
                                let q = format!("{name}.deinit");
                                let id = self.out.functions.len();
                                let mut f = shell(&self.out, &q, &[], None, false, false, Some(name.clone()), &[]);
                                f.ret = LirType::Void;
                                self.out.functions.push(f);
                                self.out.fn_index.insert(q, id);
                                self.out.classes[ci].deinit = Some(id);
                            }
                            A::ClassMember::OnReload { params, .. } => {
                                let q = format!("{name}.onReload");
                                let id = self.out.functions.len();
                                self.out.functions.push(shell(&self.out, &q, params, None, false, false, Some(name.clone()), &[]));
                                self.out.fn_index.insert(q, id);
                            }
                            _ => {}
                        }
                    }
                    if let A::Decl::Class { with, .. } = &decl.node {
                        if !with.is_empty() {
                            let ci = self.out.class_index[name];
                            if let Err(d) = self.check_conformance(name, ci, with, decl.span) {
                                self.diags.push(d);
                            }
                        }
                    }
                }
                A::Decl::Extension { target, members, .. } => {
                    if let Err(d) = self.register_extension(target, members, decl.span) {
                        self.diags.push(d);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn ext_target_name(&self, target: &A::Type, span: Span) -> Result<String, Diagnostic> {
        let target_ty = resolve_ty(&self.out, target);
        match &target_ty {
            LirType::Str => Ok("String".to_string()),
            LirType::I64 => Ok("Int".to_string()),
            LirType::Bool => Ok("Bool".to_string()),
            LirType::F64(FloatKind::Strict) => Ok("Float".to_string()),
            LirType::F64(FloatKind::Fast) => Ok("FastFloat".to_string()),
            LirType::Array(_) => Ok("Array".to_string()),
            LirType::Obj(n) => {
                if crate::instr::class_idx_by_name(&self.out, n).is_ok() {
                    Ok(short_name(n).to_string())
                } else {
                    let short = short_name(n);
                    let is_enum = self.out.enum_index.contains_key(n)
                        || self.out.enums.iter().any(|e| short_name(&e.name) == short);
                    if is_enum {
                        Ok(short.to_string())
                    } else {
                        Err(Diagnostic::new(
                            Code::E108,
                            format!("cannot extend unknown type `{n}`"),
                        )
                        .with_span(span))
                    }
                }
            }
            LirType::Enum(ei) => Ok(self.out.enums.get(*ei).map(|e| short_name(&e.name).to_string()).unwrap_or_default()),
            _ => Err(Diagnostic::new(
                Code::E108,
                "extensions support String, Int, Bool, Float, Array, enum, and class targets",
            )
            .with_span(span)),
        }
    }

    pub(super) fn register_extension(
        &mut self,
        target: &A::Type,
        members: &[A::Spanned<A::ClassMember>],
        span: Span,
    ) -> Result<(), Diagnostic> {
        let tname = self.ext_target_name(target, span)?;
        let target_ty = resolve_ty(&self.out, target);
        let target_tparams: Vec<String> = target
            .args
            .iter()
            .filter_map(|a| {
                if a.fn_sig.is_none() && a.args.is_empty() && a.path.len() == 1 && a.tuple.is_empty() {
                    Some(a.path[0].clone())
                } else {
                    None
                }
            })
            .collect();
        let target_ty = erase_tparams(target_ty, &target_tparams, &[]);
        for mem in members {
            let f = match &mem.node {
                A::ClassMember::Method(f) => f,
                _ => {
                    return Err(Diagnostic::new(Code::E108, "extensions hold methods only")
                        .with_span(mem.span));
                }
            };
            if f.is_static {
                return Err(Diagnostic::new(
                    Code::E108,
                    "extension methods cannot be static",
                )
                .with_span(mem.span));
            }
            let q = format!("__ext_{tname}__{}", f.name);
            if self.out.fn_index.contains_key(&q) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("duplicate extension method `{tname}.{}`", f.name),
                )
                .with_span(mem.span));
            }
            let id = self.out.functions.len();
            let mut all_tparams = f.type_params.clone();
            for tp in &target_tparams {
                if !all_tparams.iter().any(|p| p == tp) {
                    all_tparams.push(tp.clone());
                }
            }
            let mut shell_fn =
                shell(&self.out, &q, &f.params, f.ret.as_ref(), f.throws, f.is_unsafe, None, &all_tparams);
            shell_fn.locals.insert(0, target_ty.clone());
            shell_fn.params.insert(0, target_ty.clone());
            shell_fn.sig_params.insert(0, target_ty.clone());
            shell_fn.method_self = true;
            self.out.functions.push(shell_fn);
            self.out.fn_index.insert(q.clone(), id);
            self.ret_tys.insert(q.clone(), f.ret.clone().unwrap_or_else(|| A::Type::named("Any")));
            if let Err(d) = self.record_params(&q, &f.params) {
                return Err(d);
            }
            if !f.type_params.is_empty() {
                let params = f.params.iter().map(|p| {
                    (p.name.clone(), p.ty.clone().unwrap_or_else(|| A::Type::named("Any")))
                }).collect();
                self.generic_fns.insert(q.clone(), GenericSig {
                    tparams: f.type_params.clone(),
                    params,
                    ret: f.ret.clone().unwrap_or_else(|| A::Type::named("Any")),
                });
            }
            self.extensions.insert((tname.clone(), f.name.clone()), id);
            self.ext_self.insert(q.clone(), (*target).clone());
        }
        Ok(())
    }

    pub(super) fn check_conformance(
        &mut self,
        cname: &str,
        ci: usize,
        with: &[A::Type],
        span: Span,
    ) -> Result<(), Diagnostic> {
        for t in with {
            let iname = t.path.last().cloned().unwrap_or_default();
            let short = iname.rsplit('.').next().unwrap_or(&iname).to_string();
            let ii = match crate::instr::iface_idx_by_name(&self.out, &short) {
                Ok(ii) => ii,
                Err(_) => {
                    return Err(Lower::derr(
                        Code::E108,
                        format!("`{cname}` conforms to unknown interface `{short}`"),
                        span,
                    ));
                }
            };
            let iface = self.out.interfaces[ii].clone();
            for m in &iface.methods {
                let mr = self.conformance_method(ci, &m.name).ok_or_else(|| {
                    Lower::derr(
                        Code::E108,
                        format!("`{cname}` does not implement `{}`", m.name),
                        span,
                    )
                })?;
                let f = self.out.functions[mr.id].clone();
                let have = &f.sig_params[1..];
                if have.len() != m.params.len()
                    || have
                        .iter()
                        .zip(m.params.iter())
                        .any(|(a, b)| a != b && *b != LirType::Any)
                    || (f.ret != m.ret && m.ret != LirType::Any && !self.ret_conforms(&f.ret, &m.ret))
                {
                    return Err(Lower::derr(
                        Code::E108,
                        format!("`{cname}.{}` signature does not match interface `{}`", m.name, iface.name),
                        span,
                    ));
                }
            }
            if !self.out.classes[ci].ifaces.contains(&ii) {
                self.out.classes[ci].ifaces.push(ii);
            }
        }
        Ok(())
    }

    pub(super) fn conformance_method(&self, ci: usize, name: &str) -> Option<crate::instr::MethodRef> {
        let mut cur = Some(ci);
        while let Some(c) = cur {
            if let Some(mr) = self.out.classes[c].methods.get(name).cloned() {
                return Some(mr);
            }
            cur = self.out.classes[c].parent;
        }
        None
    }

    pub(super) fn ret_conforms(&self, have: &LirType, want: &LirType) -> bool {
        let (hname, wname) = match (have, want) {
            (LirType::Obj(h), LirType::Obj(w)) => (h.clone(), w.clone()),
            _ => return false,
        };
        let hshort = short_name(&hname).to_string();
        let wshort = short_name(&wname).to_string();
        if hshort == wshort {
            return true;
        }
        let hci = match crate::instr::class_idx_by_name(&self.out, &hname) {
            Ok(ci) => ci,
            Err(_) => return false,
        };
        let wii = match crate::instr::iface_idx_by_name(&self.out, &wshort) {
            Ok(ii) => ii,
            Err(_) => return false,
        };
        self.out.classes[hci].ifaces.contains(&wii)
    }
}
