use super::*;

impl FnLower<'_> {
    pub(super) fn lower_concat(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Concat { dst, lhs, rhs , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.concat, &mut b.func);
                let a = self.val(b, *lhs);
                let c = self.val(b, *rhs);
                let inst = b.ins().call(callee, &[a, c]);
                let v = b.inst_results(inst)[0];
                self.set(b, *dst, v);
                self.own(*dst);
        Ok(())
    }

    pub(super) fn lower_to_str(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ToStr { dst, src , ..} = ins else {
            return Err("unreachable".to_string());
        };
                match self.ftypes.get(*src as usize) {
                    Some(LirType::Any) => {
                        let callee = self.module.declare_func_in_func(self.any_to_str, &mut b.func);
                        let v = self.val(b, *src);
                        let inst = b.ins().call(callee, &[v]);
                        let r = b.inst_results(inst)[0];
                        self.set(b, *dst, r);
                        self.own(*dst);
                    }
                    Some(LirType::I64) => {
                        let callee = self.module.declare_func_in_func(self.int_to_str, &mut b.func);
                        let v = self.val(b, *src);
                        let inst = b.ins().call(callee, &[v]);
                        let r = b.inst_results(inst)[0];
                        self.set(b, *dst, r);
                        self.own(*dst);
                    }
                    Some(LirType::F64(_)) => {
                        let callee = self.module.declare_func_in_func(self.float_to_str, &mut b.func);
                        let v = self.val(b, *src);
                        let inst = b.ins().call(callee, &[v]);
                        let r = b.inst_results(inst)[0];
                        self.set(b, *dst, r);
                        self.own(*dst);
                    }
                    _ => return Err("tostr of non-scalar".to_string()),
                }
        Ok(())
    }

    pub(super) fn lower_call(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Call { dsts, target, args, err, .. } = ins else {
            return Err("unreachable".to_string());
        };
                let dst = dsts.first();
                let mut hot_slot: Option<FunctionSlotId> = None;
                let (id, release_args) = match target {
                    CallTarget::Fn(lid) => {
                        let name = &self.lir.functions[*lid].name;
                        if let Some(slots) = self.hot_slots {
                            hot_slot = slots.get(name).copied();
                        }
                        for a in args.iter() {
                            if matches!(self.ftypes.get(*a as usize), Some(LirType::Any)) {
                                let v = self.val(b, *a);
                                self.any_retain_val(b, v);
                            }
                        }
                        (
                            *self.ids.get(name).ok_or("unknown callee")?,
                            !lir::temp_sweep::callee_may_consume(self.lir, *lid),
                        )
                    }
                    CallTarget::Builtin(n) if n == "print" => {
                        for (i, a) in args.iter().enumerate() {
                            if i > 0 {
                                let space = self.str_addr(b, " ")?;
                                let payload = b.ins().iadd_imm_s(space, 32);
                                let one = b.ins().iconst(types::I64, 1);
                                let scallee = self.module.declare_func_in_func(self.print_str, &mut b.func);
                                b.ins().call(scallee, &[payload, one]);
                            }
                            let v = self.val(b, *a);
                            let t = if matches!(self.ftypes.get(*a as usize), Some(LirType::Error)) {
                                let bit = b.ins().band_imm(v, 1);
                                let is_obj = b.ins().icmp_imm_s(
                                    cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                                    bit,
                                    0,
                                );
                                let ptr_tag = b.ins().iconst(types::I32, runtime::native::TAG_PTR as i64);
                                let str_tag = b.ins().iconst(types::I32, runtime::native::TAG_STR as i64);
                                b.ins().select(is_obj, ptr_tag, str_tag)
                            } else {
                                let tag = Self::print_tag(&self.ftypes.get(*a as usize).cloned().unwrap_or(LirType::Any));
                                b.ins().iconst(types::I32, tag)
                            };
                            let callee = self.module.declare_func_in_func(self.print_val, &mut b.func);
                            b.ins().call(callee, &[v, t]);
                        }
                        let nl = self.str_addr(b, "\n")?;
                        let payload = b.ins().iadd_imm_s(nl, 32);
                        let one = b.ins().iconst(types::I64, 1);
                        let callee = self.module.declare_func_in_func(self.print_str, &mut b.func);
                        b.ins().call(callee, &[payload, one]);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_crypto_random_u64" => {
                        let callee = self.module.declare_func_in_func(self.crypto_random, &mut b.func);
                        let inst = b.ins().call(callee, &[]);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_prng_seed" => {
                        let callee = self.module.declare_func_in_func(self.prng_seed, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_prng_next" => {
                        let callee = self.module.declare_func_in_func(self.prng_next, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_file_open" => {
                        let callee = self.module.declare_func_in_func(self.file_open, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_file_close" || n == "__rnx_file_flush" => {
                        let fid = if n == "__rnx_file_close" { self.file_close } else { self.file_flush };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_file_seek" => {
                        let callee = self.module.declare_func_in_func(self.file_seek, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_file_tell" => {
                        let callee = self.module.declare_func_in_func(self.file_tell, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_file_from_handle" => {
                        let callee = self.module.declare_func_in_func(self.file_from_handle, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_io_is_tty" => {
                        let callee = self.module.declare_func_in_func(self.io_is_tty, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r8 = b.inst_results(inst)[0];
                        let r = b.ins().uextend(types::I64, r8);
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_io_winsize" => {
                        let callee = self.module.declare_func_in_func(self.io_winsize, &mut b.func);
                        let inst = b.ins().call(callee, &[]);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_io_set_raw" => {
                        let callee = self.module.declare_func_in_func(self.io_set_raw, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n.starts_with("__rnx_gmap_") =>                    {
                        let sym = format!("rnx_{}", &n[6..]);
                        let fid = *self.bytes_fns.get(&sym).ok_or_else(|| {
                            format!("cranelift subset: missing runtime `{sym}`")
                        })?;
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        if dst.is_none() {
                            b.ins().call(callee, &argv);
                            return Ok(());
                        }
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            if sym == "rnx_gmap_has" || sym == "rnx_gmap_delete" {
                                let w = b.ins().uextend(types::I64, r);
                                self.set(b, *d, w);
                            } else {
                                self.set(b, *d, r);
                                if sym == "rnx_gmap_keys" || sym == "rnx_gmap_values" {
                                    self.own(*d);
                                }
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_json_parse"
                            || n == "__rnx_json_parse_typed"
                            || n == "__rnx_json_stringify"
                            || n == "__rnx_json_stringify_into"
                            || n == "__rnx_json_unwrap" =>
                    {
                        let sym = format!("rnx_{}", &n[6..]);
                        let fid = *self.bytes_fns.get(&sym).ok_or_else(|| {
                            format!("cranelift subset: missing runtime `{sym}`")
                        })?;
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        if dst.is_none() {
                            b.ins().call(callee, &argv);
                            return Ok(());
                        }
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            if sym != "rnx_json_unwrap" {
                                self.own(*d);
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_dns_lookup_start"
                            || n == "__rnx_dns_lookup_wait"
                            || n == "__rnx_dns_lookup_get"
                            || n == "__rnx_dns_lookup_error" =>
                    {
                        let sym = format!("rnx_{}", &n[6..]);
                        let fid = *self.bytes_fns.get(&sym).ok_or_else(|| {
                            format!("cranelift subset: missing runtime `{sym}`")
                        })?;
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        if dst.is_none() {
                            b.ins().call(callee, &argv);
                            return Ok(());
                        }
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            if sym == "rnx_dns_lookup_get" || sym == "rnx_dns_lookup_error" {
                                self.own(*d);
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n.starts_with("__rnx_bytes_")
                            || n == "__rnx_file_read_bytes"
                            || n == "__rnx_file_write_bytes"
                            || n == "__rnx_fs_read_bytes"
                            || n == "__rnx_fs_write_text"
                            || n == "__rnx_fs_write_bytes" =>
                    {
                        let sym = format!("rnx_{}", &n[6..]);
                        let fid = *self.bytes_fns.get(&sym).ok_or_else(|| {
                            format!("cranelift subset: missing runtime `{sym}`")
                        })?;
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        if dst.is_none() {
                            b.ins().call(callee, &argv);
                            return Ok(());
                        }
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            if sym == "rnx_bytes_read_string" {
                                self.own(*d);
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_file_read_text" => {
                        let callee = self.module.declare_func_in_func(self.file_read, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_string_len"
                            || n == "__rnx_string_slice"
                            || n == "__rnx_string_index_of" || n == "__rnx_string_index_of_from"
                            || n == "__rnx_string_trim"
                            || n == "__rnx_string_concat"
                            || n == "__rnx_string_split"
                            || n == "__rnx_int_to_str"
                            || n == "__rnx_float_to_str"
                            || n == "__rnx_bool_to_str"
                            || n == "__rnx_string_char_code_at"
                            || n == "__rnx_string_from_char_code" =>
                    {
                        let fid = match n.as_str() {
                            "__rnx_string_len" => self.string_len,
                            "__rnx_string_slice" => self.string_slice,
                            "__rnx_string_index_of" => self.string_index_of,
                            "__rnx_string_index_of_from" => self.string_index_of_from,
                            "__rnx_string_trim" => self.string_trim,
                            "__rnx_string_concat" => self.string_concat,
                            "__rnx_string_split" => self.string_split,
                            "__rnx_string_from_char_code" => self.string_from_char_code,
                            "__rnx_int_to_str" => self.int_to_str,
                            "__rnx_float_to_str" => self.float_to_str,
                            "__rnx_bool_to_str" => self.bool_to_str,
                            _ => self.string_char_code_at,
                        };
                        let owns = matches!(
                            n.as_str(),
                            "__rnx_string_slice"
                                | "__rnx_string_trim"
                                | "__rnx_string_concat"
                                | "__rnx_string_split"
                                | "__rnx_string_from_char_code"
                                | "__rnx_int_to_str"
                                | "__rnx_float_to_str"
                                | "__rnx_bool_to_str"
                        );
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            if owns {
                                self.own(*d);
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_pool_new" =>
                    {
                        let callee = self.module.declare_func_in_func(self.pool_new, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_thread_join_val"
                            || n == "__rnx_task_await_val" =>
                    {
                        let fid = if n == "__rnx_thread_join_val" {
                            self.join_val
                        } else {
                            self.task_val
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            if matches!(self.ftypes.get(*d as usize), Some(LirType::Any)) {
                                self.own(*d);
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_thread_join_err"
                            || n == "__rnx_task_await_err" =>
                    {
                        let fid = if n == "__rnx_thread_join_err" {
                            self.join_err
                        } else {
                            self.task_err
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_array_pop" => {
                        let callee = self.module.declare_func_in_func(self.array_pop_fn, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            if matches!(
                                self.ftypes.get(*d as usize),
                                Some(LirType::Str)
                                    | Some(LirType::Obj(_))
                                    | Some(LirType::Array(_))
                                    | Some(LirType::Enum(_))
                            ) {
                                self.own(*d);
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_array_len" => {
                        let callee = self.module.declare_func_in_func(self.array_len, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_any_box" => {
                        let callee = self.module.declare_func_in_func(self.any_box, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            if matches!(self.ftypes.get(*d as usize), Some(LirType::Any)) {
                                self.own(*d);
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_any_unbox" || n == "__rnx_any_unbox_heap" => {
                        let fid = if n == "__rnx_any_unbox_heap" { self.any_unbox_heap } else { self.any_unbox };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_any_retain" => {
                        let callee = self.module.declare_func_in_func(self.any_retain, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_any_release_box" => {
                        let callee = self.module.declare_func_in_func(self.any_release_box, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_any_tag" || n == "__rnx_obj_class" =>
                    {
                        let fid = if n == "__rnx_any_tag" { self.any_tag } else { self.obj_class };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_note_type" => {
                        let callee = self.module.declare_func_in_func(self.note_type, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_error_class" || n == "__rnx_error_unbox" =>
                    {
                        let fid = if n == "__rnx_error_class" { self.error_class } else { self.error_unbox };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_error_str" => {
                        let callee = self.module.declare_func_in_func(self.error_str, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_error_release" || n == "__rnx_error_set" =>
                    {
                        let fid = if n == "__rnx_error_release" { self.error_release } else { self.error_set };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_type_name" || n == "__rnx_typeof_any" =>
                    {
                        let fid = if n == "__rnx_type_name" { self.type_name } else { self.typeof_any };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_io_pretty" => {
                        let callee = self.module.declare_func_in_func(self.io_pretty, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_array_slice" => {
                        let callee = self.module.declare_func_in_func(self.array_slice, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_file_write_text" => {
                        let callee = self.module.declare_func_in_func(self.file_write, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "assert" => {
                        let callee = self.module.declare_func_in_func(self.test_assert, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__testCheck" => {
                        let callee = self.module.declare_func_in_func(self.test_check, &mut b.func);
                        let inst = b.ins().call(callee, &[]);
                        let r8 = b.inst_results(inst)[0];
                        let r = b.ins().uextend(types::I64, r8);
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_path_exists" || n == "__rnx_path_remove" => {
                        let fid = if n == "__rnx_path_exists" { self.path_exists } else { self.path_remove };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r8 = b.inst_results(inst)[0];
                        let r = b.ins().uextend(types::I64, r8);
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_fs_exists"
                            || n == "__rnx_fs_is_file"
                            || n == "__rnx_fs_is_dir"
                            || n == "__rnx_fs_remove"
                            || n == "__rnx_fs_remove_all" =>
                    {
                        let sym = format!("rnx_{}", &n[6..]);
                        let fid = *self.bytes_fns.get(&sym).ok_or_else(|| {
                            format!("cranelift subset: missing runtime `{sym}`")
                        })?;
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r8 = b.inst_results(inst)[0];
                        let r = b.ins().uextend(types::I64, r8);
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_fs_mmap"
                            || n == "__rnx_fs_mmap_anon"
                            || n == "__rnx_fs_mmap_addr"
                            || n == "__rnx_fs_mmap_len" =>
                    {
                        let sym = format!("rnx_{}", &n[6..]);
                        let fid = *self.bytes_fns.get(&sym).ok_or_else(|| {
                            format!("cranelift subset: missing runtime `{sym}`")
                        })?;
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_fs_mmap_close" || n == "__rnx_fs_mmap_flush" =>
                    {
                        let sym = format!("rnx_{}", &n[6..]);
                        let fid = *self.bytes_fns.get(&sym).ok_or_else(|| {
                            format!("cranelift subset: missing runtime `{sym}`")
                        })?;
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_fs_stat"
                            || n == "__rnx_fs_stat_err"
                            || n == "__rnx_file_read_text_err"
                            || n == "__rnx_fs_read_dir"
                            || n == "__rnx_fs_read_dir_err"
                            || n == "__rnx_fs_glob"
                            || n == "__rnx_fs_glob_err"
                            || n == "__rnx_fs_read_link"
                            || n == "__rnx_fs_read_link_err"
                            || n == "__rnx_fs_remove_err"
                            || n == "__rnx_fs_remove_all_err"
                            || n == "__rnx_fs_mkdir_err"
                            || n == "__rnx_fs_copy_err"
                            || n == "__rnx_fs_move_err"
                            || n == "__rnx_fs_rename_err"
                            || n == "__rnx_fs_truncate_err"
                            || n == "__rnx_fs_chmod_err"
                            || n == "__rnx_fs_symlink_err"
                            || n == "__rnx_fs_fsync_err"
                            || n == "__rnx_fs_read_text"
                            || n == "__rnx_fs_read_text_err"
                            || n == "__rnx_fs_read_bytes_err"
                            || n == "__rnx_fs_write_text_err"
                            || n == "__rnx_fs_write_bytes_err"
                            || n == "__rnx_fs_mmap_err"
                            || n == "__rnx_fs_mmap_anon_err" =>
                    {
                        let sym = format!("rnx_{}", &n[6..]);
                        let fid = *self.bytes_fns.get(&sym).ok_or_else(|| {
                            format!("cranelift subset: missing runtime `{sym}`")
                        })?;
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        if dst.is_none() {
                            b.ins().call(callee, &argv);
                            return Ok(());
                        }
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_env_args_count" || n == "__rnx_env_cwd" || n == "__rnx_host_version" => {
                        let fid = if n == "__rnx_env_args_count" { self.env_count } else if n == "__rnx_env_cwd" { self.env_cwd } else { self.host_version };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let inst = b.ins().call(callee, &[]);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            if n != "__rnx_env_args_count" {
                                self.own(*d);
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_env_args_get" || n == "__rnx_env_get" => {
                        let fid = if n == "__rnx_env_args_get" { self.env_args_get } else { self.env_get };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_env_set" => {
                        let callee = self.module.declare_func_in_func(self.env_set, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_env_exit" => {
                        let callee = self.module.declare_func_in_func(self.env_exit, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_net_connect_start" || n == "__rnx_net_recv_or_wait" || n == "__rnx_net_send_or_wait" || n == "__rnx_net_recv_get" || n == "__rnx_net_listener_bind" || n == "__rnx_tls_connect_start" || n == "__rnx_tls_recv_or_wait" || n == "__rnx_tls_send_or_wait" || n == "__rnx_tls_recv_get" => {
                        let fid = if n == "__rnx_net_connect_start" { self.net_connect } else if n == "__rnx_net_recv_or_wait" { self.net_recv_wait } else if n == "__rnx_net_recv_get" { self.net_recv_get } else if n == "__rnx_net_listener_bind" { self.net_listener_bind } else if n == "__rnx_tls_connect_start" { self.tls_connect_start } else if n == "__rnx_tls_recv_or_wait" { self.tls_recv_wait } else if n == "__rnx_tls_recv_get" { self.tls_recv_get } else if n == "__rnx_tls_send_or_wait" { self.tls_send_wait } else { self.net_send_wait };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_net_take_error" || n == "__rnx_net_connect_wait" || n == "__rnx_net_close" || n == "__rnx_net_listener_port" || n == "__rnx_net_listener_accept_start" || n == "__rnx_net_listener_accept_wait" || n == "__rnx_net_listener_close" || n == "__rnx_tls_handshake_start" || n == "__rnx_tls_handshake_wait" || n == "__rnx_tls_close" => {
                        let fid = if n == "__rnx_net_take_error" { self.net_take_error } else if n == "__rnx_net_connect_wait" { self.net_connect_wait } else if n == "__rnx_net_listener_port" { self.net_listener_port } else if n == "__rnx_net_listener_accept_start" { self.net_listener_accept_start } else if n == "__rnx_net_listener_accept_wait" { self.net_listener_accept_wait } else if n == "__rnx_net_listener_close" { self.net_listener_close } else if n == "__rnx_tls_handshake_start" { self.tls_handshake_start } else if n == "__rnx_tls_handshake_wait" { self.tls_handshake_wait } else if n == "__rnx_tls_close" { self.tls_close } else { self.net_close };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_net_error_text" || n == "__rnx_tls_error_text" => {
                        let fid = if n == "__rnx_net_error_text" { self.net_error_text } else { self.tls_error_text };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_process_pid"
                            || n == "__rnx_process_all_env_count"
                            || n == "__rnx_os_cpu_count"
                            || n == "__rnx_os_uptime" =>
                    {
                        let fid = match n.as_str() {
                            "__rnx_process_pid" => self.proc_pid,
                            "__rnx_process_all_env_count" => self.proc_all_env_count,
                            "__rnx_os_cpu_count" => self.os_cpu_count,
                            _ => self.os_uptime,
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let inst = b.ins().call(callee, &[]);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_process_remove_env"
                            || n == "__rnx_process_close_stdin"
                            || n == "__rnx_process_forget" =>
                    {
                        let fid = match n.as_str() {
                            "__rnx_process_remove_env" => self.proc_remove_env,
                            "__rnx_process_close_stdin" => self.proc_close_stdin,
                            _ => self.proc_forget,
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_process_all_env_get"
                            || n == "__rnx_os_platform"
                            || n == "__rnx_os_arch"
                            || n == "__rnx_os_hostname"
                            || n == "__rnx_os_tmpdir"
                            || n == "__rnx_os_homedir" =>
                    {
                        let fid = match n.as_str() {
                            "__rnx_process_all_env_get" => self.proc_all_env_get,
                            "__rnx_os_platform" => self.os_platform,
                            "__rnx_os_arch" => self.os_arch,
                            "__rnx_os_hostname" => self.os_hostname,
                            "__rnx_os_tmpdir" => self.os_tmpdir,
                            _ => self.os_homedir,
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_process_chdir"
                            || n == "__rnx_process_pid_of"
                            || n == "__rnx_process_wait"
                            || n == "__rnx_process_try_wait"
                            || n == "__rnx_process_exit_code"
                            || n == "__rnx_process_take_stdout"
                            || n == "__rnx_process_take_stderr"
                            || n == "__rnx_process_kill" =>
                    {
                        let fid = match n.as_str() {
                            "__rnx_process_chdir" => self.proc_chdir,
                            "__rnx_process_pid_of" => self.proc_pid_of,
                            "__rnx_process_wait" => self.proc_wait,
                            "__rnx_process_try_wait" => self.proc_try_wait,
                            "__rnx_process_exit_code" => self.proc_exit_code,
                            "__rnx_process_take_stdout" => self.proc_take_stdout,
                            "__rnx_process_take_stderr" => self.proc_take_stderr,
                            _ => self.proc_kill,
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_process_write_stdin"
                            || n == "__rnx_process_read_stdout"
                            || n == "__rnx_process_read_stderr"
                            || n == "__rnx_process_spawn"
                            || n == "__rnx_process_run" =>
                    {
                        let fid = match n.as_str() {
                            "__rnx_process_write_stdin" => self.proc_write_stdin,
                            "__rnx_process_read_stdout" => self.proc_read_stdout,
                            "__rnx_process_read_stderr" => self.proc_read_stderr,
                            "__rnx_process_spawn" => self.proc_spawn,
                            _ => self.proc_run,
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_map_new" => {
                        let callee = self.module.declare_func_in_func(self.map_new, &mut b.func);
                        let inst = b.ins().call(callee, &[]);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_map_set" || n == "__rnx_map_clear" => {
                        let fid = if n == "__rnx_map_set" { self.map_set } else { self.map_clear };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_map_get" || n == "__rnx_map_len" => {
                        let fid = if n == "__rnx_map_get" { self.map_get } else { self.map_len };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_map_has" || n == "__rnx_map_delete" => {
                        let fid = if n == "__rnx_map_has" { self.map_has } else { self.map_delete };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r8 = b.inst_results(inst)[0];
                        let r = b.ins().uextend(types::I64, r8);
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_map_keys" || n == "__rnx_map_values" => {
                        let fid = if n == "__rnx_map_keys" { self.map_keys } else { self.map_values };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            self.own(*d);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_black_box"
                            || n == "__rnx_sync_atomic_get"
                            || n == "__rnx_sync_channel_recv"
                            || n == "__rnx_sync_channel_try_recv"
                            || n == "__rnx_sync_channel_len"
                            || n == "__rnx_fs_pool_depth" =>
                    {
                        let fid = match n.as_str() {
                            "__rnx_black_box" => self.black_box,
                            "__rnx_sync_atomic_get" => self.sync_atomic_get,
                            "__rnx_sync_channel_recv" => self.sync_channel_recv,
                            "__rnx_sync_channel_try_recv" => self.sync_channel_try_recv,
                            "__rnx_fs_pool_depth" => self.fs_pool_depth,
                            _ => self.sync_channel_len,
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                            if n.as_str() == "__rnx_sync_channel_recv"
                                && matches!(self.ftypes.get(*d as usize), Some(LirType::Any))
                            {
                                self.own(*d);
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_sync_atomic_fetch_add" => {
                        let callee = self.module.declare_func_in_func(self.sync_atomic_fetch_add, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_sync_atomic_set" || n == "__rnx_sync_channel_send" =>
                    {
                        let fid = if n == "__rnx_sync_atomic_set" {
                            self.sync_atomic_set
                        } else {
                            self.sync_channel_send
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_sync_channel_send_str"
                            || n == "__rnx_sync_channel_drop" =>
                    {
                        let fid = if n == "__rnx_sync_channel_send_str" {
                            self.sync_channel_send_str
                        } else {
                            self.sync_channel_drop
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_sync_channel_send_obj" => {
                        if args.len() != 2 {
                            return Err("channel obj send takes 2 args".to_string());
                        }
                        let (size, dtor) = match self.ftypes.get(args[1] as usize).cloned() {
                            Some(LirType::Obj(name)) => match self.lir.class_index.get(&name) {
                                Some(ci) => (
                                    instance_size(self.lir.classes[*ci].fields.len()) as i64,
                                    self.dtor_for_class(b, *ci)?,
                                ),
                                None => return Err("unknown class".to_string()),
                            },
                            _ => return Err("channel obj send of non-object".to_string()),
                        };
                        let callee =
                            self.module.declare_func_in_func(self.sync_channel_send_obj, &mut b.func);
                        let idv = self.val(b, args[0]);
                        let ptr = self.val(b, args[1]);
                        let n = b.ins().iconst(types::I64, size);
                        b.ins().call(callee, &[idv, ptr, n, dtor]);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_sync_channel_send_array" => {
                        if args.len() != 2 {
                            return Err("channel array send takes 2 args".to_string());
                        }
                        let elem = match self.ftypes.get(args[1] as usize).cloned() {
                            Some(LirType::Array(inner)) => *inner,
                            _ => return Err("channel array send of non-array".to_string()),
                        };
                        let dtor = self.array_elem_dtor(b, &elem)?;
                        let callee = self.module.declare_func_in_func(
                            self.sync_channel_send_array,
                            &mut b.func,
                        );
                        let idv = self.val(b, args[0]);
                        let ptr = self.val(b, args[1]);
                        let n = b.ins().iconst(types::I64, elem_size(&elem) as i64);
                        b.ins().call(callee, &[idv, ptr, n, dtor]);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_debug_live_count" => {
                        let callee =
                            self.module.declare_func_in_func(self.debug_live, &mut b.func);
                        let inst = b.ins().call(callee, &[]);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_mutex_lock"
                            || n == "__rnx_mutex_unlock"
                            || n == "__rnx_rwlock_read_lock"
                            || n == "__rnx_rwlock_read_unlock"
                            || n == "__rnx_rwlock_write_lock"
                            || n == "__rnx_rwlock_write_unlock" =>
                    {
                        let fid = match n.as_str() {
                            "__rnx_mutex_lock" => self.mutex_lock,
                            "__rnx_mutex_unlock" => self.mutex_unlock,
                            "__rnx_rwlock_read_lock" => self.rwlock_read_lock,
                            "__rnx_rwlock_read_unlock" => self.rwlock_read_unlock,
                            "__rnx_rwlock_write_lock" => self.rwlock_write_lock,
                            _ => self.rwlock_write_unlock,
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_mutex_try_lock"
                            || n == "__rnx_rwlock_try_read_lock"
                            || n == "__rnx_rwlock_try_write_lock" =>
                    {
                        let fid = match n.as_str() {
                            "__rnx_mutex_try_lock" => self.mutex_try_lock,
                            "__rnx_rwlock_try_read_lock" => self.rwlock_try_read_lock,
                            _ => self.rwlock_try_write_lock,
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r8 = b.inst_results(inst)[0];
                        let r = b.ins().uextend(types::I64, r8);
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n)
                        if n == "__rnx_condvar_wait"
                            || n == "__rnx_condvar_notify_one"
                            || n == "__rnx_condvar_notify_all" =>
                    {
                        let fid = match n.as_str() {
                            "__rnx_condvar_wait" => self.condvar_wait,
                            "__rnx_condvar_notify_one" => self.condvar_notify_one,
                            _ => self.condvar_notify_all,
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        b.ins().call(callee, &argv);
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_barrier_wait" => {
                        let callee =
                            self.module.declare_func_in_func(self.barrier_wait, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r8 = b.inst_results(inst)[0];
                        let r = b.ins().uextend(types::I64, r8);
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_condvar_wait_timeout" => {
                        let callee =
                            self.module.declare_func_in_func(self.condvar_wait_timeout, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r8 = b.inst_results(inst)[0];
                        let r = b.ins().uextend(types::I64, r8);
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_sync_atomic_cas" => {
                        let callee = self.module.declare_func_in_func(self.sync_atomic_cas, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r8 = b.inst_results(inst)[0];
                        let r = b.ins().uextend(types::I64, r8);
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n.starts_with("__rnx_math_") => {
                        let fid = match n.as_str() {
                            "__rnx_math_sqrt" => self.math_sqrt,
                            "__rnx_math_sin" => self.math_sin,
                            "__rnx_math_cos" => self.math_cos,
                            "__rnx_math_tan" => self.math_tan,
                            "__rnx_math_atan2" => self.math_atan2,
                            "__rnx_math_pow" => self.math_pow,
                            "__rnx_math_floor" => self.math_floor,
                            "__rnx_math_ceil" => self.math_ceil,
                            "__rnx_math_round" => self.math_round,
                            "__rnx_math_log" => self.math_log,
                            _ => return Err("unknown math builtin".to_string()),
                        };
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n.starts_with("__rnx_float_") => {
                        let sym = format!("rnx_{}", &n[6..]);
                        let fid = *self.bytes_fns.get(&sym).ok_or_else(|| {
                            format!("cranelift subset: missing runtime `{sym}`")
                        })?;
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_clock_mono" => {
                        let callee = self.module.declare_func_in_func(self.clock_mono, &mut b.func);
                        let inst = b.ins().call(callee, &[]);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "streq" => {
                        let callee = self.module.declare_func_in_func(self.streq, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r8 = b.inst_results(inst)[0];
                        let r = b.ins().uextend(types::I64, r8);
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "strcmp" => {
                        let callee = self.module.declare_func_in_func(self.strcmp, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_eq_any_str" => {
                        let callee = self.module.declare_func_in_func(self.eq_any_str, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Builtin(n) if n == "__rnx_eq_any" => {
                        let callee = self.module.declare_func_in_func(self.eq_any, &mut b.func);
                        let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                        let inst = b.ins().call(callee, &argv);
                        let r = b.inst_results(inst)[0];
                        if let Some(d) = dst {
                            self.set(b, *d, r);
                        }
                        return Ok(());
                    }
                    CallTarget::Value(v) => {
                        let bptr = self.val(b, *v);
                        let fid = b.ins().load(types::I64, MemFlagsData::trusted(), bptr, 8);
                        let mut argvals = Vec::with_capacity(args.len());
                        for a in args.iter() {
                            let av = self.val(b, *a);
                            argvals.push(av);
                        }
                        let cands: Vec<(usize, FuncId)> = self
                            .lir
                            .functions
                            .iter()
                            .enumerate()
                            .filter(|(_, f)| f.is_closure)
                            .map(|(i, f)| {
                                let id = *self.ids.get(&f.name).ok_or("unknown closure".to_string())?;
                                Ok((i, id))
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        let res_ty = match dst {
                            Some(d) => match self.ftypes.get(*d as usize) {
                                Some(LirType::Vec4f) => types::F32X4,
                                Some(LirType::Vec4i) => types::I32X4,
                                _ => types::I64,
                            },
                            None => types::I64,
                        };
                        let join = b.create_block();
                        let mut seal_all = vec![join];
                        let slot = if dst.is_some() {
                            Some(b.create_sized_stack_slot(
                                StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 4),
                            ))
                        } else {
                            None
                        };
                        if cands.is_empty() {
                            b.ins().jump(join, &[]);
                            b.switch_to_block(join);
                            for bb in seal_all {
                                b.seal_block(bb);
                            }
                            if let Some(d) = dst {
                                if let Some(slot) = slot {
                                    let r = b.ins().stack_load(types::I64, res_ty, slot, 0);
                                    if matches!(
                                        self.ftypes.get(*d as usize),
                                        Some(LirType::Obj(_))
                                            | Some(LirType::Str)
                                            | Some(LirType::Array(_))
                                            | Some(LirType::Enum(_))
                                            | Some(LirType::Any)
                                    ) {
                                        self.own(*d);
                                    }
                                    self.set(b, *d, r);
                                }
                            }
                            return Ok(());
                        }
                        let mut nexts = Vec::with_capacity(cands.len().saturating_sub(1));
                        for _ in 1..cands.len() {
                            let nb = b.create_block();
                            seal_all.push(nb);
                            nexts.push(nb);
                        }
                        for (i, (ci, id)) in cands.iter().enumerate() {
                            if i > 0 {
                                b.switch_to_block(nexts[i - 1]);
                            }
                            let arity = self.lir.functions[*ci].params.len();
                            let want = b.ins().iconst(types::I64, *ci as i64);
                            let fid_lo = b.ins().band_imm_u(fid, 0xFFFF_FFFF);
                            let t = b.ins().icmp(
                                cranelift_codegen::ir::condcodes::IntCC::Equal,
                                fid_lo,
                                want,
                            );
                            let cb = b.create_block();
                            seal_all.push(cb);
                            let else_bb = if i + 1 < cands.len() { nexts[i] } else { join };
                            b.ins().brif(t, cb, &[], else_bb, &[]);
                            b.switch_to_block(cb);
                            let callee = self.module.declare_func_in_func(*id, &mut b.func);
                            let take = args.len().min(arity);
                            let mut callargs: Vec<cranelift_codegen::ir::Value> =
                                argvals[..take].to_vec();
                            for j in 0..arity.saturating_sub(take) {
                                let off = (32 + j * 16) as i32;
                                callargs.push(b.ins().load(types::I64, MemFlagsData::trusted(), bptr, off));
                            }
                            let inst = b.ins().call(callee, &callargs);
                            if let Some(slot) = slot {
                                let r = b.inst_results(inst)[0];
                                b.ins().stack_store(types::I64, r, slot, 0);
                            }
                            b.ins().jump(join, &[]);
                            if i + 1 < cands.len() {
                                b.switch_to_block(nexts[i]);
                            }
                        }
                        b.switch_to_block(join);
                        for bb in seal_all {
                            b.seal_block(bb);
                        }
                        if let Some(d) = dst {
                            if let Some(slot) = slot {
                                let r = b.ins().stack_load(types::I64, res_ty, slot, 0);
                                if matches!(
                                    self.ftypes.get(*d as usize),
                                    Some(LirType::Obj(_))
                                        | Some(LirType::Str)
                                        | Some(LirType::Array(_))
                                        | Some(LirType::Enum(_))
                                        | Some(LirType::Any)
                                ) {
                                    self.own(*d);
                                }
                                self.set(b, *d, r);
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Foreign { lib, symbol } => {
                        let fid = self
                            .foreign
                            .get(&(lib.clone(), symbol.clone()))
                            .copied()
                            .ok_or("unknown foreign callee".to_string())?;
                        let fdecl = self
                            .lir
                            .foreign
                            .iter()
                            .find(|f| &f.lib == lib && &f.symbol == symbol)
                            .ok_or("unknown foreign callee".to_string())?;
                        let callee = self.module.declare_func_in_func(fid, &mut b.func);
                        let mut callargs = Vec::with_capacity(args.len());
                        for (a, want) in args.iter().zip(fdecl.params.iter()) {
                            let av = self.val(b, *a);
                            let cv = match want {
                                LirType::I8 | LirType::Bool => b.ins().ireduce(types::I8, av),
                                LirType::F64(_) => b.ins().bitcast(types::F64, MemFlagsData::new(), av),
                                _ => av,
                            };
                            callargs.push(cv);
                        }
                        let inst = b.ins().call(callee, &callargs);
                        match &fdecl.ret {
                            LirType::Void => {}
                            LirType::I8 | LirType::Bool => {
                                if let Some(d) = dst {
                                    let r = b.inst_results(inst)[0];
                                    let w = b.ins().uextend(types::I64, r);
                                    self.set(b, *d, w);
                                }
                            }
                            LirType::F64(_) => {
                                if let Some(d) = dst {
                                    let r = b.inst_results(inst)[0];
                                    let w = b.ins().bitcast(types::I64, MemFlagsData::new(), r);
                                    self.set(b, *d, w);
                                }
                            }
                            _ => {
                                if let Some(d) = dst {
                                    let r = b.inst_results(inst)[0];
                                    self.set(b, *d, r);
                                }
                            }
                        }
                        return Ok(());
                    }
                    CallTarget::Dyn { method, .. } => {
                        return Err(format!("method `{method}` on a value of unknown type is not supported in native builds; annotate the receiver type, e.g. `let hs: Array<Thread> = []`"));
                    }
                    _ => return Err("indirect call".to_string()),
                };
                let argv: Vec<_> = args.iter().map(|a| self.val(b, *a)).collect();
                let inst = match hot_slot {
                    Some(slot) => {
                        let sig = self
                            .hot_sigs
                            .and_then(|s| s.get(&id))
                            .ok_or("unknown callee signature")?
                            .clone();
                        let sigref = b.import_signature(sig);
                        let base = b.ins().iconst(types::I64, self.hot_base as i64);
                        let off = b.ins().iconst(types::I64, (slot as i64) * 8);
                        let slot_addr = b.ins().iadd(base, off);
                        let code = b.ins().load(types::I64, MemFlagsData::trusted(), slot_addr, 0);
                        b.ins().call_indirect(sigref, code, &argv)
                    }
                    None => {
                        let callee = self.module.declare_func_in_func(id, &mut b.func);
                        b.ins().call(callee, &argv)
                    }
                };
                for (i, d) in dsts.iter().enumerate() {
                    let r = b.inst_results(inst)[i];
                    if matches!(self.ftypes.get(*d as usize), Some(LirType::Obj(_)) | Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Any) | Some(LirType::Error)) {
                        self.own(*d);
                    }
                    self.set(b, *d, r);
                }
                for a in args.iter() {
                    if !release_args {
                        break;
                    }
                    if !matches!(self.ftypes.get(*a as usize), Some(LirType::Any)) {
                        continue;
                    }
                    if dsts.contains(a) || *err == Some(*a) {
                        continue;
                    }
                    let v = self.val(b, *a);
                    self.any_release_val(b, v);
                }
        Ok(())
    }
}
