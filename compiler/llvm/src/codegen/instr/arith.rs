use super::*;

pub(super) fn lower_arith(cx: &mut FnCx, lir: &Module, ins: &Instr, fname: &str) -> Result<(), Diagnostic> {
    let _ = fname;
    let _ = lir;
    match ins {
        Instr::VecNew { dst, kind, x, y, z, w , ..} => {
            let lanes = [*x, *y, *z, *w];
            let i32t = cx.context.i32_type();
            let (ty, is_f) = match kind {
                VecKind::F => (cx.context.f32_type().vec_type(4), true),
                VecKind::I => (cx.context.i32_type().vec_type(4), false),
            };
            let mut v = ty.get_undef();
            for (i, l) in lanes.iter().enumerate() {
                let idx = i32t.const_int(i as u64, false);
                let e: BasicValueEnum = if is_f {
                    f32_of_lane(cx, *l)?.into()
                } else {
                    i32_of_lane(cx, *l)?.into()
                };
                v = cx.builder.build_insert_element(v, e, idx, "ins").map_err(err)?;
            }
            store(cx, *dst, v.into())?;
        }
        Instr::VecSplat { dst, kind, val , ..} => {
            let i32t = cx.context.i32_type();
            let (ty, e): (_, BasicValueEnum) = match kind {
                VecKind::F => (cx.context.f32_type().vec_type(4), f32_of_lane(cx, *val)?.into()),
                VecKind::I => (cx.context.i32_type().vec_type(4), i32_of_lane(cx, *val)?.into()),
            };
            let mut v = ty.get_undef();
            for i in 0..4u64 {
                let idx = i32t.const_int(i, false);
                v = cx.builder.build_insert_element(v, e, idx, "ins").map_err(err)?;
            }
            store(cx, *dst, v.into())?;
        }
        Instr::VecExtract { dst, vec, lane , ..} => {
            let v = load_vec(cx, *vec)?;
            let l = load(cx, *lane)?;
            let i64t = cx.context.i64_type();
            let ok = cx.builder.build_int_compare(IntPredicate::ULT, l, i64t.const_int(4, false), "").map_err(err)?;
            let fv = cx.builder.get_insert_block().and_then(|b| b.get_parent()).ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: no insert function"))?;
            let trap_bb = cx.context.append_basic_block(fv, "vec_oob");
            let cont_bb = cx.context.append_basic_block(fv, "vec_ok");
            cx.builder.build_conditional_branch(ok, cont_bb, trap_bb).map_err(err)?;
            cx.builder.position_at_end(trap_bb);
            let msg = str_addr(cx, "vector lane out of range")?;
            let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
            let p = cx.builder.build_int_to_ptr(msg, ptr_t, "").map_err(err)?;
            cx.builder.build_call(cx.panic_str, &[p.into()], "").map_err(err)?;
            cx.builder.build_unreachable().map_err(err)?;
            cx.builder.position_at_end(cont_bb);
            let i32t = cx.context.i32_type();
            let idx = cx.builder.build_int_truncate(l, i32t, "t").map_err(err)?;
            let e = cx.builder.build_extract_element(v, idx, "ext").map_err(err)?;
            match cx.ftypes.get(*vec as usize) {
                Some(LirType::Vec4f) => store_f32(cx, *dst, e.into_float_value())?,
                _ => store_i32(cx, *dst, e.into_int_value())?,
            }
        }
        Instr::VecInsert { dst, vec, lane, val , ..} => {
            let mut v = load_vec(cx, *vec)?;
            let i32t = cx.context.i32_type();
            let idx = i32t.const_int(*lane as u64, false);
            let e: BasicValueEnum = match cx.ftypes.get(*vec as usize) {
                Some(LirType::Vec4f) => f32_of_lane(cx, *val)?.into(),
                _ => i32_of_lane(cx, *val)?.into(),
            };
            v = cx.builder.build_insert_element(v, e, idx, "ins").map_err(err)?;
            store(cx, *dst, v.into())?;
        }
        Instr::VecArith { dst, op, kind, lhs, rhs , ..} => {
            let a = load_vec(cx, *lhs)?;
            let b = load_vec(cx, *rhs)?;
            let v: BasicValueEnum = match (kind, op) {
                (VecKind::F, VecOp::Add) => cx.builder.build_float_add(a, b, "add").map_err(err)?.into(),
                (VecKind::F, VecOp::Sub) => cx.builder.build_float_sub(a, b, "sub").map_err(err)?.into(),
                (VecKind::F, VecOp::Mul) => cx.builder.build_float_mul(a, b, "mul").map_err(err)?.into(),
                (VecKind::F, VecOp::Div) => cx.builder.build_float_div(a, b, "div").map_err(err)?.into(),
                (VecKind::F, VecOp::Min) => {
                    let site =
                        cx.builder.build_call(cx.vec_minnum, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => v,
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: minnum returned void"));
                        }
                    }
                }
                (VecKind::F, VecOp::Max) => {
                    let site =
                        cx.builder.build_call(cx.vec_maxnum, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => v,
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: maxnum returned void"));
                        }
                    }
                }
                (VecKind::I, VecOp::Add) => cx.builder.build_int_add(a, b, "add").map_err(err)?.into(),
                (VecKind::I, VecOp::Sub) => cx.builder.build_int_sub(a, b, "sub").map_err(err)?.into(),
                (VecKind::I, VecOp::Mul) => cx.builder.build_int_mul(a, b, "mul").map_err(err)?.into(),
                (VecKind::I, VecOp::Div) => cx.builder.build_int_signed_div(a, b, "div").map_err(err)?.into(),
                (VecKind::I, VecOp::Min) => {
                    let c = cx.builder.build_int_compare(IntPredicate::SLT, a, b, "c").map_err(err)?;
                    cx.builder.build_select(c, a, b, "s").map_err(err)?.into()
                }
                (VecKind::I, VecOp::Max) => {
                    let c = cx.builder.build_int_compare(IntPredicate::SGT, a, b, "c").map_err(err)?;
                    cx.builder.build_select(c, a, b, "s").map_err(err)?.into()
                }
            };
            store(cx, *dst, v.into())?;
        }
        Instr::VecUnary { dst, op, src , ..} => {
            let a = load_vec(cx, *src)?;
            let v = match op {
                VecUnaryOp::Sqrt => {
                    let site = cx.builder.build_call(cx.vec_sqrt, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => v,
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: sqrt returned void"));
                        }
                    }
                }
            };
            store(cx, *dst, v.into())?;
        }
        Instr::VecDot { dst, lhs, rhs , ..} => {
            let a = load_vec(cx, *lhs)?;
            let b = load_vec(cx, *rhs)?;
            let i32t = cx.context.i32_type();
            let mut acc = cx.context.f32_type().const_zero();
            for i in 0..4u64 {
                let idx = i32t.const_int(i, false);
                let x: FloatValue =
                    cx.builder.build_extract_element(a, idx, "ext").map_err(err)?.into_float_value();
                let y: FloatValue =
                    cx.builder.build_extract_element(b, idx, "ext").map_err(err)?.into_float_value();
                let p = cx.builder.build_float_mul(x, y, "mul").map_err(err)?;
                acc = cx.builder.build_float_add(acc, p, "add").map_err(err)?;
            }
            store_f32(cx, *dst, acc)?;
        }
        Instr::Arith { op, kind, dst, lhs, rhs, span, ..} => {
            if matches!(op, ArithOp::Div | ArithOp::Mod) && !matches!(kind, NumKind::Float(_)) {
                let a = load(cx, *lhs)?;
                let b = load(cx, *rhs)?;
                let i64t = cx.context.i64_type();
                let zero = i64t.const_zero();
                let is_zero = cx.builder.build_int_compare(IntPredicate::EQ, b, zero, "divz").map_err(err)?;
                let fv = cx.builder.get_insert_block().and_then(|b| b.get_parent()).ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: no insert function"))?;
                let trap_bb = cx.context.append_basic_block(fv, "div_zero");
                let cont_bb = cx.context.append_basic_block(fv, "div_ok");
                cx.builder.build_conditional_branch(is_zero, trap_bb, cont_bb).map_err(err)?;
                cx.builder.position_at_end(trap_bb);
                let msg = str_addr(cx, "division by zero")?;
                let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
                let p = cx.builder.build_int_to_ptr(msg, ptr_t, "").map_err(err)?;
                let s = i64t.const_int(span.start as u64, false);
                let e = i64t.const_int(span.end as u64, false);
                cx.builder.build_call(cx.fatal_span, &[p.into(), s.into(), e.into()], "").map_err(err)?;
                if cx.ret_slots <= 1 {
                    cx.builder.build_return(Some(&zero)).map_err(err)?;
                } else {
                    let fields: Vec<inkwell::types::BasicTypeEnum> =
                        (0..cx.ret_slots).map(|_| cx.context.i64_type().into()).collect();
                    let st = cx.context.struct_type(&fields, false);
                    let mut agg = st.get_undef();
                    for i in 0..cx.ret_slots {
                        agg = cx.builder.build_insert_value(agg, zero, i as u32, "errz").map_err(err)?.into_struct_value();
                    }
                    cx.builder.build_return(Some(&agg)).map_err(err)?;
                }
                cx.builder.position_at_end(cont_bb);
                let neg1 = i64t.const_int(-1i64 as u64, false);
                let is_neg1 = cx.builder.build_int_compare(IntPredicate::EQ, b, neg1, "neg1").map_err(err)?;
                let is_min = cx.builder.build_int_compare(IntPredicate::EQ, a, i64t.const_int(i64::MIN as u64, false), "ismin").map_err(err)?;
                let ov = cx.builder.build_and(is_neg1, is_min, "ov").map_err(err)?;
                let one = i64t.const_int(1, false);
                let den = cx.builder.build_select(ov, one, b, "den").map_err(err)?.into_int_value();
                let sel = match op {
                    ArithOp::Div => {
                        let q = cx.builder.build_int_signed_div(a, den, "div").map_err(err)?;
                        cx.builder.build_select(ov, a, q, "s").map_err(err)?
                    }
                    _ => {
                        let r = cx.builder.build_int_signed_rem(a, den, "rem").map_err(err)?;
                        cx.builder.build_select(ov, zero, r, "s").map_err(err)?
                    }
                };
                store(cx, *dst, sel)?;
                return Ok(());
            }
            if let NumKind::Float(k) = kind {
                let fast = *k == FloatKind::Fast;
                let a = double_of(cx, *lhs)?;
                let b = double_of(cx, *rhs)?;
                let v = match op {
                    ArithOp::Add => cx.builder.build_float_add(a, b, "add"),
                    ArithOp::Sub => cx.builder.build_float_sub(a, b, "sub"),
                    ArithOp::Mul => cx.builder.build_float_mul(a, b, "mul"),
                    ArithOp::Div => cx.builder.build_float_div(a, b, "div"),
                    ArithOp::Mod => {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: float mod unsupported"));
                    }
                    ArithOp::BitAnd | ArithOp::BitOr | ArithOp::BitXor | ArithOp::Shl | ArithOp::Shr | ArithOp::Zshr => {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: bitwise needs `Int`"));
                    }
                }
                .map_err(err)?;
                fast_flags(cx, v, fast)?;
                store_double(cx, *dst, v)?;
                return Ok(());
            }
            let a = load(cx, *lhs)?;
            let b = load(cx, *rhs)?;
            let v = match op {
                ArithOp::Add => cx.builder.build_int_add(a, b, "add"),
                ArithOp::Sub => cx.builder.build_int_sub(a, b, "sub"),
                ArithOp::Mul => cx.builder.build_int_mul(a, b, "mul"),
                ArithOp::Div => cx.builder.build_int_signed_div(a, b, "div"),
                ArithOp::Mod => cx.builder.build_int_signed_rem(a, b, "rem"),
                ArithOp::BitAnd => cx.builder.build_and(a, b, "and"),
                ArithOp::BitOr => cx.builder.build_or(a, b, "or"),
                ArithOp::BitXor => cx.builder.build_xor(a, b, "xor"),
                ArithOp::Shl | ArithOp::Shr | ArithOp::Zshr => {
                    let mask = cx.context.i64_type().const_int(63, false);
                    let s = cx.builder.build_and(b, mask, "shmask").map_err(err)?;
                    match op {
                        ArithOp::Shl => cx.builder.build_left_shift(a, s, "shl"),
                        ArithOp::Shr => cx.builder.build_right_shift(a, s, true, "shr"),
                        _ => cx.builder.build_right_shift(a, s, false, "zshr"),
                    }
                }
            }
            .map_err(err)?;
            store(cx, *dst, v.into())?;
        }
        Instr::Fma { dst, a, b, c, .. } => {
            let x = double_of(cx, *a)?;
            let y = double_of(cx, *b)?;
            let z = double_of(cx, *c)?;
            let decl = cx.fma_fn.ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: missing llvm.fma"))?;
            let site = cx.builder.build_call(decl, &[x.into(), y.into(), z.into()], "fma").map_err(err)?;
            let v = match site.try_as_basic_value() {
                ValueKind::Basic(v) => v.into_float_value(),
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: fma returned void"));
                }
            };
            store_double(cx, *dst, v)?;
            return Ok(());
        }
        Instr::Cmp { op, kind, dst, lhs, rhs , ..} => {
            if matches!(kind, NumKind::Float(_)) {
                let a = double_of(cx, *lhs)?;
                let b = double_of(cx, *rhs)?;
                let pred = match op {
                    CmpOp::Eq => FloatPredicate::OEQ,
                    CmpOp::NotEq => FloatPredicate::UNE,
                    CmpOp::Lt => FloatPredicate::OLT,
                    CmpOp::LtEq => FloatPredicate::OLE,
                    CmpOp::Gt => FloatPredicate::OGT,
                    CmpOp::GtEq => FloatPredicate::OGE,
                };
                let c = cx.builder.build_float_compare(pred, a, b, "cmp").map_err(err)?;
                let v = cx
                    .builder
                    .build_int_z_extend(c, cx.context.i64_type(), "b")
                    .map_err(err)?;
                store(cx, *dst, v.into())?;
                return Ok(());
            }
            use inkwell::IntPredicate::*;
            let a = load(cx, *lhs)?;
            let b = load(cx, *rhs)?;
            let pred = match op {
                CmpOp::Eq => EQ,
                CmpOp::NotEq => NE,
                CmpOp::Lt => SLT,
                CmpOp::LtEq => SLE,
                CmpOp::Gt => SGT,
                CmpOp::GtEq => SGE,
            };
            let c = cx.builder.build_int_compare(pred, a, b, "cmp").map_err(err)?;
            let v = cx
                .builder
                .build_int_z_extend(c, cx.context.i64_type(), "b")
                .map_err(err)?;
            store(cx, *dst, v.into())?;
        }
        Instr::Not { dst, src , ..} => {
            let a = load(cx, *src)?;
            let zero = cx.context.i64_type().const_zero();
            let c = cx
                .builder
                .build_int_compare(inkwell::IntPredicate::EQ, a, zero, "not")
                .map_err(err)?;
            let v = cx
                .builder
                .build_int_z_extend(c, cx.context.i64_type(), "b")
                .map_err(err)?;
            store(cx, *dst, v.into())?;
        }
        Instr::Neg { kind, dst, src , ..} => {
            if let NumKind::Float(k) = kind {
                let fast = *k == FloatKind::Fast;
                let a = double_of(cx, *src)?;
                let v = cx.builder.build_float_neg(a, "neg").map_err(err)?;
                fast_flags(cx, v, fast)?;
                store_double(cx, *dst, v)?;
                return Ok(());
            }
            let a = load(cx, *src)?;
            let v = cx.builder.build_int_neg(a, "neg").map_err(err)?;
            store(cx, *dst, v.into())?;
        }
        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unsupported instr")),
    }
    Ok(())
}
