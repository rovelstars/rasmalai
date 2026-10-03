use super::*;

impl Lower {
    pub(super) fn link_parents(&mut self, m: &A::Module) -> Result<(), Diagnostic> {
        let mut edges: Vec<(usize, String, Span)> = Vec::new();
        for decl in &m.decls {
            if let A::Decl::Class { name, extends: Some(t), .. } = &decl.node {
                let Some(ci) = self.out.class_index.get(name).copied() else {
                    continue;
                };
                let base = t
                    .path
                    .last()
                    .map(|s| short_name(s).to_string())
                    .unwrap_or_default();
                edges.push((ci, base, decl.span));
            }
        }
        if edges.is_empty() {
            return Ok(());
        }
        let mut parent_of: BTreeMap<usize, usize> = BTreeMap::new();
        for (ci, base, span) in &edges {
            match crate::instr::class_idx_by_name(&self.out, base) {
                Ok(bi) => {
                    if self.out.classes[bi].is_struct {
                        self.diags.push(Lower::derr(
                            Code::E108,
                            format!("`{}` cannot extend struct `{base}`", self.out.classes[*ci].name),
                            *span,
                        ));
                    } else if bi == *ci {
                        self.diags.push(Lower::derr(
                            Code::E108,
                            format!("`{}` cannot extend itself", self.out.classes[*ci].name),
                            *span,
                        ));
                    } else {
                        parent_of.insert(*ci, bi);
                    }
                }
                Err(_) => {
                    self.diags.push(Lower::derr(
                        Code::E108,
                        format!("unknown base class `{base}`"),
                        *span,
                    ));
                }
            }
        }
        let mut linked: BTreeSet<usize> = BTreeSet::new();
        loop {
            let mut progress = false;
            let pending: Vec<(usize, usize)> = parent_of
                .iter()
                .filter(|(ci, _)| !linked.contains(ci))
                .map(|(ci, bi)| (*ci, *bi))
                .collect();
            if pending.is_empty() {
                break;
            }
            for (ci, bi) in pending {
                if parent_of.contains_key(&bi) && !linked.contains(&bi) {
                    continue;
                }
                self.link_one_parent(ci, bi, &edges);
                linked.insert(ci);
                progress = true;
            }
            if !progress {
                let rest: Vec<usize> = parent_of
                    .keys()
                    .copied()
                    .filter(|ci| !linked.contains(ci))
                    .collect();
                for ci in rest {
                    let span = edges
                        .iter()
                        .find(|(e, _, _)| *e == ci)
                        .map(|(_, _, s)| *s)
                        .unwrap_or(crate::instr::UNKNOWN_SPAN);
                    self.diags.push(Lower::derr(
                        Code::E108,
                        format!(
                            "cyclic inheritance involving `{}`",
                            self.out.classes[ci].name
                        ),
                        span,
                    ));
                    linked.insert(ci);
                }
                break;
            }
        }
        Ok(())
    }

    pub(super) fn link_one_parent(&mut self, ci: usize, bi: usize, edges: &[(usize, String, Span)]) {
        let child_name = self.out.classes[ci].name.clone();
        let parent_name = self.out.classes[bi].name.clone();
        let parent_fields = self.out.classes[bi].fields.clone();
        let mut grown: Vec<FieldDesc> = Vec::with_capacity(parent_fields.len() + self.out.classes[ci].fields.len());
        for f in parent_fields {
            if self.out.classes[ci].field_index.contains_key(&f.name) {
                let span = edges
                    .iter()
                    .find(|(e, _, _)| *e == ci)
                    .map(|(_, _, s)| *s)
                    .unwrap_or(crate::instr::UNKNOWN_SPAN);
                self.diags.push(Lower::derr(
                    Code::E108,
                    format!("`{child_name}` redefines inherited field `{}`", f.name),
                    span,
                ));
                continue;
            }
            let pkey = format!("{parent_name}.{}", f.name);
            let ckey = format!("{child_name}.{}", f.name);
            if let Some(pt) = self.field_opt.get(&pkey).cloned() {
                self.field_opt.insert(ckey, pt);
            }
            if self.nub_fields.contains(&pkey) {
                self.nub_fields.insert(format!("{child_name}.{}", f.name));
            }
            grown.push(f);
        }
        for f in self.out.classes[ci].fields.iter().cloned() {
            grown.push(f);
        }
        self.out.classes[ci].field_index.clear();
        for (idx, f) in grown.iter().enumerate() {
            self.out.classes[ci].field_index.insert(f.name.clone(), idx);
        }
        self.out.classes[ci].fields = grown;
        let mut chain: Vec<usize> = crate::instr::ancestors_of(&self.out, bi);
        chain.reverse();
        chain.push(bi);
        let mut merged = Vec::new();
        for a in chain {
            let aname = self.out.classes[a].name.clone();
            if let Some(ds) = self.defaults.get(&aname).cloned() {
                merged.extend(ds);
            }
        }
        if let Some(own) = self.defaults.remove(&child_name) {
            merged.extend(own);
        }
        if !merged.is_empty() {
            self.defaults.insert(child_name, merged);
        }
        self.out.classes[ci].parent = Some(bi);
    }

