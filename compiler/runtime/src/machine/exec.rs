use super::*;

impl<'a> Machine<'a> {
    pub(super) fn exec(&mut self, fr: &mut Frame, ins: &Instr) -> Result<(), ExecError> {
        match ins {
            Instr::Const { dst, lit , ..} => {
                fr.locals[*dst as usize] = match lit {
                    Lit::Int(i) => Value::Int(*i),
                    Lit::Float(f, k) => Value::Float(*f, *k),
                    Lit::Bool(b) => Value::Bool(*b),
                    Lit::Str(s) => Value::Str(s.clone()),
                    Lit::Null => Value::Null,
                };
            }
            Instr::Copy { dst, src , ..} => {
                let sd = self.local_ty(fr, *dst);
                let ss = self.local_ty(fr, *src);
                match (sd, ss) {
                    (LirType::Obj(_), LirType::Obj(_)) => {
                        let alias = heap_alias(
                            &fr.locals[*dst as usize],
                            &fr.locals[*src as usize],
                        );
                        if fr.owned.contains(dst) && !alias {
                            self.drop_local(fr, *dst)?;
                        }
                        let v = fr.locals[*src as usize].clone();
                        fr.locals[*dst as usize] = v;
                        fr.owned.remove(src);
                        fr.ever_owned.remove(src);
                        self.own(fr, *dst);
                    }
                    (LirType::Array(_), LirType::Array(_)) => {
                        let alias = heap_alias(
                            &fr.locals[*dst as usize],
                            &fr.locals[*src as usize],
                        );
                        if fr.owned.contains(dst) && !alias {
                            self.drop_local(fr, *dst)?;
                        }
                        let v = fr.locals[*src as usize].clone();
                        fr.locals[*dst as usize] = v;
                        fr.owned.remove(src);
                        fr.ever_owned.remove(src);
                        self.own(fr, *dst);
                    }
                    (LirType::Obj(_), LirType::Null) | (LirType::Null, LirType::Obj(_)) => {
                        let v = fr.locals[*src as usize].clone();
                        fr.locals[*dst as usize] = v;
                        self.own(fr, *dst);
                    }
                    (LirType::Array(_), LirType::Null) | (LirType::Null, LirType::Array(_)) => {
                        let v = fr.locals[*src as usize].clone();
                        fr.locals[*dst as usize] = v;
                        self.own(fr, *dst);
                    }
                    (LirType::Enum(_), LirType::Enum(_)) => {
                        let alias = heap_alias(
                            &fr.locals[*dst as usize],
                            &fr.locals[*src as usize],
                        );
                        if fr.owned.contains(dst) && !alias {
                            self.drop_local(fr, *dst)?;
                        }
                        let v = fr.locals[*src as usize].clone();
                        fr.locals[*dst as usize] = v;
                        fr.owned.remove(src);
                        fr.ever_owned.remove(src);
                        self.own(fr, *dst);
                    }
                    (LirType::Enum(_), LirType::Null) | (LirType::Null, LirType::Enum(_)) => {
                        let v = fr.locals[*src as usize].clone();
                        fr.locals[*dst as usize] = v;
                        self.own(fr, *dst);
                    }
                    (LirType::Obj(_), LirType::Any) | (LirType::Any, LirType::Obj(_)) => {
                        let v = fr.locals[*src as usize].clone();
                        fr.locals[*dst as usize] = self.shared(v);
                    }
                    (LirType::Obj(_), _) | (_, LirType::Obj(_)) => {
                        return Err(ExecError::Fatal("mixed object copy".to_string()));
                    }
                    (LirType::Array(_), LirType::Any) | (LirType::Any, LirType::Array(_)) => {
                        let v = fr.locals[*src as usize].clone();
                        fr.locals[*dst as usize] = self.shared(v);
                    }
                    (LirType::Array(_), _) | (_, LirType::Array(_)) => {
                        return Err(ExecError::Fatal("mixed array copy".to_string()));
                    }
                    _ => {
                        let v = fr.locals[*src as usize].clone();
                        fr.locals[*dst as usize] = self.shared(v);
                    }
                }
            }
            Instr::Convert { dst, src, kind , ..} => {
                let v = fr.locals[*src as usize].clone();
                fr.locals[*dst as usize] = match kind {
                    ConvertKind::IntToFloat(k) => match v {
                        Value::Int(i) => Value::Float(i as f64, *k),
                        Value::Float(f, _) => Value::Float(f, *k),
                        _ => return Err(ExecError::Fatal("int-to-float of non-number".to_string())),
                    },
                    ConvertKind::FloatToInt => match v {
                        Value::Float(f, _) => Value::Int(f as i64),
                        Value::Int(i) => Value::Int(i),
                        _ => return Err(ExecError::Fatal("float-to-int of non-number".to_string())),
                    },
                };
            }
            Instr::Arith { op, dst, lhs, rhs, .. } => {
                let a = fr.locals[*lhs as usize].clone();
                let b = fr.locals[*rhs as usize].clone();
                fr.locals[*dst as usize] = arith(*op, &a, &b)?;
            }
            Instr::Fma { dst, a, b, c, .. } => {
                let (x, xk) = num(&fr.locals[*a as usize])?;
                let (y, yk) = num(&fr.locals[*b as usize])?;
                let (z, zk) = num(&fr.locals[*c as usize])?;
                let fast = xk == FloatKind::Fast || yk == FloatKind::Fast || zk == FloatKind::Fast;
                let k = if fast { FloatKind::Fast } else { FloatKind::Strict };
                let r = x.mul_add(y, z);
                fr.locals[*dst as usize] = Value::Float(r, k);
            }
            Instr::Cmp { op, dst, lhs, rhs, .. } => {
                let a = fr.locals[*lhs as usize].clone();
                let b = fr.locals[*rhs as usize].clone();
                fr.locals[*dst as usize] = Value::Bool(cmp(*op, &a, &b)?);
            }
            Instr::Not { dst, src , ..} => {
                let v = fr.locals[*src as usize].clone();
                fr.locals[*dst as usize] = Value::Bool(!v.truthy());
            }
            Instr::Neg { dst, src, .. } => {
                fr.locals[*dst as usize] = match &fr.locals[*src as usize] {
                    Value::Int(i) => Value::Int(-i),
                    Value::Float(f, k) => Value::Float(-f, *k),
                    v => {
                        return Err(ExecError::Fatal(format!("neg of {}", v.display())));
                    }
                };
            }
            Instr::Concat { dst, lhs, rhs , ..} => {
                let a = fr.locals[*lhs as usize].clone();
                let b = fr.locals[*rhs as usize].clone();
                fr.locals[*dst as usize] = Value::Str(self.to_pretty(&a) + &self.to_pretty(&b));
            }
            Instr::ToStr { dst, src , ..} => {
                let v = fr.locals[*src as usize].clone();
                fr.locals[*dst as usize] = Value::Str(self.to_pretty(&v));
            }
            Instr::Range { dst, lo, hi, inclusive , ..} => {
                let (a, b) = match (
                    &fr.locals[*lo as usize],
                    &fr.locals[*hi as usize],
                ) {
                    (Value::Int(a), Value::Int(b)) => (*a, *b),
                    _ => return Err(ExecError::Fatal("range bounds must be ints".to_string())),
                };
                fr.locals[*dst as usize] = Value::Range {
                    lo: a,
                    hi: b,
                    inclusive: *inclusive,
                    step: 1,
                };
            }
            Instr::Stride { dst, range, step , ..} => {
                let (lo, hi, incl) = match &fr.locals[*range as usize] {
                    Value::Range { lo, hi, inclusive, .. } => (*lo, *hi, *inclusive),
                    v => {
                        return Err(ExecError::Fatal(format!(
                            "stride of {}",
                            v.display()
                        )));
                    }
                };
                let st = match &fr.locals[*step as usize] {
                    Value::Int(i) => *i,
                    _ => return Err(ExecError::Fatal("stride step must be int".to_string())),
                };
                fr.locals[*dst as usize] = Value::Range {
                    lo,
                    hi,
                    inclusive: incl,
                    step: st,
                };
            }
            Instr::RangeLo { dst, range , ..} => {
                let lo = match &fr.locals[*range as usize] {
                    Value::Range { lo, .. } => *lo,
                    v => return Err(ExecError::Fatal(format!("rangeof {}", v.display()))),
                };
                fr.locals[*dst as usize] = Value::Int(lo);
            }
            Instr::RangeHi { dst, range , ..} => {
                let hi = match &fr.locals[*range as usize] {
                    Value::Range { hi, .. } => *hi,
                    v => return Err(ExecError::Fatal(format!("rangeof {}", v.display()))),
                };
                fr.locals[*dst as usize] = Value::Int(hi);
            }
            Instr::RangeStep { dst, range , ..} => {
                let st = match &fr.locals[*range as usize] {
                    Value::Range { step, .. } => *step,
                    v => return Err(ExecError::Fatal(format!("rangeof {}", v.display()))),
                };
                fr.locals[*dst as usize] = Value::Int(st);
            }
            Instr::ArrayNew { dst, cap, .. } => {
                let id = self.arrays.alloc(*cap);
                fr.locals[*dst as usize] = Value::Array { id };
                self.own(fr, *dst);
            }
            Instr::ArrayPush { arr, value, .. } => {
                let v = fr.locals[*value as usize].clone();
                match &fr.locals[*arr as usize] {
                    Value::Array { id } => {
                        let sv = self.shared(v);
                        match self.arrays.with_live(*id, |live| live.elems.push(sv)) {
                            Some(()) => {}
                            None => return Err(ExecError::Fatal("push on dead array".to_string())),
                        }
                    }
                    o => {
                        return Err(ExecError::Fatal(format!(
                            "push on {}",
                            o.display()
                        )));
                    }
                }
            }
            Instr::ArrayPop { dst, arr , ..} => {
                let v = match &fr.locals[*arr as usize] {
                    Value::Array { id } => match self
                        .arrays
                        .with_live(*id, |live| live.elems.pop().unwrap_or(Value::Null))
                    {
                        Some(v) => v,
                        None => return Err(ExecError::Fatal("pop on dead array".to_string())),
                    },
                    o => {
                        return Err(ExecError::Fatal(format!("pop on {}", o.display())));
                    }
                };
                fr.locals[*dst as usize] = v;
            }
            Instr::ArrayLen { dst, arr , ..} => {
                let n = match &fr.locals[*arr as usize] {
                    Value::Array { id } => match self.arrays.len(*id) {
                        Some(n) => n as i64,
                        None => return Err(ExecError::Fatal("len of dead array".to_string())),
                    },
                    Value::Str(s) => s.len() as i64,
                    o => {
                        return Err(ExecError::Fatal(format!("len of {}", o.display())));
                    }
                };
                fr.locals[*dst as usize] = Value::Int(n);
            }
            Instr::ArrayGet { dst, arr, index, unchecked, .. } => {
                let ix = match &fr.locals[*index as usize] {
                    Value::Int(i) => *i,
                    _ => return Err(ExecError::Fatal("index must be int".to_string())),
                };
                let v = match &fr.locals[*arr as usize] {
                    Value::Array { id } => {
                        let hit = self.arrays.with_live(*id, |live| {
                            if *unchecked {
                                Some(live.elems.get(ix as usize).cloned().unwrap_or(Value::Null))
                            } else if ix >= 0 {
                                live.elems.get(ix as usize).cloned()
                            } else {
                                None
                            }
                        });
                        match hit {
                            Some(Some(v)) => v,
                            _ => {
                                return Err(ExecError::Throw(Value::Error(ErrorVal {
                                    tag: "Panic".to_string(),
                                    message: "index out of bounds".to_string(),
                                })));
                            }
                        }
                    },
                    o => {
                        return Err(ExecError::Fatal(format!("index on {}", o.display())));
                    }
                };
                fr.locals[*dst as usize] = self.shared(v);
                if matches!(self.local_ty(fr, *dst), LirType::Obj(_) | LirType::Array(_)) {
                    self.own(fr, *dst);
                }
            }
            Instr::ArraySet { arr, index, value, unchecked, .. } => {
                let ix = match &fr.locals[*index as usize] {
                    Value::Int(i) => *i,
                    _ => return Err(ExecError::Fatal("index must be int".to_string())),
                };
                let v = fr.locals[*value as usize].clone();
                let old = match &fr.locals[*arr as usize] {
                    Value::Array { id } => {
                        let sv = self.shared(v);
                        let hit = self.arrays.with_live(*id, |live| {
                            if *unchecked {
                                Some(match live.elems.get_mut(ix as usize) {
                                    Some(slot) => std::mem::replace(slot, sv),
                                    None => Value::Null,
                                })
                            } else if ix >= 0 {
                                live.elems.get_mut(ix as usize).map(|slot| std::mem::replace(slot, sv))
                            } else {
                                None
                            }
                        });
                        match hit {
                            Some(Some(old)) => old,
                            _ => {
                                return Err(ExecError::Throw(Value::Error(ErrorVal {
                                    tag: "Panic".to_string(),
                                    message: "index out of bounds".to_string(),
                                })));
                            }
                        }
                    }
                    o => {
                        return Err(ExecError::Fatal(format!("index-set on {}", o.display())));
                    }
                };
                self.drop_value(old)?;
            }
            Instr::ObjNew { dst, class, .. } => {
                let desc = self.module.classes.get(*class);
                let n = desc.map(|c| c.fields.len()).unwrap_or(0);
                let is_struct = desc.map(|c| c.is_struct).unwrap_or(false);
                if is_struct {
                    fr.locals[*dst as usize] = Value::Struct {
                        class: *class,
                        fields: vec![Value::Null; n],
                    };
                } else {
                    let (slot, epoch) = self.arena.alloc(*class, vec![Value::Null; n]);
                    fr.locals[*dst as usize] = Value::Obj { slot, epoch };
                    self.own(fr, *dst);
                }
            }
            Instr::StackAlloc { dst, class, .. } => {
                let desc = self.module.classes.get(*class);
                let n = desc.map(|c| c.fields.len()).unwrap_or(0);
                let is_struct = desc.map(|c| c.is_struct).unwrap_or(false);
                if is_struct {
                    fr.locals[*dst as usize] = Value::Struct {
                        class: *class,
                        fields: vec![Value::Null; n],
                    };
                } else {
                    let (slot, epoch) = self.arena.alloc(*class, vec![Value::Null; n]);
                    fr.locals[*dst as usize] = Value::Obj { slot, epoch };
                    self.own(fr, *dst);
                }
            }
            Instr::EnumNew { dst, enu, variant, payload , ..} => {
                let mut items = Vec::with_capacity(payload.len());
                for c in payload {
                    let v = fr.locals[*c as usize].clone();
                    items.push(self.shared(v));
                }
                let name = self
                    .module
                    .enums
                    .get(*enu)
                    .and_then(|e| e.variants.get(*variant))
                    .map(|v| v.name.clone())
                    .unwrap_or_default();
                fr.locals[*dst as usize] = Value::Enum {
                    enu: *enu,
                    variant: *variant,
                    name,
                    payload: items,
                };
                self.own(fr, *dst);
            }
            Instr::EnumPayload { dst, scrut, index , ..} => {
                let v = match &fr.locals[*scrut as usize] {
                    Value::Enum { payload, .. } => payload
                        .get(*index)
                        .cloned()
                        .ok_or_else(|| ExecError::Fatal("enum payload out of bounds".to_string()))?,
                    o => {
                        return Err(ExecError::Fatal(format!(
                            "payload of {}",
                            o.display()
                        )));
                    }
                };
                fr.locals[*dst as usize] = self.shared(v);
                self.own(fr, *dst);
            }
            Instr::EnumTag { dst, scrut , ..} => {
                let tag = match &fr.locals[*scrut as usize] {
                    Value::Enum { variant, .. } => *variant as i64,
                    o => {
                        return Err(ExecError::Fatal(format!("tag of {}", o.display())));
                    }
                };
                fr.locals[*dst as usize] = Value::Int(tag);
            }
            Instr::Extract { dst, base, index, field , ..} => {
                let v = match &fr.locals[*base as usize] {
                    Value::Array { id } => {
                        let hit = self
                            .arrays
                            .with_live(*id, |live| live.elems.get(*index).cloned());
                        match hit {
                            Some(Some(v)) => self.shared(v),
                            Some(None) => {
                                return Err(ExecError::Fatal(
                                    "destructure out of bounds".to_string(),
                                ));
                            }
                            None => {
                                return Err(ExecError::Fatal("destructure of dead array".to_string()));
                            }
                        }
                    },
                    Value::Struct { class, fields } => {
                        let fi = *self
                            .module
                            .classes
                            .get(*class)
                            .and_then(|c| c.field_index.get(field))
                            .ok_or_else(|| {
                                ExecError::Fatal(format!("no field `{field}` to destructure"))
                            })?;
                        fields.get(fi).cloned().unwrap_or(Value::Null)
                    }
                    o => {
                        return Err(ExecError::Fatal(format!(
                            "cannot destructure {}",
                            o.display()
                        )));
                    }
                };
                fr.locals[*dst as usize] = self.shared(v);
            }
            Instr::AddrOf { dst, .. } => {
                let token = self.ptr_seq;
                self.ptr_seq += 1;
                fr.locals[*dst as usize] = Value::StackToken(token);
            }
            Instr::PtrLoad { dst, ptr, .. } => {
                let addr = match &fr.locals[*ptr as usize] {
                    Value::Pointer(a) | Value::Int(a) => *a as u64,
                    Value::StackToken(_) => {
                        return Err(ExecError::Fatal(
                            "cannot read through a stack-local token; use `Pointer.fromAddress` with a real address".to_string(),
                        ));
                    }
                    o => {
                        return Err(ExecError::Fatal(format!(
                            "pointer read of {}",
                            o.display()
                        )));
                    }
                };
                let v = match self.local_ty(fr, *dst) {
                    LirType::Bool => Value::Bool(unsafe { (addr as *const u8).read_unaligned() } != 0),
                    LirType::I8 => Value::Int(unsafe { (addr as *const u8).read_unaligned() } as i64),
                    LirType::F64(_) => Value::Float(
                        f64::from_bits(unsafe { (addr as *const u64).read_unaligned() }),
                        lir::instr::FloatKind::Strict,
                    ),
                    _ => Value::Int(unsafe { (addr as *const i64).read_unaligned() }),
                };
                fr.locals[*dst as usize] = v;
            }
            Instr::PtrStore { ptr, val, .. } => {
                let addr = match &fr.locals[*ptr as usize] {
                    Value::Pointer(a) | Value::Int(a) => *a as u64,
                    Value::StackToken(_) => {
                        return Err(ExecError::Fatal(
                            "cannot write through a stack-local token; use `Pointer.fromAddress` with a real address".to_string(),
                        ));
                    }
                    o => {
                        return Err(ExecError::Fatal(format!(
                            "pointer write of {}",
                            o.display()
                        )));
                    }
                };
                match &fr.locals[*val as usize] {
                    Value::Bool(b) => unsafe {
                        (addr as *mut u8).write_unaligned(*b as u8)
                    },
                    Value::Float(f, _) => unsafe {
                        (addr as *mut u64).write_unaligned(f.to_bits())
                    },
                    Value::Int(i) => {
                        let i = *i;
                        if self.local_ty(fr, *val) == LirType::I8 {
                            unsafe { (addr as *mut u8).write_unaligned(i as u8) }
                        } else {
                            unsafe { (addr as *mut i64).write_unaligned(i) }
                        }
                    }
                    o => {
                        return Err(ExecError::Fatal(format!(
                            "pointer write needs Int, Float, or Bool, got {}",
                            o.display()
                        )));
                    }
                }
            }
            Instr::GetField { dst, obj, field , ..} => {
                let (slot, epoch) = match &fr.locals[*obj as usize] {
                    Value::Obj { slot, epoch } => (*slot, *epoch),
                    Value::Struct { fields, .. } => {
                        let v = fields.get(*field).cloned().unwrap_or(Value::Null);
                        fr.locals[*dst as usize] = self.shared(v);
                        return Ok(());
                    }
                    o => {
                        return Err(ExecError::Fatal(format!(
                            "field of {}",
                            o.display()
                        )));
                    }
                };
                let v = self
                    .arena
                    .fields(slot, epoch)
                    .and_then(|f| f.get(*field).cloned())
                    .unwrap_or(Value::Null);
                fr.locals[*dst as usize] = self.shared(v);
                if matches!(self.local_ty(fr, *dst), LirType::Obj(_) | LirType::Array(_)) {
                    self.own(fr, *dst);
                }
            }
            Instr::SetField { obj, field, value , ..} => {
                let v = fr.locals[*value as usize].clone();
                match fr.locals[*obj as usize].clone() {
                    Value::Obj { slot, epoch } => {
                        let sv = self.shared(v);
                        let old = self
                            .arena
                            .fields(slot, epoch)
                            .and_then(|f| f.get(*field).cloned())
                            .unwrap_or(Value::Null);
                        if !self.arena.set_field(slot, epoch, *field, sv) {
                            return Err(ExecError::Fatal("dead object".to_string()));
                        }
                        self.drop_value(old)?;
                    }
                    Value::Struct { class, mut fields } => {
                        if *field < fields.len() {
                            fields[*field] = self.shared(v);
                            fr.locals[*obj as usize] = Value::Struct { class, fields };
                        }
                    }
                    o => {
                        return Err(ExecError::Fatal(format!(
                            "field-set on {}",
                            o.display()
                        )));
                    }
                }
            }
            Instr::GetFieldByName { dst, obj, field , ..} => {
                let v = match &fr.locals[*obj as usize] {
                    Value::Obj { slot, epoch } => {
                        let (slot, epoch) = (*slot, *epoch);
                        let class = self
                            .arena
                            .get(slot, epoch)
                            .ok_or_else(|| ExecError::Fatal("dead object".to_string()))?;
                        let fi = *self
                            .module
                            .classes
                            .get(class)
                            .and_then(|c| c.field_index.get(field))
                            .ok_or_else(|| {
                                ExecError::Fatal(format!("unknown field `{field}`"))
                            })?;
                        self.arena
                            .fields(slot, epoch)
                            .and_then(|f| f.get(fi).cloned())
                            .unwrap_or(Value::Null)
                    }
                    Value::Struct { class, fields } => {
                        let fi = *self
                            .module
                            .classes
                            .get(*class)
                            .and_then(|c| c.field_index.get(field))
                            .ok_or_else(|| {
                                ExecError::Fatal(format!("unknown field `{field}`"))
                            })?;
                        fields.get(fi).cloned().unwrap_or(Value::Null)
                    }
                    o => {
                        return Err(ExecError::Fatal(format!(
                            "field of {}",
                            o.display()
                        )));
                    }
                };
                fr.locals[*dst as usize] = self.shared(v);
                if matches!(self.local_ty(fr, *dst), LirType::Obj(_) | LirType::Array(_)) {
                    self.own(fr, *dst);
                }
            }
            Instr::SetFieldByName { obj, field, value , ..} => {
                let fi = match &fr.locals[*obj as usize] {
                    Value::Obj { slot, epoch } => {
                        let (slot, epoch) = (*slot, *epoch);
                        let class = self
                            .arena
                            .get(slot, epoch)
                            .ok_or_else(|| ExecError::Fatal("dead object".to_string()))?;
                        *self
                            .module
                            .classes
                            .get(class)
                            .and_then(|c| c.field_index.get(field))
                            .ok_or_else(|| {
                                ExecError::Fatal(format!("unknown field `{field}`"))
                            })?
                    }
                    Value::Struct { class, .. } => *self
                        .module
                        .classes
                        .get(*class)
                        .and_then(|c| c.field_index.get(field))
                        .ok_or_else(|| ExecError::Fatal(format!("unknown field `{field}`")))?,
                    o => {
                        return Err(ExecError::Fatal(format!(
                            "field-set on {}",
                            o.display()
                        )));
                    }
                };
                let v = fr.locals[*value as usize].clone();
                match fr.locals[*obj as usize].clone() {
                    Value::Obj { slot, epoch } => {
                        let sv = self.shared(v);
                        let old = self
                            .arena
                            .fields(slot, epoch)
                            .and_then(|f| f.get(fi).cloned())
                            .unwrap_or(Value::Null);
                        if !self.arena.set_field(slot, epoch, fi, sv) {
                            return Err(ExecError::Fatal("dead object".to_string()));
                        }
                        self.drop_value(old)?;
                    }
                    Value::Struct { class, mut fields } => {
                        if fi < fields.len() {
                            fields[fi] = self.shared(v);
                            fr.locals[*obj as usize] = Value::Struct { class, fields };
                        }
                    }
                    _ => {}
                }
            }
            Instr::ClosureNew {
                dst,
                func,
                captures,
                decay,
                decay_this,
                ..
            } => {
                let mut caps = Vec::with_capacity(captures.len());
                for (i, c) in captures.iter().enumerate() {
                    let mut v = fr.locals[*c as usize].clone();
                    if *decay_this && i + 1 == captures.len() {
                        v = match v {
                            Value::Obj { slot, epoch } => Value::GenRef {
                                slot: self.arena.get(slot, epoch).map(|_| slot),
                                epoch,
                            },
                            other => other,
                        };
                    } else {
                        v = self.shared(v);
                    }
                    caps.push(v);
                }
                fr.locals[*dst as usize] = Value::Closure(ClosureVal {
                    func: *func,
                    captures: caps,
                    decay: *decay,
                    decay_this: *decay_this,
                });
            }
            Instr::Call { span, dsts, err, target, args , ..} => {
                self.exec_call(fr, *span, dsts, *err, target, args)?;
            }
            Instr::GenRefOf { dst, obj , ..} => {
                let g = match &fr.locals[*obj as usize] {
                    Value::Obj { slot, epoch } => Value::GenRef {
                        slot: self.arena.get(*slot, *epoch).map(|_| *slot),
                        epoch: *epoch,
                    },
                    _ => Value::GenRef { slot: None, epoch: 0 },
                };
                fr.locals[*dst as usize] = g;
            }
            Instr::GenRefEmpty { dst , ..} => {
                fr.locals[*dst as usize] = Value::GenRef { slot: None, epoch: 0 };
            }
            Instr::ThreadSpawn { dst, func, closure, ..} => {
                let token = match closure {
                    Some(l) => match fr.locals[*l as usize].clone() {
                        Value::Closure(c) => {
                            let mut caps = Vec::with_capacity(c.captures.len());
                            for v in c.captures {
                                caps.push(self.shared(v));
                            }
                            crate::threads::spawn_closure_with(
                                self.module as *const _,
                                c.func,
                                caps,
                                self.heaps(),
                            )
                        }
                        o => {
                            return Err(ExecError::Fatal(format!("spawn of {}", o.display())));
                        }
                    },
                    None => crate::threads::spawn_with(self.module as *const _, *func, self.heaps()),
                };
                fr.locals[*dst as usize] = Value::Int(token);
            }
            Instr::Cast { dst, src , ..} => {
                let alias =
                    heap_alias(&fr.locals[*dst as usize], &fr.locals[*src as usize]);
                if fr.owned.contains(dst) && !alias {
                    self.drop_local(fr, *dst)?;
                }
                let v = fr.locals[*src as usize].clone();
                fr.locals[*dst as usize] = v;
                fr.owned.remove(src);
                fr.ever_owned.remove(src);
                match self.local_ty(fr, *dst) {
                    LirType::Obj(_) | LirType::Array(_) | LirType::Enum(_) => self.own(fr, *dst),
                    _ => {}
                }
            }
            Instr::ThreadJoin { dst, handle , ..} => {
                let token = match &fr.locals[*handle as usize] {
                    Value::Int(t) => *t,
                    o => {
                        return Err(ExecError::Fatal(format!("join of {}", o.display())));
                    }
                };
                let v = crate::threads::join(token)?;
                self.output.extend(crate::threads::take_output(token));
                fr.locals[*dst as usize] = v;
            }
            Instr::PoolInit { dst, id, workers , ..} => {
                let pid = self.pool_int(fr, *id, "pool id")?;
                let n = self.pool_int(fr, *workers, "worker count")?;
                unsafe { crate::native::rnx_thread_pool_init(pid, n) };
                fr.locals[*dst as usize] = Value::Pointer(pid);
            }
            Instr::PoolSubmit { dst, pool, func, arg, closure, ret_tag, ..} => {
                let pid = self.pool_handle(fr, *pool)?;
                crate::native::pool_check(pid).map_err(ExecError::Fatal)?;
                let a = match arg {
                    Some(l) => Some(self.pool_int(fr, *l, "task arg")?),
                    None => None,
                };
                crate::native::pool_set_interp_runner(crate::threads::pool_interp_run);
                let handle = match closure {
                    Some(l) => match fr.locals[*l as usize].clone() {
                        Value::Closure(_) => {
                            let bbox = crate::threads::closure_box_of(&fr.locals[*l as usize])
                                .map_err(ExecError::Fatal)? as usize;
                            unsafe {
                                crate::native::rnx_retain(bbox as *mut u8);
                            }
                            let heaps = Box::into_raw(Box::new(self.heaps())) as usize;
                            let slot = crate::native::task_box_new(*ret_tag);
                            let handle = crate::native::task_box_handle(&slot);
                            crate::native::pool_submit_task(
                                pid,
                                crate::native::PoolTask {
                                    target: crate::native::PoolTarget::ClosureInterp {
                                        module: self.module as *const _ as usize,
                                        heaps,
                                        bbox,
                                    },
                                    call: match a {
                                        Some(v) => crate::native::PoolCall::Once { arg: v, has_arg: true },
                                        None => crate::native::PoolCall::Once { arg: 0, has_arg: false },
                                    },
                                    result: Some(slot),
                                    ret_tag: *ret_tag,
                                },
                            );
                            handle as i64
                        }
                        o => {
                            return Err(ExecError::Fatal(format!("submit of {}", o.display())));
                        }
                    },
                    None => {
                        let heaps = Box::into_raw(Box::new(self.heaps())) as usize;
                        let slot = crate::native::task_box_new(*ret_tag);
                        let handle = crate::native::task_box_handle(&slot);
                        crate::native::pool_submit_task(
                            pid,
                            crate::native::PoolTask {
                                target: crate::native::PoolTarget::Interp {
                                    module: self.module as *const _ as usize,
                                    func: *func,
                                    heaps,
                                },
                                call: match a {
                                    Some(v) => crate::native::PoolCall::Once { arg: v, has_arg: true },
                                    None => crate::native::PoolCall::Once { arg: 0, has_arg: false },
                                },
                                result: Some(slot),
                                ret_tag: *ret_tag,
                            },
                        );
                        handle as i64
                    }
                };
                fr.locals[*dst as usize] = Value::Pointer(handle);
            }
            Instr::PoolParallelFor { pool, start, end, chunk, func, closure, ..} => {
                let pid = self.pool_handle(fr, *pool)?;
                crate::native::pool_check(pid).map_err(ExecError::Fatal)?;
                let s = self.pool_int(fr, *start, "start")?;
                let e = self.pool_int(fr, *end, "end")?;
                let c = self.pool_int(fr, *chunk, "chunk")?;
                crate::native::pool_set_interp_runner(crate::threads::pool_interp_run);
                let bbox = match closure {
                    Some(l) => match fr.locals[*l as usize].clone() {
                        Value::Closure(_) => Some(
                            crate::threads::closure_box_of(&fr.locals[*l as usize])
                                .map_err(ExecError::Fatal)? as usize,
                        ),
                        o => {
                            return Err(ExecError::Fatal(format!("parallelFor of {}", o.display())));
                        }
                    },
                    None => None,
                };
                if e > s && c > 0 {
                    let mut lo = s;
                    while lo < e {
                        let hi = (lo + c).min(e);
                        if let Some(b) = bbox {
                            unsafe {
                                crate::native::rnx_retain(b as *mut u8);
                            }
                        }
                        let heaps = Box::into_raw(Box::new(self.heaps())) as usize;
                        crate::native::pool_submit_task(
                            pid,
                            crate::native::PoolTask {
                                target: match bbox {
                                    Some(b) => crate::native::PoolTarget::ClosureInterp {
                                        module: self.module as *const _ as usize,
                                        heaps,
                                        bbox: b,
                                    },
                                    None => crate::native::PoolTarget::Interp {
                                        module: self.module as *const _ as usize,
                                        func: *func,
                                        heaps,
                                    },
                                },
                                call: crate::native::PoolCall::Range { start: lo, end: hi },
                                result: None,
                                ret_tag: 0,
                            },
                        );
                        lo = hi;
                    }
                }
                crate::native::pool_check(pid).map_err(ExecError::Fatal)?;
                crate::native::pool_join(pid);
            }
            Instr::PoolJoin { pool , ..} => {
                let pid = self.pool_handle(fr, *pool)?;
                crate::native::pool_check(pid).map_err(ExecError::Fatal)?;
                crate::native::pool_join(pid);
            }
            Instr::PoolShutdown { pool , ..} => {
                let pid = self.pool_handle(fr, *pool)?;
                unsafe { crate::native::rnx_thread_pool_shutdown(pid) };
            }
            Instr::VecNew { dst, kind, x, y, z, w , ..} => {
                let lanes = [*x, *y, *z, *w];
                match kind {
                    lir::instr::VecKind::F => {
                        let mut v = [0f32; 4];
                        for (i, l) in lanes.iter().enumerate() {
                            v[i] = self.vec_float(fr, *l)? as f32;
                        }
                        fr.locals[*dst as usize] = Value::Vec4f(v);
                    }
                    lir::instr::VecKind::I => {
                        let mut v = [0i32; 4];
                        for (i, l) in lanes.iter().enumerate() {
                            v[i] = self.pool_int(fr, *l, "lane")? as i32;
                        }
                        fr.locals[*dst as usize] = Value::Vec4i(v);
                    }
                }
            }
            Instr::VecSplat { dst, kind, val , ..} => {
                match kind {
                    lir::instr::VecKind::F => {
                        let f = self.vec_float(fr, *val)? as f32;
                        fr.locals[*dst as usize] = Value::Vec4f([f; 4]);
                    }
                    lir::instr::VecKind::I => {
                        let v = self.pool_int(fr, *val, "splat")? as i32;
                        fr.locals[*dst as usize] = Value::Vec4i([v; 4]);
                    }
                }
            }
            Instr::VecExtract { dst, vec, lane , ..} => {
                let i = self.pool_int(fr, *lane, "lane")?;
                if i < 0 || i > 3 {
                    return Err(ExecError::Fatal(format!("vector lane {i} out of range")));
                }
                match &fr.locals[*vec as usize] {
                    Value::Vec4f(v) => {
                        let f = v[i as usize] as f64;
                        fr.locals[*dst as usize] =
                            Value::Float(f, lir::instr::FloatKind::Strict);
                    }
                    Value::Vec4i(v) => {
                        fr.locals[*dst as usize] = Value::Int(v[i as usize] as i64);
                    }
                    o => {
                        return Err(ExecError::Fatal(format!("extract of {}", o.display())));
                    }
                }
            }
            Instr::VecInsert { dst, vec, lane, val , ..} => {
                if *lane > 3 {
                    return Err(ExecError::Fatal(format!("vector lane {lane} out of range")));
                }
                let i = *lane as usize;
                match &fr.locals[*vec as usize].clone() {
                    Value::Vec4f(v) => {
                        let mut out = *v;
                        out[i] = self.vec_float(fr, *val)? as f32;
                        fr.locals[*dst as usize] = Value::Vec4f(out);
                    }
                    Value::Vec4i(v) => {
                        let mut out = *v;
                        out[i] = self.pool_int(fr, *val, "lane")? as i32;
                        fr.locals[*dst as usize] = Value::Vec4i(out);
                    }
                    o => {
                        return Err(ExecError::Fatal(format!("insert of {}", o.display())));
                    }
                }
            }
            Instr::VecArith { dst, op, kind, lhs, rhs , ..} => {
                match kind {
                    lir::instr::VecKind::F => {
                        let a = self.vec_f32(fr, *lhs)?;
                        let b = self.vec_f32(fr, *rhs)?;
                        let mut v = [0f32; 4];
                        for i in 0..4 {
                            v[i] = match op {
                                lir::instr::VecOp::Add => a[i] + b[i],
                                lir::instr::VecOp::Sub => a[i] - b[i],
                                lir::instr::VecOp::Mul => a[i] * b[i],
                                lir::instr::VecOp::Div => a[i] / b[i],
                                lir::instr::VecOp::Min => a[i].min(b[i]),
                                lir::instr::VecOp::Max => a[i].max(b[i]),
                            };
                        }
                        fr.locals[*dst as usize] = Value::Vec4f(v);
                    }
                    lir::instr::VecKind::I => {
                        let a = self.vec_i32(fr, *lhs)?;
                        let b = self.vec_i32(fr, *rhs)?;
                        let mut v = [0i32; 4];
                        for i in 0..4 {
                            v[i] = match op {
                                lir::instr::VecOp::Add => a[i].wrapping_add(b[i]),
                                lir::instr::VecOp::Sub => a[i].wrapping_sub(b[i]),
                                lir::instr::VecOp::Mul => a[i].wrapping_mul(b[i]),
                                lir::instr::VecOp::Div => {
                                    if b[i] == 0 {
                                        return Err(ExecError::Fatal(
                                            "division by zero".to_string(),
                                        ));
                                    }
                                    a[i].wrapping_div(b[i])
                                }
                                lir::instr::VecOp::Min => a[i].min(b[i]),
                                lir::instr::VecOp::Max => a[i].max(b[i]),
                            };
                        }
                        fr.locals[*dst as usize] = Value::Vec4i(v);
                    }
                }
            }
            Instr::VecUnary { dst, op, src , ..} => {
                let a = self.vec_f32(fr, *src)?;
                let mut v = [0f32; 4];
                for i in 0..4 {
                    v[i] = match op {
                        lir::instr::VecUnaryOp::Sqrt => a[i].sqrt(),
                    };
                }
                fr.locals[*dst as usize] = Value::Vec4f(v);
            }
            Instr::VecDot { dst, lhs, rhs , ..} => {
                let a = self.vec_f32(fr, *lhs)?;
                let b = self.vec_f32(fr, *rhs)?;
                let mut acc = a[0] * b[0] + a[1] * b[1];
                acc = acc + a[2] * b[2];
                acc = acc + a[3] * b[3];
                fr.locals[*dst as usize] =
                    Value::Float(acc as f64, lir::instr::FloatKind::Strict);
            }
            Instr::GenRefGet { dst, gref , ..} => {
                let v = match &fr.locals[*gref as usize] {
                    Value::GenRef { slot: Some(s), epoch } => match self.arena.get(*s, *epoch) {
                        Some(_) => {
                            let o = Value::Obj { slot: *s, epoch: *epoch };
                            self.arena.retain(*s);
                            o
                        }
                        None => Value::Null,
                    },
                    _ => Value::Null,
                };
                fr.locals[*dst as usize] = v;
                if matches!(self.local_ty(fr, *dst), LirType::Obj(_) | LirType::Array(_)) {
                    self.own(fr, *dst);
                }
            }
            Instr::Retain { obj , ..} => {
                match &fr.locals[*obj as usize] {
                    Value::Obj { slot, .. } => self.arena.retain(*slot),
                    Value::Array { id } => self.arrays.retain(*id),
                    _ => {}
                }
            }
            Instr::Release { obj , ..} => {
                self.drop_local(fr, *obj)?;
            }
            Instr::ReleaseAs { obj, .. } => {
                self.drop_local(fr, *obj)?;
            }
            Instr::ReleaseField { obj, field , ..} => {
                let v = match &fr.locals[*obj as usize] {
                    Value::Obj { slot, epoch } => self
                        .arena
                        .fields(*slot, *epoch)
                        .and_then(|f| f.get(*field).cloned())
                        .unwrap_or(Value::Null),
                    _ => Value::Null,
                };
                self.drop_value(v)?;
            }
            Instr::GenRefInvalidate { .. } => {}
            Instr::Defer { body , ..} => {
                fr.defers.push(body.clone());
            }
            Instr::RunDefers { keep , ..} => {
                self.drain_defers(fr, *keep)?;
            }
            Instr::Assert { cond, message , ..} => {
                let c = fr.locals[*cond as usize].clone();
                if !c.truthy() {
                    let m = fr.locals[*message as usize].clone();
                    return Err(ExecError::Throw(Value::Error(ErrorVal {
                        tag: "Assert".to_string(),
                        message: self.to_pretty(&m),
                    })));
                }
            }
            Instr::Panic { message , ..} => {
                let m = fr.locals[*message as usize].clone();
                return Err(ExecError::Throw(Value::Error(ErrorVal {
                    tag: "Panic".to_string(),
                    message: self.to_pretty(&m),
                })));
            }
        }
        Ok(())
    }


