use super::*;

impl<'a> Builder<'a> {
    pub(super) fn lower_closure(
        &mut self,
        decay: bool,
        params: &[A::Param],
        ret: Option<A::Type>,
        throws: bool,
        body: &A::FnBody,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let seq = self.closure_seq;
        self.closure_seq += 1;
        let fname = format!("{}closure#{seq}", self.closure_prefix);
        let own: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let free = closure_free_vars(body, params);
        let mut caps: Vec<Capture> = Vec::new();
        for scope in &self.scopes {
            for (n, (l, _, t)) in scope {
                if own.iter().any(|o| o == n) {
                    continue;
                }
                if !free.contains(n) {
                    continue;
                }
                if caps.iter().any(|c| c.name.as_ref() == Some(n)) {
                    continue;
                }
                caps.push(Capture {
                    name: Some(n.clone()),
                    ty: t.clone(),
                    src: *l,
                    is_this: false,
                    ret: self.closure_ret.get(l).cloned(),
                });
            }
        }
        let mut decay_this = false;
        if decay && body_uses_this(body) {
            if let Some(slf) = self.self_local {
                let ty = self.func.locals[slf as usize].clone();
                caps.push(Capture {
                    name: None,
                    ty,
                    src: slf,
                    is_this: true,
                    ret: None,
                });
                decay_this = true;
            }
        }
        let mut argv = Vec::with_capacity(caps.len());
        for c in &caps {
            argv.push(c.src);
        }
        let dst = self.local(LirType::Closure);
        let ret_ty = ret.as_ref().map(|t| self.resolve_here(t)).unwrap_or(LirType::Any);
        if Self::any_box_tag(&ret_ty).is_some() {
            self.closure_ret.insert(dst, ret_ty);
        }
        let ix = self.blocks[self.current].len();
        self.emit(Instr::ClosureNew { span, 
            dst,
            func: usize::MAX,
            captures: argv,
            decay,
            decay_this,
        });
        self.patches.push((self.current, ix, fname.clone()));
        self.pending.push(PendingClosure {
            name: fname,
            params: params.to_vec(),
            param_hints: self.closure_hints.last().cloned().unwrap_or_default(),
            ret: ret.clone(),
            throws,
            body: body.clone(),
            captures: caps,
            decay_this,
            prefix: format!("{}closure#{seq}::", self.closure_prefix),
            enclosing: self.enclosing.clone(),
        });
        let _ = span;
        Ok((dst, Simple::Other))
    }

    pub(super) fn ext_hint_target(&self, ty: &LirType, field: &str) -> Option<(String, usize)> {
        if matches!(ty, LirType::Array(_)) && (field == "map" || field == "filter") {
            return None;
        }
        if let LirType::Obj(name) = ty {
            if let Some(ci) = self.module.class_index.get(name)
                && self.module.classes[*ci].methods.contains_key(field) {
                    return None;
                }
            if self.module.interface_index.contains_key(name) {
                return None;
            }
            let short = name.rsplit('.').next().unwrap_or(name);
            if short == "Channel" && field == "send" {
                return None;
            }
        }
        if let LirType::Enum(ei) = ty {
            let ename = self.module.enums.get(*ei).map(|d| d.name.as_str()).unwrap_or("");
            if ename == "std.prelude.Result" {
                match field {
                    "isOk" | "isErr" | "unwrap" | "unwrapOr" | "map" => {
                        return None;
                    }
                    _ => {}
                }
            }
        }
        if matches!(ty, LirType::Pointer(_)) && (field == "join" || field == "await") {
            return None;
        }
        let eid = self.find_extension(ty, field)?;
        let mname = self.module.functions[eid].name.clone();
        if !self.ext_self.contains_key(&mname) {
            return None;
        }
        Some((mname, eid))
    }

