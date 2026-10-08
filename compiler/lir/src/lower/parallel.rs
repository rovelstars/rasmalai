use super::*;
use rayon::prelude::*;

pub fn lower_parallel(m: &A::Module, jobs: usize) -> Result<Module, Diagnostic> {
    if jobs <= 1 {
        return lower(m);
    }
    let mut l = Lower {
        out: Module::default(),
        defaults: BTreeMap::new(),
        param_info: BTreeMap::new(),
        param_tys: BTreeMap::new(),
        field_opt: BTreeMap::new(),
        ret_tys: BTreeMap::new(),
        nub_fields: BTreeSet::new(),
        generic_fns: BTreeMap::new(),
        extensions: BTreeMap::new(),
        ext_self: BTreeMap::new(),
        mod_consts: BTreeMap::new(),
        diags: Vec::new(),
    };
    let expanded = l.expand_traits(m);
    let expanded = l.expand_inheritance(&expanded);
    l.classes(&expanded)?;
    l.link_parents(&expanded)?;
    l.signatures(&expanded)?;
    l.collect_foreign(&expanded)?;
    l.collect_consts(&expanded)?;
    l.collect_namespaces();
    l.link_members(&expanded)?;
    l.bodies_parallel(&expanded, jobs)?;
    l.synthesize_dtors();
    if let Some(d) = l.diags.into_iter().next() {
        return Err(d);
    }
    Ok(l.out)
}

struct Shared<'a> {
    module: &'a Module,
    defaults: &'a BTreeMap<String, Vec<(String, A::Spanned<A::Expr>)>>,
    param_info: &'a BTreeMap<String, Vec<(String, Option<A::Spanned<A::Expr>>)>>,
    param_tys: &'a BTreeMap<String, Vec<(String, A::Type)>>,
    field_opt: &'a BTreeMap<String, LirType>,
    ret_tys: &'a BTreeMap<String, A::Type>,
    nub_fields: &'a BTreeSet<String>,
    generic_fns: &'a BTreeMap<String, GenericSig>,
    extensions: &'a BTreeMap<(String, String), usize>,
    ext_self: &'a BTreeMap<String, A::Type>,
    mod_consts: &'a BTreeMap<String, (Lit, LirType, Simple)>,
}

struct BodyOut {
    id: usize,
    func: Function,
    patches: Vec<(BlockId, usize, String)>,
    pending: Vec<PendingClosure>,
    diags: Vec<Diagnostic>,
    result: Result<(), Diagnostic>,
}

fn lower_one(
    s: &Shared<'_>,
    supers: &BTreeMap<usize, usize>,
    id: usize,
    src: BodySrc,
) -> BodyOut {
    let prefix = s
        .module
        .functions
        .get(id)
        .map(|f| format!("{}::", f.name))
        .unwrap_or_default();
    let mut b = Builder::new(
        s.module,
        s.defaults,
        s.param_info,
        s.param_tys,
        s.field_opt,
        s.ret_tys,
        s.nub_fields,
        s.generic_fns,
        s.extensions,
        s.ext_self,
        s.mod_consts,
        id,
        prefix,
    );
    b.implicit_super = supers.get(&id).copied();
    let is_closure = matches!(src, BodySrc::Closure(_));
    if let BodySrc::Fn(f) = &src {
        b.fn_tparams = f.type_params.clone();
        if let Some(fname) = s.module.functions.get(id).map(|f| f.name.clone()) {
            if let Some(target) = s.ext_self.get(&fname) {
                for a in &target.args {
                    if a.fn_sig.is_none()
                        && a.args.is_empty()
                        && a.tuple.is_empty()
                        && a.path.len() == 1
                        && !b.fn_tparams.iter().any(|p| p == &a.path[0])
                    {
                        b.fn_tparams.push(a.path[0].clone());
                    }
                }
            }
        }
    }
    let r = match src {
        BodySrc::Fn(f) => b.lower_fn_body(&f),
        BodySrc::Init(params, body) => b.lower_init_body(params, body),
        BodySrc::Bare(params, body) => b.lower_bare_body(params, body),
        BodySrc::Closure(p) => {
            b.closure_prefix = p.prefix.clone();
            b.enclosing = p.enclosing.clone();
            b.lower_closure_body(p)
        }
    };
    let (mut func, patches, pending, diags) = b.finish();
    if is_closure {
        func.is_closure = true;
    }
    BodyOut {
        id,
        func,
        patches,
        pending,
        diags,
        result: r,
    }
}