    pub(super) fn method_sigs_match(&self, have_id: usize, want_params: &[LirType], want_ret: &LirType) -> bool {
        let f = &self.out.functions[have_id];
        let have = f.sig_params.get(1..).unwrap_or(&[]);
        if have.len() != want_params.len() {
            return false;
        }
        if have
            .iter()
            .zip(want_params.iter())
            .any(|(a, b)| a != b && *b != LirType::Any)
        {
            return false;
        }
        f.ret == *want_ret || *want_ret == LirType::Any || self.ret_conforms(&f.ret, want_ret)
    }

    pub(super) fn link_members(&mut self, m: &A::Module) -> Result<(), Diagnostic> {
        let mut member_spans: BTreeMap<(String, String), Span> = BTreeMap::new();
        for decl in &m.decls {
            let (cname, members) = match &decl.node {
                A::Decl::Class { name, members, .. } | A::Decl::Struct { name, members, .. } => {
                    (name.clone(), members)
                }
                _ => continue,
            };
            for mem in members {
                if let A::ClassMember::Method(f) = &mem.node {
                    member_spans.insert((cname.clone(), f.name.clone()), mem.span);
                }
            }
        }
        let mut order: Vec<usize> = (0..self.out.classes.len()).collect();
        order.sort_by_key(|ci| crate::instr::ancestors_of(&self.out, *ci).len());
        for ci in order {
            let Some(pi) = self.out.classes[ci].parent else {
                continue;
            };
            let child_name = self.out.classes[ci].name.clone();
            let parent_methods = self.out.classes[pi].methods.clone();
            for (mname, mr) in parent_methods {
                if mr.private {
                    continue;
                }
                if let Some(existing) = self.out.classes[ci].methods.get(&mname).cloned() {
                    let want = self.out.functions[mr.id].clone();
                    let want_params = want.sig_params.get(1..).unwrap_or(&[]).to_vec();
                    if !self.method_sigs_match(existing.id, &want_params, &want.ret) {
                        let span = member_spans
                            .get(&(child_name.clone(), mname.clone()))
                            .copied()
                            .unwrap_or(crate::instr::UNKNOWN_SPAN);
                        self.diags.push(Lower::derr(
                            Code::E108,
                            format!("`{child_name}.{mname}` signature does not match inherited method"),
                            span,
                        ));
                    }
                } else {
                    self.out.classes[ci].methods.insert(mname, mr);
                }
            }
            let parent_ifaces = self.out.classes[pi].ifaces.clone();
            for ii in parent_ifaces {
                if !self.out.classes[ci].ifaces.contains(&ii) {
                    self.out.classes[ci].ifaces.push(ii);
                }
            }
            if self.out.classes[ci].deinit.is_none() {
                self.out.classes[ci].deinit = self.out.classes[pi].deinit;
            }
        }
        Ok(())
    }