    pub(super) fn ext_closure_hint(
        &self,
        mname: &str,
        idx: usize,
        prefix: &[(A::Type, LirType)],
    ) -> Option<Vec<LirType>> {
        let tys = self.param_tys.get(mname)?;
        let (_, pty) = tys.get(idx)?;
        let sig = pty.fn_sig.as_ref()?;
        let mut tparams: Vec<String> = Vec::new();
        if let Some(g) = self.generic_fns.get(mname) {
            tparams.extend(g.tparams.iter().cloned());
        }
        if let Some(self_ty) = self.ext_self.get(mname) {
            for a in &self_ty.args {
                if a.fn_sig.is_none() && a.args.is_empty() && a.tuple.is_empty() && a.path.len() == 1 {
                    let n = &a.path[0];
                    if !tparams.iter().any(|p| p == n) {
                        tparams.push(n.clone());
                    }
                }
            }
        }
        let mut subst: BTreeMap<String, LirType> = BTreeMap::new();
        for (decl, actual) in prefix.iter() {
            let _ = self.unify_generic(&tparams, decl, actual, &mut subst);
        }
        let mut out = Vec::with_capacity(sig.params.len());
        for pt in &sig.params {
            if pt.fn_sig.is_none() && pt.args.is_empty() && pt.tuple.is_empty() && pt.path.len() == 1
                && let Some(s) = subst.get(&pt.path[0]) {
                    out.push(s.clone());
                    continue;
                }
            out.push(self.resolve_here(pt));
        }
        Some(out)
    }

    pub(super) fn closure_hint_for(
        &self,
        fname: &str,
        idx: usize,
        prefix: &[(A::Type, LirType)],
    ) -> Option<Vec<LirType>> {
        let tys = self.param_tys.get(fname)?;
        let (_, pty) = tys.get(idx)?;
        let sig = pty.fn_sig.as_ref()?;
        let mut subst: BTreeMap<String, LirType> = BTreeMap::new();
        if let Some(g) = self.generic_fns.get(fname) {
            for (decl, actual) in prefix.iter() {
                let _ = self.unify_generic(&g.tparams, decl, actual, &mut subst);
            }
        }
        let mut out = Vec::with_capacity(sig.params.len());
        for pt in &sig.params {
            if pt.fn_sig.is_none() && pt.args.is_empty() && pt.tuple.is_empty() && pt.path.len() == 1 {
                if let Some(s) = subst.get(&pt.path[0]) {
                    out.push(s.clone());
                    continue;
                }
            }
            out.push(self.resolve_here(pt));
        }
        Some(out)
    }

    pub(super) fn lower_closure_body(&mut self, p: PendingClosure) -> Result<(), Diagnostic> {
        self.deny_tuple_sig()?;
        self.throws_fn = p.throws;
        for (i, param) in p.params.iter().enumerate() {
            let hint = p.param_hints.get(i).cloned().unwrap_or(LirType::Any);
            let ty = param
                .ty
                .as_ref()
                .map(|t| self.resolve_here(t))
                .filter(|t| *t != LirType::Any)
                .unwrap_or(hint);
            self.def(param.name.clone(), i as Local, simple_of(&ty), ty.clone());
            if let Some(sig) = param.ty.as_ref().and_then(|t| t.fn_sig.as_ref()) {
                if let Some(r) = sig.ret.as_ref().map(|r| self.resolve_here(r)) {
                    if Self::any_box_tag(&r).is_some() {
                        self.closure_ret.insert(i as Local, r);
                    }
                }
            }
            if matches!(ty, LirType::I64 | LirType::Bool | LirType::F64(_) | LirType::Str) {
                let tmp = self.local(ty.clone());
                self.emit_any_unbox(tmp, i as Local, crate::instr::UNKNOWN_SPAN);
                self.def(param.name.clone(), tmp, simple_of(&ty), ty);
            }
        }
        for (j, cap) in p.captures.iter().enumerate() {
            let local = (p.params.len() + j) as Local;
            if cap.is_this {
                self.this_alias = Some(local);
            } else if let Some(n) = &cap.name {
                self.def(n.clone(), local, simple_of(&cap.ty), cap.ty.clone());
                if let Some(r) = cap.ret.clone() {
                    if Self::any_box_tag(&r).is_some() {
                        self.closure_ret.insert(local, r);
                    }
                }
            }
        }
        let _ = p.decay_this;
        match &p.body {
            A::FnBody::Block(b) => self.lower_block(b),
            A::FnBody::Expr(e) => {
                let (v, _) = self.lower_expr(&e.node, e.span)?;
                self.deny_tuple(v, "in a return value", e.span)?;
                let rt = self.func.ret.clone();
                let rvt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
                let fresh = matches!(e.node, A::Expr::Array(_));
                self.check_array_view(&rvt, &rt, fresh, e.span, "return value")?;
                let v = self.coerce_to_slot(v, &rt, e.span);
                self.emit_run_defers(0);
                self.set_term(Terminator::Ret(vec![v]));
                Ok(())
            }
        }
    }
}
