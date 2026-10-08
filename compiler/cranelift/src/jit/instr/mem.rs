use super::*;

impl<M: Module> FnLower<'_, M> {
    pub(super) fn lower_obj_new(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ObjNew { dst, class, instance_size: size , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.alloc, &mut b.func);
                let n = b.ins().iconst(types::I64, *size as i64);
                let eight = b.ins().iconst(types::I64, 8);
                let inst = b.ins().call(callee, &[n, eight]);
                let ptr = b.inst_results(inst)[0];
                let one = b.ins().iconst(types::I32, 1);
                let zero32 = b.ins().iconst(types::I32, 0);
                let zero64 = b.ins().iconst(types::I64, 0);
                let type_idx = b.ins().iconst(types::I64, *class as i64 + 1);
                b.ins().store(MemFlagsData::trusted(), one, ptr, 0);
                b.ins().store(MemFlagsData::trusted(), zero32, ptr, 4);
                b.ins().store(MemFlagsData::trusted(), type_idx, ptr, 8);
                let fields = (*size as usize).saturating_sub(HEADER_SIZE) / SLOT_SIZE;
                for i in 0..fields {
                    b.ins().store(MemFlagsData::trusted(), zero64, ptr, (HEADER_SIZE + i * SLOT_SIZE) as i32);
                }
                self.set(b, *dst, ptr);
                self.own(*dst);
                let cname = self.lir.classes.get(*class).map(|c| c.name.rsplit('.').next().unwrap_or(&c.name).to_string()).unwrap_or_default();
                let name_ptr = self.str_addr(b, &cname)?;
                let callee = self.module.declare_func_in_func(self.note_type, &mut b.func);
                b.ins().call(callee, &[type_idx, name_ptr]);
                let dtor = self.dtor_for_class(b, *class)?;
                let sz = b.ins().iconst(types::I64, *size as i64);
                self.track_heap(b, ptr, runtime::native::HEAP_OBJ, dtor, sz);
                if let Some(desc) = self.lir.classes.get(*class) {
                    let fields = lir::instr::pretty_field_desc(desc);
                    let desc_ptr = self.str_addr(b, &fields)?;
                    let callee = self.module.declare_func_in_func(self.note_fields, &mut b.func);
                    b.ins().call(callee, &[type_idx, desc_ptr]);
                }
        Ok(())
    }

    pub(super) fn lower_stack_alloc(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::StackAlloc { dst, class, instance_size: size , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let slot = b.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    *size as u32,
                    3,
                ));
                let ptr = b.ins().stack_addr(types::I64, slot, 0);
                let one = b.ins().iconst(types::I32, 1);
                let zero32 = b.ins().iconst(types::I32, 0);
                let zero64 = b.ins().iconst(types::I64, 0);
                let type_idx = b.ins().iconst(types::I64, *class as i64 + 1);
                b.ins().store(MemFlagsData::trusted(), one, ptr, 0);
                b.ins().store(MemFlagsData::trusted(), zero32, ptr, 4);
                b.ins().store(MemFlagsData::trusted(), type_idx, ptr, 8);
                let fields = (*size as usize).saturating_sub(HEADER_SIZE) / SLOT_SIZE;
                for i in 0..fields {
                    b.ins().store(MemFlagsData::trusted(), zero64, ptr, (HEADER_SIZE + i * SLOT_SIZE) as i32);
                }
                self.set(b, *dst, ptr);
                let cname = self.lir.classes.get(*class).map(|c| c.name.rsplit('.').next().unwrap_or(&c.name).to_string()).unwrap_or_default();
                let name_ptr = self.str_addr(b, &cname)?;
                let callee = self.module.declare_func_in_func(self.note_type, &mut b.func);
                b.ins().call(callee, &[type_idx, name_ptr]);
                if let Some(desc) = self.lir.classes.get(*class) {
                    let fields = lir::instr::pretty_field_desc(desc);
                    let desc_ptr = self.str_addr(b, &fields)?;
                    let callee = self.module.declare_func_in_func(self.note_fields, &mut b.func);
                    b.ins().call(callee, &[type_idx, desc_ptr]);
                }
        Ok(())
    }

    pub(super) fn lower_enum_new(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::EnumNew { dst, enu, variant, payload , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let (size, max, ptys) = match self.lir.enums.get(*enu) {
                    Some(d) => match d.variants.get(*variant) {
                        Some(v) => (enum_instance_size(d), enum_max_payloads(d), v.payload.clone()),
                        None => return Err("unknown variant".to_string()),
                    },
                    None => return Err("unknown enum".to_string()),
                };
                if ptys.len() != payload.len() {
                    return Err("enum payload arity".to_string());
                }
                let callee = self.module.declare_func_in_func(self.alloc, &mut b.func);
                let n = b.ins().iconst(types::I64, size as i64);
                let eight = b.ins().iconst(types::I64, 8);
                let inst = b.ins().call(callee, &[n, eight]);
                let ptr = b.inst_results(inst)[0];
                let one = b.ins().iconst(types::I32, 1);
                let zero32 = b.ins().iconst(types::I32, 0);
                let zero64 = b.ins().iconst(types::I64, 0);
                b.ins().store(MemFlagsData::trusted(), one, ptr, 0);
                b.ins().store(MemFlagsData::trusted(), zero32, ptr, 4);
                b.ins().store(MemFlagsData::trusted(), zero64, ptr, 8);
                let tag = b.ins().iconst(types::I64, *variant as i64);
                b.ins().store(MemFlagsData::trusted(), tag, ptr, 16);
                for (i, (l, ty)) in payload.iter().zip(ptys.iter()).enumerate() {
                    let v = self.val(b, *l);
                    let declared_heap = match ty {
                        LirType::Obj(name) => self.lir.class_index.contains_key(name),
                        LirType::Str | LirType::Array(_) | LirType::Enum(_) => true,
                        _ => false,
                    };
                    if declared_heap {
                        let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(callee, &[v]);
                    }
                    if matches!(ty, LirType::Any)
                        || matches!(ty, LirType::Obj(_)) && !declared_heap
                    {
                        if matches!(self.ftypes.get(*l as usize), Some(LirType::Obj(_)) | Some(LirType::Enum(_)) | Some(LirType::Array(_))) {
                            if self.erased_obj(*l) {
                                self.any_retain_val(b, v);
                            } else {
                                let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                                b.ins().call(callee, &[v]);
                            }
                        } else {
                            self.any_retain_val(b, v);
                        }
                    }
                    b.ins().store(MemFlagsData::trusted(), v, ptr, (24 + i * SLOT_SIZE) as i32);
                }
                for i in payload.len()..max {
                    b.ins().store(MemFlagsData::trusted(), zero64, ptr, (24 + i * SLOT_SIZE) as i32);
                }
                self.set(b, *dst, ptr);
                self.own(*dst);
                if let Some(LirType::Enum(ei)) = self.ftypes.get(*dst as usize).cloned() {
                    if let Some(d) = self.lir.enums.get(ei) {
                        let edtor = self.enum_dtor_addr(b, ei)?;
                        let esz = b.ins().iconst(types::I64, enum_instance_size(d) as i64);
                        self.track_heap(b, ptr, runtime::native::HEAP_ENUM, edtor, esz);
                    }
                }
                if let Some(d) = self.lir.enums.get(*enu) {
                    if let Some(v) = d.variants.get(*variant) {
                        let desc = lir::instr::pretty_variant_desc(v);
                        let desc_ptr = self.str_addr(b, &desc)?;
                        let callee = self.module.declare_func_in_func(self.note_enum, &mut b.func);
                        let e = b.ins().iconst(types::I64, *enu as i64);
                        let vv = b.ins().iconst(types::I64, *variant as i64);
                        b.ins().call(callee, &[ptr, e, vv, desc_ptr]);
                    }
                }
        Ok(())
    }

    pub(super) fn lower_enum_payload(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::EnumPayload { dst, scrut, index , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let max = match self.ftypes.get(*scrut as usize) {
                    Some(LirType::Enum(ei)) => match self.lir.enums.get(*ei) {
                        Some(d) => enum_max_payloads(d),
                        None => return Err("unknown enum".to_string()),
                    },
                    _ => return Err("payload of non-enum".to_string()),
                };
                if *index >= max {
                    return Err("enum payload out of bounds".to_string());
                }
                let ptr = self.val(b, *scrut);
                let v = b.ins().load(types::I64, MemFlagsData::trusted(), ptr, (24 + index * SLOT_SIZE) as i32);
                self.set(b, *dst, v);
                if matches!(self.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                    if self.erased_obj(*dst) {
                        self.any_retain_val(b, v);
                    } else {
                        let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(callee, &[v]);
                    }
                    self.own(*dst);
                }
                if matches!(self.ftypes.get(*dst as usize), Some(LirType::Any)) {
                    if !self.in_enum_dtor {
                        self.any_retain_val(b, v);
                    }
                    self.own(*dst);
                }
        Ok(())
    }

    pub(super) fn lower_enum_tag(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::EnumTag { dst, scrut , ..} = ins else {
            return Err("unreachable".to_string());
        };
                if !matches!(self.ftypes.get(*scrut as usize), Some(LirType::Enum(_))) {
                    return Err("tag of non-enum".to_string());
                }
                let ptr = self.val(b, *scrut);
                let v = b.ins().load(types::I64, MemFlagsData::trusted(), ptr, 16);
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_get_field(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::GetField { dst, obj, field, span , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let recv = self.val(b, *obj);
                self.trap_if_null(b, *span, "field of null", recv)?;
                let (ptr, off) = self.indexed_field(b, *obj, *field)?;
                let v = b.ins().load(types::I64, MemFlagsData::trusted(), ptr, off as i32);
                self.set(b, *dst, v);
                if matches!(self.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                    if self.erased_obj(*dst) {
                        self.any_retain_val(b, v);
                    } else {
                        let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(callee, &[v]);
                    }
                    self.own(*dst);
                }
                if matches!(self.ftypes.get(*dst as usize), Some(LirType::Any)) {
                    self.any_retain_val(b, v);
                    self.own(*dst);
                }
        Ok(())
    }

    pub(super) fn lower_set_field(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::SetField { obj, field, value, span , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let recv = self.val(b, *obj);
                self.trap_if_null(b, *span, "field-set on null", recv)?;
                let (ptr, off) = self.indexed_field(b, *obj, *field)?;
                let v = self.val(b, *value);
                if matches!(self.ftypes.get(*value as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                    if self.erased_obj(*value) {
                        self.any_retain_val(b, v);
                    } else {
                        let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(callee, &[v]);
                    }
                }
                if matches!(self.ftypes.get(*value as usize), Some(LirType::Any)) {
                    self.any_retain_val(b, v);
                }
                let old = b.ins().load(types::I64, MemFlagsData::trusted(), ptr, off as i32);
                b.ins().store(MemFlagsData::trusted(), v, ptr, off as i32);
                let ci = self.field_class(*obj, *field)?;
                let fty = self.lir.classes[ci].fields[*field].ty.clone();
                if matches!(fty, LirType::Any) {
                    self.any_release_val(b, old);
                } else if matches!(fty, LirType::Str) {
                    self.release_str_val(b, old);
                } else if matches!(fty, LirType::Array(_)) {
                    match fty {
                        LirType::Array(inner) => self.release_array_val(b, old, &inner)?,
                        _ => {}
                    }
                } else if matches!(fty, LirType::Obj(_)) {
                    let size = self.field_obj_size(*obj, *field)?;
                    let dtor = self.field_dtor_addr(b, *obj, *field)?;
                    let callee = self.module.declare_func_in_func(self.release, &mut b.func);
                    let n = b.ins().iconst(types::I64, size as i64);
                    b.ins().call(callee, &[old, n, dtor]);
                } else if matches!(fty, LirType::Enum(_)) {
                    let (n, dtor) = match &fty {
                        LirType::Enum(ei) => {
                            let desc = self.lir.enums.get(*ei).ok_or("unknown enum".to_string())?;
                            let size = enum_instance_size(desc);
                            let dtor = self.enum_dtor_addr(b, *ei)?;
                            (b.ins().iconst(types::I64, size as i64), dtor)
                        }
                        _ => return Err("unreachable".to_string()),
                    };
                    let callee = self.module.declare_func_in_func(self.release, &mut b.func);
                    b.ins().call(callee, &[old, n, dtor]);
                }
        Ok(())
    }

    pub(super) fn lower_get_field_by_name(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::GetFieldByName { dst, obj, field, span , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let recv = self.val(b, *obj);
                self.trap_if_null(b, *span, "field of null", recv)?;
                let (ptr, off) = self.field_addr(b, *obj, field)?;
                let v = b.ins().load(types::I64, MemFlagsData::trusted(), ptr, off as i32);
                self.set(b, *dst, v);
                if matches!(self.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                    if self.erased_obj(*dst) {
                        self.any_retain_val(b, v);
                    } else {
                        let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(callee, &[v]);
                    }
                    self.own(*dst);
                }
                if matches!(self.ftypes.get(*dst as usize), Some(LirType::Any)) {
                    self.any_retain_val(b, v);
                    self.own(*dst);
                }
        Ok(())
    }

    pub(super) fn lower_set_field_by_name(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::SetFieldByName { obj, field, value, span , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let recv = self.val(b, *obj);
                self.trap_if_null(b, *span, "field-set on null", recv)?;
                let (ptr, off) = self.field_addr(b, *obj, field)?;
                let v = self.val(b, *value);
                if matches!(self.ftypes.get(*value as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                    if self.erased_obj(*value) {
                        self.any_retain_val(b, v);
                    } else {
                        let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(callee, &[v]);
                    }
                }
                if matches!(self.ftypes.get(*value as usize), Some(LirType::Any)) {
                    self.any_retain_val(b, v);
                }
                let old = b.ins().load(types::I64, MemFlagsData::trusted(), ptr, off as i32);
                b.ins().store(MemFlagsData::trusted(), v, ptr, off as i32);
                let name = match self.ftypes.get(*obj as usize) {
                    Some(LirType::Obj(name)) => name.clone(),
                    _ => return Err("field of non-object".to_string()),
                };
                let ci = self.lir.class_index.get(&name).copied().ok_or("unknown class".to_string())?;
                let fi = self.lir.classes[ci].field_index.get(field).copied().ok_or("unknown field".to_string())?;
                let fty = self.lir.classes[ci].fields[fi].ty.clone();
                if matches!(fty, LirType::Any) {
                    self.any_release_val(b, old);
                } else if matches!(fty, LirType::Str) {
                    self.release_str_val(b, old);
                } else if matches!(fty, LirType::Array(_)) {
                    match fty {
                        LirType::Array(inner) => self.release_array_val(b, old, &inner)?,
                        _ => {}
                    }
                } else if matches!(fty, LirType::Enum(_)) {
                    let (n, dtor) = match &fty {
                        LirType::Enum(ei) => {
                            let desc = self.lir.enums.get(*ei).ok_or("unknown enum".to_string())?;
                            let size = enum_instance_size(desc);
                            let dtor = self.enum_dtor_addr(b, *ei)?;
                            (b.ins().iconst(types::I64, size as i64), dtor)
                        }
                        _ => return Err("unreachable".to_string()),
                    };
                    let callee = self.module.declare_func_in_func(self.release, &mut b.func);
                    b.ins().call(callee, &[old, n, dtor]);
                } else if matches!(fty, LirType::Obj(_)) {
                    let (n, dtor) = match &fty {
                        LirType::Obj(name) => {
                            let fci = self.lir.class_index.get(name).copied().ok_or("unknown field class".to_string())?;
                            let size = instance_size(self.lir.classes[fci].fields.len());
                            let dtor = self.dtor_for_class(b, fci)?;
                            (b.ins().iconst(types::I64, size as i64), dtor)
                        }
                        _ => return Err("unreachable".to_string()),
                    };
                    let callee = self.module.declare_func_in_func(self.release, &mut b.func);
                    b.ins().call(callee, &[old, n, dtor]);
                }
        Ok(())
    }

    pub(super) fn lower_gen_ref_of(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::GenRefOf { dst, obj , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.genref_create, &mut b.func);
                let ptr = self.val(b, *obj);
                let inst = b.ins().call(callee, &[ptr]);
                let v = b.inst_results(inst)[0];
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_gen_ref_empty(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::GenRefEmpty { dst , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let z = b.ins().iconst(types::I64, 0);
                self.set(b, *dst, z);
        Ok(())
    }

    pub(super) fn lower_gen_ref_get(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::GenRefGet { dst, gref , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.genref_get, &mut b.func);
                let packed = self.val(b, *gref);
                let inst = b.ins().call(callee, &[packed]);
                let v = b.inst_results(inst)[0];
                self.set(b, *dst, v);
                if matches!(self.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                    let rcallee = self.module.declare_func_in_func(self.retain, &mut b.func);
                    b.ins().call(rcallee, &[v]);
                    self.own(*dst);
                }
        Ok(())
    }

    pub(super) fn lower_gen_ref_invalidate(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::GenRefInvalidate { obj , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.genref_invalidate, &mut b.func);
                let ptr = self.val(b, *obj);
                b.ins().call(callee, &[ptr]);
        Ok(())
    }

    pub(super) fn lower_retain(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Retain { obj , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                let ptr = self.val(b, *obj);
                b.ins().call(callee, &[ptr]);
        Ok(())
    }

    pub(super) fn lower_closure_new(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ClosureNew { dst, func, captures, decay, .. } = ins else {
            return Err("unreachable".to_string());
        };
                if *decay {
                    return Err("decayed closure".to_string());
                }
                let fid = b.ins().iconst(types::I64, runtime::native::rnx_closure_tag(*func as u64) as i64);
                let nc = b.ins().iconst(types::I64, captures.len() as i64);
                let new_callee = self.module.declare_func_in_func(self.closure_new, &mut b.func);
                let inst = b.ins().call(new_callee, &[fid, nc]);
                let bx = b.inst_results(inst)[0];
                for (i, c) in captures.iter().enumerate() {
                    let v = self.val(b, *c);
                    let cty = self.ftypes.get(*c as usize).cloned();
                    let tag = match cty.as_ref() {
                        Some(LirType::Str) => runtime::native::TAG_STR as i64,
                        Some(LirType::Closure) => runtime::native::TAG_CLOSURE as i64,
                        Some(LirType::I64) | Some(LirType::Bool) | Some(LirType::Null) => {
                            runtime::native::TAG_INT as i64
                        }
                        Some(LirType::Obj(_)) => runtime::native::TAG_OBJ as i64,
                        Some(LirType::Array(inner)) if array_capturable(inner) => {
                            runtime::native::TAG_ARRAY as i64
                        }
                        _ => return Err("closure captures unsupported value".to_string()),
                    };
                    if tag != runtime::native::TAG_INT as i64 {
                        let rcallee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(rcallee, &[v]);
                    }
                    let zero = b.ins().iconst(types::I64, 0);
                    let (size_v, dtor_v, esize_v, edtor_v) =
                        match cty.as_ref() {
                            Some(LirType::Obj(name)) => {
                                match self.lir.class_index.get(name) {
                                    Some(ci) => {
                                        let size = b.ins().iconst(
                                            types::I64,
                                            lir::instr::instance_size(
                                                self.lir.classes[*ci].fields.len(),
                                            )
                                                as i64,
                                        );
                                        let dtor = self.dtor_for_class(b, *ci).map_err(|e| {
                                            format!("closure capture dtor: {e}")
                                        })?;
                                        (size, dtor, zero, zero)
                                    }
                                    None => {
                                        return Err(format!(
                                            "closure captures unknown class `{name}`"
                                        ));
                                    }
                                }
                            }
                            Some(LirType::Array(inner)) => {
                                let esize = b.ins().iconst(
                                    types::I64,
                                    lir::instr::elem_size(inner) as i64,
                                );
                                let edtor = self.array_elem_dtor(b, inner).map_err(|e| {
                                    format!("closure capture elem dtor: {e}")
                                })?;
                                (zero, zero, esize, edtor)
                            }
                            _ => (zero, zero, zero, zero),
                        };
                    let ii = b.ins().iconst(types::I64, i as i64);
                    let tt = b.ins().iconst(types::I64, tag);
                    let set_callee = self.module.declare_func_in_func(self.closure_set, &mut b.func);
                    b.ins().call(
                        set_callee,
                        &[bx, ii, tt, v, size_v, dtor_v, esize_v, edtor_v],
                    );
                }
                self.set(b, *dst, bx);
                self.own(*dst);
                let zero = b.ins().iconst(types::I64, 0);
                self.track_heap(b, bx, runtime::native::HEAP_BOX, zero, zero);
        Ok(())
    }

    pub(super) fn lower_array_new(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ArrayNew { dst, cap, elem_size , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.array_new, &mut b.func);
                let c = b.ins().iconst(types::I64, *cap as i64);
                let e = b.ins().iconst(types::I64, *elem_size as i64);
                let inst = b.ins().call(callee, &[c, e]);
                let v = b.inst_results(inst)[0];
                self.set(b, *dst, v);
                self.own(*dst);
                if let Some(LirType::Array(inner)) = self.ftypes.get(*dst as usize).cloned() {
                    self.track_array(b, v, &inner)?;
                }
        Ok(())
    }

    pub(super) fn lower_array_push(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ArrayPush { arr, value, elem_size , ..} = ins else {
            return Err("unreachable".to_string());
        };
                if matches!(self.ftypes.get(*value as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                    let v = self.val(b, *value);
                    if self.erased_obj(*value) {
                        self.any_retain_val(b, v);
                    } else {
                        let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(callee, &[v]);
                    }
                }
                if matches!(self.ftypes.get(*value as usize), Some(LirType::Any)) {
                    let v = self.val(b, *value);
                    self.any_retain_val(b, v);
                }
                if *elem_size == 8 {
                    use cranelift_codegen::ir::condcodes::IntCC;
                    let a = self.val(b, *arr);
                    let v = self.val(b, *value);
                    let slow_bb = b.create_block();
                    let fast_bb = b.create_block();
                    let join_bb = b.create_block();
                    let len = b.ins().load(types::I64, MemFlagsData::trusted(), a, 16);
                    let cap = b.ins().load(types::I64, MemFlagsData::trusted(), a, 24);
                    let full = b.ins().icmp(IntCC::Equal, len, cap);
                    let isnull = b.ins().icmp_imm_s(IntCC::Equal, a, 0);
                    let slow_cond = b.ins().bor(isnull, full);
                    b.ins().brif(slow_cond, slow_bb, &[], fast_bb, &[]);
                    b.switch_to_block(slow_bb);
                    let callee = self.module.declare_func_in_func(self.array_push, &mut b.func);
                    let e = b.ins().iconst(types::I64, 8);
                    b.ins().call(callee, &[a, v, e]);
                    b.ins().jump(join_bb, &[]);
                    b.switch_to_block(fast_bb);
                    let datap = b.ins().load(types::I64, MemFlagsData::trusted(), a, 32);
                    let eight = b.ins().iconst(types::I64, 8);
                    let off = b.ins().imul(len, eight);
                    let slot = b.ins().iadd(datap, off);
                    b.ins().store(MemFlagsData::trusted(), v, slot, 0);
                    let new_len = b.ins().iadd_imm_s(len, 1);
                    b.ins().store(MemFlagsData::trusted(), new_len, a, 16);
                    b.ins().jump(join_bb, &[]);
                    b.switch_to_block(join_bb);
                    b.seal_block(slow_bb);
                    b.seal_block(fast_bb);
                    b.seal_block(join_bb);
                    return Ok(());
                }
                let callee = self.module.declare_func_in_func(self.array_push, &mut b.func);
                let a = self.val(b, *arr);
                let v = self.val(b, *value);
                let e = b.ins().iconst(types::I64, *elem_size as i64);
                b.ins().call(callee, &[a, v, e]);
        Ok(())
    }

    pub(super) fn lower_array_get(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ArrayGet { dst, arr, index, elem_size, unchecked , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let fid = if *unchecked { self.array_get_unchecked } else { self.array_get };
                let callee = self.module.declare_func_in_func(fid, &mut b.func);
                let a = self.val(b, *arr);
                let i = self.val(b, *index);
                let e = b.ins().iconst(types::I64, *elem_size as i64);
                let inst = b.ins().call(callee, &[a, i, e]);
                let v = b.inst_results(inst)[0];
                self.set(b, *dst, v);
                if matches!(self.ftypes.get(*dst as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                    if self.erased_obj(*dst) {
                        self.any_retain_val(b, v);
                    } else {
                        let rcallee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(rcallee, &[v]);
                    }
                    self.own(*dst);
                }
                if matches!(self.ftypes.get(*dst as usize), Some(LirType::Any)) {
                    self.any_retain_val(b, v);
                    self.own(*dst);
                }
        Ok(())
    }

    pub(super) fn lower_array_set(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ArraySet { arr, index, value, elem_size, unchecked , ..} = ins else {
            return Err("unreachable".to_string());
        };
                if matches!(self.ftypes.get(*value as usize), Some(LirType::Any)) {
                    let v = self.val(b, *value);
                    self.any_retain_val(b, v);
                }
                if matches!(self.ftypes.get(*value as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_))) {
                    let v = self.val(b, *value);
                    if self.erased_obj(*value) {
                        self.any_retain_val(b, v);
                    } else {
                        let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(callee, &[v]);
                    }
                }
                let old = self.array_get_val(b, *arr, *index, *elem_size, *unchecked)?;
                let fid = if *unchecked { self.array_set_unchecked } else { self.array_set };
                let callee = self.module.declare_func_in_func(fid, &mut b.func);
                let a = self.val(b, *arr);
                let i = self.val(b, *index);
                let v = self.val(b, *value);
                let e = b.ins().iconst(types::I64, *elem_size as i64);
                b.ins().call(callee, &[a, i, v, e]);
                self.release_array_elem(b, *arr, old)?;
        Ok(())
    }

    pub(super) fn lower_array_len(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ArrayLen { dst, arr, span , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let recv = self.val(b, *arr);
                self.trap_if_null(b, *span, "len of null", recv)?;
                let callee = self.module.declare_func_in_func(self.array_len, &mut b.func);
                let a = self.val(b, *arr);
                let inst = b.ins().call(callee, &[a]);
                let v = b.inst_results(inst)[0];
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_ptr_load(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::PtrLoad { dst, ptr, .. } = ins else {
            return Err("unreachable".to_string());
        };
                let p = self.val(b, *ptr);
                let flags = MemFlagsData::new();
                let v = match self.ftypes.get(*dst as usize) {
                    Some(LirType::Bool) | Some(LirType::I8) => {
                        let byte = b.ins().load(types::I8, flags, p, 0);
                        b.ins().uextend(types::I64, byte)
                    }
                    _ => b.ins().load(types::I64, flags, p, 0),
                };
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_ptr_store(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::PtrStore { ptr, val, .. } = ins else {
            return Err("unreachable".to_string());
        };
                let p = self.val(b, *ptr);
                let v = self.val(b, *val);
                let flags = MemFlagsData::new();
                match self.ftypes.get(*val as usize) {
                    Some(LirType::Bool) | Some(LirType::I8) => {
                        let byte = b.ins().ireduce(types::I8, v);
                        b.ins().store(flags, byte, p, 0);
                    }
                    _ => {
                        b.ins().store(flags, v, p, 0);
                    }
                };
        Ok(())
    }

    pub(super) fn lower_release(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Release { obj , ..} = ins else {
            return Err("unreachable".to_string());
        };
                self.release_any(b, *obj, None)?;
        Ok(())
    }

    pub(super) fn lower_release_as(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ReleaseAs { obj, class, .. } = ins else {
            return Err("unreachable".to_string());
        };
                let size = instance_size(self.lir.classes[*class].fields.len()) as i64;
                let ptr = self.val(b, *obj);
                let dtor = self.dtor_for_class(b, *class)?;
                let callee = self.module.declare_func_in_func(self.release, &mut b.func);
                let n = b.ins().iconst(types::I64, size);
                b.ins().call(callee, &[ptr, n, dtor]);
                self.owned.remove(obj);
                self.ever_owned.remove(obj);
        Ok(())
    }

    pub(super) fn lower_release_field(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ReleaseField { obj, field , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let (ptr, off) = self.indexed_field(b, *obj, *field)?;
                let v = b.ins().load(types::I64, MemFlagsData::trusted(), ptr, off as i32);
                let ci = self.field_class(*obj, *field)?;
                let fty = self.lir.classes[ci].fields[*field].ty.clone();
                if matches!(fty, LirType::Any) {
                    self.any_release_val(b, v);
                } else if matches!(fty, LirType::Str) {
                    self.release_str_val(b, v);
                } else if matches!(fty, LirType::Array(_)) {
                    match fty {
                        LirType::Array(inner) => self.release_array_val(b, v, &inner)?,
                        _ => {}
                    }
                } else if matches!(fty, LirType::Enum(_)) {
                    match fty {
                        LirType::Enum(ei) => {
                            let desc = self.lir.enums.get(ei).ok_or("unknown enum".to_string())?;
                            let size = enum_instance_size(desc);
                            let dtor = self.enum_dtor_addr(b, ei)?;
                            let callee = self.module.declare_func_in_func(self.release, &mut b.func);
                            let n = b.ins().iconst(types::I64, size as i64);
                            b.ins().call(callee, &[v, n, dtor]);
                        }
                        _ => {}
                    }
                } else {
                    let size = self.field_obj_size(*obj, *field)?;
                    let dtor = self.field_dtor_addr(b, *obj, *field)?;
                    let callee = self.module.declare_func_in_func(self.release, &mut b.func);
                    let n = b.ins().iconst(types::I64, size as i64);
                    b.ins().call(callee, &[v, n, dtor]);
                }
        Ok(())
    }
}