    fn exec_call(
        &mut self,
        fr: &mut Frame,
        span: Span,
        dsts: &[Local],
        err: Option<Local>,
        target: &CallTarget,
        args: &[Local],
    ) -> Result<(), ExecError> {
        let argv: Vec<Value> = args.iter().map(|a| fr.locals[*a as usize].clone()).collect();
        let r = match target {
            CallTarget::Fn(id) => self.run_fn(*id, argv),
            CallTarget::Method { class, method } => self.run_fn(*method, argv).map_err(|e| {
                let _ = class;
                e
            }),
            CallTarget::Builtin(name) => Ok(vec![self.builtin(name, argv, span)?]),
            CallTarget::Dyn { obj, method } => {
                let base = fr.locals[*obj as usize].clone();
                self.dyn_call(base, method, argv).map(|v| vec![v])
            }
            CallTarget::Value(v) => {
                let f = fr.locals[*v as usize].clone();
                self.value_call(f, argv).map(|v| vec![v])
            }
            CallTarget::Foreign { lib, symbol } => self
                .foreign_call(lib, symbol, argv)
                .map(|v| vec![v]),
        };
        match r {
            Ok(vs) => {
                for (d, v) in dsts.iter().zip(vs.iter()) {
                    fr.locals[*d as usize] = v.clone();
                    if matches!(self.local_ty(fr, *d), LirType::Obj(_) | LirType::Array(_)) {
                        self.own(fr, *d);
                    }
                }
                if let Some(e) = err {
                    fr.locals[e as usize] = Value::Null;
                }
                Ok(())
            }
            Err(ExecError::Throw(ev)) => match err {
                Some(e) => {
                    fr.locals[e as usize] = ev;
                    for d in dsts {
                        fr.locals[*d as usize] = Value::Null;
                    }
                    Ok(())
                }
                None => Err(ExecError::Throw(ev)),
            },
            Err(e) => Err(e),
        }
    }


