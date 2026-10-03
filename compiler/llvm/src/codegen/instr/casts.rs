use super::*;

pub(super) fn lower_casts(cx: &mut FnCx, lir: &Module, ins: &Instr, fname: &str) -> Result<(), Diagnostic> {
    let _ = fname;
    let _ = lir;
    match ins {
        Instr::Const { dst, lit , ..} => {
            match lit {
                Lit::Str(text) => {
                    let v = str_addr(cx, text)?;
                    store(cx, *dst, v.into())?;
                }
                _ => {
                    let v = match lit {
                        Lit::Int(i) => cx.context.i64_type().const_int(*i as u64, true),
                        Lit::Bool(b) => cx.context.i64_type().const_int(*b as u64, false),
                        Lit::Null => cx.context.i64_type().const_zero(),
                        Lit::Float(f, _) => cx.context.i64_type().const_int(f.to_bits(), false),
                        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: non-scalar const")),
                    };
                    store(cx, *dst, v.into())?;
                }
            }
        }
        Instr::Cast { dst, src , ..} => {
            let dst_hold = matches!(cx.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Error));
            if dst_hold && *dst != *src {
                let keep = Some(load(cx, *src)?);
                if cx.owned.contains(dst) && (cx.stack.contains(dst) || write_dominates(cx, *dst)) {
                    release_any(cx, lir, *dst, keep)?;
                }
                let v = load(cx, *src)?;
                store(cx, *dst, v.into())?;
                if !cx.stack.contains(src) {
                    cx.owned.remove(src);
                    cx.ever_owned.remove(src);
                }
                if !cx.stack.contains(dst) {
                    own(cx, *dst);
                }
            } else {
                let v = load(cx, *src)?;
                store(cx, *dst, v.into())?;
            }
        }
        Instr::Copy { dst, src , ..} => {
            let src_hold = matches!(cx.ftypes.get(*src as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Error));
            let dst_hold = matches!(cx.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Error));
            let dst_any = matches!(cx.ftypes.get(*dst as usize), Some(LirType::Any));
            if dst_any && *dst != *src {
                let v = load(cx, *src)?;
                any_retain_val(cx, v)?;
                if cx.owned.contains(dst) && (cx.stack.contains(dst) || write_dominates(cx, *dst)) {
                    release_any(cx, lir, *dst, None)?;
                }
                store(cx, *dst, v.into())?;
                if !cx.stack.contains(dst) {
                    own(cx, *dst);
                }
            } else if dst_hold && *dst != *src {
                let keep = Some(load(cx, *src)?);
                if cx.owned.contains(dst) && (cx.stack.contains(dst) || write_dominates(cx, *dst)) {
                    release_any(cx, lir, *dst, keep)?;
                }
                let v = load(cx, *src)?;
                store(cx, *dst, v.into())?;
                if src_hold && !cx.stack.contains(src) {
                    let borrowed = !cx.owned.contains(src);
                    cx.owned.remove(src);
                    cx.ever_owned.remove(src);
                    if borrowed && !cx.stack.contains(dst) {
                        // Borrow: src is a borrowed parameter, literal, or
                        // another unowned alias, so dst cannot inherit a
                        // counted reference. Take our own: without this
                        // retain, a later release-old (or scope drain) on
                        // dst would free memory dst does not own (a callee
                        // releases nothing it did not retain).
                        retain_val(cx, lir, *src, v)?;
                    }
                    if !cx.stack.contains(dst) {
                        own(cx, *dst);
                    }
                } else if !cx.stack.contains(dst) {
                    if matches!(cx.ftypes.get(*src as usize), Some(LirType::Any)) {
                        retain_val(cx, lir, *src, v)?;
                    }
                    own(cx, *dst);
                }
            } else {
                match cx.ftypes.get(*src as usize) {
                    Some(LirType::Vec4f) | Some(LirType::Vec4i) => {
                        let v = load_vec(cx, *src)?;
                        store(cx, *dst, v.into())?;
                    }
                    _ => {
                        let v = load(cx, *src)?;
                        store(cx, *dst, v.into())?;
                    }
                }
            }
        }
        Instr::Convert { dst, src, kind , ..} => {
            let f64t = cx.context.f64_type();
            match kind {
                ConvertKind::IntToFloat(_) => {
                    let bits = load(cx, *src)?;
                    let f = match cx.ftypes.get(*src as usize) {
                        Some(LirType::F64(_)) => cx
                            .builder
                            .build_bit_cast(bits, f64t, "f")
                            .map_err(err)?
                            .into_float_value(),
                        _ => cx.builder.build_signed_int_to_float(bits, f64t, "f").map_err(err)?,
                    };
                    store_double(cx, *dst, f)?;
                }
                ConvertKind::FloatToInt => {
                    let f = double_of(cx, *src)?;
                    let i = cx.builder.build_float_to_signed_int(f, cx.context.i64_type(), "i").map_err(err)?;
                    store(cx, *dst, i.into())?;
                }
            }
        }
        Instr::Concat { dst, lhs, rhs , ..} => {
            let a = load(cx, *lhs)?;
            let b = load(cx, *rhs)?;
            let pa = as_ptr(cx, a)?;
            let pb = as_ptr(cx, b)?;
            let site = cx.builder.build_call(cx.concat, &[pa.into(), pb.into()], "").map_err(err)?;
            match site.try_as_basic_value() {
                ValueKind::Basic(v) => {
                    let r = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                    store(cx, *dst, r.into())?;
                    own(cx, *dst);
                }
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: concat returned void"));
                }
            }
        }
        Instr::ToStr { dst, src , ..} => {
            let site = match cx.ftypes.get(*src as usize) {
                Some(LirType::Any) => {
                    let v = load(cx, *src)?;
                    cx.builder.build_call(cx.any_to_str, &[v.into()], "").map_err(err)?
                }
                Some(LirType::I64) | Some(LirType::I8) => {
                    let v = load(cx, *src)?;
                    cx.builder.build_call(cx.int_to_str, &[v.into()], "").map_err(err)?
                }
                Some(LirType::F64(_)) => {
                    let v = load(cx, *src)?;
                    cx.builder.build_call(cx.float_to_str, &[v.into()], "").map_err(err)?
                }
                _ => return Err(Diagnostic::new(Code::E108, "llvm subset: tostr of non-scalar")),
            };
            match site.try_as_basic_value() {
                ValueKind::Basic(rv) => {
                    let r = cx.builder.build_ptr_to_int(rv.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                    store(cx, *dst, r.into())?;
                    own(cx, *dst);
                }
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: to_str returned void"));
                }
            }
        }
        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unsupported instr")),
    }
    Ok(())
}
