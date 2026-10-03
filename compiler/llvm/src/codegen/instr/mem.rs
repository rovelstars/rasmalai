use super::*;

pub(super) fn lower_mem(cx: &mut FnCx, lir: &Module, ins: &Instr, fname: &str) -> Result<(), Diagnostic> {
    let _ = fname;
    let _ = lir;
    match ins {
        Instr::ObjNew { dst, class, instance_size: size , ..} => {
            let n = cx.context.i64_type().const_int(*size as u64, false);
            let eight = cx.context.i64_type().const_int(8, false);
            let site = cx.builder.build_call(cx.alloc, &[n.into(), eight.into()], "").map_err(err)?;
            let raw_ptr = match site.try_as_basic_value() {
                ValueKind::Basic(v) => v.into_pointer_value(),
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: alloc returned void"));
                }
            };
            let ptr = cx.builder.build_ptr_to_int(raw_ptr, cx.context.i64_type(), "").map_err(err)?;
            store(cx, *dst, ptr.into())?;
            own(cx, *dst);
            let i32t = cx.context.i32_type();
            let one = i32t.const_int(1, false);
            let zero32 = i32t.const_zero();
            let zero64 = cx.context.i64_type().const_zero();
            let raw = load(cx, *dst)?;
            let type_idx = cx.context.i64_type().const_int(*class as u64 + 1, false);
            for (off, word) in [(0usize, BasicValueEnum::from(one)), (4, BasicValueEnum::from(zero32)), (8, BasicValueEnum::from(type_idx))] {
                let at = byte_ptr(cx, raw, off)?;
                cx.builder.build_store(at, word).map_err(err)?;
            }
            let fields = (*size as usize).saturating_sub(HEADER_SIZE) / SLOT_SIZE;
            for i in 0..fields {
                let at = byte_ptr(cx, raw, HEADER_SIZE + i * SLOT_SIZE)?;
                cx.builder.build_store(at, zero64).map_err(err)?;
            }
            let cname = lir.classes.get(*class).map(|c| c.name.rsplit('.').next().unwrap_or(&c.name).to_string()).unwrap_or_default();
            let name_ptr = str_addr(cx, &cname)?;
            cx.builder.build_call(cx.note_type, &[type_idx.into(), name_ptr.into()], "").map_err(err)?;
            if let Some(desc) = lir.classes.get(*class) {
                let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
                let dtor = match desc.dtor {
                    Some(fid) => {
                        let fname = &lir.functions[fid].name;
                        let fv = cx.funcs.get(fname).copied().ok_or_else(|| {
                            Diagnostic::new(Code::E108, format!("llvm subset: unknown dtor `{fname}`"))
                        })?;
                        fv.as_global_value().as_pointer_value()
                    }
                    None => ptr_t.const_null(),
                };
                let dd = cx.builder.build_ptr_to_int(dtor, cx.context.i64_type(), "").map_err(err)?;
                let sz = cx.context.i64_type().const_int(*size as u64, false);
                track_heap(cx, raw, runtime::native::HEAP_OBJ, dd, sz)?;
                let fields_desc = lir::instr::pretty_field_desc(desc);
                let desc_ptr = str_addr(cx, &fields_desc)?;
                cx.builder.build_call(cx.note_fields, &[type_idx.into(), desc_ptr.into()], "").map_err(err)?;
                if let Some(ns) = lir.namespaces.get(&desc.name) {
                    let ns_desc = lir::instr::pretty_namespace_desc(ns);
                    let ns_ptr = str_addr(cx, &ns_desc)?;
                    cx.builder.build_call(cx.note_namespace, &[type_idx.into(), ns_ptr.into()], "").map_err(err)?;
                }
            }
        }
        Instr::StackAlloc { dst, class, instance_size: size , ..} => {
            let slot = *cx.stack_slots.get(cx.stack_next).ok_or_else(|| {
                Diagnostic::new(Code::E108, "llvm subset: missing stack slot")
            })?;
            cx.stack_next += 1;
            let ptr = cx.builder.build_ptr_to_int(slot, cx.context.i64_type(), "").map_err(err)?;
            store(cx, *dst, ptr.into())?;
            let i32t = cx.context.i32_type();
            let one = i32t.const_int(1, false);
            let zero32 = i32t.const_zero();
            let zero64 = cx.context.i64_type().const_zero();
            let raw = load(cx, *dst)?;
            let type_idx = cx.context.i64_type().const_int(*class as u64 + 1, false);
            for (off, word) in [(0usize, BasicValueEnum::from(one)), (4, BasicValueEnum::from(zero32)), (8, BasicValueEnum::from(type_idx))] {
                let at = byte_ptr(cx, raw, off)?;
                cx.builder.build_store(at, word).map_err(err)?;
            }
            let fields = (*size as usize).saturating_sub(HEADER_SIZE) / SLOT_SIZE;
            for i in 0..fields {
                let at = byte_ptr(cx, raw, HEADER_SIZE + i * SLOT_SIZE)?;
                cx.builder.build_store(at, zero64).map_err(err)?;
            }
            let cname = lir.classes.get(*class).map(|c| c.name.rsplit('.').next().unwrap_or(&c.name).to_string()).unwrap_or_default();
            let name_ptr = str_addr(cx, &cname)?;
            cx.builder.build_call(cx.note_type, &[type_idx.into(), name_ptr.into()], "").map_err(err)?;
            if let Some(desc) = lir.classes.get(*class) {
                let fields_desc = lir::instr::pretty_field_desc(desc);
                let desc_ptr = str_addr(cx, &fields_desc)?;
                cx.builder.build_call(cx.note_fields, &[type_idx.into(), desc_ptr.into()], "").map_err(err)?;
                if let Some(ns) = lir.namespaces.get(&desc.name) {
                    let ns_desc = lir::instr::pretty_namespace_desc(ns);
                    let ns_ptr = str_addr(cx, &ns_desc)?;
                    cx.builder.build_call(cx.note_namespace, &[type_idx.into(), ns_ptr.into()], "").map_err(err)?;
                }
            }
        }
        Instr::EnumNew { dst, enu, variant, payload , ..} => {
            let (size, max, ptys) = match lir.enums.get(*enu) {
                Some(d) => match d.variants.get(*variant) {
                    Some(v) => (enum_instance_size(d) as u64, enum_max_payloads(d), v.payload.clone()),
                    None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown variant")),
                },
                None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown enum")),
            };
            if ptys.len() != payload.len() {
                return Err(Diagnostic::new(Code::E108, "llvm subset: enum payload arity"));
            }
            let n = cx.context.i64_type().const_int(size, false);
            let eight = cx.context.i64_type().const_int(8, false);
            let site = cx.builder.build_call(cx.alloc, &[n.into(), eight.into()], "").map_err(err)?;
            let raw_ptr = match site.try_as_basic_value() {
                ValueKind::Basic(v) => v.into_pointer_value(),
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: alloc returned void"));
                }
            };
            let ptr = cx.builder.build_ptr_to_int(raw_ptr, cx.context.i64_type(), "").map_err(err)?;
            store(cx, *dst, ptr.into())?;
            own(cx, *dst);
            let i32t = cx.context.i32_type();
            let one = i32t.const_int(1, false);
            let zero32 = i32t.const_zero();
            let zero64 = cx.context.i64_type().const_zero();
            let raw = load(cx, *dst)?;
            for (off, word) in [(0usize, BasicValueEnum::from(one)), (4, BasicValueEnum::from(zero32)), (8, BasicValueEnum::from(zero64))] {
                let at = byte_ptr(cx, raw, off)?;
                cx.builder.build_store(at, word).map_err(err)?;
            }
            let tag = cx.context.i64_type().const_int(*variant as u64, false);
            let at = byte_ptr(cx, raw, 16)?;
            cx.builder.build_store(at, tag).map_err(err)?;
            for (i, (l, ty)) in payload.iter().zip(ptys.iter()).enumerate() {
                let v = load(cx, *l)?;
                let declared_heap = match ty {
                    LirType::Obj(name) => lir.class_index.contains_key(name),
                    LirType::Str | LirType::Array(_) | LirType::Enum(_) => true,
                    _ => false,
                };
                if declared_heap {
                    let p = as_ptr(cx, v)?;
                    cx.builder.build_call(cx.retain, &[p.into()], "").map_err(err)?;
                }
                if matches!(ty, LirType::Any)
                    || matches!(ty, LirType::Obj(_)) && !declared_heap
                {
                    if matches!(cx.ftypes.get(*l as usize), Some(LirType::Obj(_)) | Some(LirType::Enum(_)) | Some(LirType::Array(_))) {
                        retain_val(cx, lir, *l, v)?;
                    } else {
                        any_retain_val(cx, v)?;
                    }
                }
                let at = byte_ptr(cx, raw, 24 + i * SLOT_SIZE)?;
                cx.builder.build_store(at, v).map_err(err)?;
            }
            for i in payload.len()..max {
                let at = byte_ptr(cx, raw, 24 + i * SLOT_SIZE)?;
                cx.builder.build_store(at, zero64).map_err(err)?;
            }
            if let Some(d) = lir.enums.get(*enu) {
                let edtor = enum_dtor_ptr(cx, lir, *enu)?;
                let ed = cx.builder.build_ptr_to_int(edtor, cx.context.i64_type(), "").map_err(err)?;
                let esz = cx.context.i64_type().const_int(enum_instance_size(d) as u64, false);
                track_heap(cx, raw, runtime::native::HEAP_ENUM, ed, esz)?;
                if let Some(v) = d.variants.get(*variant) {
                    let desc = lir::instr::pretty_variant_desc(v);
                    let desc_ptr = str_addr(cx, &desc)?;
                    let e = cx.context.i64_type().const_int(*enu as u64, false);
                    let vv = cx.context.i64_type().const_int(*variant as u64, false);
                    cx.builder.build_call(cx.note_enum, &[raw.into(), e.into(), vv.into(), desc_ptr.into()], "").map_err(err)?;
                }
            }
        }
        Instr::EnumPayload { dst, scrut, index , ..} => {
            let max = match cx.ftypes.get(*scrut as usize) {
                Some(LirType::Enum(ei)) => match lir.enums.get(*ei) {
                    Some(d) => enum_max_payloads(d),
                    None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown enum")),
                },
                _ => return Err(Diagnostic::new(Code::E108, "llvm subset: payload of non-enum")),
            };
            if *index >= max {
                return Err(Diagnostic::new(Code::E108, "llvm subset: enum payload out of bounds"));
            }
            let base = load(cx, *scrut)?;
            let at = byte_ptr(cx, base, 24 + index * SLOT_SIZE)?;
            let v = cx.builder.build_load(cx.context.i64_type(), at, "").map_err(err)?.into_int_value();
            store(cx, *dst, v.into())?;
            if matches!(cx.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                retain_val(cx, lir, *dst, v)?;
                own(cx, *dst);
            }
            if matches!(cx.ftypes.get(*dst as usize), Some(LirType::Any)) {
                if !cx.in_enum_dtor {
                    any_retain_val(cx, v)?;
                }
                own(cx, *dst);
            }
        }
        Instr::EnumTag { dst, scrut , ..} => {
            if !matches!(cx.ftypes.get(*scrut as usize), Some(LirType::Enum(_))) {
                return Err(Diagnostic::new(Code::E108, "llvm subset: tag of non-enum"));
            }
            let base = load(cx, *scrut)?;
            let at = byte_ptr(cx, base, 16)?;
            let v = cx.builder.build_load(cx.context.i64_type(), at, "").map_err(err)?.into_int_value();
            store(cx, *dst, v.into())?;
        }
        Instr::GetField { dst, obj, field , ..} => {
            let base = load(cx, *obj)?;
            let at = byte_ptr(cx, base, field_offset(*field))?;
            let v = cx.builder.build_load(cx.context.i64_type(), at, "").map_err(err)?.into_int_value();
            store(cx, *dst, v.into())?;
            if matches!(cx.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                retain_val(cx, lir, *dst, v)?;
                own(cx, *dst);
            }
            if matches!(cx.ftypes.get(*dst as usize), Some(LirType::Any)) {
                any_retain_val(cx, v)?;
                own(cx, *dst);
            }
        }
        Instr::SetField { obj, field, value , ..} => {
            let (ci, fty) = match cx.ftypes.get(*obj as usize) {
                Some(LirType::Obj(name)) => match lir.class_index.get(name) {
                    Some(ci) => match lir.classes[*ci].fields.get(*field) {
                        Some(f) => (*ci, f.ty.clone()),
                        None => return Err(Diagnostic::new(Code::E108, "llvm subset: bad field store")),
                    },
                    None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown class")),
                },
                _ => return Err(Diagnostic::new(Code::E108, "llvm subset: field of non-object")),
            };
            let _ = ci;
            let base = load(cx, *obj)?;
            let at = byte_ptr(cx, base, field_offset(*field))?;
            let v = load(cx, *value)?;
            if matches!(cx.ftypes.get(*value as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                retain_val(cx, lir, *value, v)?;
            }
            if matches!(cx.ftypes.get(*value as usize), Some(LirType::Any)) {
                any_retain_val(cx, v)?;
            }
            let old = cx.builder.build_load(cx.context.i64_type(), at, "").map_err(err)?.into_int_value();
            cx.builder.build_store(at, v).map_err(err)?;
            if matches!(fty, LirType::Any) {
                any_release_val(cx, old)?;
            } else if matches!(fty, LirType::Str) {
                release_str_val(cx, old)?;
            } else if matches!(fty, LirType::Array(_)) {
                match fty {
                    LirType::Array(inner) => {
                        let ptr = as_ptr(cx, old)?;
                        let n = cx.context.i64_type().const_int(lir::instr::elem_size(&inner) as u64, false);
                        let dtor = array_elem_dtor(cx, lir, &inner)?;
                        cx.builder.build_call(cx.release_array, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
                    }
                    _ => {}
                }
            } else if matches!(fty, LirType::Enum(_)) {
                let ptr = as_ptr(cx, old)?;
                let (n, dtor) = match &fty {
                    LirType::Enum(ei) => match lir.enums.get(*ei) {
                        Some(d) => {
                            let n = cx.context.i64_type().const_int(enum_instance_size(d) as u64, false);
                            let dtor = enum_dtor_ptr(cx, lir, *ei)?;
                            (n, dtor)
                        }
                        None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown enum")),
                    },
                    _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unreachable")),
                };
                cx.builder.build_call(cx.release, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
            } else if matches!(fty, LirType::Obj(_)) {
                let ptr = as_ptr(cx, old)?;
                let (n, dtor) = match &fty {
                    LirType::Obj(name) => match lir.class_index.get(name) {
                        Some(fci) => {
                            let size = instance_size(lir.classes[*fci].fields.len()) as u64;
                            let n = cx.context.i64_type().const_int(size, false);
                            let dtor = field_dtor_ptr(cx, lir, *obj, *field)?;
                            (n, dtor)
                        }
                        None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown field class")),
                    },
                    _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unreachable")),
                };
                cx.builder.build_call(cx.release, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
            }
        }
        Instr::GetFieldByName { dst, obj, field , ..} => {
            let off = match cx.ftypes.get(*obj as usize) {
                Some(LirType::Obj(name)) => match lir.class_index.get(name) {
                    Some(ci) => match lir.classes[*ci].field_index.get(field) {
                        Some(fi) => field_offset(*fi),
                        None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown field")),
                    },
                    None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown class")),
                },
                _ => return Err(Diagnostic::new(Code::E108, "llvm subset: field of non-object")),
            };
            let base = load(cx, *obj)?;
            let at = byte_ptr(cx, base, off)?;
            let v = cx.builder.build_load(cx.context.i64_type(), at, "").map_err(err)?.into_int_value();
            store(cx, *dst, v.into())?;
            if matches!(cx.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                retain_val(cx, lir, *dst, v)?;
                own(cx, *dst);
            }
        }
        Instr::SetFieldByName { obj, field, value , ..} => {
            let (fi, fty) = match cx.ftypes.get(*obj as usize) {
                Some(LirType::Obj(name)) => match lir.class_index.get(name) {
                    Some(ci) => match lir.classes[*ci].field_index.get(field) {
                        Some(fi) => (*fi, lir.classes[*ci].fields[*fi].ty.clone()),
                        None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown field")),
                    },
                    None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown class")),
                },
                _ => return Err(Diagnostic::new(Code::E108, "llvm subset: field of non-object")),
            };
            let off = field_offset(fi);
            let base = load(cx, *obj)?;
            let at = byte_ptr(cx, base, off)?;
            let v = load(cx, *value)?;
            if matches!(cx.ftypes.get(*value as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                retain_val(cx, lir, *value, v)?;
            }
            if matches!(cx.ftypes.get(*value as usize), Some(LirType::Any)) {
                any_retain_val(cx, v)?;
            }
            let old = cx.builder.build_load(cx.context.i64_type(), at, "").map_err(err)?.into_int_value();
            cx.builder.build_store(at, v).map_err(err)?;
            if matches!(fty, LirType::Any) {
                any_release_val(cx, old)?;
            } else if matches!(fty, LirType::Str) {
                release_str_val(cx, old)?;
            } else if matches!(fty, LirType::Array(_)) {
                match fty {
                    LirType::Array(inner) => {
                        let ptr = as_ptr(cx, old)?;
                        let n = cx.context.i64_type().const_int(lir::instr::elem_size(&inner) as u64, false);
                        let dtor = array_elem_dtor(cx, lir, &inner)?;
                        cx.builder.build_call(cx.release_array, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
                    }
                    _ => {}
                }
            } else if matches!(fty, LirType::Enum(_)) {
                let ptr = as_ptr(cx, old)?;
                let (n, dtor) = match &fty {
                    LirType::Enum(ei) => match lir.enums.get(*ei) {
                        Some(d) => {
                            let n = cx.context.i64_type().const_int(enum_instance_size(d) as u64, false);
                            let dtor = enum_dtor_ptr(cx, lir, *ei)?;
                            (n, dtor)
                        }
                        None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown enum")),
                    },
                    _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unreachable")),
                };
                cx.builder.build_call(cx.release, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
            } else if matches!(fty, LirType::Obj(_)) {
                let ptr = as_ptr(cx, old)?;
                let (n, dtor) = match &fty {
                    LirType::Obj(name) => match lir.class_index.get(name) {
                        Some(fci) => {
                            let size = instance_size(lir.classes[*fci].fields.len()) as u64;
                            let n = cx.context.i64_type().const_int(size, false);
                            let dtor = field_dtor_ptr(cx, lir, *obj, fi)?;
                            (n, dtor)
                        }
                        None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown field class")),
                    },
                    _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unreachable")),
                };
                cx.builder.build_call(cx.release, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
            }
        }
        Instr::GenRefOf { dst, obj , ..} => {
            let ptr = load(cx, *obj)?;
            let site = cx.builder.build_call(cx.genref_create, &[ptr.into()], "").map_err(err)?;
            match site.try_as_basic_value() {
                ValueKind::Basic(v) => store(cx, *dst, v)?,
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: genref_create returned void"));
                }
            }
        }
        Instr::GenRefEmpty { dst , ..} => {
            store(cx, *dst, cx.context.i64_type().const_zero().into())?;
        }
        Instr::GenRefGet { dst, gref , ..} => {
            let packed = load(cx, *gref)?;
            let site = cx.builder.build_call(cx.genref_get, &[packed.into()], "").map_err(err)?;
            match site.try_as_basic_value() {
                ValueKind::Basic(v) => {
                    store(cx, *dst, v)?;
                    if matches!(cx.ftypes.get(*dst as usize), Some(LirType::Obj(_))) {
                        let got = v.into_int_value();
                        let ptr = as_ptr(cx, got)?;
                        cx.builder.build_call(cx.retain, &[ptr.into()], "").map_err(err)?;
                        own(cx, *dst);
                    }
                }
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: genref_get returned void"));
                }
            }
        }
        Instr::GenRefInvalidate { obj , ..} => {
            let raw = load(cx, *obj)?;
            cx.builder.build_call(cx.genref_invalidate, &[raw.into()], "").map_err(err)?;
        }
        Instr::Retain { obj , ..} => {
            let raw = load(cx, *obj)?;
            let ptr = as_ptr(cx, raw)?;
            cx.builder.build_call(cx.retain, &[ptr.into()], "").map_err(err)?;
        }
        Instr::ClosureNew { dst, func, captures, decay, .. } => {
            if *decay {
                return Err(Diagnostic::new(Code::E108, "llvm subset: decayed closure"));
            }
            let i64t = cx.context.i64_type();
            let fid = i64t.const_int(runtime::native::rnx_closure_tag(*func as u64), false);
            let nc = i64t.const_int(captures.len() as u64, false);
            let site = cx.builder.build_call(cx.closure_new, &[fid.into(), nc.into()], "").map_err(err)?;
            let bx = match site.try_as_basic_value() {
                ValueKind::Basic(v) => v.into_pointer_value(),
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: closure_new returned void"));
                }
            };
            for (i, c) in captures.iter().enumerate() {
                let raw = load(cx, *c)?;
                let cty = cx.ftypes.get(*c as usize).cloned();
                let tag = match cty.as_ref() {
                    Some(LirType::Str) => runtime::native::TAG_STR as u64,
                    Some(LirType::Closure) => runtime::native::TAG_CLOSURE as u64,
                    Some(LirType::I64) | Some(LirType::I8) | Some(LirType::Bool) | Some(LirType::Null) => {
                        runtime::native::TAG_INT as u64
                    }
                    Some(LirType::Obj(_)) => runtime::native::TAG_OBJ as u64,
                    Some(LirType::Array(inner)) if llvm_array_capturable(inner) => {
                        runtime::native::TAG_ARRAY as u64
                    }
                    _ => {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: closure captures unsupported value"));
                    }
                };
                if tag != runtime::native::TAG_INT as u64 {
                    retain_val(cx, lir, *c, raw)?;
                }
                let zero = i64t.const_int(0, false);
                let null_ptr = cx.context.ptr_type(inkwell::AddressSpace::default()).const_null();
                let (size_v, dtor_v, esize_v, edtor_v): (
                    inkwell::values::IntValue,
                    inkwell::values::PointerValue,
                    inkwell::values::IntValue,
                    inkwell::values::PointerValue,
                ) = match cty.as_ref() {
                    Some(LirType::Obj(name)) => match lir.class_index.get(name) {
                        Some(ci) => {
                            let size = i64t.const_int(
                                lir::instr::instance_size(lir.classes[*ci].fields.len()) as u64,
                                false,
                            );
                            let dtor = array_elem_dtor(cx, lir, &LirType::Obj(name.clone())).map_err(|e| {
                                Diagnostic::new(Code::E108, format!("llvm subset: capture dtor: {e}"))
                            })?;
                            (size, dtor, zero, null_ptr)
                        }
                        None => {
                            return Err(Diagnostic::new(
                                Code::E108,
                                format!("llvm subset: closure captures unknown class `{name}`"),
                            ));
                        }
                    },
                    Some(LirType::Array(inner)) => {
                        let esize = i64t.const_int(lir::instr::elem_size(inner) as u64, false);
                        let edtor = array_elem_dtor(cx, lir, inner).map_err(|e| {
                            Diagnostic::new(Code::E108, format!("llvm subset: capture elem dtor: {e}"))
                        })?;
                        (zero, null_ptr, esize, edtor)
                    }
                    _ => (zero, null_ptr, zero, null_ptr),
                };
                let ii = i64t.const_int(i as u64, false);
                let tt = i64t.const_int(tag, false);
                cx.builder.build_call(
                    cx.closure_set,
                    &[
                        bx.into(),
                        ii.into(),
                        tt.into(),
                        raw.into(),
                        size_v.into(),
                        dtor_v.into(),
                        esize_v.into(),
                        edtor_v.into(),
                    ],
                    "",
                ).map_err(err)?;
            }
            let h = cx.builder.build_ptr_to_int(bx, i64t, "").map_err(err)?;
            store(cx, *dst, h.into())?;
            own(cx, *dst);
            let zb = cx.context.i64_type().const_zero();
            track_heap(cx, h, runtime::native::HEAP_BOX, zb, zb)?;
        }
        Instr::ArrayNew { dst, cap, elem_size , ..} => {
            let c = cx.context.i64_type().const_int(*cap as u64, false);
            let e = cx.context.i64_type().const_int(*elem_size as u64, false);
            let site = cx.builder.build_call(cx.array_new, &[c.into(), e.into()], "").map_err(err)?;
            match site.try_as_basic_value() {
                ValueKind::Basic(v) => {
                    let r = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                    store(cx, *dst, r.into())?;
                    own(cx, *dst);
                    if let Some(LirType::Array(inner)) = cx.ftypes.get(*dst as usize).cloned() {
                        let edtor = array_elem_dtor(cx, lir, &inner)?;
                        let ed = cx.builder.build_ptr_to_int(edtor, cx.context.i64_type(), "").map_err(err)?;
                        track_heap(cx, r, runtime::native::HEAP_ARRAY, e, ed)?;
                        let (kind, aux) = lir::instr::pretty_kind_code(&inner);
                        let k = cx.context.i64_type().const_int(kind, false);
                        let a = cx.context.i64_type().const_int(aux, false);
                        cx.builder.build_call(cx.note_array_kind, &[r.into(), k.into(), a.into()], "").map_err(err)?;
                    }
                }
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: array_new returned void"));
                }
            }
        }
        Instr::ArrayPush { arr, value, elem_size , ..} => {
            if matches!(cx.ftypes.get(*value as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                let raw = load(cx, *value)?;
                retain_val(cx, lir, *value, raw)?;
            }
            if matches!(cx.ftypes.get(*value as usize), Some(LirType::Any)) {
                let raw = load(cx, *value)?;
                any_retain_val(cx, raw)?;
            }
            if *elem_size == 8 {
                inline_array_push(cx, *arr, *value)?;
                return Ok(());
            }
            let a = load(cx, *arr)?;
            let ap = as_ptr(cx, a)?;
            let v = load(cx, *value)?;
            let e = cx.context.i64_type().const_int(*elem_size as u64, false);
            cx.builder.build_call(cx.array_push, &[ap.into(), v.into(), e.into()], "").map_err(err)?;
        }
        Instr::ArrayGet { dst, arr, index, elem_size, unchecked , ..} => {
            if let Some(elem) = array_inline_elem(cx, *arr, *elem_size) {
                inline_array_get(cx, lir, *arr, *index, *dst, elem, !*unchecked)?;
            } else {
                let a = load(cx, *arr)?;
                let ap = as_ptr(cx, a)?;
                let i = load(cx, *index)?;
                let e = cx.context.i64_type().const_int(*elem_size as u64, false);
                let callee = if *unchecked { cx.array_get_unchecked } else { cx.array_get };
                let site = cx.builder.build_call(callee, &[ap.into(), i.into(), e.into()], "").map_err(err)?;
                match site.try_as_basic_value() {
                    ValueKind::Basic(v) => {
                        store(cx, *dst, v)?;
                        if matches!(cx.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                            let got = v.into_int_value();
                            retain_val(cx, lir, *dst, got)?;
                            own(cx, *dst);
                        }
                        if matches!(cx.ftypes.get(*dst as usize), Some(LirType::Any)) {
                            any_retain_val(cx, v.into_int_value())?;
                            own(cx, *dst);
                        }
                    }
                    ValueKind::Instruction(_) => {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: array_get returned void"));
                    }
                }
            }
        }
        Instr::ArraySet { arr, index, value, elem_size, unchecked , ..} => {
            let prim_value = matches!(
                cx.ftypes.get(*value as usize),
                Some(LirType::I64) | Some(LirType::I8) | Some(LirType::F64(_))
            );
            if prim_value && array_inline_elem(cx, *arr, *elem_size).is_some() {
                inline_array_set(cx, *arr, *index, *value, !*unchecked)?;
            } else {
                if matches!(cx.ftypes.get(*value as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                    let raw = load(cx, *value)?;
                    retain_val(cx, lir, *value, raw)?;
                }
                if matches!(cx.ftypes.get(*value as usize), Some(LirType::Any)) {
                    let raw = load(cx, *value)?;
                    any_retain_val(cx, raw)?;
                }
                let old = array_get_old(cx, *arr, *index, *elem_size, *unchecked)?;
                let a = load(cx, *arr)?;
                let ap = as_ptr(cx, a)?;
                let i = load(cx, *index)?;
                let v = load(cx, *value)?;
                let e = cx.context.i64_type().const_int(*elem_size as u64, false);
                let callee = if *unchecked { cx.array_set_unchecked } else { cx.array_set };
                cx.builder.build_call(callee, &[ap.into(), i.into(), v.into(), e.into()], "").map_err(err)?;
                array_release_old(cx, lir, *arr, old)?;
            }
        }
        Instr::ArrayLen { dst, arr , ..} => {
            match cx.ftypes.get(*arr as usize) {
                Some(LirType::Array(_)) | Some(LirType::Any) => {
                    inline_array_len(cx, *arr, *dst)?;
                }
                _ => {
                    let a = load(cx, *arr)?;
                    let ap = as_ptr(cx, a)?;
                    let site = cx.builder.build_call(cx.array_len, &[ap.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => store(cx, *dst, v)?,
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: array_len returned void"));
                        }
                    }
                }
            }
        }
        Instr::Release { obj , ..} => {
            release_any(cx, lir, *obj, None)?;
        }
        Instr::PtrLoad { span: _, dst, ptr, volatile } => {
            let p = load(cx, *ptr)?;
            let at = as_ptr(cx, p)?;
            let i64t = cx.context.i64_type();
            let loaded = match cx.ftypes.get(*dst as usize) {
                Some(LirType::Bool) | Some(LirType::I8) => {
                    let v = cx.builder.build_load(cx.context.i8_type(), at, "").map_err(err)?;
                    let ext = cx.builder.build_int_z_extend(v.into_int_value(), i64t, "").map_err(err)?;
                    if *volatile {
                        v.as_instruction_value()
                            .ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: volatile load has no instruction"))?
                            .set_volatile(true)
                            .map_err(|_| Diagnostic::new(Code::E108, "llvm subset: cannot mark load volatile"))?;
                    }
                    store(cx, *dst, ext.into())?;
                    None
                }
                _ => Some(cx.builder.build_load(i64t, at, "").map_err(err)?),
            };
            if let Some(site) = loaded {
                if *volatile {
                    site.as_instruction_value()
                        .ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: volatile load has no instruction"))?
                        .set_volatile(true)
                        .map_err(|_| Diagnostic::new(Code::E108, "llvm subset: cannot mark load volatile"))?;
                }
                store(cx, *dst, site)?;
            }
        }
        Instr::PtrStore { span: _, ptr, val, volatile } => {
            let p = load(cx, *ptr)?;
            let at = as_ptr(cx, p)?;
            let v = load(cx, *val)?;
            let stored: BasicValueEnum = match cx.ftypes.get(*val as usize) {
                Some(LirType::Bool) | Some(LirType::I8) => cx.builder.build_int_truncate(v, cx.context.i8_type(), "").map_err(err)?.into(),
                _ => v.into(),
            };
            let site = cx.builder.build_store(at, stored).map_err(err)?;
            if *volatile {
                site.set_volatile(true)
                    .map_err(|_| Diagnostic::new(Code::E108, "llvm subset: cannot mark store volatile"))?;
            }
        }
        Instr::ReleaseAs { obj, class, .. } => {
            let desc = lir.classes.get(*class).ok_or_else(|| {
                Diagnostic::new(Code::E108, "llvm subset: release of unknown class")
            })?;
            let base = load(cx, *obj)?;
            let ptr = as_ptr(cx, base)?;
            let n = cx.context.i64_type().const_int(lir::instr::instance_size(desc.fields.len()) as u64, false);
            let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
            let dtor = match desc.dtor {
                Some(fid) => {
                    let fname = &lir.functions[fid].name;
                    let fv = cx.funcs.get(fname).copied().ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("llvm subset: unknown dtor `{fname}`"))
                    })?;
                    fv.as_global_value().as_pointer_value()
                }
                None => ptr_t.const_null(),
            };
            cx.builder.build_call(cx.release, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
            cx.owned.remove(obj);
            cx.ever_owned.remove(obj);
        }
        Instr::ReleaseField { obj, field , ..} => {
            let (ci, off) = match cx.ftypes.get(*obj as usize) {
                Some(LirType::Obj(name)) => match lir.class_index.get(name) {
                    Some(ci) if *field < lir.classes[*ci].fields.len() => {
                        (*ci, field_offset(*field))
                    }
                    _ => return Err(Diagnostic::new(Code::E108, "llvm subset: bad field release")),
                },
                _ => return Err(Diagnostic::new(Code::E108, "llvm subset: field of non-object")),
            };
            let _ = ci;
            let base = load(cx, *obj)?;
            let at = byte_ptr(cx, base, off)?;
            let v = cx.builder.build_load(cx.context.i64_type(), at, "").map_err(err)?.into_int_value();
            match lir.classes[ci].fields[*field].ty.clone() {
                LirType::Any => {
                    any_release_val(cx, v)?;
                    return Ok(());
                }
                LirType::Str => {
                    release_str_val(cx, v)?;
                    return Ok(());
                }
                LirType::Array(inner) => {
                    let ptr = as_ptr(cx, v)?;
                    let n = cx.context.i64_type().const_int(lir::instr::elem_size(&inner) as u64, false);
                    let dtor = array_elem_dtor(cx, lir, &inner)?;
                    cx.builder.build_call(cx.release_array, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
                    return Ok(());
                }
                _ => {}
            }
            let (n, dtor) = match lir.classes[ci].fields[*field].ty.clone() {
                LirType::Enum(ei) => {
                    let size = match lir.enums.get(ei) {
                        Some(d) => enum_instance_size(d) as u64,
                        None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown enum")),
                    };
                    let n = cx.context.i64_type().const_int(size, false);
                    let dtor = enum_dtor_ptr(cx, lir, ei)?;
                    (n, dtor)
                }
                LirType::Obj(name) => match lir.class_index.get(&name) {
                    Some(fci) => {
                        let size = instance_size(lir.classes[*fci].fields.len()) as u64;
                        let n = cx.context.i64_type().const_int(size, false);
                        let dtor = field_dtor_ptr(cx, lir, *obj, *field)?;
                        (n, dtor)
                    }
                    None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown field class")),
                },
                _ => return Err(Diagnostic::new(Code::E108, "llvm subset: release of non-object field")),
            };
            let ptr = as_ptr(cx, v)?;
            cx.builder.build_call(cx.release, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
        }
        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unsupported instr")),
    }
    Ok(())
}