    fn foreign_call(&mut self, lib: &str, symbol: &str, argv: Vec<Value>) -> Result<Value, ExecError> {
        let _ = self;
        let addr = crate::native::resolve_foreign(lib, symbol).ok_or_else(|| {
            ExecError::Fatal(format!("unknown foreign symbol `{symbol}` in native lib `{lib}`"))
        })?;
        let mut ints = Vec::with_capacity(argv.len());
        for v in &argv {
            match v {
                Value::Int(i) => ints.push(*i),
                Value::Bool(b) => ints.push(*b as i64),
                Value::Pointer(a) => ints.push(*a),
                o => {
                    return Err(ExecError::Fatal(format!(
                        "foreign calls need Int, Bool, or Pointer arguments, got {}",
                        o.display()
                    )));
                }
            }
        }
        let r = unsafe {
            match ints.as_slice() {
                [] => {
                    let f: unsafe extern "C" fn() -> i64 = std::mem::transmute(addr);
                    f()
                }
                [a] => {
                    let f: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute(addr);
                    f(*a)
                }
                [a, b] => {
                    let f: unsafe extern "C" fn(i64, i64) -> i64 = std::mem::transmute(addr);
                    f(*a, *b)
                }
                [a, b, c] => {
                    let f: unsafe extern "C" fn(i64, i64, i64) -> i64 = std::mem::transmute(addr);
                    f(*a, *b, *c)
                }
                [a, b, c, d] => {
                    let f: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 =
                        std::mem::transmute(addr);
                    f(*a, *b, *c, *d)
                }
                [a, b, c, d, e] => {
                    let f: unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64 =
                        std::mem::transmute(addr);
                    f(*a, *b, *c, *d, *e)
                }
                [a, b, c, d, e, g] => {
                    let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64 =
                        std::mem::transmute(addr);
                    f(*a, *b, *c, *d, *e, *g)
                }
                _ => {
                    return Err(ExecError::Fatal(
                        "foreign calls take at most 6 arguments".to_string(),
                    ));
                }
            }
        };
        Ok(Value::Int(r))
    }


