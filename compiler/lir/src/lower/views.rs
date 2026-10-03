use super::*;
use diagnostics::{Code, Diagnostic, Span};
impl<'a> Builder<'a> {
    pub(super) fn any_box_tag(ty: &LirType) -> Option<i64> {
        // Must match TAG_INT/TAG_BOOL/TAG_FLOAT/TAG_STR in runtime native.rs.
        match ty {
            LirType::I64 => Some(0),
            LirType::Bool => Some(1),
            LirType::F64(_) => Some(2),
            LirType::Str => Some(3),
            _ => None,
        }
    }

    pub(super) fn emit_any_box(&mut self, dst: Local, tag: i64, src: Local, span: Span) {
        let tag_local = self.local(LirType::I64);
        self.emit(Instr::Const { span, dst: tag_local, lit: Lit::Int(tag) });
        self.emit(Instr::Call {
            span,
            dsts: vec![dst],
            err: None,
            target: CallTarget::Builtin("__rnx_any_box".to_string()),
            args: vec![tag_local, src],
        });
    }

    pub(super) fn emit_any_unbox(&mut self, dst: Local, src: Local, span: Span) {
        self.emit(Instr::Call {
            span,
            dsts: vec![dst],
            err: None,
            target: CallTarget::Builtin("__rnx_any_unbox".to_string()),
            args: vec![src],
        });
    }

    pub(super) fn nub_has(&self, v: Local) -> bool {
        self.nub.contains(&v)
    }

    pub(super) fn nub_scope_has(&self, v: Local) -> bool {
        self.scopes
            .iter()
            .any(|s| s.values().any(|(l, _, _)| *l == v))
    }

    pub(super) fn nub_store(&mut self, dst: Local, src: Local, span: Span) {
        if dst == src {
            self.nub_mark(dst);
            return;
        }
        if self.nub_scope_has(src) {
            self.nub_retain(src, span);
        } else {
            self.nub_unmark(src);
        }
        self.emit_copy(dst, src, span);
        self.nub_mark(dst);
    }

    pub(super) fn nub_mark(&mut self, v: Local) {
        self.nub.insert(v);
    }

    pub(super) fn nub_unmark(&mut self, v: Local) {
        self.nub.remove(&v);
    }

    pub(super) fn nub_retain(&mut self, v: Local, span: Span) {
        self.emit(Instr::Call {
            span,
            dsts: vec![],
            err: None,
            target: CallTarget::Builtin("__rnx_any_retain".to_string()),
            args: vec![v],
        });
    }

    pub(super) fn nub_release(&mut self, v: Local, span: Span) {
        self.emit(Instr::Call {
            span,
            dsts: vec![],
            err: None,
            target: CallTarget::Builtin("__rnx_any_release_box".to_string()),
            args: vec![v],
        });
    }

    pub(super) fn nub_box(&mut self, dst: Local, src: Local, span: Span) {
        let st = self.func.locals.get(src as usize).cloned().unwrap_or(LirType::Any);
        let tag = Self::any_box_tag(&st).unwrap_or(0);
        self.emit_any_box(dst, tag, src, span);
        self.nub.insert(dst);
    }

    pub(super) fn nub_arg(&mut self, a: Local, slot: &LirType, span: Span) -> Local {
        if self.nub_has(a) {
            if *slot == LirType::Any {
                if self.nub_scope_has(a) {
                    self.nub_retain(a, span);
                } else {
                    self.nub_unmark(a);
                }
                return a;
            }
            if self.nub_scope_has(a) {
                let dst = self.local(slot.clone());
                self.nub_store(dst, a, span);
                return dst;
            }
            self.nub_unmark(a);
            return a;
        }
        let at = self.func.locals.get(a as usize).cloned().unwrap_or(LirType::Any);
        match at {
            LirType::I64 | LirType::Bool | LirType::F64(_) => {
                let dst = self.local(at);
                self.nub_box(dst, a, span);
                dst
            }
            LirType::Null => a,
            LirType::Any => {
                let dst = self.local(slot.clone());
                self.emit_any_unbox(dst, a, span);
                dst
            }
            _ => a,
        }
    }

