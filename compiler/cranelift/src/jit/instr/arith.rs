use super::*;

impl FnLower<'_> {
    pub(super) fn lower_const(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Const { dst, lit , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let v = match lit {
                    Lit::Int(i) => b.ins().iconst(types::I64, *i),
                    Lit::Bool(v) => b.ins().iconst(types::I64, *v as i64),
                    Lit::Null => b.ins().iconst(types::I64, 0),
                    Lit::Str(text) => self.str_addr(b, text)?,
                    Lit::Float(f, _) => b.ins().iconst(types::I64, f.to_bits() as i64),
                };
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_cast(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Cast { dst, src , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let dst_hold = matches!(self.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)));
                if dst_hold && *dst != *src {
                    let keep = self.alias_keep(b, *src);
                    if self.owned.contains(dst) && (self.stack.contains(dst) || self.write_dominates(*dst)) {
                        self.release_any(b, *dst, keep)?;
                    }
                    let v = self.val(b, *src);
                    self.set(b, *dst, v);
                    if !self.stack.contains(src) {
                        self.owned.remove(src);
                        self.ever_owned.remove(src);
                    }
                    if !self.stack.contains(dst) {
                        self.own(*dst);
                    }
                } else {
                    let v = self.val(b, *src);
                    self.set(b, *dst, v);
                }
        Ok(())
    }

    pub(super) fn lower_copy(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Copy { dst, src , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let src_hold = matches!(self.ftypes.get(*src as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Error));
                let dst_hold = matches!(self.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Error));
                let dst_any = matches!(self.ftypes.get(*dst as usize), Some(LirType::Any));
                if dst_any && *dst != *src {
                    let v = self.val(b, *src);
                    self.any_retain_val(b, v);
                    if self.owned.contains(dst) && (self.stack.contains(dst) || self.write_dominates(*dst)) {
                        self.release_any(b, *dst, None)?;
                    }
                    self.set(b, *dst, v);
                    if !self.stack.contains(dst) {
                        self.own(*dst);
                    }
                } else if dst_hold && *dst != *src {
                    let keep = self.alias_keep(b, *src);
                    if self.owned.contains(dst) && (self.stack.contains(dst) || self.write_dominates(*dst)) {
                        self.release_any(b, *dst, keep)?;
                    }
                    let v = self.val(b, *src);
                    self.set(b, *dst, v);
                    if src_hold && !self.stack.contains(src) {
                        let borrowed = !self.owned.contains(src);
                        self.owned.remove(src);
                        self.ever_owned.remove(src);
                        if borrowed && !self.stack.contains(dst) {
                            // Borrow: src is a borrowed parameter, literal, or
                            // another unowned alias, so dst cannot inherit a
                            // counted reference. Take our own: without this
                            // retain, a later release-old (or scope drain) on
                            // dst would free memory dst does not own (a callee
                            // releases nothing it did not retain).
                            if self.erased_obj(*src) {
                                self.any_retain_val(b, v);
                            } else {
                                let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                                b.ins().call(callee, &[v]);
                            }
                        }
                        if !self.stack.contains(dst) {
                            self.own(*dst);
                        }
                    } else if !self.stack.contains(dst) {
                        if matches!(self.ftypes.get(*src as usize), Some(LirType::Any)) {
                            // Any-typed source holding a heap value (e.g. an
                            // awaited promise payload): mirror the
                            // interpreter's shared() retain so dst owns its
                            // reference independently of the source. Must be
                            // tag-dispatched: the value may be a box.
                            self.any_retain_val(b, v);
                        }
                        self.own(*dst);
                    }
                } else {
                    let v = self.val(b, *src);
                    self.set(b, *dst, v);
                }
        Ok(())
    }

    pub(super) fn lower_arith(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Arith { op, kind, dst, lhs, rhs, span, ..} = ins else {
            return Err("unreachable".to_string());
        };
                if matches!(kind, NumKind::Float(_)) {
                    let a = self.f64_of(b, *lhs)?;
                    let c = self.f64_of(b, *rhs)?;
                    let v = match op {
                        ArithOp::Add => b.ins().fadd(a, c),
                        ArithOp::Sub => b.ins().fsub(a, c),
                        ArithOp::Mul => b.ins().fmul(a, c),
                        ArithOp::Div => b.ins().fdiv(a, c),
                        ArithOp::Mod => return Err("float mod unsupported".to_string()),
                        ArithOp::BitAnd | ArithOp::BitOr | ArithOp::BitXor | ArithOp::Shl | ArithOp::Shr | ArithOp::Zshr => {
                            return Err("bitwise operations need `Int` operands".to_string())
                        }
                    };
                    let r = self.bits_of_f64(b, v);
                    self.set(b, *dst, r);
                    return Ok(());
                }
                let a = self.val(b, *lhs);
                let c = self.val(b, *rhs);
                if matches!(op, ArithOp::Div | ArithOp::Mod) {
                    return self.lower_checked_div(b, *span, *op, *dst, a, c);
                }
                let v = match op {
                    ArithOp::Add => b.ins().iadd(a, c),
                    ArithOp::Sub => b.ins().isub(a, c),
                    ArithOp::Mul => b.ins().imul(a, c),
                    ArithOp::Div | ArithOp::Mod => return Err("unreachable".to_string()),
                    ArithOp::BitAnd => b.ins().band(a, c),
                    ArithOp::BitOr => b.ins().bor(a, c),
                    ArithOp::BitXor => b.ins().bxor(a, c),
                    ArithOp::Shl | ArithOp::Shr | ArithOp::Zshr => {
                        let mask = b.ins().iconst(types::I64, 63);
                        let s = b.ins().band(c, mask);
                        match op {
                            ArithOp::Shl => b.ins().ishl(a, s),
                            ArithOp::Shr => b.ins().sshr(a, s),
                            _ => b.ins().ushr(a, s),
                        }
                    }
                };
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_checked_div(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        span: diagnostics::Span,
        op: ArithOp,
        dst: Local,
        a: cranelift_codegen::ir::Value,
        c: cranelift_codegen::ir::Value,
    ) -> Result<(), String> {
        use cranelift_codegen::ir::condcodes::IntCC;
        let is_zero = b.ins().icmp_imm_s(IntCC::Equal, c, 0);
        let trap_bb = b.create_block();
        let ok_bb = b.create_block();
        b.ins().brif(is_zero, trap_bb, &[], ok_bb, &[]);
        b.switch_to_block(trap_bb);
        let callee = self.module.declare_func_in_func(self.fatal_span, &mut b.func);
        let msg = self.str_addr(b, "division by zero")?;
        let start = b.ins().iconst(types::I64, span.start as i64);
        let end = b.ins().iconst(types::I64, span.end as i64);
        b.ins().call(callee, &[msg, start, end]);
        let mut rs = Vec::with_capacity(self.ret_slots);
        for _ in 0..self.ret_slots {
            rs.push(b.ins().iconst(types::I64, 0));
        }
        b.ins().return_(&rs);
        b.switch_to_block(ok_bb);
        b.seal_block(trap_bb);
        b.seal_block(ok_bb);
        let neg_one = b.ins().icmp_imm_s(IntCC::Equal, c, -1);
        let is_min = b.ins().icmp_imm_s(IntCC::Equal, a, i64::MIN);
        let ov = b.ins().band(neg_one, is_min);
        let one = b.ins().iconst(types::I64, 1);
        let den = b.ins().select(ov, one, c);
        let v = match op {
            ArithOp::Div => {
                let q = b.ins().sdiv(a, den);
                b.ins().select(ov, a, q)
            }
            _ => {
                let r = b.ins().srem(a, den);
                let z = b.ins().iconst(types::I64, 0);
                b.ins().select(ov, z, r)
            }
        };
        self.set(b, dst, v);
        Ok(())
    }

    pub(super) fn lower_fma(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Fma { dst, a, b: bb, c, .. } = ins else {
            return Err("unreachable".to_string());
        };
                let x = self.f64_of(b, *a)?;
                let y = self.f64_of(b, *bb)?;
                let z = self.f64_of(b, *c)?;
                let v = b.ins().fma(x, y, z);
                let r = self.bits_of_f64(b, v);
                self.set(b, *dst, r);
        Ok(())
    }

    pub(super) fn lower_cmp(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Cmp { op, kind, dst, lhs, rhs , ..} = ins else {
            return Err("unreachable".to_string());
        };
                if matches!(kind, NumKind::Float(_)) {
                    use cranelift_codegen::ir::condcodes::FloatCC;
                    let a = self.f64_of(b, *lhs)?;
                    let c = self.f64_of(b, *rhs)?;
                    let cc = match op {
                        CmpOp::Eq => FloatCC::Equal,
                        CmpOp::NotEq => FloatCC::NotEqual,
                        CmpOp::Lt => FloatCC::LessThan,
                        CmpOp::LtEq => FloatCC::LessThanOrEqual,
                        CmpOp::Gt => FloatCC::GreaterThan,
                        CmpOp::GtEq => FloatCC::GreaterThanOrEqual,
                    };
                    let t = b.ins().fcmp(cc, a, c);
                    let v = b.ins().uextend(types::I64, t);
                    self.set(b, *dst, v);
                    return Ok(());
                }
                use cranelift_codegen::ir::condcodes::IntCC;
                let a = self.val(b, *lhs);
                let c = self.val(b, *rhs);
                let cc = match op {
                    CmpOp::Eq => IntCC::Equal,
                    CmpOp::NotEq => IntCC::NotEqual,
                    CmpOp::Lt => IntCC::SignedLessThan,
                    CmpOp::LtEq => IntCC::SignedLessThanOrEqual,
                    CmpOp::Gt => IntCC::SignedGreaterThan,
                    CmpOp::GtEq => IntCC::SignedGreaterThanOrEqual,
                };
                let t = b.ins().icmp(cc, a, c);
                let v = b.ins().uextend(types::I64, t);
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_not(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Not { dst, src , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let a = self.val(b, *src);
                let t = b.ins().icmp_imm_s(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    a,
                    0,
                );
                let v = b.ins().uextend(types::I64, t);
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_neg(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Neg { kind, dst, src , ..} = ins else {
            return Err("unreachable".to_string());
        };
                if matches!(kind, NumKind::Float(_)) {
                    let a = self.f64_of(b, *src)?;
                    let v = b.ins().fneg(a);
                    let r = self.bits_of_f64(b, v);
                    self.set(b, *dst, r);
                    return Ok(());
                }
                let a = self.val(b, *src);
                let v = b.ins().ineg(a);
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_convert(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Convert { dst, src, kind , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let v = self.val(b, *src);
                match kind {
                    ConvertKind::IntToFloat(_) => {
                        let f = match self.ftypes.get(*src as usize) {
                            Some(LirType::F64(_)) => b.ins().bitcast(types::F64, MemFlagsData::new(), v),
                            _ => b.ins().fcvt_from_sint(types::F64, v),
                        };
                        let r = self.bits_of_f64(b, f);
                        self.set(b, *dst, r);
                    }
                    ConvertKind::FloatToInt => {
                        let f = b.ins().bitcast(types::F64, MemFlagsData::new(), v);
                        let i = b.ins().fcvt_to_sint(types::I64, f);
                        self.set(b, *dst, i);
                    }
                }
        Ok(())
    }

    pub(super) fn lower_vec_new(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::VecNew { dst, kind, x, y, z, w , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let lanes = [*x, *y, *z, *w];
                let mut v = match kind {
                    lir::instr::VecKind::F => {
                        let f = self.f32_of_lane(b, lanes[0])?;
                        b.ins().splat(types::F32X4, f)
                    }
                    lir::instr::VecKind::I => {
                        let i = self.i32_of_lane(b, lanes[0]);
                        b.ins().splat(types::I32X4, i)
                    }
                };
                for (i, l) in lanes.iter().enumerate().skip(1) {
                    v = match kind {
                        lir::instr::VecKind::F => {
                            let f = self.f32_of_lane(b, *l)?;
                            b.ins().insertlane(v, f, i as u8)
                        }
                        lir::instr::VecKind::I => {
                            let n = self.i32_of_lane(b, *l);
                            b.ins().insertlane(v, n, i as u8)
                        }
                    };
                }
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_vec_splat(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::VecSplat { dst, kind, val , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let v = match kind {
                    lir::instr::VecKind::F => {
                        let f = self.f32_of_lane(b, *val)?;
                        b.ins().splat(types::F32X4, f)
                    }
                    lir::instr::VecKind::I => {
                        let n = self.i32_of_lane(b, *val);
                        b.ins().splat(types::I32X4, n)
                    }
                };
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_vec_extract(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::VecExtract { dst, vec, lane , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let v = self.val(b, *vec);
                let l = self.val(b, *lane);
                let three = b.ins().iconst(types::I64, 3);
                let oob = b.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::UnsignedGreaterThan,
                    l,
                    three,
                );
                let ok_bb = b.create_block();
                let trap_bb = b.create_block();
                b.ins().brif(oob, trap_bb, &[], ok_bb, &[]);
                b.switch_to_block(trap_bb);
                let callee = self.module.declare_func_in_func(self.panic_str, &mut b.func);
                let msg = self.str_addr(b, "vector lane out of range")?;
                b.ins().call(callee, &[msg]);
                b.ins().jump(ok_bb, &[]);
                b.switch_to_block(ok_bb);
                b.seal_block(trap_bb);
                b.seal_block(ok_bb);
                let is_float = matches!(self.ftypes.get(*vec as usize), Some(LirType::Vec4f));
                let e = self.vec_extract_dyn(b, v, l, is_float);
                if is_float {
                    let r = self.i64_of_f32(b, e);
                    self.set(b, *dst, r);
                } else {
                    let r = self.i64_of_i32(b, e);
                    self.set(b, *dst, r);
                }
        Ok(())
    }

    pub(super) fn lower_vec_insert(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::VecInsert { dst, vec, lane, val , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let v = self.val(b, *vec);
                let is_float = matches!(self.ftypes.get(*vec as usize), Some(LirType::Vec4f));
                let n = if is_float {
                    self.f32_of_lane(b, *val)?
                } else {
                    self.i32_of_lane(b, *val)
                };
                let r = b.ins().insertlane(v, n, *lane);
                self.set(b, *dst, r);
        Ok(())
    }

    pub(super) fn lower_vec_arith(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::VecArith { dst, op, kind, lhs, rhs , ..} = ins else {
            return Err("unreachable".to_string());
        };
                use lir::instr::VecOp;
                let a = self.val(b, *lhs);
                let c = self.val(b, *rhs);
                let v = match (kind, op) {
                    (lir::instr::VecKind::F, VecOp::Add) => b.ins().fadd(a, c),
                    (lir::instr::VecKind::F, VecOp::Sub) => b.ins().fsub(a, c),
                    (lir::instr::VecKind::F, VecOp::Mul) => b.ins().fmul(a, c),
                    (lir::instr::VecKind::F, VecOp::Div) => b.ins().fdiv(a, c),
                    (lir::instr::VecKind::F, VecOp::Min) | (lir::instr::VecKind::F, VecOp::Max) => {
                        let want_min = matches!(op, VecOp::Min);
                        let fz = b.ins().f32const(0.0);
                        let mut r = b.ins().splat(types::F32X4, fz);
                        for i in 0..4u8 {
                            let x = b.ins().extractlane(a, i);
                            let y = b.ins().extractlane(c, i);
                            let m = if want_min { b.ins().fmin(x, y) } else { b.ins().fmax(x, y) };
                            let xnan = b.ins().fcmp(
                                cranelift_codegen::ir::condcodes::FloatCC::Unordered,
                                x,
                                x,
                            );
                            let ynan = b.ins().fcmp(
                                cranelift_codegen::ir::condcodes::FloatCC::Unordered,
                                y,
                                y,
                            );
                            let t = b.ins().select(xnan, y, m);
                            let q = b.ins().select(ynan, x, t);
                            r = b.ins().insertlane(r, q, i);
                        }
                        r
                    }
                    (lir::instr::VecKind::I, VecOp::Add) => b.ins().iadd(a, c),
                    (lir::instr::VecKind::I, VecOp::Sub) => b.ins().isub(a, c),
                    (lir::instr::VecKind::I, VecOp::Mul) => b.ins().imul(a, c),
                    (lir::instr::VecKind::I, VecOp::Div)
                    | (lir::instr::VecKind::I, VecOp::Min)
                    | (lir::instr::VecKind::I, VecOp::Max) => {
                        let zero = b.ins().iconst(types::I32, 0);
                        let mut r = b.ins().splat(types::I32X4, zero);
                        for i in 0..4u8 {
                            let x = b.ins().extractlane(a, i);
                            let y = b.ins().extractlane(c, i);
                            let q = match op {
                                VecOp::Div => b.ins().sdiv(x, y),
                                VecOp::Min => b.ins().smin(x, y),
                                _ => b.ins().smax(x, y),
                            };
                            r = b.ins().insertlane(r, q, i);
                        }
                        r
                    }
                };
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_vec_unary(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::VecUnary { dst, op, src , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let a = self.val(b, *src);
                let v = match op {
                    lir::instr::VecUnaryOp::Sqrt => b.ins().sqrt(a),
                };
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_vec_dot(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::VecDot { dst, lhs, rhs , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let a = self.val(b, *lhs);
                let c = self.val(b, *rhs);
                let a0 = b.ins().extractlane(a, 0);
                let c0 = b.ins().extractlane(c, 0);
                let a1 = b.ins().extractlane(a, 1);
                let c1 = b.ins().extractlane(c, 1);
                let a2 = b.ins().extractlane(a, 2);
                let c2 = b.ins().extractlane(c, 2);
                let a3 = b.ins().extractlane(a, 3);
                let c3 = b.ins().extractlane(c, 3);
                let p0 = b.ins().fmul(a0, c0);
                let p1 = b.ins().fmul(a1, c1);
                let p2 = b.ins().fmul(a2, c2);
                let p3 = b.ins().fmul(a3, c3);
                let s01 = b.ins().fadd(p0, p1);
                let s23 = b.ins().fadd(p2, p3);
                let s = b.ins().fadd(s01, s23);
                let r = self.i64_of_f32(b, s);
                self.set(b, *dst, r);
        Ok(())
    }
}