    fn value_call(&mut self, f: Value, args: Vec<Value>) -> Result<Value, ExecError> {
        match f {
            Value::Closure(c) => {
                let mut argv = args;
                if c.decay_this {
                    match c.captures.last() {
                        Some(Value::GenRef { slot: Some(s), epoch }) => {
                            if self.arena.get(*s, *epoch).is_none() {
                                return Ok(Value::Null);
                            }
                        }
                        Some(Value::GenRef { slot: None, .. }) => return Ok(Value::Null),
                        _ => {}
                    }
                }
                for cap in &c.captures {
                    argv.push(cap.clone());
                }
                self.run_fn(c.func, argv)
                    .map(|vs| vs.into_iter().next().unwrap_or(Value::Null))
            }
            _ => Err(ExecError::Fatal("call of non-function".to_string())),
        }
    }


    fn dyn_call(
        &mut self,
        base: Value,
        method: &str,
        args: Vec<Value>,
    ) -> Result<Value, ExecError> {
        match &base {
            Value::Obj { slot, epoch } => {
                let class = self
                    .arena
                    .get(*slot, *epoch)
                    .ok_or_else(|| ExecError::Fatal("dead object".to_string()))?;
                let mid = self
                    .module
                    .classes
                    .get(class)
                    .and_then(|c| c.methods.get(method).map(|r| r.id))
                    .ok_or_else(|| {
                        ExecError::Fatal(format!("unknown method `{method}`"))
                    })?;
                let mut argv = vec![base];
                argv.extend(args);
                self.run_fn(mid, argv)
                    .map(|vs| vs.into_iter().next().unwrap_or(Value::Null))
            }
            Value::Array { id } => {
                let id = *id;
                match method {
                    "len" | "length" => match self.arrays.len(id) {
                        Some(n) => Ok(Value::Int(n as i64)),
                        None => Err(ExecError::Fatal("len of dead array".to_string())),
                    },
                    "isEmpty" => match self.arrays.len(id) {
                        Some(n) => Ok(Value::Bool(n == 0)),
                        None => Err(ExecError::Fatal("len of dead array".to_string())),
                    },
                    "push" => {
                        let v = args.into_iter().next().unwrap_or(Value::Null);
                        let sv = self.shared(v);
                        match self.arrays.with_live(id, |live| live.elems.push(sv)) {
                            Some(()) => Ok(Value::Null),
                            None => Err(ExecError::Fatal("push on dead array".to_string())),
                        }
                    }
                    "pop" => match self
                        .arrays
                        .with_live(id, |live| live.elems.pop().unwrap_or(Value::Null))
                    {
                        Some(v) => Ok(v),
                        None => Err(ExecError::Fatal("pop on dead array".to_string())),
                    },
                    _ => Err(ExecError::Fatal(format!("unknown array method `{method}`"))),
                }
            }
            Value::Str(s) => match method {
                "len" => Ok(Value::Int(s.len() as i64)),
                _ => Err(ExecError::Fatal(format!("unknown string method `{method}`"))),
            },
            Value::Range { step, .. } if method == "stride" => {
                let _ = step;
                let st = match args.into_iter().next() {
                    Some(Value::Int(i)) => i,
                    _ => return Err(ExecError::Fatal("stride step must be int".to_string())),
                };
                match base {
                    Value::Range { lo, hi, inclusive, .. } => Ok(Value::Range {
                        lo,
                        hi,
                        inclusive,
                        step: st,
                    }),
                    _ => Err(ExecError::Fatal("stride of non-range".to_string())),
                }
            }
            Value::GenRef { .. } if method == "get" => {
                let v = match &base {
                    Value::GenRef { slot: Some(s), epoch } => match self.arena.get(*s, *epoch) {
                        Some(_) => {
                            self.arena.retain(*s);
                            Value::Obj { slot: *s, epoch: *epoch }
                        }
                        None => Value::Null,
                    },
                    _ => Value::Null,
                };
                Ok(v)
            }
            Value::Null | Value::Module(_) => match method {
                "print" | "error" | "warn" => {
                    let line: Vec<String> = args.iter().map(|v| v.display()).collect();
                    self.output.push(line.join(" "));
                    Ok(Value::Null)
                }
                _ => Err(ExecError::Fatal(format!("unknown method `{method}`"))),
            },
            _ => Err(ExecError::Fatal(format!("unknown method `{method}`"))),
        }
    }