    pub(super) fn nub_cond(&mut self, v: Local, span: Span) -> Local {
        if self.nub_has(v) {
            let ty = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Bool);
            let dst = self.local(ty);
            self.emit_any_unbox(dst, v, span);
            dst
        } else {
            v
        }
    }

    pub(super) fn nub_use(&mut self, v: Local, span: Span) -> Local {
        if self.nub.contains(&v) {
            let ty = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::I64);
            let dst = self.local(ty);
            self.emit_any_unbox(dst, v, span);
            dst
        } else {
            v
        }
    }

    pub(super) fn emit_retain(&mut self, dst: Local, span: Span) {
        let ty = self.func.locals.get(dst as usize).cloned().unwrap_or(LirType::Any);
        match &ty {
            LirType::Str | LirType::Array(_) | LirType::Enum(_) => {
                self.emit(Instr::Retain { span, obj: dst });
            }
            LirType::Obj(name) => {
                if self.module.class_index.contains_key(name) {
                    self.emit(Instr::Retain { span, obj: dst });
                } else {
                    self.emit(Instr::Call {
                        span,
                        dsts: vec![],
                        err: None,
                        target: CallTarget::Builtin("__rnx_any_retain".to_string()),
                        args: vec![dst],
                    });
                }
            }
            LirType::Any => {
                self.emit(Instr::Call {
                    span,
                    dsts: vec![],
                    err: None,
                    target: CallTarget::Builtin("__rnx_any_retain".to_string()),
                    args: vec![dst],
                });
            }
            _ => {}
        }
    }

    pub(super) fn deny_tuple(&self, v: Local, ctx: &str, span: Span) -> Result<(), Diagnostic> {
        if self.tuples.contains_key(&v) {
            return Err(self.err(
                Code::E108,
                format!("tuple value cannot be used {ctx}"),
                span,
            ));
        }
        if self.records.contains_key(&v) {
            return Err(self.err(
                Code::E108,
                format!("record value cannot be used {ctx}"),
                span,
            ));
        }
        if self.ranges.contains_key(&v) {
            return Err(self.err(
                Code::E108,
                format!("range value cannot be used {ctx}"),
                span,
            ));
        }
        Ok(())
    }

    pub(super) fn emit_any_crossing(&mut self, dst: Local, src: Local, span: Span) -> bool {
        if dst == src {
            return false;
        }
        let st = self.func.locals.get(src as usize).cloned().unwrap_or(LirType::Any);
        let dt = self.func.locals.get(dst as usize).cloned().unwrap_or(LirType::Any);
        if dt == LirType::Any {
            if let Some(tag) = Self::any_box_tag(&st) {
                self.emit_any_box(dst, tag, src, span);
                return true;
            }
            return false;
        }
        if st == LirType::Any {
            match &dt {
                LirType::I64 | LirType::Bool | LirType::F64(_) | LirType::Str => {
                    self.emit_any_unbox(dst, src, span);
                    return true;
                }
                LirType::Array(_) | LirType::Obj(_) => {
                    return false;
                }
                _ => return false,
            }
        }
        false
    }

    pub(super) fn unbox_to_int(&mut self, v: Local, span: Span) -> Local {
        let dst = self.local(LirType::I64);
        self.emit(Instr::Call { span, dsts: vec![dst], err: None, target: CallTarget::Builtin("__rnx_any_unbox".to_string()), args: vec![v] });
        dst
    }

    pub(super) fn unbox_operand(&mut self, v: Local, span: Span) -> Local {
        if self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any) == LirType::Any {
            let dst = self.local(LirType::I64);
            self.emit_any_unbox(dst, v, span);
            dst
        } else {
            v
        }
    }

    pub(super) fn coerce_to_slot(&mut self, v: Local, slot: &LirType, span: Span) -> Local {
        let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
        if slot == &LirType::Any {
            if Self::any_box_tag(&vt).is_some() {
                let dst = self.local(LirType::Any);
                self.emit_any_box(dst, Self::any_box_tag(&vt).unwrap_or(0), v, span);
                return dst;
            }
            return v;
        }
        if let LirType::Pointer(want) = slot {
            if matches!(vt, LirType::Pointer(_)) && !matches!(**want, LirType::Any) {
                let dst = self.local(slot.clone());
                self.emit_copy(dst, v, span);
                return dst;
            }
            return v;
        }
        if vt == LirType::Any {
            match slot {
                LirType::I64 | LirType::I8 | LirType::Bool | LirType::F64(_) | LirType::Str => {
                    let dst = self.local(slot.clone());
                    self.emit_any_unbox(dst, v, span);
                    return dst;
                }
                _ => return v,
            }
        }
        if slot == &LirType::I8 && vt == LirType::I64 {
            let dst = self.local(LirType::I8);
            self.emit_copy(dst, v, span);
            return dst;
        }
        v
    }

    pub(super) fn elem_size_of(&self, arr: Local) -> usize {
        match self.func.locals.get(arr as usize) {
            Some(LirType::Array(inner)) => elem_size(inner),
            _ => 8,
        }
    }

    pub(super) fn const_int(&mut self, n: i64, span: Span) -> Local {
        let dst = self.local(LirType::I64);
        self.emit(Instr::Const { span, dst, lit: Lit::Int(n) });
        dst
    }

    pub(super) fn emit_array_extend(
        &mut self,
        dst: Local,
        src: Local,
        elem_ty: &LirType,
        start: Local,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let es_get = self.elem_size_of(src);
        let es_push = elem_size(elem_ty);
        let len = self.local(LirType::I64);
        self.emit(Instr::ArrayLen { span, dst: len, arr: src });
        let i = self.local(LirType::I64);
        self.emit_copy(i, start, span);
        let head = self.new_block();
        let body = self.new_block();
        let exit = self.new_block();
        self.set_term(Terminator::Br(head));
        self.set_current(head);
        let cond = self.local(LirType::Bool);
        self.emit(Instr::Cmp { span, op: CmpOp::Lt, kind: NumKind::Int, dst: cond, lhs: i, rhs: len });
        self.set_term(Terminator::BrIf { span, cond, then_bb: body, else_bb: exit });
        self.set_current(body);
        let item = self.local(LirType::Any);
        self.emit(Instr::ArrayGet { span, dst: item, arr: src, index: i, elem_size: es_get, unchecked: true });
        let item = self.coerce_to_slot(item, elem_ty, span);
        self.emit(Instr::ArrayPush { span, arr: dst, value: item, elem_size: es_push });
        let one = self.const_int(1, span);
        let next = self.local(LirType::I64);
        self.emit(Instr::Arith { span, op: ArithOp::Add, kind: NumKind::Int, dst: next, lhs: i, rhs: one });
        self.emit_copy(i, next, span);
        self.set_term(Terminator::Br(head));
        self.set_current(exit);
        Ok(())
    }

    pub(super) fn deny_nested_tuple(ty: &LirType, span: Span) -> Result<(), Diagnostic> {
        match ty {
            LirType::Tuple(items) => {
                for it in items {
                    if matches!(it, LirType::Tuple(_) | LirType::Range) {
                        return Err(Diagnostic::new(
                            Code::E108,
                            "nested tuples and ranges in tuples are not supported",
                        )
                        .with_span(span));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(super) fn static_type_name(module: &Module, ty: &LirType) -> Option<String> {
        match ty {
            LirType::I64 => Some("Int".to_string()),
            LirType::Bool => Some("Bool".to_string()),
            LirType::F64(_) => Some("Float".to_string()),
            LirType::Str => Some("String".to_string()),
            LirType::Null => Some("Null".to_string()),
            LirType::Array(_) => Some("Array".to_string()),
            LirType::Range => Some("Range".to_string()),
            LirType::Closure => Some("Closure".to_string()),
            LirType::Pointer(_) => Some("Pointer".to_string()),
            LirType::GenRef(_) => Some("GenRef".to_string()),
            LirType::Tuple(_) => Some("Tuple".to_string()),
            LirType::Obj(name) => Some(name.rsplit('.').next().unwrap_or(name).to_string()),
            LirType::Enum(ei) => module
                .enums
                .get(*ei)
                .map(|e| e.name.rsplit('.').next().unwrap_or(&e.name).to_string()),
            _ => None,
        }
    }

    pub(super) fn emit_tag_check(&mut self, dst: Local, v: Local, tag: i64, span: Span) {
        let t = self.local(LirType::I64);
        self.emit(Instr::Call { span, dsts: vec![t], err: None, target: CallTarget::Builtin("__rnx_any_tag".to_string()), args: vec![v] });
        let k = self.const_int(tag, span);
        self.emit(Instr::Cmp { span, op: CmpOp::Eq, kind: NumKind::Int, dst, lhs: t, rhs: k });
    }

    pub(super) fn emit_class_check(&mut self, dst: Local, v: Local, ci: usize, span: Span) {
        let mut cis = vec![ci];
        cis.extend(crate::instr::subclasses_of(self.module, ci));
        let class_fn = if self.func.locals.get(v as usize) == Some(&LirType::Error) {
            "__rnx_error_class"
        } else {
            "__rnx_obj_class"
        };
        if cis.len() == 1 {
            let c = self.local(LirType::I64);
            self.emit(Instr::Call { span, dsts: vec![c], err: None, target: CallTarget::Builtin(class_fn.to_string()), args: vec![v] });
            let k = self.const_int(ci as i64 + 1, span);
            self.emit(Instr::Cmp { span, op: CmpOp::Eq, kind: NumKind::Int, dst, lhs: c, rhs: k });
            return;
        }
        let cls = self.local(LirType::I64);
        self.emit(Instr::Call { span, dsts: vec![cls], err: None, target: CallTarget::Builtin(class_fn.to_string()), args: vec![v] });
        let merge = self.new_block();
        let mut cases = Vec::with_capacity(cis.len());
        for c in &cis {
            let arm = self.new_block();
            cases.push((crate::instr::SwitchPat::Int(*c as i64 + 1), arm));
        }
        let default = self.new_block();
        let arms: Vec<BlockId> = cases.iter().map(|(_, b)| *b).collect();
        self.set_term(crate::instr::Terminator::Switch { span, scrut: cls, cases, default });
        for arm in arms {
            self.set_current(arm);
            self.emit_const_bool(dst, true, span);
            self.set_term(crate::instr::Terminator::Br(merge));
        }
        self.set_current(default);
        self.emit_const_bool(dst, false, span);
        self.set_term(crate::instr::Terminator::Br(merge));
        self.set_current(merge);
    }

    pub(super) fn implementors(&self, ii: usize) -> Vec<usize> {
        self.module
            .classes
            .iter()
            .enumerate()
            .filter(|(_, c)| c.ifaces.contains(&ii))
            .map(|(ci, _)| ci)
            .collect()
    }

    pub(super) fn emit_iface_check(&mut self, dst: Local, v: Local, ii: usize, span: Span) {
        let class_fn = if self.func.locals.get(v as usize) == Some(&LirType::Error) {
            "__rnx_error_class"
        } else {
            "__rnx_obj_class"
        };
        let cls = self.local(LirType::I64);
        self.emit(Instr::Call { span, dsts: vec![cls], err: None, target: CallTarget::Builtin(class_fn.to_string()), args: vec![v] });
        let cis = self.implementors(ii);
        let merge = self.new_block();
        let mut cases = Vec::with_capacity(cis.len());
        for ci in &cis {
            let arm = self.new_block();
            cases.push((crate::instr::SwitchPat::Int(*ci as i64 + 1), arm));
        }
        let default = self.new_block();
        let arms: Vec<BlockId> = cases.iter().map(|(_, b)| *b).collect();
        self.set_term(crate::instr::Terminator::Switch { span, scrut: cls, cases, default });
        for arm in arms {
            self.set_current(arm);
            self.emit_const_bool(dst, true, span);
            self.set_term(crate::instr::Terminator::Br(merge));
        }
        self.set_current(default);
        self.emit_const_bool(dst, false, span);
        self.set_term(crate::instr::Terminator::Br(merge));
        self.set_current(merge);
    }

    pub(super) fn find_extension(&self, ty: &LirType, method: &str) -> Option<usize> {
        let target: String = match ty {
            LirType::Str => "String".to_string(),
            LirType::I64 => "Int".to_string(),
            LirType::Bool => "Bool".to_string(),
            LirType::F64(FloatKind::Strict) => "Float".to_string(),
            LirType::F64(FloatKind::Fast) => "FastFloat".to_string(),
            LirType::Array(_) => "Array".to_string(),
            LirType::Enum(ei) => short_name(&self.module.enums.get(*ei)?.name).to_string(),
            LirType::Obj(n) => {
                if self.module.interface_index.contains_key(n) {
                    return None;
                }
                short_name(n).to_string()
            }
            _ => return None,
        };
        if let Some(id) = self.extensions.get(&(target.clone(), method.to_string())).copied() {
            return Some(id);
        }
        if matches!(ty, LirType::F64(_)) {
            let alt = if target == "Float" { "FastFloat" } else { "Float" };
            if let Some(id) = self.extensions.get(&(alt.to_string(), method.to_string())).copied() {
                return Some(id);
            }
        }
        None
    }

    pub(super) fn emit_iface_release(&mut self, obj: Local, ii: usize, span: Span) {
        let cls = self.local(LirType::I64);
        self.emit(Instr::Call { span, dsts: vec![cls], err: None, target: CallTarget::Builtin("__rnx_obj_class".to_string()), args: vec![obj] });
        let cis = self.implementors(ii);
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
            self.emit(Instr::ReleaseAs { span, obj, class: ci });
            self.set_term(crate::instr::Terminator::Br(merge));
        }
        self.set_current(default);
        self.set_term(crate::instr::Terminator::Br(merge));
        self.set_current(merge);
    }

    pub(super) fn emit_const_bool(&mut self, dst: Local, b: bool, span: Span) {
        self.emit(Instr::Const { span, dst, lit: Lit::Bool(b) });
    }

    pub(super) fn deny_tuple_sig(&mut self) -> Result<(), Diagnostic> {
        for p in self.func.params.clone() {
            if matches!(p, LirType::Tuple(_)) {
                return self.fail(self.err(
                    Code::E108,
                    "tuples cannot cross function boundaries",
                    crate::instr::UNKNOWN_SPAN,
                ));
            }
        }
        if matches!(self.func.ret, LirType::Tuple(_)) {
            return self.fail(self.err(
                Code::E108,
                "tuples cannot cross function boundaries",
                crate::instr::UNKNOWN_SPAN,
            ));
        }
        Ok(())
    }

    pub(super) fn check_array_view(
        &mut self,
        have: &LirType,
        want: &LirType,
        fresh: bool,
        span: Span,
        ctx: &str,
    ) -> Result<(), Diagnostic> {
        if fresh {
            return Ok(());
        }
        if let (LirType::Array(a), LirType::Array(b)) = (have, want) {
            if !matches!(a.as_ref(), LirType::Any) && a.as_ref() != b.as_ref() {
                return self.fail(self.err(
                    Code::E108,
                    format!(
                        "array element type mismatch: {ctx} has `{}` but the target expects `{}`; annotate both sides with the same element type",
                        switch_ty_name(have),
                        switch_ty_name(want),
                    ),
                    span,
                ).with_hint("a bare `Array` stores boxed elements while `Array<Int>` stores raw values; widening a live array to a bare view corrupts typed reads"));
            }
        }
        Ok(())
    }

}