impl Lower {
    fn ingest(&mut self, queue: &mut Vec<(usize, BodySrc)>, mut out: BodyOut) -> Result<(), Diagnostic> {
        self.out.functions[out.id] = out.func;
        self.diags.append(&mut out.diags);
        out.result?;
        for p in out.pending {
            let cid = self.out.functions.len();
            let mut params: Vec<LirType> = p
                .params
                .iter()
                .map(|x| {
                    x.ty.as_ref()
                        .map(|t| resolve_ty(&self.out, t))
                        .unwrap_or(LirType::Any)
                })
                .collect();
            for c in &p.captures {
                params.push(c.ty.clone());
            }
            self.out.functions.push(Function {
                name: p.name.clone(),
                params: params.clone(),
                sig_params: params.clone(),
                ret: p
                    .ret
                    .as_ref()
                    .map(|t| resolve_ty(&self.out, t))
                    .unwrap_or(LirType::Any),
                throws: p.throws,
                is_unsafe: false,
                method_self: false,
                is_pub: false,
                is_closure: false,
                locals: params,
                blocks: vec![],
            });
            self.out.fn_index.insert(p.name.clone(), cid);
            queue.push((cid, BodySrc::Closure(p)));
        }
        for (bb, ix, name) in out.patches {
            let cid = *self
                .out
                .fn_index
                .get(&name)
                .ok_or_else(|| Diagnostic::new(Code::E108, "closure target lost"))?;
            match &mut self.out.functions[out.id].blocks[bb].instrs[ix] {
                Instr::ClosureNew { func, .. } => *func = cid,
                _ => {
                    return Err(Diagnostic::new(Code::E108, "closure patch lost"));
                }
            }
        }
        Ok(())
    }

    fn bodies_parallel(&mut self, m: &A::Module, jobs: usize) -> Result<(), Diagnostic> {
        let mut queue: Vec<(usize, BodySrc)> = Vec::new();
        let mut implicit_super: BTreeMap<usize, usize> = BTreeMap::new();
        self.propagate_init_throws(m);
        for decl in &m.decls {
            match &decl.node {
                A::Decl::Fn(f) => {
                    let id = self.out.fn_index[&f.name];
                    queue.push((id, BodySrc::Fn(f.clone())));
                }
                A::Decl::Class { name, members, .. }
                | A::Decl::Struct { name, members, .. } => {
                    for mem in members {
                        match &mem.node {
                            A::ClassMember::Method(f) => {
                                let q = format!("{}.{}", name, f.name);
                                queue.push((self.out.fn_index[&q], BodySrc::Fn(f.clone())));
                            }
                            A::ClassMember::Init { params, body } => {
                                let q = format!("{name}.init");
                                let id = self.out.fn_index[&q];
                                self.plan_super(m, name, body, mem.span, id, &mut implicit_super);
                                queue.push((
                                    id,
                                    BodySrc::Init(params.clone(), body.clone()),
                                ));
                            }
                            A::ClassMember::Deinit(body) => {
                                let q = format!("{name}.deinit");
                                queue.push((self.out.fn_index[&q], BodySrc::Bare(vec![], body.clone())));
                            }
                            A::ClassMember::OnReload { params, body } => {
                                let q = format!("{name}.onReload");
                                queue.push((
                                    self.out.fn_index[&q],
                                    BodySrc::Bare(params.clone(), body.clone()),
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                A::Decl::Extension { target, members, .. } => {
                    let tname = self
                        .ext_target_name(target, crate::instr::UNKNOWN_SPAN)
                        .unwrap_or_default();
                    for mem in members {
                        if let A::ClassMember::Method(f) = &mem.node {
                            let q = format!("__ext_{tname}__{}", f.name);
                            if let Some(id) = self.out.fn_index.get(&q).copied() {
                                queue.push((id, BodySrc::Fn(f.clone())));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        while !queue.is_empty() {
            let level: Vec<(usize, BodySrc)> = std::mem::take(&mut queue);
            let outputs: Vec<BodyOut> = crate::parallel::with_pool(jobs, || {
                let s = Shared {
                    module: &self.out,
                    defaults: &self.defaults,
                    param_info: &self.param_info,
                    param_tys: &self.param_tys,
                    field_opt: &self.field_opt,
                    ret_tys: &self.ret_tys,
                    nub_fields: &self.nub_fields,
                    generic_fns: &self.generic_fns,
                    extensions: &self.extensions,
                    ext_self: &self.ext_self,
                    mod_consts: &self.mod_consts,
                };
                level
                    .into_par_iter()
                    .map(|(id, src)| lower_one(&s, &implicit_super, id, src))
                    .collect()
            });
            for out in outputs {
                self.ingest(&mut queue, out)?;
            }
        }
        Ok(())
    }
}
