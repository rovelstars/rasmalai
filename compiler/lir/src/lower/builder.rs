use super::*;

impl Lower {
    pub(super) fn derr(code: Code, msg: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic::new(code, msg).with_span(span)
    }
}

fn implicit_return_ok(ret: &LirType) -> bool {
    matches!(ret, LirType::Void | LirType::Any | LirType::Null)
}

fn term_targets(t: &Terminator) -> Vec<BlockId> {
    match t {
        Terminator::Ret(_) | Terminator::Unreachable { .. } => Vec::new(),
        Terminator::Br(bb) => vec![*bb],
        Terminator::BrIf { then_bb, else_bb, .. } => vec![*then_bb, *else_bb],
        Terminator::BrErr { catch_bb, next_bb, .. } => vec![*catch_bb, *next_bb],
        Terminator::Switch { cases, default, .. } => {
            let mut out: Vec<BlockId> = cases.iter().map(|(_, bb)| *bb).collect();
            out.push(*default);
            out
        }
        Terminator::Throw { catch, .. } => catch.map(|(bb, _, _)| vec![bb]).unwrap_or_default(),
        Terminator::Rethrow { catch_bb, .. } => vec![*catch_bb],
    }
}

impl<'a> Builder<'a> {
    pub(super) fn new(
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
        id: usize,
        prefix: String,
    ) -> Builder<'a> {
        let func = module.functions[id].clone();
        let has_self = func.method_self;
        let enclosing = func.name.rsplit_once('.').and_then(|(head, _)| {
            if has_self {
                Some(head.to_string())
            } else {
                None
            }
        });
        Builder {
            module,
            defaults,
            param_info,
            param_tys,
            field_opt,
            ret_tys,
            nub_fields,
            nub: BTreeSet::new(),
            precise: BTreeMap::new(),
            var_targs: BTreeMap::new(),
            targ_log: Vec::new(),
            scope_marks: Vec::new(),
            generic_fns,
            extensions,
            ext_self,
            mod_consts,
            func,
            blocks: vec![Vec::new()],
            terms: vec![None],
            current: 0,
            terminated: false,
            scopes: vec![BTreeMap::new()],
            self_local: if has_self { Some(0) } else { None },
            this_alias: None,
            throws_fn: false,
            is_init: false,
            implicit_super: None,
            loop_stack: Vec::new(),
            catch_stack: Vec::new(),
            fallthrough: None,
            defer_count: 0,
            unsafe_depth: 0,
            enclosing,
            fn_tparams: Vec::new(),
            moved: BTreeSet::new(),
            fresh_arrays: BTreeSet::new(),
            tuples: BTreeMap::new(),
            records: BTreeMap::new(),
            ranges: BTreeMap::new(),
            closure_seq: 0,
            closure_prefix: prefix,
            closure_hints: Vec::new(),
            closure_ret: BTreeMap::new(),
            synth_seq: 0,
            pending: Vec::new(),
            patches: Vec::new(),
            diags: Vec::new(),
            failed: false,
        }
    }

    pub(super) fn finish(mut self) -> (Function, Vec<(BlockId, usize, String)>, Vec<PendingClosure>, Vec<Diagnostic>) {
        let mut reachable = vec![false; self.blocks.len()];
        if let Some(entry) = reachable.get_mut(0) {
            *entry = true;
        }
        let mut changed = true;
        while changed {
            changed = false;
            for (idx, term) in self.terms.iter().enumerate() {
                if !reachable.get(idx).copied().unwrap_or(false) {
                    continue;
                }
                if let Some(t) = term {
                    for bb in term_targets(t) {
                        if let Some(slot) = reachable.get_mut(bb) {
                            if !*slot {
                                *slot = true;
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
        if !self.terminated {
            let bb = self.current;
            let span = self.block_span(bb).or_else(|| self.last_real_span());
            if reachable.get(bb).copied().unwrap_or(true) {
                if implicit_return_ok(&self.func.ret) {
                    self.set_term(Terminator::Ret(vec![]));
                } else if span.is_some() {
                    let name = self.func.name.clone();
                    self.ice_note(format!("function `{name}` can reach the end without returning a value"), span);
                    self.set_term(Terminator::Ret(vec![]));
                } else {
                    self.set_term(Terminator::Ret(vec![]));
                }
            } else {
                self.set_term(Terminator::Ret(vec![]));
            }
        }
        let mut idx = 0;
        while idx < self.terms.len() {
            if self.terms[idx].is_none() {
                if idx == 0 || reachable.get(idx).copied().unwrap_or(true) {
                    let span = self.block_span(idx).or_else(|| self.last_real_span());
                    let name = self.func.name.clone();
                    self.ice_note(format!("unterminated block {idx} in `{name}`"), span);
                }
                self.terms[idx] = Some(Terminator::Ret(vec![]));
            }
            idx += 1;
        }
        let mut blocks = Vec::with_capacity(self.blocks.len());
        let mut missing = Vec::new();
        let old_blocks = std::mem::take(&mut self.blocks);
        let old_terms = std::mem::take(&mut self.terms);
        for (idx, (instrs, term)) in old_blocks.into_iter().zip(old_terms.into_iter()).enumerate() {
            match term {
                Some(t) => blocks.push(crate::instr::Block { instrs, term: t }),
                None => {
                    missing.push(idx);
                    blocks.push(crate::instr::Block { instrs, term: Terminator::Ret(vec![]) });
                }
            }
        }
        for idx in missing {
            let name = self.func.name.clone();
            self.ice_note(format!("block {idx} in `{name}` left unterminated"), None);
        }
        self.func.blocks = blocks;
        (self.func, self.patches, self.pending, self.diags)
    }

    pub(super) fn fail<T>(&mut self, d: Diagnostic) -> Result<T, Diagnostic> {
        self.diags.push(d.clone());
        self.failed = true;
        Err(d)
    }

    pub(super) fn err(&self, code: Code, msg: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic::new(code, msg).with_span(span)
    }

    pub(super) fn ice_note(&mut self, msg: impl Into<String>, span: Option<Span>) {
        let mut d = Diagnostic::new(
            Code::E108,
            format!("internal compiler error: {}", msg.into()),
        );
        if let Some(span) = span {
            d = d.with_span(span);
        }
        self.diags.push(d);
        self.failed = true;
    }

    pub(super) fn ice<T>(&mut self, msg: impl Into<String>, span: Option<Span>) -> Result<T, Diagnostic> {
        let mut d = Diagnostic::new(
            Code::E108,
            format!("internal compiler error: {}", msg.into()),
        );
        if let Some(span) = span {
            d = d.with_span(span);
        }
        self.diags.push(d.clone());
        self.failed = true;
        Err(d)
    }

    pub(super) fn unknown_name(&self, n: &str, span: Span) -> Diagnostic {
        let mut d = Diagnostic::new(Code::E108, format!("unknown `{n}`")).with_span(span);
        if let Some(hint) = removed_option_hint(n) {
            d = d.with_hint(hint);
        }
        d
    }

    pub(super) fn unknown_variant(&self, vname: &str, span: Span) -> Diagnostic {
        let mut d =
            Diagnostic::new(Code::E108, format!("unknown variant `{vname}`")).with_span(span);
        if let Some(hint) = removed_option_hint(vname) {
            d = d.with_hint(hint);
        }
        d
    }

    pub(super) fn new_block(&mut self) -> BlockId {
        self.blocks.push(Vec::new());
        self.terms.push(None);
        self.blocks.len() - 1
    }

    pub(super) fn set_current(&mut self, bb: BlockId) {
        self.current = bb;
        self.terminated = self.terms[bb].is_some();
    }

    pub(super) fn set_term(&mut self, t: Terminator) {
        self.terms[self.current] = Some(t);
        self.terminated = true;
    }

    pub(super) fn block_span(&self, bb: BlockId) -> Option<Span> {
        let instrs = self.blocks.get(bb)?;
        for ins in instrs.iter().rev() {
            let s = ins.span();
            if s != crate::instr::UNKNOWN_SPAN {
                return Some(s);
            }
        }
        None
    }

    pub(super) fn last_real_span(&self) -> Option<Span> {
        let mut bb = self.blocks.len();
        while bb > 0 {
            bb -= 1;
            if let Some(s) = self.block_span(bb) {
                return Some(s);
            }
        }
        None
    }

    pub(super) fn emit(&mut self, i: Instr) {
        self.blocks[self.current].push(i);
    }

    pub(super) fn emit_copy(&mut self, dst: Local, src: Local, span: Span) {
        match self.closure_ret.get(&src).cloned() {
            Some(r) => {
                self.closure_ret.insert(dst, r);
            }
            None => {
                self.closure_ret.remove(&dst);
            }
        }
        if let Some(elems) = self.tuples.get(&src).cloned() {
            self.tuples.insert(dst, elems);
            return;
        }
        if let Some(fields) = self.records.get(&src).cloned() {
            self.records.insert(dst, fields);
            return;
        }
        if let Some(parts) = self.ranges.get(&src).cloned() {
            self.ranges.insert(dst, parts);
            return;
        }
        if dst != src {
            if let Some(pt) = self.precise.get(&src).cloned() {
                self.precise.insert(dst, pt);
            }
            let dto = matches!(self.func.locals.get(dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Error));
            let sto = matches!(self.func.locals.get(src as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Error));
            if dto && sto {
                self.moved.insert(src);
            }
        }
        if self.emit_any_crossing(dst, src, span) {
            return;
        }
        self.emit(Instr::Copy { span, dst, src });
    }

    pub(super) fn emit_cast(&mut self, dst: Local, src: Local, span: Span) {
        if dst != src {
            let dto = matches!(
                self.func.locals.get(dst as usize),
                Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))
            );
            if dto {
                self.moved.insert(src);
            }
        }
        let src_is_error = self.func.locals.get(src as usize) == Some(&LirType::Error);
        let dst_is_obj = matches!(
            self.func.locals.get(dst as usize),
            Some(LirType::Obj(_)) | Some(LirType::Array(_)) | Some(LirType::Enum(_))
        );
        if src_is_error && dst_is_obj {
            self.emit(Instr::Call {
                span,
                dsts: vec![dst],
                err: None,
                target: CallTarget::Builtin("__rnx_error_unbox".to_string()),
                args: vec![src],
            });
            return;
        }
        if self.emit_any_crossing(dst, src, span) {
            return;
        }
        self.emit(Instr::Cast { span, dst, src });
    }

    pub(super) fn local(&mut self, ty: LirType) -> Local {
        let id = self.func.locals.len() as Local;
        self.func.locals.push(ty);
        id
    }

    pub(super) fn def(&mut self, name: String, local: Local, simple: Simple, ty: LirType) {
        match self.scopes.last_mut() {
            Some(scope) => {
                scope.insert(name, (local, simple, ty));
            }
            None => self.ice_note("block scope stack underflow", None),
        }
    }
}
