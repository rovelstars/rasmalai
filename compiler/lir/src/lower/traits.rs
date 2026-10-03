use super::*;

impl Lower {
    pub(super) fn trait_iface_desc(
        &mut self,
        name: &str,
        members: &[A::Spanned<A::ClassMember>],
        span: Span,
    ) -> Result<(), Diagnostic> {
        if self.out.interface_index.contains_key(name) {
            return Err(Lower::derr(Code::E108, format!("duplicate interface `{name}`"), span));
        }
        let mut desc = InterfaceDesc {
            name: name.to_string(),
            type_params: Vec::new(),
            methods: Vec::new(),
            method_index: BTreeMap::new(),
        };
        for mem in members {
            let A::ClassMember::Method(f) = &mem.node else {
                continue;
            };
            if f.is_static {
                continue;
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
        self.out.interface_index.insert(name.to_string(), self.out.interfaces.len());
        self.out.interfaces.push(desc);
        Ok(())
    }

    pub(super) fn expand_traits(&mut self, m: &A::Module) -> A::Module {
        let mut traits: BTreeMap<String, (String, Vec<A::Spanned<A::ClassMember>>)> = BTreeMap::new();
        for decl in &m.decls {
            if let A::Decl::Trait { name, members, .. } = &decl.node {
                let short = short_name(name).to_string();
                if let Some((prev, _)) = traits.get(&short) {
                    if prev != name {
                        self.diags.push(Lower::derr(
                            Code::E108,
                            format!("ambiguous trait `{short}`"),
                            decl.span,
                        ));
                        continue;
                    }
                }
                traits.entry(short).or_insert_with(|| (name.clone(), members.clone()));
            }
        }
        if traits.is_empty() {
            return m.clone();
        }
        let iface_names: BTreeSet<String> = m
            .decls
            .iter()
            .filter_map(|d| match &d.node {
                A::Decl::Interface { name, .. } => Some(short_name(name).to_string()),
                _ => None,
            })
            .collect();
        let mut out = m.clone();
        for decl in out.decls.iter_mut() {
            let A::Decl::Class { name: cname, with, members, .. } = &mut decl.node else {
                continue;
            };
            let span = decl.span;
            let cname = cname.clone();
            for t in with.iter() {
                let tshort = t
                    .path
                    .last()
                    .map(|s| short_name(s).to_string())
                    .unwrap_or_default();
                if iface_names.contains(&tshort) {
                    continue;
                }
                let Some((_, tmembers)) = traits.get(&tshort) else {
                    continue;
                };
                let tmembers = tmembers.clone();
                for tm in tmembers {
                    match &tm.node {
                        A::ClassMember::Field(f) => {
                            let clash = members.iter().any(|x| match &x.node {
                                A::ClassMember::Field(of) => of.name == f.name,
                                _ => false,
                            });
                            if clash {
                                self.diags.push(Lower::derr(
                                    Code::E108,
                                    format!("`{cname}` redefines trait field `{}`", f.name),
                                    span,
                                ));
                                continue;
                            }
                            members.push(tm.clone());
                        }
                        A::ClassMember::Method(f) => {
                            let present = members.iter().any(|x| match &x.node {
                                A::ClassMember::Method(of) => of.name == f.name,
                                _ => false,
                            });
                            if !present {
                                members.push(tm.clone());
                            }
                        }
                        A::ClassMember::Init { .. } => {
                            let present = members.iter().any(|x| {
                                matches!(&x.node, A::ClassMember::Init { .. })
                            });
                            if !present {
                                members.push(tm.clone());
                            }
                        }
                        A::ClassMember::Deinit(_) => {
                            let present = members.iter().any(|x| {
                                matches!(&x.node, A::ClassMember::Deinit(_))
                            });
                            if !present {
                                members.push(tm.clone());
                            }
                        }
                        A::ClassMember::OnReload { .. } => {
                            members.push(tm.clone());
                        }
                    }
                }
            }
        }
        out
    }

    pub(super) fn expand_inheritance(&mut self, m: &A::Module) -> A::Module {
        let mut members_of: BTreeMap<String, Vec<A::Spanned<A::ClassMember>>> = BTreeMap::new();
        let mut extends_of: BTreeMap<String, String> = BTreeMap::new();
        for decl in &m.decls {
            if let A::Decl::Class { name, extends, members, .. } = &decl.node {
                members_of.insert(short_name(name).to_string(), members.clone());
                if let Some(t) = extends {
                    let base = t
                        .path
                        .last()
                        .map(|s| short_name(s).to_string())
                        .unwrap_or_default();
                    extends_of.insert(short_name(name).to_string(), base);
                }
            }
        }
        if extends_of.is_empty() {
            return m.clone();
        }
        let mut order: Vec<String> = Vec::new();
        let mut done: BTreeSet<String> = BTreeSet::new();
        let mut active: BTreeSet<String> = BTreeSet::new();
        for name in members_of.keys().cloned().collect::<Vec<_>>() {
            self.inherit_order(&name, &extends_of, &mut order, &mut done, &mut active);
        }
        let mut grown = members_of.clone();
        for name in &order {
            let Some(base) = extends_of.get(name).cloned() else {
                continue;
            };
            let Some(pmembers) = grown.get(&base).cloned() else {
                continue;
            };
            let entry = grown.get_mut(name).expect("class members");
            for pm in pmembers {
                let A::ClassMember::Method(f) = &pm.node else {
                    continue;
                };
                if f.access == A::Access::Private {
                    continue;
                }
                let present = entry.iter().any(|x| match &x.node {
                    A::ClassMember::Method(of) => of.name == f.name,
                    _ => false,
                });
                if !present {
                    entry.push(pm.clone());
                }
            }
        }
        let mut out = m.clone();
        for decl in out.decls.iter_mut() {
            if let A::Decl::Class { name, members, .. } = &mut decl.node {
                if let Some(expanded) = grown.get(short_name(name)) {
                    *members = expanded.clone();
                }
            }
        }
        out
    }

    pub(super) fn inherit_order(
        &mut self,
        name: &str,
        extends_of: &BTreeMap<String, String>,
        order: &mut Vec<String>,
        done: &mut BTreeSet<String>,
        active: &mut BTreeSet<String>,
    ) {
        if done.contains(name) || !active.insert(name.to_string()) {
            return;
        }
        if let Some(base) = extends_of.get(name) {
            self.inherit_order(base, extends_of, order, done, active);
        }
        active.remove(name);
        done.insert(name.to_string());
        order.push(name.to_string());
    }

    pub(super) fn propagate_init_throws(&mut self, m: &A::Module) {
        loop {
            let mut changed = false;
            for decl in &m.decls {
                let A::Decl::Class { name, members, .. } = &decl.node else {
                    continue;
                };
                let Some(ci) = self.out.class_index.get(name).copied() else {
                    continue;
                };
                let Some(pi) = self.out.classes[ci].parent else {
                    continue;
                };
                let pname = self.out.classes[pi].name.clone();
                let Some(pid) = self.out.fn_index.get(&format!("{pname}.init")).copied() else {
                    continue;
                };
                if !self.out.functions[pid].throws {
                    continue;
                }
                for mem in members {
                    let A::ClassMember::Init { body, .. } = &mem.node else {
                        continue;
                    };
                    let q = format!("{name}.init");
                    let Some(id) = self.out.fn_index.get(&q).copied() else {
                        continue;
                    };
                    if self.out.functions[id].throws {
                        continue;
                    }
                    let explicit = body_uses_super(&A::FnBody::Block(body.clone()));
                    let pshort = short_name(&pname).to_string();
                    let mut aparams: Option<usize> = None;
                    for d in &m.decls {
                        if let A::Decl::Class { name: n, members: ms, .. } = &d.node {
                            if short_name(n) != pshort {
                                continue;
                            }
                            for mm in ms {
                                if let A::ClassMember::Init { params, .. } = &mm.node {
                                    aparams = Some(params.len());
                                }
                            }
                        }
                    }
                    let n = aparams.unwrap_or_else(|| {
                        self.out.functions[pid].sig_params.len().saturating_sub(1)
                    });
                    if explicit || n == 0 {
                        self.out.functions[id].throws = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    pub(super) fn plan_super(
        &mut self,
        m: &A::Module,
        name: &str,
        body: &A::Block,
        span: Span,
        id: usize,
        implicit_super: &mut BTreeMap<usize, usize>,
    ) {
        let Some(ci) = self.out.class_index.get(name).copied() else {
            return;
        };
        let Some(pi) = self.out.classes[ci].parent else {
            return;
        };
        if body_uses_super(&A::FnBody::Block(body.clone())) {
            return;
        }
        let pname = self.out.classes[pi].name.clone();
        let Some(pid) = self.out.fn_index.get(&format!("{pname}.init")).copied() else {
            return;
        };
        let pshort = short_name(&pname).to_string();
        let mut aparams: Option<usize> = None;
        for decl in &m.decls {
            if let A::Decl::Class { name: n, members, .. } = &decl.node {
                if short_name(n) != pshort {
                    continue;
                }
                for mm in members {
                    if let A::ClassMember::Init { params, .. } = &mm.node {
                        aparams = Some(params.len());
                    }
                }
            }
        }
        let n = aparams.unwrap_or_else(|| self.out.functions[pid].sig_params.len().saturating_sub(1));
        if n == 0 {
            implicit_super.insert(id, pid);
        } else {
            self.diags.push(Lower::derr(
                Code::E108,
                format!("`{name}.init` must call `super(...)`: base `{pname}.init` requires arguments"),
                span,
            ));
        }
    }
}