    pub(super) fn exec_term(&mut self, fr: &mut Frame, term: Terminator) -> Result<Flow, ExecError> {
        match term {
            Terminator::Ret(v) => Ok(Flow::Return {
                values: v.iter().map(|l| fr.locals[*l as usize].clone()).collect(),
                locals: v.clone(),
            }),
            Terminator::Br(bb) => Ok(Flow::Goto(bb)),
            Terminator::BrIf { cond, then_bb, else_bb, .. } => {
                let c = fr.locals[cond as usize].clone();
                Ok(Flow::Goto(if c.truthy() { then_bb } else { else_bb }))
            }
            Terminator::BrErr { err, catch_bb, catch_bind, next_bb, depth, .. } => {
                self.drain_defers(fr, depth)?;
                match fr.locals[err as usize].clone() {
                    Value::Null => Ok(Flow::Goto(next_bb)),
                    ev => {
                        fr.locals[catch_bind as usize] = ev;
                        Ok(Flow::Goto(catch_bb))
                    }
                }
            }
            Terminator::Switch { scrut, cases, default, .. } => {
                let v = fr.locals[scrut as usize].clone();
                for (pat, bb) in &cases {
                    if pat_matches(self.module, &self.arena, &v, pat) {
                        return Ok(Flow::Goto(*bb));
                    }
                }
                Ok(Flow::Goto(default))
            }
            Terminator::Throw { src, catch, .. } => {
                let v = fr.locals[src as usize].clone();
                let ev = self.to_error(v);
                match catch {
                    Some((bb, err, depth)) => {
                        self.drain_defers(fr, depth)?;
                        fr.locals[err as usize] = ev;
                        Ok(Flow::Goto(bb))
                    }
                    None => {
                        self.drain_defers(fr, 0)?;
                        Err(ExecError::Throw(ev))
                    }
                }
            }
            Terminator::Rethrow { catch_bb, err, depth, .. } => {
                let ev = fr.locals[err as usize].clone();
                self.drain_defers(fr, depth)?;
                fr.locals[err as usize] = ev.clone();
                Ok(Flow::Goto(catch_bb))
            }
            Terminator::Unreachable { .. } => Err(ExecError::Fatal("unreachable executed".to_string())),
        }
    }

}
