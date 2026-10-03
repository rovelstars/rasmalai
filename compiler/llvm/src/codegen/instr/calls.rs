use super::*;

pub(super) fn lower_calls(cx: &mut FnCx, lir: &Module, ins: &Instr, fname: &str) -> Result<(), Diagnostic> {
    let _ = fname;
    let _ = lir;
    match ins {
        Instr::ThreadSpawn { dst, func, closure, ret_tag, ..} => {
            if let Some(l) = closure {
                let raw = load(cx, *l)?;
                let ptr = as_ptr(cx, raw)?;
                let i64t = cx.context.i64_type();
                let tag = i64t.const_int(*ret_tag as u64, false);
            let site = cx.builder.build_call(cx.spawn_closure, &[ptr.into(), tag.into()], "").map_err(err)?;
                match site.try_as_basic_value() {
                    ValueKind::Basic(v) => {
                        store(cx, *dst, v)?;
                    }
                    ValueKind::Instruction(_) => {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: spawn_closure returned void"));
                    }
                }
                return Ok(());
            }
            let fname = &lir.functions[*func].name;
            let fv = cx.funcs.get(fname).copied().ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("llvm subset: unknown thread fn `{fname}`"))
            })?;
            let entry = fv.as_global_value().as_pointer_value();
            let zero = cx.context.ptr_type(inkwell::AddressSpace::default()).const_null();
            let site = cx.builder.build_call(cx.thread_spawn, &[entry.into(), zero.into()], "").map_err(err)?;
            match site.try_as_basic_value() {
                ValueKind::Basic(v) => {
                    store(cx, *dst, v)?;
                }
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: thread_spawn returned void"));
                }
            }
        }
        Instr::ThreadJoin { dst, handle , ..} => {
            let raw = load(cx, *handle)?;
            let site = cx.builder.build_call(cx.thread_join, &[raw.into()], "").map_err(err)?;
            match site.try_as_basic_value() {
                ValueKind::Basic(v) => {
                    store(cx, *dst, v.into())?;
                }
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: thread_join returned void"));
                }
            }
        }
        Instr::PoolInit { dst, id, workers , ..} => {
            let i = load(cx, *id)?;
            let w = load(cx, *workers)?;
            cx.builder.build_call(cx.pool_init, &[i.into(), w.into()], "").map_err(err)?;
            store(cx, *dst, i.into())?;
        }
        Instr::PoolSubmit { dst, pool, func, arg, closure, ret_tag, ..} => {
            let p = load(cx, *pool)?;
            let i64t = cx.context.i64_type();
            let tag = i64t.const_int(*ret_tag as u64, false);
            if let Some(l) = closure {
                let raw = load(cx, *l)?;
                let bx = as_ptr(cx, raw)?;
                let bi = cx.builder.build_ptr_to_int(bx, i64t, "").map_err(err)?;
                let (a, has): (BasicValueEnum, BasicValueEnum) = match arg {
                    Some(x) => (load(cx, *x)?.into(), i64t.const_int(1, false).into()),
                    None => (i64t.const_int(0, false).into(), i64t.const_int(0, false).into()),
                };
                let site = cx.builder.build_call(cx.submit_closure, &[p.into(), bi.into(), a.into(), has.into(), tag.into()], "").map_err(err)?;
                match site.try_as_basic_value() {
                    ValueKind::Basic(v) => {
                        let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), i64t, "").map_err(err)?;
                        store(cx, *dst, h.into())?;
                    }
                    ValueKind::Instruction(_) => {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: submit_closure returned void"));
                    }
                }
                return Ok(());
            }
            let fname = &lir.functions[*func].name;
            let fv = cx.funcs.get(fname).copied().ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("llvm subset: unknown pool fn `{fname}`"))
            })?;
            let entry = cx
                .builder
                .build_ptr_to_int(fv.as_global_value().as_pointer_value(), cx.context.i64_type(), "")
                .map_err(err)?;
            let p = load(cx, *pool)?;
            let i64t = cx.context.i64_type();
            let (a, has): (BasicValueEnum, BasicValueEnum) = match arg {
                Some(l) => (load(cx, *l)?.into(), i64t.const_int(1, false).into()),
                None => (i64t.const_int(0, false).into(), i64t.const_int(0, false).into()),
            };
            let site = cx.builder
                .build_call(cx.submit_handle, &[p.into(), entry.into(), a.into(), has.into(), tag.into()], "")
                .map_err(err)?;
            match site.try_as_basic_value() {
                ValueKind::Basic(v) => {
                    let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), i64t, "").map_err(err)?;
                    store(cx, *dst, h.into())?;
                }
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: submit returned void"));
                }
            }
        }
        Instr::PoolParallelFor { pool, start, end, chunk, func, closure, ..} => {
            let p = load(cx, *pool)?;
            let s = load(cx, *start)?;
            let e = load(cx, *end)?;
            let c = load(cx, *chunk)?;
            if let Some(l) = closure {
                let raw = load(cx, *l)?;
                let bx = as_ptr(cx, raw)?;
                cx.builder
                    .build_call(cx.parallel_closure, &[p.into(), bx.into(), s.into(), e.into(), c.into()], "")
                    .map_err(err)?;
                return Ok(());
            }
            let fname = &lir.functions[*func].name;
            let fv = cx.funcs.get(fname).copied().ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("llvm subset: unknown pool fn `{fname}`"))
            })?;
            let entry = cx
                .builder
                .build_ptr_to_int(fv.as_global_value().as_pointer_value(), cx.context.i64_type(), "")
                .map_err(err)?;
            let p = load(cx, *pool)?;
            let s = load(cx, *start)?;
            let e = load(cx, *end)?;
            let c = load(cx, *chunk)?;
            cx.builder
                .build_call(cx.pool_parallel_for, &[p.into(), entry.into(), s.into(), e.into(), c.into()], "")
                .map_err(err)?;
        }
        Instr::PoolJoin { pool , ..} => {
            let p = load(cx, *pool)?;
            cx.builder.build_call(cx.pool_join, &[p.into()], "").map_err(err)?;
        }
        Instr::PoolShutdown { pool , ..} => {
            let p = load(cx, *pool)?;
            cx.builder.build_call(cx.pool_shutdown, &[p.into()], "").map_err(err)?;
        }
        Instr::Call { dsts, target, args, err: call_err, span, .. } => {
            let dst = dsts.first();
            if let CallTarget::Builtin(n) = target {
                if n == "print" {
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            let space = str_addr(cx, " ")?;
                            let at = byte_ptr(cx, space, 32)?;
                            let one = cx.context.i64_type().const_int(1, false);
                            cx.builder.build_call(cx.print_str, &[at.into(), one.into()], "").map_err(err)?;
                        }
                        let v = load(cx, *a)?;
                        let tag = if matches!(cx.ftypes.get(*a as usize), Some(LirType::Error)) {
                            let bit = cx.builder.build_and(v, cx.context.i64_type().const_int(1, false), "errbit").map_err(err)?;
                            let zero = cx.context.i64_type().const_zero();
                            let is_obj = cx.builder.build_int_compare(inkwell::IntPredicate::NE, bit, zero, "errobj").map_err(err)?;
                            let ptr_tag = cx.context.i32_type().const_int(runtime::native::TAG_PTR as u64, false);
                            let str_tag = cx.context.i32_type().const_int(runtime::native::TAG_STR as u64, false);
                            cx.builder.build_select(is_obj, ptr_tag, str_tag, "errtag").map_err(err)?.into_int_value()
                        } else {
                            cx.context.i32_type().const_int(print_tag(&cx.ftypes.get(*a as usize).cloned().unwrap_or(LirType::Any)), false)
                        };
                        cx.builder.build_call(cx.print_val, &[v.into(), tag.into()], "").map_err(err)?;
                    }
                    let nl = str_addr(cx, "\n")?;
                    let at = byte_ptr(cx, nl, 32)?;
                    let one = cx.context.i64_type().const_int(1, false);
                    cx.builder.build_call(cx.print_str, &[at.into(), one.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "streq" {
                    let argv: Vec<_> = args
                        .iter()
                        .map(|a| {
                            load(cx, *a).and_then(|v| {
                                as_ptr(cx, v).map(|p| p.into())
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let site = cx.builder.build_call(cx.streq, &argv, "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: streq returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "strcmp" {
                    let argv: Vec<_> = args
                        .iter()
                        .map(|a| {
                            load(cx, *a).and_then(|v| {
                                as_ptr(cx, v).map(|p| p.into())
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let site = cx.builder.build_call(cx.strcmp, &argv, "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: strcmp returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_eq_any_str" {
                    if args.len() != 2 {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: eq_any_str arity"));
                    }
                    let a = load(cx, args[0])?;
                    let b = load(cx, args[1])?;
                    let bp = as_ptr(cx, b)?;
                    let site = cx.builder.build_call(cx.eq_any_str, &[a.into(), bp.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: eq_any_str returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_eq_any" {
                    if args.len() != 2 {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: eq_any arity"));
                    }
                    let a = load(cx, args[0])?;
                    let b = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.eq_any, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: eq_any returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_crypto_random_u64" {
                    let site = cx.builder.build_call(cx.crypto_random, &[], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: crypto returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_prng_seed" {
                    let argv: Vec<_> = args
                        .iter()
                        .map(|a| {
                            load(cx, *a).and_then(|v| {
                                as_ptr(cx, v).map(|p| p.into())
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    if argv.len() != 2 {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: prng seed arity"));
                    }
                    let seed = load(cx, args[1])?;
                    cx.builder.build_call(cx.prng_seed, &[argv[0], seed.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_prng_next" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let site = cx.builder.build_call(cx.prng_next, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: prng next returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_file_open" {
                    let path = load(cx, args[0])?;
                    let pp = as_ptr(cx, path)?;
                    let mode = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.file_open, &[pp.into(), mode.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: file open returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_file_close" || n == "__rnx_file_flush" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let callee = if n == "__rnx_file_close" { cx.file_close } else { cx.file_flush };
                    cx.builder.build_call(callee, &[ptr.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n.starts_with("__rnx_bytes_")
                    || n == "__rnx_file_read_bytes"
                    || n == "__rnx_file_write_bytes"
                {
                    let sym = format!("rnx_{}", &n[6..]);
                    let callee = cx.bytes_fns.get(&sym).copied().ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("llvm subset: missing runtime `{sym}`"))
                    })?;
                    let handle = || -> Result<inkwell::values::PointerValue<'_>, Diagnostic> {
                        let raw = load(cx, args[0])?;
                        as_ptr(cx, raw)
                    };
                    let arg1 = || load(cx, args[1]);
                    match (args.len(), dst) {
                        (1, Some(d)) => {
                            if sym == "rnx_bytes_alloc" || sym == "rnx_bytes_data" {
                                let raw = load(cx, args[0])?;
                                let argv: Vec<inkwell::values::BasicMetadataValueEnum> =
                                    if sym == "rnx_bytes_alloc" {
                                        vec![raw.into()]
                                    } else {
                                        vec![as_ptr(cx, raw)?.into()]
                                    };
                                let site = cx.builder.build_call(callee, &argv, "").map_err(err)?;
                                match site.try_as_basic_value() {
                                    ValueKind::Basic(v) => {
                                        let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                        store(cx, *d, h.into())?;
                                    }
                                    ValueKind::Instruction(_) => {
                                        return Err(Diagnostic::new(Code::E108, "llvm subset: bytes alloc returned void"));
                                    }
                                }
                            } else {
                                let ptr = handle()?;
                                let site = cx.builder.build_call(callee, &[ptr.into()], "").map_err(err)?;
                                match site.try_as_basic_value() {
                                    ValueKind::Basic(v) => store(cx, *d, v)?,
                                    ValueKind::Instruction(_) => {
                                        return Err(Diagnostic::new(Code::E108, format!("llvm subset: {sym} returned void")));
                                    }
                                }
                            }
                        }
                        (1, None) => {
                            let ptr = handle()?;
                            cx.builder.build_call(callee, &[ptr.into()], "").map_err(err)?;
                        }
                        (2, Some(d)) => {
                            let ptr = handle()?;
                            let off = arg1()?;
                            let site = cx.builder.build_call(callee, &[ptr.into(), off.into()], "").map_err(err)?;
                            match site.try_as_basic_value() {
                                ValueKind::Basic(v) => store(cx, *d, v)?,
                                ValueKind::Instruction(_) => {
                                    return Err(Diagnostic::new(Code::E108, format!("llvm subset: {sym} returned void")));
                                }
                            }
                        }
                        (3, None) => {
                            let ptr = handle()?;
                            let a = arg1()?;
                            let b = load(cx, args[2])?;
                            cx.builder.build_call(callee, &[ptr.into(), a.into(), b.into()], "").map_err(err)?;
                        }
                        (3, Some(d)) => {
                            let ptr = handle()?;
                            let a = arg1()?;
                            let raw = load(cx, args[2])?;
                            let site = if sym == "rnx_bytes_write_string" {
                                let sp = as_ptr(cx, raw)?;
                                cx.builder.build_call(callee, &[ptr.into(), a.into(), sp.into()], "").map_err(err)?
                            } else {
                                cx.builder.build_call(callee, &[ptr.into(), a.into(), raw.into()], "").map_err(err)?
                            };
                            match site.try_as_basic_value() {
                                ValueKind::Basic(v) => {
                                    if sym == "rnx_bytes_read_string" {
                                        let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                        store(cx, *d, h.into())?;
                                        own(cx, *d);
                                    } else {
                                        store(cx, *d, v)?;
                                    }
                                }
                                ValueKind::Instruction(_) => {
                                    return Err(Diagnostic::new(Code::E108, format!("llvm subset: {sym} returned void")));
                                }
                            }
                        }
                        (4, None) => {
                            let ptr = handle()?;
                            let a = arg1()?;
                            let b = load(cx, args[2])?;
                            let c = load(cx, args[3])?;
                            cx.builder.build_call(callee, &[ptr.into(), a.into(), b.into(), c.into()], "").map_err(err)?;
                        }
                        (4, Some(d)) => {
                            let raw_fh = load(cx, args[0])?;
                            let fh = as_ptr(cx, raw_fh)?;
                            let raw_bh = load(cx, args[1])?;
                            let bh = as_ptr(cx, raw_bh)?;
                            let a = load(cx, args[2])?;
                            let b = load(cx, args[3])?;
                            let site = cx.builder.build_call(callee, &[fh.into(), bh.into(), a.into(), b.into()], "").map_err(err)?;
                            match site.try_as_basic_value() {
                                ValueKind::Basic(v) => store(cx, *d, v)?,
                                ValueKind::Instruction(_) => {
                                    return Err(Diagnostic::new(Code::E108, format!("llvm subset: {sym} returned void")));
                                }
                            }
                        }
                        _ => {
                            return Err(Diagnostic::new(Code::E108, format!("llvm subset: bad `{n}` shape")));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_file_read_text" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let site = cx.builder.build_call(cx.file_read, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: file read returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_file_read_text_err" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let site = cx.builder.build_call(cx.file_read_err, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: file read err returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_string_len" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let site = cx.builder.build_call(cx.string_len, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: string_len returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_string_slice" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = load(cx, args[1])?;
                    let c = load(cx, args[2])?;
                    let site = cx.builder.build_call(cx.string_slice, &[a.into(), b.into(), c.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: string_slice returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_string_index_of" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = as_ptr(cx, load(cx, args[1])?)?;
                    let site = cx.builder.build_call(cx.string_index_of, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: string_index_of returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_string_index_of_from" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = as_ptr(cx, load(cx, args[1])?)?;
                    let c = load(cx, args[2])?;
                    let site = cx.builder.build_call(cx.string_index_of_from, &[a.into(), b.into(), c.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: string_index_of_from returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_string_trim" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let site = cx.builder.build_call(cx.string_trim, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: string_trim returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_string_concat" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = as_ptr(cx, load(cx, args[1])?)?;
                    let site = cx.builder.build_call(cx.concat, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: string_concat returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_string_split" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = as_ptr(cx, load(cx, args[1])?)?;
                    let site = cx.builder.build_call(cx.string_split, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: string_split returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_int_to_str" || n == "__rnx_float_to_str" || n == "__rnx_bool_to_str" {
                    let v = load(cx, args[0])?;
                    let callee = if n == "__rnx_int_to_str" {
                        cx.int_to_str
                    } else if n == "__rnx_float_to_str" {
                        cx.float_to_str
                    } else {
                        cx.bool_to_str
                    };
                    let site = cx.builder.build_call(callee, &[v.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(bv) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(bv.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, format!("llvm subset: {n} returned void")));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_string_char_code_at" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.string_char_code_at, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: string_char_code_at returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_string_from_char_code" {
                    let v = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.string_from_char_code, &[v.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(bv) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(bv.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: string_from_char_code returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_array_pop" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.array_pop_fn, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                                match cx.ftypes.get(*d as usize) {
                                    Some(LirType::Str) | Some(LirType::Obj(_)) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) => {
                                        own(cx, *d);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: array_pop returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_array_len" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let site = cx.builder.build_call(cx.array_len, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: array_len returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_any_box" {
                    let a = load(cx, args[0])?;
                    let b = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.any_box, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                                if matches!(cx.ftypes.get(*d as usize), Some(LirType::Any)) {
                                    own(cx, *d);
                                }
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: any_box returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_any_unbox" || n == "__rnx_any_unbox_heap" {
                    let a = load(cx, args[0])?;
                    let callee = if n == "__rnx_any_unbox_heap" { cx.any_unbox_heap } else { cx.any_unbox };
                    let site = cx.builder.build_call(callee, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: any_unbox returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_any_retain" {
                    let a = load(cx, args[0])?;
                    cx.builder.build_call(cx.any_retain, &[a.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_any_release_box" {
                    let a = load(cx, args[0])?;
                    cx.builder.build_call(cx.any_release_box, &[a.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_any_tag" || n == "__rnx_obj_class" {
                    let f = if n == "__rnx_any_tag" { cx.any_tag } else { cx.obj_class };
                    let a = load(cx, args[0])?;
                    let site = cx.builder.build_call(f, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: tag call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_note_type" {
                    let a = load(cx, args[0])?;
                    let b = load(cx, args[1])?;
                    cx.builder.build_call(cx.note_type, &[a.into(), b.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_error_class" || n == "__rnx_error_unbox" {
                    let f = if n == "__rnx_error_class" { cx.error_class } else { cx.error_unbox };
                    let a = load(cx, args[0])?;
                    let site = cx.builder.build_call(f, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: error call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_error_str" {
                    let a = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.error_str, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: error str returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_error_release" || n == "__rnx_error_set" {
                    let f = if n == "__rnx_error_release" { cx.error_release } else { cx.error_set };
                    let a = load(cx, args[0])?;
                    cx.builder.build_call(f, &[a.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_type_name" || n == "__rnx_typeof_any" {
                    let f = if n == "__rnx_type_name" { cx.type_name } else { cx.typeof_any };
                    let a = load(cx, args[0])?;
                    let site = cx.builder.build_call(f, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: typename call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_io_pretty" {
                    let a = load(cx, args[0])?;
                    let fd = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.io_pretty, &[a.into(), fd.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: io_pretty call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_array_slice" {
                    let mut argv: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
                    for a in args {
                        argv.push(load(cx, *a)?.into());
                    }
                    let site = cx.builder.build_call(cx.array_slice, &argv, "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: array_slice returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_pool_new" {
                    let w = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.pool_new, &[w.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: pool_new returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_thread_join_val" {
                    let raw = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.join_val, &[raw.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                                if matches!(cx.ftypes.get(*d as usize), Some(LirType::Any)) {
                                    own(cx, *d);
                                }
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: join returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_task_await_val" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let site = cx.builder.build_call(cx.task_val, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                                if matches!(cx.ftypes.get(*d as usize), Some(LirType::Any)) {
                                    own(cx, *d);
                                }
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: join returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_thread_join_err" {
                    let raw = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.join_err, &[raw.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: join err returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_task_await_err" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let site = cx.builder.build_call(cx.task_err, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: join err returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_file_write_text" {
                    let raw = load(cx, args[0])?;
                    let hp = as_ptr(cx, raw)?;
                    let text = load(cx, args[1])?;
                    let tp = as_ptr(cx, text)?;
                    let site = cx.builder.build_call(cx.file_write, &[hp.into(), tp.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: file write returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_file_seek" {
                    let raw = load(cx, args[0])?;
                    let hp = as_ptr(cx, raw)?;
                    let pos = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.file_seek, &[hp.into(), pos.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: file seek returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_file_tell" {
                    let raw = load(cx, args[0])?;
                    let hp = as_ptr(cx, raw)?;
                    let site = cx.builder.build_call(cx.file_tell, &[hp.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: file tell returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_file_from_handle" {
                    let fd = load(cx, args[0])?;
                    let readable = load(cx, args[1])?;
                    let writable = load(cx, args[2])?;
                    let site = cx.builder.build_call(cx.file_from_handle, &[fd.into(), readable.into(), writable.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: file from handle returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_io_is_tty" {
                    let fd = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.io_is_tty, &[fd.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: io is tty returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_io_winsize" {
                    let site = cx.builder.build_call(cx.io_winsize, &[], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: io winsize returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_io_set_raw" {
                    let fd = load(cx, args[0])?;
                    let enabled = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.io_set_raw, &[fd.into(), enabled.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: io set raw returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "assert" {
                    let c = load(cx, args[0])?;
                    let m = load(cx, args[1])?;
                    cx.builder.build_call(cx.test_assert, &[c.into(), m.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__testCheck" {
                    let site = cx.builder.build_call(cx.test_check, &[], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: test check returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_path_exists" || n == "__rnx_path_remove" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let callee = if n == "__rnx_path_exists" { cx.path_exists } else { cx.path_remove };
                    let site = cx.builder.build_call(callee, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: path op returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_exists" || n == "__rnx_fs_is_file" || n == "__rnx_fs_is_dir" || n == "__rnx_fs_remove" || n == "__rnx_fs_remove_all" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let callee = if n == "__rnx_fs_exists" {
                        cx.fs_exists
                    } else if n == "__rnx_fs_is_file" {
                        cx.fs_is_file
                    } else if n == "__rnx_fs_is_dir" {
                        cx.fs_is_dir
                    } else if n == "__rnx_fs_remove" {
                        cx.fs_remove
                    } else {
                        cx.fs_remove_all
                    };
                    let site = cx.builder.build_call(callee, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs probe returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_stat"
                    || n == "__rnx_fs_stat_err"
                    || n == "__rnx_fs_read_dir"
                    || n == "__rnx_fs_read_dir_err"
                    || n == "__rnx_fs_glob"
                    || n == "__rnx_fs_glob_err"
                    || n == "__rnx_fs_read_link"
                    || n == "__rnx_fs_read_link_err"
                    || n == "__rnx_fs_remove_err"
                    || n == "__rnx_fs_remove_all_err"
                    || n == "__rnx_fs_fsync_err"
                    || n == "__rnx_fs_read_text"
                    || n == "__rnx_fs_read_text_err"
                    || n == "__rnx_fs_read_bytes_err"
                {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let callee = if n == "__rnx_fs_stat" {
                        cx.fs_stat
                    } else if n == "__rnx_fs_stat_err" {
                        cx.fs_stat_err
                    } else if n == "__rnx_fs_read_dir" {
                        cx.fs_read_dir
                    } else if n == "__rnx_fs_read_dir_err" {
                        cx.fs_read_dir_err
                    } else if n == "__rnx_fs_glob" {
                        cx.fs_glob
                    } else if n == "__rnx_fs_glob_err" {
                        cx.fs_glob_err
                    } else if n == "__rnx_fs_read_link" {
                        cx.fs_read_link
                    } else if n == "__rnx_fs_read_link_err" {
                        cx.fs_read_link_err
                    } else if n == "__rnx_fs_remove_err" {
                        cx.fs_remove_err
                    } else if n == "__rnx_fs_remove_all_err" {
                        cx.fs_remove_all_err
                    } else if n == "__rnx_fs_fsync_err" {
                        cx.fs_fsync_err
                    } else if n == "__rnx_fs_read_text" {
                        cx.fs_read_text
                    } else if n == "__rnx_fs_read_text_err" {
                        cx.fs_read_text_err
                    } else {
                        cx.fs_read_bytes_err
                    };
                    let site = cx.builder.build_call(callee, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_move_err" || n == "__rnx_fs_rename_err" || n == "__rnx_fs_symlink_err" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = as_ptr(cx, load(cx, args[1])?)?;
                    let callee = if n == "__rnx_fs_move_err" {
                        cx.fs_move_err
                    } else if n == "__rnx_fs_rename_err" {
                        cx.fs_rename_err
                    } else {
                        cx.fs_symlink_err
                    };
                    let site = cx.builder.build_call(callee, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs move call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_mkdir_err" || n == "__rnx_fs_truncate_err" || n == "__rnx_fs_chmod_err" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = load(cx, args[1])?;
                    let callee = if n == "__rnx_fs_mkdir_err" {
                        cx.fs_mkdir_err
                    } else if n == "__rnx_fs_truncate_err" {
                        cx.fs_truncate_err
                    } else {
                        cx.fs_chmod_err
                    };
                    let site = cx.builder.build_call(callee, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs sized call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_copy_err" || n == "__rnx_fs_write_text_err" || n == "__rnx_fs_write_bytes_err" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = as_ptr(cx, load(cx, args[1])?)?;
                    let c = load(cx, args[2])?;
                    let callee = if n == "__rnx_fs_copy_err" {
                        cx.fs_copy_err
                    } else if n == "__rnx_fs_write_text_err" {
                        cx.fs_write_text_err
                    } else {
                        cx.fs_write_bytes_err
                    };
                    let site = cx.builder.build_call(callee, &[a.into(), b.into(), c.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs triple call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_write_text" || n == "__rnx_fs_write_bytes" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = as_ptr(cx, load(cx, args[1])?)?;
                    let c = load(cx, args[2])?;
                    let callee = if n == "__rnx_fs_write_text" { cx.fs_write_text } else { cx.fs_write_bytes };
                    let site = cx.builder.build_call(callee, &[a.into(), b.into(), c.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs write returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_read_bytes" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let site = cx.builder.build_call(cx.fs_read_bytes, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs read bytes returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_mmap" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.fs_mmap, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs mmap returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_mmap_err" {
                    let a = as_ptr(cx, load(cx, args[0])?)?;
                    let b = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.fs_mmap_err, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs mmap err returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_mmap_anon" {
                    let a = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.fs_mmap_anon, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs mmap anon returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_mmap_anon_err" {
                    let a = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.fs_mmap_anon_err, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs mmap anon err returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_mmap_addr" || n == "__rnx_fs_mmap_len" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let callee = if n == "__rnx_fs_mmap_addr" { cx.fs_mmap_addr } else { cx.fs_mmap_len };
                    let site = cx.builder.build_call(callee, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: fs mmap probe returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_fs_mmap_flush" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    cx.builder.build_call(cx.fs_mmap_flush, &[ptr.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_fs_mmap_close" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    cx.builder.build_call(cx.fs_mmap_close, &[ptr.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_env_args_count" || n == "__rnx_env_cwd" || n == "__rnx_host_version" {
                    let callee = if n == "__rnx_env_args_count" { cx.env_count } else if n == "__rnx_env_cwd" { cx.env_cwd } else { cx.host_version };
                    let site = cx.builder.build_call(callee, &[], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                if n == "__rnx_env_cwd" || n == "__rnx_host_version" {
                                    let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                    store(cx, *d, h.into())?;
                                    own(cx, *d);
                                } else {
                                    store(cx, *d, v)?;
                                }
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: env call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_env_args_get" || n == "__rnx_env_get" {
                    let callee = if n == "__rnx_env_args_get" { cx.env_args_get } else { cx.env_get };
                    let argv: Vec<inkwell::values::BasicMetadataValueEnum> = if n == "__rnx_env_args_get" {
                        let i = load(cx, args[0])?;
                        vec![i.into()]
                    } else {
                        let raw = load(cx, args[0])?;
                        vec![as_ptr(cx, raw)?.into()]
                    };
                    let site = cx.builder.build_call(callee, &argv, "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: env call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_env_set" {
                    let ka = load(cx, args[0])?;
                    let va = load(cx, args[1])?;
                    let kp = as_ptr(cx, ka)?;
                    let vp = as_ptr(cx, va)?;
                    cx.builder.build_call(cx.env_set, &[kp.into(), vp.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_net_connect_start" || n == "__rnx_net_listener_bind" {
                    let hp = as_ptr(cx, load(cx, args[0])?)?;
                    let port = load(cx, args[1])?;
                    let callee = if n == "__rnx_net_connect_start" { cx.net_connect } else { cx.net_listener_bind };
                    let site = cx.builder.build_call(callee, &[hp.into(), port.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: net call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_tls_connect_start" {
                    let tcp = load(cx, args[0])?;
                    let dp = as_ptr(cx, load(cx, args[1])?)?;
                    let site = cx.builder.build_call(cx.tls_connect_start, &[tcp.into(), dp.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: tls call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_net_take_error" || n == "__rnx_net_connect_wait" || n == "__rnx_net_close" || n == "__rnx_net_listener_port" || n == "__rnx_net_listener_accept_start" || n == "__rnx_net_listener_accept_wait" || n == "__rnx_net_listener_close" || n == "__rnx_tls_handshake_start" || n == "__rnx_tls_handshake_wait" || n == "__rnx_tls_close" {
                    let callee = if n == "__rnx_net_take_error" { cx.net_take_error } else if n == "__rnx_net_connect_wait" { cx.net_connect_wait } else if n == "__rnx_net_listener_port" { cx.net_listener_port } else if n == "__rnx_net_listener_accept_start" { cx.net_listener_accept_start } else if n == "__rnx_net_listener_accept_wait" { cx.net_listener_accept_wait } else if n == "__rnx_net_listener_close" { cx.net_listener_close } else if n == "__rnx_tls_handshake_start" { cx.tls_handshake_start } else if n == "__rnx_tls_handshake_wait" { cx.tls_handshake_wait } else if n == "__rnx_tls_close" { cx.tls_close } else { cx.net_close };
                    let a = load(cx, args[0])?;
                    let site = cx.builder.build_call(callee, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: net call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_net_recv_or_wait" || n == "__rnx_net_recv_get" || n == "__rnx_net_send_or_wait" || n == "__rnx_tls_recv_or_wait" || n == "__rnx_tls_recv_get" || n == "__rnx_tls_send_or_wait" {
                    let callee = if n == "__rnx_net_recv_or_wait" { cx.net_recv_wait } else if n == "__rnx_net_recv_get" { cx.net_recv_get } else if n == "__rnx_tls_recv_or_wait" { cx.tls_recv_wait } else if n == "__rnx_tls_recv_get" { cx.tls_recv_get } else if n == "__rnx_tls_send_or_wait" { cx.tls_send_wait } else { cx.net_send_wait };
                    let a = load(cx, args[0])?;
                    let b = load(cx, args[1])?;
                    let site = cx.builder.build_call(callee, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: net call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_net_error_text" || n == "__rnx_tls_error_text" {
                    let a = load(cx, args[0])?;
                    let callee = if n == "__rnx_net_error_text" { cx.net_error_text } else { cx.tls_error_text };
                    let site = cx.builder.build_call(callee, &[a.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: net call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_env_exit" {
                    let code = load(cx, args[0])?;
                    cx.builder.build_call(cx.env_exit, &[code.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_process_pid"
                    || n == "__rnx_process_all_env_count"
                    || n == "__rnx_os_cpu_count"
                    || n == "__rnx_os_uptime"
                {
                    let callee = match n.as_str() {
                        "__rnx_process_pid" => cx.proc_pid,
                        "__rnx_process_all_env_count" => cx.proc_all_env_count,
                        "__rnx_os_cpu_count" => cx.os_cpu_count,
                        _ => cx.os_uptime,
                    };
                    let site = cx.builder.build_call(callee, &[], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: process call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_os_platform"
                    || n == "__rnx_os_arch"
                    || n == "__rnx_os_hostname"
                    || n == "__rnx_os_tmpdir"
                    || n == "__rnx_os_homedir"
                {
                    let callee = match n.as_str() {
                        "__rnx_os_platform" => cx.os_platform,
                        "__rnx_os_arch" => cx.os_arch,
                        "__rnx_os_hostname" => cx.os_hostname,
                        "__rnx_os_tmpdir" => cx.os_tmpdir,
                        _ => cx.os_homedir,
                    };
                    let site = cx.builder.build_call(callee, &[], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: os call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_process_remove_env"
                    || n == "__rnx_process_close_stdin"
                    || n == "__rnx_process_forget"
                {
                    let callee = match n.as_str() {
                        "__rnx_process_remove_env" => cx.proc_remove_env,
                        "__rnx_process_close_stdin" => cx.proc_close_stdin,
                        _ => cx.proc_forget,
                    };
                    if n == "__rnx_process_remove_env" {
                        let raw = load(cx, args[0])?;
                        let p = as_ptr(cx, raw)?;
                        cx.builder.build_call(callee, &[p.into()], "").map_err(err)?;
                    } else {
                        let h = load(cx, args[0])?;
                        cx.builder.build_call(callee, &[h.into()], "").map_err(err)?;
                    }
                    return Ok(());
                }
                if n == "__rnx_process_all_env_get" || n == "__rnx_process_chdir" {
                    let callee = if n == "__rnx_process_all_env_get" { cx.proc_all_env_get } else { cx.proc_chdir };
                    let raw = load(cx, args[0])?;
                    let argv: Vec<inkwell::values::BasicMetadataValueEnum> = if n == "__rnx_process_all_env_get" {
                        vec![raw.into()]
                    } else {
                        vec![as_ptr(cx, raw)?.into()]
                    };
                    let site = cx.builder.build_call(callee, &argv, "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                if n == "__rnx_process_all_env_get" {
                                    let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                    store(cx, *d, h.into())?;
                                    own(cx, *d);
                                } else {
                                    store(cx, *d, v)?;
                                }
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: process call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_process_pid_of"
                    || n == "__rnx_process_wait"
                    || n == "__rnx_process_try_wait"
                    || n == "__rnx_process_exit_code"
                    || n == "__rnx_process_take_stdout"
                    || n == "__rnx_process_take_stderr"
                {
                    let callee = match n.as_str() {
                        "__rnx_process_pid_of" => cx.proc_pid_of,
                        "__rnx_process_wait" => cx.proc_wait,
                        "__rnx_process_try_wait" => cx.proc_try_wait,
                        "__rnx_process_exit_code" => cx.proc_exit_code,
                        "__rnx_process_take_stdout" => cx.proc_take_stdout,
                        _ => cx.proc_take_stderr,
                    };
                    let h = load(cx, args[0])?;
                    let site = cx.builder.build_call(callee, &[h.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: process call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_process_kill" {
                    let a = load(cx, args[0])?;
                    let b = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.proc_kill, &[a.into(), b.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: process call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_process_write_stdin"
                    || n == "__rnx_process_read_stdout"
                    || n == "__rnx_process_read_stderr"
                {
                    let callee = match n.as_str() {
                        "__rnx_process_write_stdin" => cx.proc_write_stdin,
                        "__rnx_process_read_stdout" => cx.proc_read_stdout,
                        _ => cx.proc_read_stderr,
                    };
                    let h = load(cx, args[0])?;
                    let raw = load(cx, args[1])?;
                    let bp = as_ptr(cx, raw)?;
                    let off = load(cx, args[2])?;
                    let len = load(cx, args[3])?;
                    let site = cx.builder.build_call(callee, &[h.into(), bp.into(), off.into(), len.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: process call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_process_spawn" || n == "__rnx_process_run" {
                    let callee = if n == "__rnx_process_spawn" { cx.proc_spawn } else { cx.proc_run };
                    let cmd = as_ptr(cx, load(cx, args[0])?)?;
                    let argsp = as_ptr(cx, load(cx, args[1])?)?;
                    let cwd = as_ptr(cx, load(cx, args[2])?)?;
                    let envp = as_ptr(cx, load(cx, args[3])?)?;
                    let si = load(cx, args[4])?;
                    let so = load(cx, args[5])?;
                    let se = load(cx, args[6])?;
                    let site = cx.builder.build_call(callee, &[cmd.into(), argsp.into(), cwd.into(), envp.into(), si.into(), so.into(), se.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: process call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_black_box"
                    || n == "__rnx_sync_atomic_get"
                    || n == "__rnx_sync_channel_recv"
                    || n == "__rnx_sync_channel_try_recv"
                    || n == "__rnx_sync_channel_len"
                    || n == "__rnx_fs_pool_depth"
                {
                    let callee = match n.as_str() {
                        "__rnx_black_box" => cx.black_box,
                        "__rnx_sync_atomic_get" => cx.sync_atomic_get,
                        "__rnx_sync_channel_recv" => cx.sync_channel_recv,
                        "__rnx_sync_channel_try_recv" => cx.sync_channel_try_recv,
                        "__rnx_fs_pool_depth" => cx.fs_pool_depth,
                        _ => cx.sync_channel_len,
                    };
                    let i = load(cx, args[0])?;
                    let site = cx.builder.build_call(callee, &[i.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                                if n.as_str() == "__rnx_sync_channel_recv"
                                    && matches!(cx.ftypes.get(*d as usize), Some(LirType::Any))
                                {
                                    own(cx, *d);
                                }
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: sync call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_sync_atomic_fetch_add" {
                    let id = load(cx, args[0])?;
                    let delta = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.sync_atomic_fetch_add, &[id.into(), delta.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: sync call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_sync_atomic_set" || n == "__rnx_sync_channel_send" {
                    let id = load(cx, args[0])?;
                    let val = load(cx, args[1])?;
                    let callee = if n == "__rnx_sync_atomic_set" {
                        cx.sync_atomic_set
                    } else {
                        cx.sync_channel_send
                    };
                    cx.builder.build_call(callee, &[id.into(), val.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_sync_channel_send_str" {
                    let id = load(cx, args[0])?;
                    let val = load(cx, args[1])?;
                    let vp = as_ptr(cx, val)?;
                    cx.builder
                        .build_call(cx.sync_channel_send_str, &[id.into(), vp.into()], "")
                        .map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_sync_channel_drop" {
                    let id = load(cx, args[0])?;
                    cx.builder.build_call(cx.sync_channel_drop, &[id.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_sync_channel_send_obj" {
                    if args.len() != 2 {
                        return Err(Diagnostic::new(
                            Code::E108,
                            "llvm subset: channel obj send takes 2 args",
                        ));
                    }
                    let size = obj_size(lir, cx, args[1])?;
                    let id = load(cx, args[0])?;
                    let ptr = load(cx, args[1])?;
                    let pp = as_ptr(cx, ptr)?;
                    let n = cx.context.i64_type().const_int(size, false);
                    let dtor = dtor_ptr(cx, lir, args[1])?;
                    cx.builder
                        .build_call(cx.sync_channel_send_obj, &[id.into(), pp.into(), n.into(), dtor.into()], "")
                        .map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_sync_channel_send_array" {
                    if args.len() != 2 {
                        return Err(Diagnostic::new(
                            Code::E108,
                            "llvm subset: channel array send takes 2 args",
                        ));
                    }
                    let inner = match cx.ftypes.get(args[1] as usize).cloned() {
                        Some(LirType::Array(inner)) => *inner,
                        _ => {
                            return Err(Diagnostic::new(
                                Code::E108,
                                "llvm subset: channel array send of non-array",
                            ));
                        }
                    };
                    let id = load(cx, args[0])?;
                    let ptr = load(cx, args[1])?;
                    let pp = as_ptr(cx, ptr)?;
                    let n = cx.context.i64_type().const_int(lir::instr::elem_size(&inner) as u64, false);
                    let dtor = array_elem_dtor(cx, lir, &inner)?;
                    cx.builder
                        .build_call(cx.sync_channel_send_array, &[id.into(), pp.into(), n.into(), dtor.into()], "")
                        .map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_debug_live_count" {
                    let site = cx.builder.build_call(cx.debug_live, &[], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(
                                Code::E108,
                                "llvm subset: debug call returned void",
                            ));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_mutex_lock"
                    || n == "__rnx_mutex_unlock"
                    || n == "__rnx_rwlock_read_lock"
                    || n == "__rnx_rwlock_read_unlock"
                    || n == "__rnx_rwlock_write_lock"
                    || n == "__rnx_rwlock_write_unlock"
                {
                    let callee = match n.as_str() {
                        "__rnx_mutex_lock" => cx.mutex_lock,
                        "__rnx_mutex_unlock" => cx.mutex_unlock,
                        "__rnx_rwlock_read_lock" => cx.rwlock_read_lock,
                        "__rnx_rwlock_read_unlock" => cx.rwlock_read_unlock,
                        "__rnx_rwlock_write_lock" => cx.rwlock_write_lock,
                        _ => cx.rwlock_write_unlock,
                    };
                    let id = load(cx, args[0])?;
                    cx.builder.build_call(callee, &[id.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_mutex_try_lock"
                    || n == "__rnx_rwlock_try_read_lock"
                    || n == "__rnx_rwlock_try_write_lock"
                {
                    let callee = match n.as_str() {
                        "__rnx_mutex_try_lock" => cx.mutex_try_lock,
                        "__rnx_rwlock_try_read_lock" => cx.rwlock_try_read_lock,
                        _ => cx.rwlock_try_write_lock,
                    };
                    let id = load(cx, args[0])?;
                    let site = cx.builder.build_call(callee, &[id.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: lock call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_condvar_wait"
                    || n == "__rnx_condvar_notify_one"
                    || n == "__rnx_condvar_notify_all"
                {
                    let callee = match n.as_str() {
                        "__rnx_condvar_wait" => cx.condvar_wait,
                        "__rnx_condvar_notify_one" => cx.condvar_notify_one,
                        _ => cx.condvar_notify_all,
                    };
                    let argv: Vec<_> = args
                        .iter()
                        .map(|a| load(cx, *a).map(|v| v.into()))
                        .collect::<Result<Vec<_>, _>>()?;
                    cx.builder.build_call(callee, &argv, "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_barrier_wait" {
                    let id = load(cx, args[0])?;
                    let th = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.barrier_wait, &[id.into(), th.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: lock call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_condvar_wait_timeout" {
                    let cv = load(cx, args[0])?;
                    let m = load(cx, args[1])?;
                    let ms = load(cx, args[2])?;
                    let site = cx.builder.build_call(cx.condvar_wait_timeout, &[cv.into(), m.into(), ms.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: lock call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_sync_atomic_cas" {
                    let id = load(cx, args[0])?;
                    let exp = load(cx, args[1])?;
                    let nxt = load(cx, args[2])?;
                    let site = cx.builder.build_call(cx.sync_atomic_cas, &[id.into(), exp.into(), nxt.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: sync cas returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_map_new" {
                    let site = cx.builder.build_call(cx.map_new, &[], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: map new returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_map_set" || n == "__rnx_map_clear" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    if n == "__rnx_map_set" {
                        let key = load(cx, args[1])?;
                        let kp = as_ptr(cx, key)?;
                        let val = load(cx, args[2])?;
                        cx.builder.build_call(cx.map_set, &[ptr.into(), kp.into(), val.into()], "").map_err(err)?;
                    } else {
                        cx.builder.build_call(cx.map_clear, &[ptr.into()], "").map_err(err)?;
                    }
                    return Ok(());
                }
                if n == "__rnx_map_get" || n == "__rnx_map_len" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let site = if n == "__rnx_map_get" {
                        let key = load(cx, args[1])?;
                        let kp = as_ptr(cx, key)?;
                        cx.builder.build_call(cx.map_get, &[ptr.into(), kp.into()], "").map_err(err)?
                    } else {
                        cx.builder.build_call(cx.map_len, &[ptr.into()], "").map_err(err)?
                    };
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: map call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_map_has" || n == "__rnx_map_delete" {
                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let key = load(cx, args[1])?;
                    let kp = as_ptr(cx, key)?;
                    let callee = if n == "__rnx_map_has" { cx.map_has } else { cx.map_delete };
                    let site = cx.builder.build_call(callee, &[ptr.into(), kp.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: map bool returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_map_keys" || n == "__rnx_map_values" {                    let raw = load(cx, args[0])?;
                    let ptr = as_ptr(cx, raw)?;
                    let callee = if n == "__rnx_map_keys" { cx.map_keys } else { cx.map_values };
                    let site = cx.builder.build_call(callee, &[ptr.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: map array returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_gmap_new" {
                    if !args.is_empty() {
                        return Err(Diagnostic::new(Code::E108, "`__rnx_gmap_new` takes no args"));
                    }
                    let site = cx.builder.build_call(cx.gmap_new, &[], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: gmap new returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_gmap_free" || n == "__rnx_gmap_clear" {
                    let h = load(cx, args[0])?;
                    let callee = if n == "__rnx_gmap_free" { cx.gmap_free } else { cx.gmap_clear };
                    cx.builder.build_call(callee, &[h.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_gmap_set" {
                    let h = load(cx, args[0])?;
                    let k = load(cx, args[1])?;
                    let v = load(cx, args[2])?;
                    cx.builder.build_call(cx.gmap_set, &[h.into(), k.into(), v.into()], "").map_err(err)?;
                    return Ok(());
                }
                if n == "__rnx_gmap_get" {
                    let h = load(cx, args[0])?;
                    let k = load(cx, args[1])?;
                    let site = cx.builder.build_call(cx.gmap_get, &[h.into(), k.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: gmap get returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_gmap_has" || n == "__rnx_gmap_delete" {
                    let h = load(cx, args[0])?;
                    let k = load(cx, args[1])?;
                    let callee = if n == "__rnx_gmap_has" { cx.gmap_has } else { cx.gmap_delete };
                    let site = cx.builder.build_call(callee, &[h.into(), k.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            let w = cx.builder.build_int_z_extend(v.into_int_value(), cx.context.i64_type(), "").map_err(err)?;
                            if let Some(d) = dst {
                                store(cx, *d, w.into())?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: gmap bool returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_gmap_len" {
                    let h = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.gmap_len, &[h.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: gmap len returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_gmap_keys" || n == "__rnx_gmap_values" {
                    let h = load(cx, args[0])?;
                    let callee = if n == "__rnx_gmap_keys" { cx.gmap_keys } else { cx.gmap_values };
                    let site = cx.builder.build_call(callee, &[h.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(v.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: gmap array returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_json_parse" {
                    let t = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.json_parse, &[t.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: json parse returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_json_parse_typed" {
                    let t = load(cx, args[0])?;
                    let d = load(cx, args[1])?;
                    let site =
                        cx.builder.build_call(cx.json_parse_typed, &[t.into(), d.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(p) => {
                            if let Some(dst) = dst {
                                let h = cx.builder.build_ptr_to_int(p.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *dst, h.into())?;
                                own(cx, *dst);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: json parse typed returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_json_stringify_into" {
                    let v = load(cx, args[0])?;
                    let b = load(cx, args[1])?;
                    let p = load(cx, args[2])?;
                    let site = cx.builder.build_call(
                        cx.json_stringify_into,
                        &[v.into(), b.into(), p.into()],
                        "",
                    ).map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(w) => {
                            if let Some(d) = dst {
                                store(cx, *d, w)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: json stringify_into returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_json_stringify" {
                    let v = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.json_stringify, &[v.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(p) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(p.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: json stringify returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_json_unwrap" {
                    let v = load(cx, args[0])?;
                    let site = cx.builder.build_call(cx.json_unwrap, &[v.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(r) => {
                            if let Some(d) = dst {
                                store(cx, *d, r)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: json unwrap returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_dns_lookup_start" || n == "__rnx_dns_lookup_wait" {
                    let v = load(cx, args[0])?;
                    let callee = if n == "__rnx_dns_lookup_start" {
                        cx.dns_lookup_start
                    } else {
                        cx.dns_lookup_wait
                    };
                    let site = cx.builder.build_call(callee, &[v.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(r) => {
                            if let Some(d) = dst {
                                store(cx, *d, r)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: dns lookup returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_dns_lookup_get" || n == "__rnx_dns_lookup_error" {
                    let v = load(cx, args[0])?;
                    let callee = if n == "__rnx_dns_lookup_get" {
                        cx.dns_lookup_get
                    } else {
                        cx.dns_lookup_error
                    };
                    let site = cx.builder.build_call(callee, &[v.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(p) => {
                            if let Some(d) = dst {
                                let h = cx.builder.build_ptr_to_int(p.into_pointer_value(), cx.context.i64_type(), "").map_err(err)?;
                                store(cx, *d, h.into())?;
                                own(cx, *d);
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: dns text returned void"));
                        }
                    }
                    return Ok(());
                }
                if n.starts_with("__rnx_math_") {
                    if n.as_str() == "__rnx_math_sqrt" && args.len() == 1 {
                        if let Some(bits) = try_sqrt_intrinsic(cx, args[0])? {
                            if let Some(d) = dst {
                                store(cx, *d, bits.into())?;
                            }
                            return Ok(());
                        }
                    }
                    let callee = match n.as_str() {
                        "__rnx_math_sqrt" => cx.math_sqrt,
                        "__rnx_math_sin" => cx.math_sin,
                        "__rnx_math_cos" => cx.math_cos,
                        "__rnx_math_tan" => cx.math_tan,
                        "__rnx_math_atan2" => cx.math_atan2,
                        "__rnx_math_pow" => cx.math_pow,
                        "__rnx_math_floor" => cx.math_floor,
                        "__rnx_math_ceil" => cx.math_ceil,
                        "__rnx_math_round" => cx.math_round,
                        "__rnx_math_log" => cx.math_log,
                        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown math builtin")),
                    };
                    let argv: Vec<inkwell::values::BasicMetadataValueEnum> = args
                        .iter()
                        .map(|a| load(cx, *a).map(|v| v.into()))
                        .collect::<Result<Vec<_>, _>>()?;
                    let site = cx.builder.build_call(callee, &argv, "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: math call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n.starts_with("__rnx_float_") {
                    let sym = format!("rnx_{}", &n[6..]);
                    let callee = cx.bytes_fns.get(&sym).copied().ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("llvm subset: missing runtime `{sym}`"))
                    })?;
                    let argv: Vec<inkwell::values::BasicMetadataValueEnum> = args
                        .iter()
                        .map(|a| load(cx, *a).map(|v| v.into()))
                        .collect::<Result<Vec<_>, _>>()?;
                    let site = cx.builder.build_call(callee, &argv, "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: float call returned void"));
                        }
                    }
                    return Ok(());
                }
                if n == "__rnx_clock_mono" {
                    let site = cx.builder.build_call(cx.clock_mono, &[], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(v) => {
                            if let Some(d) = dst {
                                store(cx, *d, v)?;
                            }
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: clock returned void"));
                        }
                    }
                    return Ok(());
                }
                return Err(Diagnostic::new(Code::E108, "llvm subset: indirect call").with_span(*span));
            }
            if let CallTarget::Value(v) = target {
                let i64t = cx.context.i64_type();
                let fv = cx
                    .builder
                    .get_insert_block()
                    .and_then(|b| b.get_parent())
                    .ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: no insert function"))?;
                let bv = load(cx, *v)?;
                let fidp = byte_ptr(cx, bv, 8)?;
                let fid: inkwell::values::IntValue = cx.builder.build_load(i64t, fidp, "fid").map_err(err)?.into_int_value();
                let mut argvals: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::with_capacity(args.len());
                for a in args.iter() {
                    let loaded = load_value(cx, *a).map(|x| x.into())?;
                    if matches!(cx.ftypes.get(*a as usize), Some(LirType::Any)) {
                        if let inkwell::values::BasicMetadataValueEnum::IntValue(iv) = loaded {
                            any_retain_val(cx, iv)?;
                        }
                    }
                    argvals.push(loaded);
                }
                let mut cands: Vec<(usize, inkwell::values::FunctionValue, usize)> = Vec::new();
                for (i, f) in lir.functions.iter().enumerate() {
                    if !f.is_closure {
                        continue;
                    }
                    let callee = cx.funcs.get(&f.name).copied().ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("llvm subset: unknown closure `{}`", f.name))
                    })?;
                    cands.push((i, callee, f.params.len()));
                }
                let slot: Option<inkwell::values::PointerValue> = match dst {
                    Some(d) => {
                        let ty = local_alloca_ty(cx.context, &cx.ftypes.get(*d as usize).cloned().unwrap_or(LirType::Any));
                        Some(cx.builder.build_alloca(ty, "clo").map_err(err)?)
                    }
                    None => None,
                };
                let join = cx.context.append_basic_block(fv, "clo_join");
                if cands.is_empty() {
                    cx.builder.build_unconditional_branch(join).map_err(err)?;
                }
                let mut nexts = Vec::with_capacity(cands.len().saturating_sub(1));
                for _ in 1..cands.len() {
                    nexts.push(cx.context.append_basic_block(fv, "clo_next"));
                }
                for (i, (ci, callee, arity)) in cands.iter().enumerate() {
                    if i > 0 {
                        cx.builder.position_at_end(nexts[i - 1]);
                    }
                    let cb = cx.context.append_basic_block(fv, "clo_cb");
                    let else_bb = if i + 1 < cands.len() { nexts[i] } else { join };
                    let want = i64t.const_int(*ci as u64, false);
                    let mask = i64t.const_int(runtime::native::CLOSURE_FID_MASK, false);
                    let fid_lo = cx.builder.build_and(fid, mask, "fidlo").map_err(err)?;
                    let t = cx.builder.build_int_compare(inkwell::IntPredicate::EQ, fid_lo, want, "clo_eq").map_err(err)?;
                    cx.builder.build_conditional_branch(t, cb, else_bb).map_err(err)?;
                    cx.builder.position_at_end(cb);
                    let take = args.len().min(*arity);
                    let mut callargs: Vec<inkwell::values::BasicMetadataValueEnum> =
                        argvals[..take].to_vec();
                    for j in 0..(*arity).saturating_sub(take) {
                        let at = byte_ptr(cx, bv, 32 + j * 16)?;
                        let cv: inkwell::values::IntValue = cx.builder.build_load(i64t, at, "cap").map_err(err)?.into_int_value();
                        callargs.push(cv.into());
                    }
                    let site = cx.builder.build_call(*callee, &callargs, "clo_call").map_err(err)?;
                    if let Some(s) = slot {
                        match site.try_as_basic_value() {
                            ValueKind::Basic(r) => {
                                cx.builder.build_store(s, r).map_err(err)?;
                            }
                            ValueKind::Instruction(_) => {
                                return Err(Diagnostic::new(Code::E108, "llvm subset: closure returned void"));
                            }
                        }
                    }
                    cx.builder.build_unconditional_branch(join).map_err(err)?;
                }
                cx.builder.position_at_end(join);
                if let Some(d) = dst {
                    let ty = local_alloca_ty(cx.context, &cx.ftypes.get(*d as usize).cloned().unwrap_or(LirType::Any));
                    let r: inkwell::values::BasicValueEnum = cx.builder.build_load(ty, slot.unwrap(), "clo_res").map_err(err)?;
                    store(cx, *d, r)?;
                    if matches!(cx.ftypes.get(*d as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Any)) {
                        own(cx, *d);
                    }
                }
                let _ = (fname, args, dsts, call_err);
                return Ok(());
            }
            let id = match target {
                CallTarget::Fn(id) => *id,
                CallTarget::Foreign { symbol, .. } => {
                    let fdecl = lir
                        .foreign
                        .iter()
                        .find(|f| &f.symbol == symbol)
                        .ok_or_else(|| {
                            Diagnostic::new(
                                Code::E108,
                                format!("llvm subset: unknown foreign `{symbol}`"),
                            )
                        })?;
                    let callee = cx.funcs.get(symbol).copied().ok_or_else(|| {
                        Diagnostic::new(
                            Code::E108,
                            format!("llvm subset: undeclared foreign `{symbol}`"),
                        )
                    })?;
                    let mut argv: Vec<BasicMetadataValueEnum> =
                        Vec::with_capacity(args.len());
                    for (a, want) in args.iter().zip(fdecl.params.iter()) {
                        let v = load(cx, *a)?;
                        let cv: BasicMetadataValueEnum = match want {
                            LirType::I8 | LirType::Bool => cx
                                .builder
                                .build_int_truncate(v, cx.context.i8_type(), "")
                                .map_err(err)?
                                .into(),
                            LirType::F64(_) => cx
                                .builder
                                .build_bit_cast(v, cx.context.f64_type(), "")
                                .map_err(err)?
                                .into(),
                            LirType::Pointer(_) => as_ptr(cx, v)?.into(),
                            _ => v.into(),
                        };
                        argv.push(cv);
                    }
                    let site = cx.builder.build_call(callee, &argv, "").map_err(err)?;
                    let ret = match &fdecl.ret {
                        LirType::Void => None,
                        LirType::I8 | LirType::Bool => match site.try_as_basic_value() {
                            ValueKind::Basic(v) => Some(
                                cx.builder
                                    .build_int_z_extend(
                                        v.into_int_value(),
                                        cx.context.i64_type(),
                                        "",
                                    )
                                    .map_err(err)?
                                    .into(),
                            ),
                            ValueKind::Instruction(_) => {
                                return Err(Diagnostic::new(
                                    Code::E108,
                                    "llvm subset: foreign returned void",
                                ));
                            }
                        },
                        LirType::F64(_) => match site.try_as_basic_value() {
                            ValueKind::Basic(v) => Some(
                                cx.builder
                                    .build_bit_cast(v, cx.context.i64_type(), "")
                                    .map_err(err)?,
                            ),
                            ValueKind::Instruction(_) => {
                                return Err(Diagnostic::new(
                                    Code::E108,
                                    "llvm subset: foreign returned void",
                                ));
                            }
                        },
                        LirType::Pointer(_) => match site.try_as_basic_value() {
                            ValueKind::Basic(v) => Some(
                                cx.builder
                                    .build_ptr_to_int(
                                        v.into_pointer_value(),
                                        cx.context.i64_type(),
                                        "",
                                    )
                                    .map_err(err)?
                                    .into(),
                            ),
                            ValueKind::Instruction(_) => {
                                return Err(Diagnostic::new(
                                    Code::E108,
                                    "llvm subset: foreign returned void",
                                ));
                            }
                        },
                        _ => match site.try_as_basic_value() {
                            ValueKind::Basic(v) => Some(v),
                            ValueKind::Instruction(_) => {
                                return Err(Diagnostic::new(
                                    Code::E108,
                                    "llvm subset: foreign returned void",
                                ));
                            }
                        },
                    };
                    if let (Some(d), Some(r)) = (dst, ret) {
                        store(cx, *d, r)?;
                    }
                    return Ok(());
                }
                _ => {
                    return match target {
                        CallTarget::Dyn { method, .. } => Err(Diagnostic::new(
                            Code::E108,
                            format!("method `{method}` on a value of unknown type is not supported in native builds"),
                        )
                        .with_span(*span)
                        .with_hint("annotate the receiver type, e.g. `let hs: Array<Thread> = []`, so the call resolves statically")),
                        _ => Err(Diagnostic::new(Code::E108, "llvm subset: indirect call").with_span(*span)),
                    }
                }
            };
            let name = &lir.functions.get(id).ok_or_else(|| {
                Diagnostic::new(Code::E108, "llvm subset: bad callee")
            })?.name;
            let callee = cx.funcs.get(name).copied().ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("llvm subset: unknown callee `{name}`"))
            })?;
            let argv: Vec<_> = args
                .iter()
                .map(|a| {
                    let v = load_value(cx, *a).map(|x| x.into())?;
                    if matches!(cx.ftypes.get(*a as usize), Some(LirType::Any)) {
                        if let inkwell::values::BasicMetadataValueEnum::IntValue(iv) = v {
                            any_retain_val(cx, iv)?;
                        }
                    }
                    Ok(v)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let site = cx.builder.build_call(callee, &argv, "call").map_err(err)?;
            let callee_ret = lir.functions.get(id).map(|f| f.ret.clone()).unwrap_or(LirType::Any);
            let rets = lir::instr::flat_sig(&callee_ret);
            if rets.len() > 1 {
                let sv = match site.try_as_basic_value() {
                    ValueKind::Basic(inkwell::values::BasicValueEnum::StructValue(sv)) => sv,
                    _ => {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: tuple call value"));
                    }
                };
                for (i, d) in dsts.iter().enumerate() {
                    let ev = cx.builder.build_extract_value(sv, i as u32, "tup").map_err(err)?;
                    store(cx, *d, ev)?;
                    if matches!(cx.ftypes.get(*d as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Any) | Some(LirType::Error)) {
                        own(cx, *d);
                    }
                }
            } else if let Some(d) = dst {
                match site.try_as_basic_value() {
                    ValueKind::Basic(v) => {
                        store(cx, *d, v)?;
                        if matches!(cx.ftypes.get(*d as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Any) | Some(LirType::Error)) {
                            own(cx, *d);
                        }
                    }
                    ValueKind::Instruction(_) => {
                        return Err(Diagnostic::new(Code::E108, "llvm subset: void call value"));
                    }
                }
            }
            release_call_args(cx, lir, Some(id), args, dsts, *call_err)?;
        }
        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unsupported instr")),
    }
    Ok(())
}
