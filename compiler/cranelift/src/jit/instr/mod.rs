pub(super) use super::*;

mod arith;
mod calls;
mod control;
mod mem;

pub(super) struct FnLower<'a> {
    pub(super) lir: &'a LirModule,
    pub(super) ids: &'a BTreeMap<String, FuncId>,
    pub(super) foreign: &'a BTreeMap<(String, String), FuncId>,
    pub(super) hot_sigs: Option<&'a BTreeMap<FuncId, Signature>>,
    pub(super) hot_slots: Option<&'a BTreeMap<String, FunctionSlotId>>,
    pub(super) hot_base: usize,
    pub(super) alloc: FuncId,
    pub(super) release: FuncId,
    pub(super) genref_create: FuncId,
    pub(super) genref_get: FuncId,
    pub(super) genref_invalidate: FuncId,
    pub(super) retain: FuncId,
    pub(super) concat: FuncId,
    pub(super) streq: FuncId,
    pub(super) strcmp: FuncId,
    pub(super) print_val: FuncId,
    pub(super) print_str: FuncId,
    pub(super) int_to_str: FuncId,
    pub(super) float_to_str: FuncId,
    pub(super) bool_to_str: FuncId,
    pub(super) any_to_str: FuncId,
    pub(super) eq_any_str: FuncId,
    pub(super) eq_any: FuncId,
    pub(super) release_str: FuncId,
    pub(super) array_new: FuncId,
    pub(super) array_push: FuncId,
    pub(super) array_get: FuncId,
    pub(super) array_set: FuncId,
    pub(super) array_get_unchecked: FuncId,
    pub(super) array_set_unchecked: FuncId,
    pub(super) array_len: FuncId,
    pub(super) release_array: FuncId,
    pub(super) thread_spawn: FuncId,
    pub(super) thread_join: FuncId,
    pub(super) clock_mono: FuncId,
    pub(super) crypto_random: FuncId,
    pub(super) prng_seed: FuncId,
    pub(super) prng_next: FuncId,
    pub(super) file_open: FuncId,
    pub(super) file_close: FuncId,
    pub(super) file_read: FuncId,
    pub(super) file_write: FuncId,
    pub(super) file_flush: FuncId,
    pub(super) file_seek: FuncId,
    pub(super) file_tell: FuncId,
    pub(super) file_from_handle: FuncId,
    pub(super) io_is_tty: FuncId,
    pub(super) io_winsize: FuncId,
    pub(super) io_set_raw: FuncId,
    pub(super) path_exists: FuncId,
    pub(super) path_remove: FuncId,
    pub(super) test_assert: FuncId,
    pub(super) test_check: FuncId,
    pub(super) env_count: FuncId,
    pub(super) env_args_get: FuncId,
    pub(super) env_get: FuncId,
    pub(super) env_set: FuncId,
    pub(super) env_cwd: FuncId,
    pub(super) host_version: FuncId,
    pub(super) env_exit: FuncId,
    pub(super) net_connect: FuncId,
    pub(super) net_take_error: FuncId,
    pub(super) net_connect_wait: FuncId,
    pub(super) net_recv_wait: FuncId,
    pub(super) net_send_wait: FuncId,
    pub(super) net_recv_get: FuncId,
    pub(super) net_error_text: FuncId,
    pub(super) net_close: FuncId,
    pub(super) net_listener_bind: FuncId,
    pub(super) net_listener_port: FuncId,
    pub(super) net_listener_accept_start: FuncId,
    pub(super) net_listener_accept_wait: FuncId,
    pub(super) net_listener_close: FuncId,
    pub(super) tls_connect_start: FuncId,
    pub(super) tls_handshake_start: FuncId,
    pub(super) tls_handshake_wait: FuncId,
    pub(super) tls_recv_wait: FuncId,
    pub(super) tls_send_wait: FuncId,
    pub(super) tls_recv_get: FuncId,
    pub(super) tls_error_text: FuncId,
    pub(super) tls_close: FuncId,
    pub(super) proc_pid: FuncId,
    pub(super) proc_remove_env: FuncId,
    pub(super) proc_all_env_count: FuncId,
    pub(super) proc_all_env_get: FuncId,
    pub(super) proc_chdir: FuncId,
    pub(super) proc_spawn: FuncId,
    pub(super) proc_run: FuncId,
    pub(super) proc_pid_of: FuncId,
    pub(super) proc_write_stdin: FuncId,
    pub(super) proc_read_stdout: FuncId,
    pub(super) proc_read_stderr: FuncId,
    pub(super) proc_close_stdin: FuncId,
    pub(super) proc_wait: FuncId,
    pub(super) proc_try_wait: FuncId,
    pub(super) proc_kill: FuncId,
    pub(super) proc_take_stdout: FuncId,
    pub(super) proc_take_stderr: FuncId,
    pub(super) proc_exit_code: FuncId,
    pub(super) proc_forget: FuncId,
    pub(super) os_platform: FuncId,
    pub(super) os_arch: FuncId,
    pub(super) os_hostname: FuncId,
    pub(super) os_tmpdir: FuncId,
    pub(super) os_homedir: FuncId,
    pub(super) os_cpu_count: FuncId,
    pub(super) os_uptime: FuncId,
    pub(super) map_new: FuncId,
    pub(super) map_set: FuncId,
    pub(super) map_get: FuncId,
    pub(super) map_has: FuncId,
    pub(super) map_delete: FuncId,
    pub(super) map_len: FuncId,
    pub(super) map_clear: FuncId,
    pub(super) map_keys: FuncId,
    pub(super) map_values: FuncId,
    pub(super) sync_atomic_get: FuncId,
    pub(super) black_box: FuncId,
    pub(super) sync_atomic_set: FuncId,
    pub(super) sync_atomic_fetch_add: FuncId,
    pub(super) sync_atomic_cas: FuncId,
    pub(super) sync_channel_send: FuncId,
    pub(super) sync_channel_send_str: FuncId,
    pub(super) sync_channel_send_obj: FuncId,
    pub(super) sync_channel_send_array: FuncId,
    pub(super) sync_channel_recv: FuncId,
    pub(super) sync_channel_try_recv: FuncId,
    pub(super) sync_channel_len: FuncId,
    pub(super) fs_pool_depth: FuncId,
    pub(super) sync_channel_drop: FuncId,
    pub(super) debug_live: FuncId,
    pub(super) mutex_lock: FuncId,
    pub(super) mutex_unlock: FuncId,
    pub(super) mutex_try_lock: FuncId,
    pub(super) rwlock_read_lock: FuncId,
    pub(super) rwlock_read_unlock: FuncId,
    pub(super) rwlock_write_lock: FuncId,
    pub(super) rwlock_write_unlock: FuncId,
    pub(super) rwlock_try_read_lock: FuncId,
    pub(super) rwlock_try_write_lock: FuncId,
    pub(super) condvar_wait: FuncId,
    pub(super) condvar_wait_timeout: FuncId,
    pub(super) condvar_notify_one: FuncId,
    pub(super) condvar_notify_all: FuncId,
    pub(super) barrier_wait: FuncId,
    pub(super) pool_init: FuncId,
    pub(super) pool_parallel_for: FuncId,
    pub(super) pool_join: FuncId,
    pub(super) pool_shutdown: FuncId,
    pub(super) math_sqrt: FuncId,
    pub(super) math_sin: FuncId,
    pub(super) math_cos: FuncId,
    pub(super) math_tan: FuncId,
    pub(super) math_atan2: FuncId,
    pub(super) math_pow: FuncId,
    pub(super) math_floor: FuncId,
    pub(super) math_ceil: FuncId,
    pub(super) math_round: FuncId,
    pub(super) math_log: FuncId,
    pub(super) string_len: FuncId,
    pub(super) string_slice: FuncId,
    pub(super) string_index_of: FuncId,
    pub(super) string_index_of_from: FuncId,
    pub(super) string_trim: FuncId,
    pub(super) string_concat: FuncId,
    pub(super) string_split: FuncId,
    pub(super) string_char_code_at: FuncId,
    pub(super) string_from_char_code: FuncId,
    pub(super) array_pop_fn: FuncId,
    pub(super) any_box: FuncId,
    pub(super) any_unbox: FuncId,
    pub(super) any_unbox_heap: FuncId,
    pub(super) any_retain: FuncId,
    pub(super) any_release_box: FuncId,
    pub(super) any_release: FuncId,
    pub(super) heap_track: FuncId,
    pub(super) closure_new: FuncId,
    pub(super) closure_set: FuncId,
    pub(super) closure_release: FuncId,
    pub(super) panic_str: FuncId,
    pub(super) fatal_span: FuncId,
    pub(super) spawn_closure: FuncId,
    pub(super) join_val: FuncId,
    pub(super) join_err: FuncId,
    pub(super) task_val: FuncId,
    pub(super) task_err: FuncId,
    pub(super) pool_new: FuncId,
    pub(super) submit_handle: FuncId,
    pub(super) submit_closure: FuncId,
    pub(super) parallel_closure: FuncId,
    pub(super) defer_push: FuncId,
    pub(super) defer_pop: FuncId,
    pub(super) defer_len: FuncId,
    pub(super) any_tag: FuncId,
    pub(super) obj_class: FuncId,
    pub(super) error_set: FuncId,
    pub(super) error_take: FuncId,
    pub(super) error_class: FuncId,
    pub(super) error_unbox: FuncId,
    pub(super) error_str: FuncId,
    pub(super) error_release: FuncId,
    pub(super) note_type: FuncId,
    pub(super) type_name: FuncId,
    pub(super) typeof_any: FuncId,
    pub(super) io_pretty: FuncId,
    pub(super) note_array_kind: FuncId,
    pub(super) note_fields: FuncId,
    pub(super) note_enum: FuncId,
    pub(super) array_slice: FuncId,
    pub(super) bytes_fns: &'a BTreeMap<String, FuncId>,
    pub(super) statics: &'a BTreeMap<String, DataId>,
    pub(super) module: &'a mut JITModule,
    pub(super) vars: Vec<cranelift_frontend::Variable>,
    pub(super) ftypes: Vec<LirType>,
    pub(super) ret_slots: usize,
    pub(super) stack: BTreeSet<Local>,
    pub(super) owned: BTreeSet<Local>,
    pub(super) ever_owned: BTreeSet<Local>,
    pub(super) nborrowed: usize,
    pub(super) in_enum_dtor: bool,
    pub(super) defers: Vec<Vec<Instr>>,
    pub(super) defer_base: cranelift_frontend::Variable,
    pub(super) dom: lir::licm::DominatorTree,
    pub(super) cur_block: usize,
    pub(super) last_write: BTreeMap<Local, usize>,
}