    pub(super) fn class_desc(
        &mut self,
        name: &str,
        type_params: &[String],
        members: &[A::Spanned<A::ClassMember>],
        is_struct: bool,
    ) -> Result<(), Diagnostic> {
        let mut desc = ClassDesc {
            name: name.to_string(),
            is_struct,
            type_params: type_params.to_vec(),
            fields: Vec::new(),
            field_index: BTreeMap::new(),
            methods: BTreeMap::new(),
            deinit: None,
            dtor: None,
            ifaces: Vec::new(),
            parent: None,
        };
        for mem in members {
            if let A::ClassMember::Field(f) = &mem.node {
                desc.field_index.insert(f.name.clone(), desc.fields.len());
                let fty = match f.ty.as_ref() {
                    Some(t) if is_type_param(type_params, t) => LirType::Any,
                    Some(t) => erase_tparams(resolve_ty(&self.out, t), type_params, &[]),
                    None => LirType::Any,
                };
                if let Some(t) = f.ty.as_ref() {
                    if nub_ty(t) {
                        self.nub_fields.insert(format!("{name}.{}", f.name));
                    }
                }
                if let Some(t) = f.ty.as_ref().and_then(option_arg) {
                    let pt = if is_type_param(type_params, t) {
                        LirType::Any
                    } else {
                        resolve_ty(&self.out, t)
                    };
                    self.field_opt.insert(format!("{name}.{}", f.name), pt);
                }
                desc.fields.push(FieldDesc {
                    name: f.name.clone(),
                    ty: fty,
                    private: f.access == A::Access::Private,
                    owner: name.to_string(),
                });
                if let Some(v) = &f.value {
                    self.defaults
                        .entry(name.to_string())
                        .or_default()
                        .push((f.name.clone(), v.clone()));
                }
            }
        }
        self.out.class_index.insert(name.to_string(), self.out.classes.len());
        self.out.classes.push(desc);
        Ok(())
    }

    pub(super) fn record_desc(
        &mut self,
        name: &str,
        fields: &[A::RecordField],
        span: Span,
    ) -> Result<(), Diagnostic> {
        if self.out.class_index.contains_key(name) || self.out.enum_index.contains_key(name) {
            return Err(Diagnostic::new(Code::E108, format!("duplicate type `{name}`"))
                .with_span(span));
        }
        let mut desc = ClassDesc {
            name: name.to_string(),
            is_struct: true,
            type_params: Vec::new(),
            fields: Vec::new(),
            field_index: BTreeMap::new(),
            methods: BTreeMap::new(),
            deinit: None,
            dtor: None,
            ifaces: Vec::new(),
            parent: None,
        };
        for f in fields {
            desc.field_index.insert(f.name.clone(), desc.fields.len());
            desc.fields.push(FieldDesc {
                name: f.name.clone(),
                ty: resolve_ty(&self.out, &f.ty),
                private: false,
                owner: name.to_string(),
            });
        }
        self.out.class_index.insert(name.to_string(), self.out.classes.len());
        self.out.classes.push(desc);
        Ok(())
    }

    pub(super) fn enum_desc(
        &mut self,
        name: &str,
        type_params: &[String],
        members: &[A::EnumMember],
        span: Span,
    ) -> Result<(), Diagnostic> {
        if self.out.class_index.contains_key(name) || self.out.enum_index.contains_key(name) {
            return Err(Diagnostic::new(Code::E108, format!("duplicate type `{name}`"))
                .with_span(span));
        }
        let mut desc = EnumDesc {
            name: name.to_string(),
            variants: Vec::new(),
            variant_index: BTreeMap::new(),
            dtor: None,
        };
        for m in members {
            if desc.variant_index.contains_key(&m.name) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("duplicate variant `{}.{}`", name, m.name),
                )
                .with_span(span));
            }
            let mut payload = Vec::with_capacity(m.payload.len());
            for t in &m.payload {
                if t.fn_sig.is_none() && t.args.is_empty() && t.path.len() == 1
                    && type_params.contains(&t.path[0])
                {
                    payload.push(LirType::Any);
                } else {
                    payload.push(resolve_ty(&self.out, t));
                }
            }
            desc.variant_index.insert(m.name.clone(), desc.variants.len());
            desc.variants.push(VariantDesc { name: m.name.clone(), payload });
        }
        self.out.enum_index.insert(name.to_string(), self.out.enums.len());
        self.out.enums.push(desc);
        Ok(())
    }

    pub(super) fn record_params(&mut self, qname: &str, params: &[A::Param]) -> Result<(), Diagnostic> {
        let mut seen_default = false;
        let mut info = Vec::with_capacity(params.len());
        for p in params {
            if p.default.is_some() {
                seen_default = true;
            } else if seen_default {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("mandatory parameter `{}` follows a defaulted parameter", p.name),
                )
                .with_span(p.span));
            }
            info.push((p.name.clone(), p.default.clone()));
        }
        self.param_info.insert(qname.to_string(), info);
        let tys = params
            .iter()
            .map(|p| (p.name.clone(), p.ty.clone().unwrap_or_else(|| A::Type::named("Any"))))
            .collect();
        self.param_tys.insert(qname.to_string(), tys);
        Ok(())
    }
}