impl FnLower<'_> {
    pub(super) fn val(&mut self, b: &mut FunctionBuilder<'_>, l: Local) -> cranelift_codegen::ir::Value {
        b.use_var(self.vars[l as usize])
    }

    pub(super) fn set(&mut self, b: &mut FunctionBuilder<'_>, dst: Local, v: cranelift_codegen::ir::Value) {
        b.def_var(self.vars[dst as usize], v);
        self.last_write.insert(dst, self.cur_block);
    }

    pub(super) fn write_dominates(&self, dst: Local) -> bool {
        self.last_write.get(&dst).is_some_and(|w| self.dom.dominates(*w, self.cur_block))
    }

    pub(super) fn emit_run_defers(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        keep: usize,
    ) -> Result<(), String> {
        if self.defers.is_empty() {
            return Ok(());
        }
        let bodies = self.defers.clone();
        let len_callee = self.module.declare_func_in_func(self.defer_len, &mut b.func);
        let pop_callee = self.module.declare_func_in_func(self.defer_pop, &mut b.func);
        let head = b.create_block();
        let pop_bb = b.create_block();
        let done = b.create_block();
        let resume_next = b.create_block();
        let mut extra = vec![head, pop_bb, done, resume_next];
        b.ins().jump(head, &[]);
        b.switch_to_block(head);
        let inst = b.ins().call(len_callee, &[]);
        let len = b.inst_results(inst)[0];
        let base = b.use_var(self.defer_base);
        let target = b.ins().iadd_imm_s(base, keep as i64);
        let cond = b.ins().icmp(
            cranelift_codegen::ir::condcodes::IntCC::UnsignedLessThanOrEqual,
            len,
            target,
        );
        b.ins().brif(cond, done, &[], pop_bb, &[]);
        b.switch_to_block(pop_bb);
        let inst = b.ins().call(pop_callee, &[]);
        let id = b.inst_results(inst)[0];
        let mut next = b.create_block();
        extra.push(next);
        b.ins().jump(next, &[]);
        for (n, body) in bodies.iter().enumerate() {
            b.switch_to_block(next);
            let want = b.ins().iconst(types::I64, n as i64);
            let is_mine = b.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::Equal,
                id,
                want,
            );
            let do_bb = b.create_block();
            extra.push(do_bb);
            next = b.create_block();
            extra.push(next);
            b.ins().brif(is_mine, do_bb, &[], next, &[]);
            b.switch_to_block(do_bb);
            for ins in body {
                self.instr(b, ins)?;
            }
            b.ins().jump(head, &[]);
        }
        b.switch_to_block(next);
        b.ins().jump(done, &[]);
        b.switch_to_block(done);
        b.ins().jump(resume_next, &[]);
        b.switch_to_block(resume_next);
        for bb in extra {
            b.seal_block(bb);
        }
        Ok(())
    }

    pub(super) fn obj_size(&self, l: Local) -> Result<usize, String> {        match self.ftypes.get(l as usize) {
            Some(LirType::Obj(name)) => match self.lir.class_index.get(name) {
                Some(ci) => Ok(instance_size(self.lir.classes[*ci].fields.len())),
                None => Err("unknown class".to_string()),
            },
            _ => Err("not an object".to_string()),
        }
    }

    pub(super) fn str_addr(&mut self, b: &mut FunctionBuilder<'_>, text: &str) -> Result<cranelift_codegen::ir::Value, String> {        let id = self.statics.get(text).copied().ok_or("missing static str".to_string())?;
        let gv = self.module.declare_data_in_func(id, &mut b.func);
        Ok(b.ins().symbol_value(types::I64, gv))
    }

    pub(super) fn f64_of(&mut self, b: &mut FunctionBuilder<'_>, l: Local) -> Result<cranelift_codegen::ir::Value, String> {
        let v = self.val(b, l);
        match self.ftypes.get(l as usize) {
            Some(LirType::F64(_)) => Ok(b.ins().bitcast(types::F64, MemFlagsData::new(), v)),
            _ => Ok(b.ins().fcvt_from_sint(types::F64, v)),
        }
    }

    pub(super) fn bits_of_f64(&mut self, b: &mut FunctionBuilder<'_>, v: cranelift_codegen::ir::Value) -> cranelift_codegen::ir::Value {
        b.ins().bitcast(types::I64, MemFlagsData::new(), v)
    }

    pub(super) fn f32_of_lane(&mut self, b: &mut FunctionBuilder<'_>, l: Local) -> Result<cranelift_codegen::ir::Value, String> {
        let v = self.val(b, l);
        let f = b.ins().bitcast(types::F64, MemFlagsData::new(), v);
        Ok(b.ins().fdemote(types::F32, f))
    }

    pub(super) fn i64_of_f32(&mut self, b: &mut FunctionBuilder<'_>, v: cranelift_codegen::ir::Value) -> cranelift_codegen::ir::Value {
        let f = b.ins().fpromote(types::F64, v);
        b.ins().bitcast(types::I64, MemFlagsData::new(), f)
    }

    pub(super) fn i32_of_lane(&mut self, b: &mut FunctionBuilder<'_>, l: Local) -> cranelift_codegen::ir::Value {
        let v = self.val(b, l);
        b.ins().ireduce(types::I32, v)
    }

    pub(super) fn i64_of_i32(&mut self, b: &mut FunctionBuilder<'_>, v: cranelift_codegen::ir::Value) -> cranelift_codegen::ir::Value {
        b.ins().sextend(types::I64, v)
    }

    pub(super) fn vec_extract_dyn(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        vec: cranelift_codegen::ir::Value,
        lane: cranelift_codegen::ir::Value,
        is_float: bool,
    ) -> cranelift_codegen::ir::Value {
        use cranelift_codegen::ir::condcodes::IntCC;
        let mut out = b.ins().extractlane(vec, 0);
        for i in 1..4u8 {
            let is = b.ins().icmp_imm_s(IntCC::Equal, lane, i as i64);
            let e = b.ins().extractlane(vec, i);
            out = b.ins().select(is, e, out);
        }
        let _ = is_float;
        out
    }

    pub(super) fn release_str(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        l: Local,
        keep: Option<cranelift_codegen::ir::Value>,
    ) -> Result<(), String> {
        let ptr = self.guarded_ptr(b, l, keep);
        self.release_str_val(b, ptr);
        self.owned.remove(&l);
        self.ever_owned.remove(&l);
        Ok(())
    }

    pub(super) fn release_str_val(&mut self, b: &mut FunctionBuilder<'_>, v: cranelift_codegen::ir::Value) {
        let callee = self.module.declare_func_in_func(self.release_str, &mut b.func);
        b.ins().call(callee, &[v]);
    }

    pub(super) fn any_retain_val(&mut self, b: &mut FunctionBuilder<'_>, v: cranelift_codegen::ir::Value) {
        let callee = self.module.declare_func_in_func(self.any_retain, &mut b.func);
        b.ins().call(callee, &[v]);
    }

    pub(super) fn track_heap(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        ptr: cranelift_codegen::ir::Value,
        kind: u64,
        aux1: cranelift_codegen::ir::Value,
        aux2: cranelift_codegen::ir::Value,
    ) {
        let callee = self.module.declare_func_in_func(self.heap_track, &mut b.func);
        let k = b.ins().iconst(types::I64, kind as i64);
        b.ins().call(callee, &[ptr, k, aux1, aux2]);
    }

    pub(super) fn track_array(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        ptr: cranelift_codegen::ir::Value,
        elem: &LirType,
    ) -> Result<(), String> {
        let esize = b.ins().iconst(types::I64, elem_size(elem) as i64);
        let edtor = self.array_elem_dtor(b, elem)?;
        self.track_heap(b, ptr, runtime::native::HEAP_ARRAY, esize, edtor);
        let callee = self.module.declare_func_in_func(self.note_array_kind, &mut b.func);
        let (kind, aux) = lir::instr::pretty_kind_code(elem);
        let k = b.ins().iconst(types::I64, kind as i64);
        let a = b.ins().iconst(types::I64, aux as i64);
        b.ins().call(callee, &[ptr, k, a]);
        Ok(())
    }

    // True when a local's static type is an object whose class metadata is
    // missing: an erased type parameter (bare `T`/`U` in generic code). The
    // slot still holds the uniform value representation, so ownership must go
    // through the tag-dispatched any_retain/any_release pair, never the raw
    // class retain/release (those abort on inline values and misread boxes).
    pub(super) fn erased_obj(&self, l: Local) -> bool {
        match self.ftypes.get(l as usize) {
            Some(LirType::Obj(name)) => !self.lir.class_index.contains_key(name),
            _ => false,
        }
    }

    pub(super) fn any_release_val(&mut self, b: &mut FunctionBuilder<'_>, v: cranelift_codegen::ir::Value) {
        let callee = self.module.declare_func_in_func(self.any_release, &mut b.func);
        b.ins().call(callee, &[v]);
    }

    pub(super) fn alias_keep(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        src: Local,
    ) -> Option<cranelift_codegen::ir::Value> {
        match self.ftypes.get(src as usize) {
            Some(LirType::Vec4f) | Some(LirType::Vec4i) => None,
            _ => Some(self.val(b, src)),
        }
    }

    pub(super) fn guarded_ptr(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        l: Local,
        keep: Option<cranelift_codegen::ir::Value>,
    ) -> cranelift_codegen::ir::Value {
        let ptr = self.val(b, l);
        match keep {
            None => ptr,
            Some(k) => {
                let same =
                    b.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, ptr, k);
                let zero = b.ins().iconst(types::I64, 0);
                b.ins().select(same, zero, ptr)
            }
        }
    }

    pub(super) fn release_any(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        l: Local,
        keep: Option<cranelift_codegen::ir::Value>,
    ) -> Result<(), String> {
        match self.ftypes.get(l as usize).cloned() {
            Some(LirType::Closure) => {
                let ptr = self.guarded_ptr(b, l, keep);
                let callee = self.module.declare_func_in_func(self.closure_release, &mut b.func);
                b.ins().call(callee, &[ptr]);
                self.owned.remove(&l);
                self.ever_owned.remove(&l);
                Ok(())
            }
            Some(LirType::Str) => self.release_str(b, l, keep),
            Some(LirType::Error) => {
                let ptr = self.guarded_ptr(b, l, keep);
                let callee = self.module.declare_func_in_func(self.error_release, &mut b.func);
                b.ins().call(callee, &[ptr]);
                self.owned.remove(&l);
                self.ever_owned.remove(&l);
                Ok(())
            }
            Some(LirType::Any) => {
                let ptr = self.guarded_ptr(b, l, keep);
                self.any_release_val(b, ptr);
                self.owned.remove(&l);
                self.ever_owned.remove(&l);
                Ok(())
            }
            Some(LirType::Array(inner)) => {
                let ptr = self.guarded_ptr(b, l, keep);
                self.release_array_val(b, ptr, &inner)?;
                self.owned.remove(&l);
                self.ever_owned.remove(&l);
                Ok(())
            }
            Some(LirType::Enum(ei)) => {
                let desc = self.lir.enums.get(ei).ok_or("unknown enum".to_string())?;
                let size = enum_instance_size(desc) as i64;
                let ptr = self.guarded_ptr(b, l, keep);
                let dtor = self.enum_dtor_addr(b, ei)?;
                let callee = self.module.declare_func_in_func(self.release, &mut b.func);
                let n = b.ins().iconst(types::I64, size);
                b.ins().call(callee, &[ptr, n, dtor]);
                self.owned.remove(&l);
                self.ever_owned.remove(&l);
                Ok(())
            }
            Some(LirType::Obj(name)) if self.lir.class_index.get(&name).is_none() => {
                // Erased type parameter (e.g. bare `T` in generic prelude code):
                // no class metadata exists, but the slot holds the uniform
                // value representation, so tag-dispatched release is exact for
                // boxed/inline values and a no-op leak for raw heap objects.
                // Must not error here: the interpreter frees these exactly.
                let ptr = self.guarded_ptr(b, l, keep);
                self.any_release_val(b, ptr);
                self.owned.remove(&l);
                self.ever_owned.remove(&l);
                Ok(())
            }
            _ => self.drop_owned(b, l, keep),
        }
    }

    pub(super) fn print_tag(ty: &LirType) -> i64 {
        match ty {
            LirType::I64 => runtime::native::TAG_INT as i64,
            LirType::Bool => runtime::native::TAG_BOOL as i64,
            LirType::F64(_) => runtime::native::TAG_FLOAT as i64,
            LirType::Str => runtime::native::TAG_STR as i64,
            LirType::Error => runtime::native::TAG_STR as i64,
            _ => runtime::native::TAG_PTR as i64,
        }
    }

    pub(super) fn own(&mut self, l: Local) {
        if (l as usize) < self.nborrowed {
            return;
        }
        self.owned.insert(l);
        self.ever_owned.insert(l);
    }

    pub(super) fn dtor_addr(&mut self, b: &mut FunctionBuilder<'_>, l: Local) -> Result<cranelift_codegen::ir::Value, String> {
        let name = match self.ftypes.get(l as usize) {
            Some(LirType::Obj(name)) => name.clone(),
            _ => return Err("dtor of non-object".to_string()),
        };
        let ci = self.lir.class_index.get(&name).copied().ok_or("unknown class".to_string())?;
        match self.lir.classes[ci].dtor {
            Some(fid) => {
                let fname = self.lir.functions[fid].name.clone();
                let id = *self.ids.get(&fname).ok_or("unknown dtor".to_string())?;
                let fr = self.module.declare_func_in_func(id, &mut b.func);
                Ok(b.ins().func_addr(types::I64, fr))
            }
            None => Ok(b.ins().iconst(types::I64, 0)),
        }
    }

    pub(super) fn dtor_for_class(&mut self, b: &mut FunctionBuilder<'_>, ci: usize) -> Result<cranelift_codegen::ir::Value, String> {
        match self.lir.classes[ci].dtor {
            Some(fid) => {
                let fname = self.lir.functions[fid].name.clone();
                let id = *self.ids.get(&fname).ok_or("unknown dtor".to_string())?;
                let fr = self.module.declare_func_in_func(id, &mut b.func);
                Ok(b.ins().func_addr(types::I64, fr))
            }
            None => Ok(b.ins().iconst(types::I64, 0)),
        }
    }

    pub(super) fn enum_dtor_addr(&mut self, b: &mut FunctionBuilder<'_>, ei: usize) -> Result<cranelift_codegen::ir::Value, String> {
        match self.lir.enums.get(ei).and_then(|e| e.dtor) {
            Some(fid) => {
                let fname = self.lir.functions[fid].name.clone();
                let id = *self.ids.get(&fname).ok_or("unknown enum dtor".to_string())?;
                let fr = self.module.declare_func_in_func(id, &mut b.func);
                Ok(b.ins().func_addr(types::I64, fr))
            }
            None => Ok(b.ins().iconst(types::I64, 0)),
        }
    }

    pub(super) fn array_elem_dtor(&mut self, b: &mut FunctionBuilder<'_>, elem: &LirType) -> Result<cranelift_codegen::ir::Value, String> {
        match elem {
            LirType::Obj(name) => match self.lir.class_index.get(name) {
                Some(ci) => self.dtor_for_class(b, *ci),
                None => Ok(b.ins().iconst(types::I64, 0)),
            },
            LirType::Str => {
                let fr = self.module.declare_func_in_func(self.release_str, &mut b.func);
                Ok(b.ins().func_addr(types::I64, fr))
            }
            LirType::Array(_) => {
                let key = type_key(elem);
                match self.lir.array_dtors.get(&key) {
                    Some(fid) => {
                        let fname = self.lir.functions[*fid].name.clone();
                        let id = *self.ids.get(&fname).ok_or("unknown array dtor".to_string())?;
                        let fr = self.module.declare_func_in_func(id, &mut b.func);
                        Ok(b.ins().func_addr(types::I64, fr))
                    }
                    None => Err(format!("missing array dtor for {key}")),
                }
            }
            LirType::Enum(ei) => self.enum_dtor_addr(b, *ei),
            LirType::Any => {
                let fr = self.module.declare_func_in_func(self.any_release, &mut b.func);
                Ok(b.ins().func_addr(types::I64, fr))
            }
            _ => Ok(b.ins().iconst(types::I64, 0)),
        }
    }

    pub(super) fn release_array_val(&mut self, b: &mut FunctionBuilder<'_>, v: cranelift_codegen::ir::Value, elem: &LirType) -> Result<(), String> {
        let dtor = self.array_elem_dtor(b, elem)?;
        let callee = self.module.declare_func_in_func(self.release_array, &mut b.func);
        let n = b.ins().iconst(types::I64, elem_size(elem) as i64);
        b.ins().call(callee, &[v, n, dtor]);
        Ok(())
    }

    pub(super) fn array_get_val(&mut self, b: &mut FunctionBuilder<'_>, arr: Local, index: Local, elem_size: usize, unchecked: bool) -> Result<cranelift_codegen::ir::Value, String> {
        let fid = if unchecked { self.array_get_unchecked } else { self.array_get };
        let callee = self.module.declare_func_in_func(fid, &mut b.func);
        let a = self.val(b, arr);
        let i = self.val(b, index);
        let e = b.ins().iconst(types::I64, elem_size as i64);
        let inst = b.ins().call(callee, &[a, i, e]);
        Ok(b.inst_results(inst)[0])
    }

    pub(super) fn release_array_elem(&mut self, b: &mut FunctionBuilder<'_>, arr: Local, v: cranelift_codegen::ir::Value) -> Result<(), String> {
        match self.ftypes.get(arr as usize).cloned() {
            Some(LirType::Array(inner)) => match *inner {
                LirType::Obj(name) => match self.lir.class_index.get(&name) {
                    Some(ci) => {
                        let size = instance_size(self.lir.classes[*ci].fields.len());
                        let dtor = self.dtor_for_class(b, *ci)?;
                        let callee = self.module.declare_func_in_func(self.release, &mut b.func);
                        let n = b.ins().iconst(types::I64, size as i64);
                        b.ins().call(callee, &[v, n, dtor]);
                        Ok(())
                    }
                    None => Ok(()),
                },
                LirType::Str => {
                    self.release_str_val(b, v);
                    Ok(())
                }
                LirType::Array(nested) => self.release_array_val(b, v, &nested),
                LirType::Enum(ei) => {
                    let size = match self.lir.enums.get(ei) {
                        Some(d) => enum_instance_size(d) as i64,
                        None => return Err("unknown enum".to_string()),
                    };
                    let dtor = self.enum_dtor_addr(b, ei)?;
                    let callee = self.module.declare_func_in_func(self.release, &mut b.func);
                    let n = b.ins().iconst(types::I64, size);
                    b.ins().call(callee, &[v, n, dtor]);
                    Ok(())
                }
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }

    pub(super) fn drop_owned(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        l: Local,
        keep: Option<cranelift_codegen::ir::Value>,
    ) -> Result<(), String> {
        let size = self.obj_size(l)? as i64;
        let ptr = self.guarded_ptr(b, l, keep);
        let dtor = self.dtor_addr(b, l)?;
        let callee = self.module.declare_func_in_func(self.release, &mut b.func);
        let n = b.ins().iconst(types::I64, size);
        b.ins().call(callee, &[ptr, n, dtor]);
        self.owned.remove(&l);
        self.ever_owned.remove(&l);
        Ok(())
    }

    pub(super) fn field_class(&self, obj: Local, field: usize) -> Result<usize, String> {
        let name = match self.ftypes.get(obj as usize) {
            Some(LirType::Obj(name)) => name.clone(),
            _ => return Err("field of non-object".to_string()),
        };
        let ci = self.lir.class_index.get(&name).copied().ok_or("unknown class".to_string())?;
        if field >= self.lir.classes[ci].fields.len() {
            return Err("field out of range".to_string());
        }
        Ok(ci)
    }

    pub(super) fn field_obj_size(&self, obj: Local, field: usize) -> Result<usize, String> {
        let ci = self.field_class(obj, field)?;
        match &self.lir.classes[ci].fields[field].ty {
            LirType::Obj(name) => match self.lir.class_index.get(name) {
                Some(fci) => Ok(instance_size(self.lir.classes[*fci].fields.len())),
                None => Err("unknown field class".to_string()),
            },
            _ => Err("release of non-object field".to_string()),
        }
    }

    pub(super) fn field_dtor_addr(&mut self, b: &mut FunctionBuilder<'_>, obj: Local, field: usize) -> Result<cranelift_codegen::ir::Value, String> {
        let ci = self.field_class(obj, field)?;
        let dtor = match &self.lir.classes[ci].fields[field].ty {
            LirType::Obj(name) => match self.lir.class_index.get(name) {
                Some(fci) => self.lir.classes[*fci].dtor,
                None => return Err("unknown field class".to_string()),
            },
            _ => return Err("release of non-object field".to_string()),
        };
        match dtor {
            Some(fid) => {
                let fname = self.lir.functions[fid].name.clone();
                let id = *self.ids.get(&fname).ok_or("unknown dtor".to_string())?;
                let fr = self.module.declare_func_in_func(id, &mut b.func);
                Ok(b.ins().func_addr(types::I64, fr))
            }
            None => Ok(b.ins().iconst(types::I64, 0)),
        }
    }

    pub(super) fn indexed_field(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        obj: Local,
        field: usize,
    ) -> Result<(cranelift_codegen::ir::Value, usize), String> {
        match self.ftypes.get(obj as usize) {
            Some(LirType::Obj(_)) => Ok((self.val(b, obj), field_offset(field))),
            _ => Err("field of non-object".to_string()),
        }
    }

    pub(super) fn field_addr(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        obj: Local,
        field: &str,
    ) -> Result<(cranelift_codegen::ir::Value, usize), String> {
        let name = match self.ftypes.get(obj as usize) {
            Some(LirType::Obj(name)) => name.clone(),
            _ => return Err("field of non-object".to_string()),
        };
        let ci = self.lir.class_index.get(&name).copied().ok_or("unknown class".to_string())?;
        let fi = self.lir.classes[ci]
            .field_index
            .get(field)
            .copied()
            .ok_or("unknown field".to_string())?;
        Ok((self.val(b, obj), field_offset(fi)))
    }

    pub(super) fn instr(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        match ins {
            Instr::Const { .. } => self.lower_const(b, ins)?,
            Instr::Cast { .. } => self.lower_cast(b, ins)?,
            Instr::Copy { .. } => self.lower_copy(b, ins)?,
            Instr::Arith { .. } => self.lower_arith(b, ins)?,
            Instr::Fma { .. } => self.lower_fma(b, ins)?,
            Instr::Cmp { .. } => self.lower_cmp(b, ins)?,
            Instr::Not { .. } => self.lower_not(b, ins)?,
            Instr::Neg { .. } => self.lower_neg(b, ins)?,
            Instr::Convert { .. } => self.lower_convert(b, ins)?,
            Instr::ObjNew { .. } => self.lower_obj_new(b, ins)?,
            Instr::StackAlloc { .. } => self.lower_stack_alloc(b, ins)?,
            Instr::EnumNew { .. } => self.lower_enum_new(b, ins)?,
            Instr::EnumPayload { .. } => self.lower_enum_payload(b, ins)?,
            Instr::EnumTag { .. } => self.lower_enum_tag(b, ins)?,
            Instr::GetField { .. } => self.lower_get_field(b, ins)?,
            Instr::SetField { .. } => self.lower_set_field(b, ins)?,
            Instr::GetFieldByName { .. } => self.lower_get_field_by_name(b, ins)?,
            Instr::SetFieldByName { .. } => self.lower_set_field_by_name(b, ins)?,
            Instr::GenRefOf { .. } => self.lower_gen_ref_of(b, ins)?,
            Instr::GenRefEmpty { .. } => self.lower_gen_ref_empty(b, ins)?,
            Instr::GenRefGet { .. } => self.lower_gen_ref_get(b, ins)?,
            Instr::GenRefInvalidate { .. } => self.lower_gen_ref_invalidate(b, ins)?,
            Instr::ThreadSpawn { .. } => self.lower_thread_spawn(b, ins)?,
            Instr::ThreadJoin { .. } => self.lower_thread_join(b, ins)?,
            Instr::PoolInit { .. } => self.lower_pool_init(b, ins)?,
            Instr::PoolSubmit { .. } => self.lower_pool_submit(b, ins)?,
            Instr::PoolParallelFor { .. } => self.lower_pool_parallel_for(b, ins)?,
            Instr::PoolJoin { .. } => self.lower_pool_join(b, ins)?,
            Instr::PoolShutdown { .. } => self.lower_pool_shutdown(b, ins)?,
            Instr::VecNew { .. } => self.lower_vec_new(b, ins)?,
            Instr::VecSplat { .. } => self.lower_vec_splat(b, ins)?,
            Instr::VecExtract { .. } => self.lower_vec_extract(b, ins)?,
            Instr::VecInsert { .. } => self.lower_vec_insert(b, ins)?,
            Instr::VecArith { .. } => self.lower_vec_arith(b, ins)?,
            Instr::VecUnary { .. } => self.lower_vec_unary(b, ins)?,
            Instr::VecDot { .. } => self.lower_vec_dot(b, ins)?,
            Instr::Retain { .. } => self.lower_retain(b, ins)?,
            Instr::ClosureNew { .. } => self.lower_closure_new(b, ins)?,
            Instr::ArrayNew { .. } => self.lower_array_new(b, ins)?,
            Instr::ArrayPush { .. } => self.lower_array_push(b, ins)?,
            Instr::ArrayGet { .. } => self.lower_array_get(b, ins)?,
            Instr::ArraySet { .. } => self.lower_array_set(b, ins)?,
            Instr::ArrayLen { .. } => self.lower_array_len(b, ins)?,
            Instr::PtrLoad { .. } => self.lower_ptr_load(b, ins)?,
            Instr::PtrStore { .. } => self.lower_ptr_store(b, ins)?,
            Instr::Release { .. } => self.lower_release(b, ins)?,
            Instr::ReleaseAs { .. } => self.lower_release_as(b, ins)?,
            Instr::ReleaseField { .. } => self.lower_release_field(b, ins)?,
            Instr::Concat { .. } => self.lower_concat(b, ins)?,
            Instr::ToStr { .. } => self.lower_to_str(b, ins)?,
            Instr::Call { .. } => self.lower_call(b, ins)?,
            Instr::Defer { .. } => self.lower_defer(b, ins)?,
            Instr::RunDefers { .. } => self.lower_run_defers(b, ins)?,
            _ => return Err("unsupported instr".to_string()),
        }
        Ok(())
    }

}
