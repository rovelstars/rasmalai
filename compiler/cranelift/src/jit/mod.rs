pub(super) use cranelift_codegen::ir::{types, AbiParam, Function as ClFunction, InstBuilder, MemFlagsData, Signature, StackSlotData, StackSlotKind, UserFuncName};
pub(super) use cranelift_codegen::isa::CallConv;
pub(super) use cranelift_codegen::settings;
pub(super) use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
pub(super) use cranelift_jit::{JITBuilder, JITModule};
pub(super) use cranelift_module::{default_libcall_names, DataDescription, DataId, FuncId, Linkage, Module};
pub(super) use diagnostics::{Code, Diagnostic};
pub(super) use lir::instr::{Module as LirModule, Function as LirFunction, *};
pub(super) use std::collections::{BTreeMap, BTreeSet};
pub(super) use std::sync::atomic::{AtomicU64, Ordering};

mod compile;
mod instr;

use self::compile::{check_supported, lower_fn};

use self::instr::FnLower;


pub type FunctionSlotId = u32;
pub type JitError = diagnostics::Diagnostic;

#[derive(Default)]
pub struct CodegenTimings {
    pub init: std::time::Duration,
    pub declare: std::time::Duration,
    pub define: std::time::Duration,
    pub clif_lower: std::time::Duration,
    pub clif_backend: std::time::Duration,
    pub finalize: std::time::Duration,
    pub fn_count: usize,
}

#[derive(Clone, Copy)]
pub struct HotCtx<'a> {
    pub sigs: &'a BTreeMap<FuncId, Signature>,
    pub slots: &'a BTreeMap<String, FunctionSlotId>,
    pub table_base: usize,
}

struct RtIds {
    alloc: FuncId,
    release: FuncId,
    genref_create: FuncId,
    genref_get: FuncId,
    genref_invalidate: FuncId,
    retain: FuncId,
    concat: FuncId,
    streq: FuncId,
    strcmp: FuncId,
    print_val: FuncId,
    print_str: FuncId,
    int_to_str: FuncId,
    float_to_str: FuncId,
    bool_to_str: FuncId,
    any_to_str: FuncId,
    eq_any_str: FuncId,
    eq_any: FuncId,
    release_str: FuncId,
    array_new: FuncId,
    array_push: FuncId,
    array_get: FuncId,
    array_set: FuncId,
    array_get_unchecked: FuncId,
    array_set_unchecked: FuncId,
    array_len: FuncId,
    release_array: FuncId,
    thread_spawn: FuncId,
    thread_join: FuncId,
    clock_mono: FuncId,
    crypto_random: FuncId,
    prng_seed: FuncId,
    prng_next: FuncId,
    file_open: FuncId,
    file_close: FuncId,
    file_read: FuncId,
    file_write: FuncId,
    file_flush: FuncId,
    file_seek: FuncId,
    file_tell: FuncId,
    file_from_handle: FuncId,
    io_is_tty: FuncId,
    io_winsize: FuncId,
    io_set_raw: FuncId,
    path_exists: FuncId,
    path_remove: FuncId,
    test_assert: FuncId,
    test_check: FuncId,
    env_count: FuncId,
    env_args_get: FuncId,
    env_get: FuncId,
    env_set: FuncId,
    env_cwd: FuncId,
    host_version: FuncId,
    env_exit: FuncId,
    net_connect: FuncId,
    net_take_error: FuncId,
    net_connect_wait: FuncId,
    net_recv_wait: FuncId,
    net_send_wait: FuncId,
    net_recv_get: FuncId,
    net_error_text: FuncId,
    net_close: FuncId,
    net_listener_bind: FuncId,
    net_listener_port: FuncId,
    net_listener_accept_start: FuncId,
    net_listener_accept_wait: FuncId,
    net_listener_close: FuncId,
    tls_connect_start: FuncId,
    tls_handshake_start: FuncId,
    tls_handshake_wait: FuncId,
    tls_recv_wait: FuncId,
    tls_send_wait: FuncId,
    tls_recv_get: FuncId,
    tls_error_text: FuncId,
    tls_close: FuncId,
    proc_pid: FuncId,
    proc_remove_env: FuncId,
    proc_all_env_count: FuncId,
    proc_all_env_get: FuncId,
    proc_chdir: FuncId,
    proc_spawn: FuncId,
    proc_run: FuncId,
    proc_pid_of: FuncId,
    proc_write_stdin: FuncId,
    proc_read_stdout: FuncId,
    proc_read_stderr: FuncId,
    proc_close_stdin: FuncId,
    proc_wait: FuncId,
    proc_try_wait: FuncId,
    proc_kill: FuncId,
    proc_take_stdout: FuncId,
    proc_take_stderr: FuncId,
    proc_exit_code: FuncId,
    proc_forget: FuncId,
    os_platform: FuncId,
    os_arch: FuncId,
    os_hostname: FuncId,
    os_tmpdir: FuncId,
    os_homedir: FuncId,
    os_cpu_count: FuncId,
    os_uptime: FuncId,
    map_new: FuncId,
    map_set: FuncId,
    map_get: FuncId,
    map_has: FuncId,
    map_delete: FuncId,
    map_len: FuncId,
    map_clear: FuncId,
    map_keys: FuncId,
    map_values: FuncId,
    sync_atomic_get: FuncId,
    black_box: FuncId,
    sync_atomic_set: FuncId,
    sync_atomic_fetch_add: FuncId,
    sync_atomic_cas: FuncId,
    sync_channel_send: FuncId,
    sync_channel_send_str: FuncId,
    sync_channel_send_obj: FuncId,
    sync_channel_send_array: FuncId,
    sync_channel_recv: FuncId,
    sync_channel_try_recv: FuncId,
    sync_channel_len: FuncId,
    fs_pool_depth: FuncId,
    sync_channel_drop: FuncId,
    debug_live: FuncId,
    mutex_lock: FuncId,
    mutex_unlock: FuncId,
    mutex_try_lock: FuncId,
    rwlock_read_lock: FuncId,
    rwlock_read_unlock: FuncId,
    rwlock_write_lock: FuncId,
    rwlock_write_unlock: FuncId,
    rwlock_try_read_lock: FuncId,
    rwlock_try_write_lock: FuncId,
    condvar_wait: FuncId,
    condvar_wait_timeout: FuncId,
    condvar_notify_one: FuncId,
    condvar_notify_all: FuncId,
    barrier_wait: FuncId,
    pool_init: FuncId,
    pool_parallel_for: FuncId,
    pool_join: FuncId,
    pool_shutdown: FuncId,
    math_sqrt: FuncId,
    math_sin: FuncId,
    math_cos: FuncId,
    math_tan: FuncId,
    math_atan2: FuncId,
    math_pow: FuncId,
    math_floor: FuncId,
    math_ceil: FuncId,
    math_round: FuncId,
    math_log: FuncId,
    string_len: FuncId,
    string_trim: FuncId,
    string_concat: FuncId,
    string_split: FuncId,
    string_index_of: FuncId,
    string_index_of_from: FuncId,
    string_char_code_at: FuncId,
    string_from_char_code: FuncId,
    string_slice: FuncId,
    array_pop_fn: FuncId,
    any_box: FuncId,
    any_unbox: FuncId,
    any_unbox_heap: FuncId,
    any_retain: FuncId,
    any_release_box: FuncId,
    any_release: FuncId,
    heap_track: FuncId,
    closure_new: FuncId,
    closure_set: FuncId,
    closure_release: FuncId,
    panic_str: FuncId,
    spawn_closure: FuncId,
    join_val: FuncId,
    join_err: FuncId,
    task_val: FuncId,
    task_err: FuncId,
    pool_new: FuncId,
    submit_handle: FuncId,
    submit_closure: FuncId,
    parallel_closure: FuncId,
    defer_push: FuncId,
    defer_pop: FuncId,
    defer_len: FuncId,
    any_tag: FuncId,
    obj_class: FuncId,
    error_set: FuncId,
    error_take: FuncId,
    error_class: FuncId,
    error_unbox: FuncId,
    error_str: FuncId,
    error_release: FuncId,
    note_type: FuncId,
    type_name: FuncId,
    typeof_any: FuncId,
    io_pretty: FuncId,
    note_array_kind: FuncId,
    note_fields: FuncId,
    note_enum: FuncId,
    array_slice: FuncId,
}

pub struct Jit {
    module: JITModule,
    funcs: BTreeMap<String, FuncId>,
    sizes: BTreeMap<String, usize>,
    vec_rets: BTreeSet<String>,
    hot: bool,
    slots: BTreeMap<String, FunctionSlotId>,
    slot_names: Vec<String>,
    table: Option<Box<[AtomicU64]>>,
    versions: BTreeMap<String, u64>,
    sigs: BTreeMap<FuncId, Signature>,
    rt: RtIds,
    bytes_fns: BTreeMap<String, FuncId>,
    statics: BTreeMap<String, DataId>,
    foreign_ids: BTreeMap<(String, String), FuncId>,
}

fn fn_sig(f: &LirFunction) -> Signature {
    let mut sig = Signature::new(CallConv::SystemV);
    for t in &f.params {
        sig.params.push(AbiParam::new(match t {
            LirType::Vec4f => types::F32X4,
            LirType::Vec4i => types::I32X4,
            _ => types::I64,
        }));
    }
    for t in lir::instr::flat_sig(&f.ret) {
        sig.returns.push(AbiParam::new(match t {
            LirType::Vec4f => types::F32X4,
            LirType::Vec4i => types::I32X4,
            _ => types::I64,
        }));
    }
    sig
}

fn foreign_sig(f: &ForeignFn) -> Signature {
    let mut sig = Signature::new(CallConv::SystemV);
    for t in &f.params {
        sig.params.push(AbiParam::new(match t {
            LirType::I8 | LirType::Bool => types::I8,
            LirType::F64(_) => types::F64,
            _ => types::I64,
        }));
    }
    match &f.ret {
        LirType::Void => {}
        LirType::I8 | LirType::Bool => sig.returns.push(AbiParam::new(types::I8)),
        LirType::F64(_) => sig.returns.push(AbiParam::new(types::F64)),
        _ => sig.returns.push(AbiParam::new(types::I64)),
    }
    sig
}

fn sigs_equal(a: &Signature, b: &Signature) -> bool {
    if a.call_conv != b.call_conv || a.params.len() != b.params.len() || a.returns.len() != b.returns.len() {
        return false;
    }
    let param_eq = |x: &AbiParam, y: &AbiParam| {
        x.value_type == y.value_type && x.extension == y.extension && x.purpose == y.purpose
    };
    a.params.iter().zip(b.params.iter()).all(|(x, y)| param_eq(x, y))
        && a.returns.iter().zip(b.returns.iter()).all(|(x, y)| param_eq(x, y))
}

impl Jit {
    pub fn compile(lir: &LirModule) -> Result<Jit, Diagnostic> {
        Self::compile_inner(lir, false, 0)
    }

    pub fn compile_hot(lir: &LirModule) -> Result<Jit, Diagnostic> {
        Self::compile_inner(lir, true, 0)
    }

    pub fn compile_hot_reserve(lir: &LirModule, spare: usize) -> Result<Jit, Diagnostic> {
        Self::compile_inner(lir, true, spare)
    }

    fn compile_inner(lir: &LirModule, hot: bool, spare: usize) -> Result<Jit, Diagnostic> {
        Ok(Self::compile_inner_timed(lir, hot, spare)?.0)
    }

    pub fn compile_with_timings(lir: &LirModule) -> Result<(Jit, CodegenTimings), Diagnostic> {
        Self::compile_inner_timed(lir, false, 0)
    }

    pub fn slot_of(&self, name: &str) -> Option<FunctionSlotId> {
        self.slots.get(name).copied()
    }

    pub fn hot_swap_function(&mut self, slot: FunctionSlotId, new_module: &LirModule) -> Result<(), JitError> {
        if !self.hot {
            return Err(Diagnostic::new(Code::E108, "hot swap needs a hot JIT (compile_hot)"));
        }
        let name = self.slot_names.get(slot as usize).cloned().ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("unknown dispatch slot {slot}"))
        })?;
        let f_new = new_module.functions.iter().find(|f| f.name == name).ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("hot swap: `{name}` missing from new module"))
        })?;
        check_supported(new_module, f_new)?;
        let orig_id = self.funcs.get(&name).copied().ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("hot swap: unknown function `{name}`"))
        })?;
        let new_sig = fn_sig(f_new);
        let orig_sig = self.sigs.get(&orig_id).ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("hot swap: no signature for `{name}`"))
        })?;
        if !sigs_equal(&new_sig, orig_sig) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("hot swap: signature change for `{name}` needs caller recompile"),
            ));
        }
        let ver = self.versions.get(&name).copied().unwrap_or(0) + 1;
        let sym = format!("{name}#v{ver}");
        let new_id = self
            .module
            .declare_function(&sym, Linkage::Local, &new_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare {sym}: {e}")))?;
        let mut codegen_ctx = self.module.make_context();
        codegen_ctx.func =
            ClFunction::with_name_signature(UserFuncName::user(0, new_id.as_u32()), new_sig);
        let table_base = self.table.as_ref().map(|t| t.as_ptr() as usize).unwrap_or(0);
        let hot_ctx = HotCtx { sigs: &self.sigs, slots: &self.slots, table_base };
        let mut ctx = FunctionBuilderContext::new();
        lower_fn(
            new_module,
            f_new,
            &self.funcs,
            &self.foreign_ids,
            &self.rt,
            Some(hot_ctx),
            &self.bytes_fns,
            &self.statics,
            &mut self.module,
            &mut codegen_ctx.func,
            &mut ctx,
        )?;
        self.module
            .define_function(new_id, &mut codegen_ctx)
            .map_err(|e| Diagnostic::new(Code::E108, format!("define {sym}: {e}")))?;
        self.module
            .finalize_definitions()
            .map_err(|e| Diagnostic::new(Code::E108, format!("finalize {sym}: {e}")))?;
        let addr = self.module.get_finalized_function(new_id) as u64;
        if let Some(t) = self.table.as_ref() {
            t[slot as usize].store(addr, Ordering::Release);
        }
        self.versions.insert(name.clone(), ver);
        let size = codegen_ctx
            .compiled_code()
            .map(|c| c.code_info().total_size as usize)
            .unwrap_or(0);
        self.sizes.insert(name, size);
        Ok(())
    }

    pub fn define_new_function(&mut self, name: &str, module: &LirModule) -> Result<FunctionSlotId, JitError> {
        if !self.hot {
            return Err(Diagnostic::new(Code::E108, "define needs a hot JIT (compile_hot)"));
        }
        if self.funcs.contains_key(name) {
            return Err(Diagnostic::new(Code::E108, format!("`{name}` already defined; use hot_swap_function")));
        }
        let f_new = module.functions.iter().find(|f| f.name == name).ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("define: `{name}` missing from module"))
        })?;
        check_supported(module, f_new)?;
        let slot = self.slot_names.len();
        if slot >= self.table.as_ref().map(|t| t.len()).unwrap_or(0) {
            return Err(Diagnostic::new(Code::E108, "dispatch table full; restart the session"));
        }
        let new_sig = fn_sig(f_new);
        let new_id = self
            .module
            .declare_function(name, Linkage::Local, &new_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare {name}: {e}")))?;
        let mut codegen_ctx = self.module.make_context();
        codegen_ctx.func =
            ClFunction::with_name_signature(UserFuncName::user(0, new_id.as_u32()), new_sig.clone());
        let table_base = self.table.as_ref().map(|t| t.as_ptr() as usize).unwrap_or(0);
        let mut ctx = FunctionBuilderContext::new();
        self.funcs.insert(name.to_string(), new_id);
        self.sigs.insert(new_id, new_sig.clone());
        let hot_ctx = HotCtx { sigs: &self.sigs, slots: &self.slots, table_base };
        lower_fn(
            module,
            f_new,
            &self.funcs,
            &self.foreign_ids,
            &self.rt,
            Some(hot_ctx),
            &self.bytes_fns,
            &self.statics,
            &mut self.module,
            &mut codegen_ctx.func,
            &mut ctx,
        )?;
        self.module
            .define_function(new_id, &mut codegen_ctx)
            .map_err(|e| Diagnostic::new(Code::E108, format!("define {name}: {e}")))?;
        self.module
            .finalize_definitions()
            .map_err(|e| Diagnostic::new(Code::E108, format!("finalize {name}: {e}")))?;
        let addr = self.module.get_finalized_function(new_id) as u64;
        if let Some(t) = self.table.as_ref() {
            t[slot].store(addr, Ordering::Release);
        }
        self.slots.insert(name.to_string(), slot as FunctionSlotId);
        self.slot_names.push(name.to_string());
        let size = codegen_ctx
            .compiled_code()
            .map(|c| c.code_info().total_size as usize)
            .unwrap_or(0);
        self.sizes.insert(name.to_string(), size);
        Ok(slot as FunctionSlotId)
    }

    pub fn perf_entries(&self) -> Vec<(String, usize, usize)> {
        let mut out = Vec::with_capacity(self.funcs.len());
        for (name, id) in &self.funcs {
            let addr = if self.hot {
                self.slots
                    .get(name)
                    .and_then(|s| self.table.as_ref().map(|t| t[*s as usize].load(Ordering::Acquire) as usize))
                    .unwrap_or(0)
            } else {
                self.module.get_finalized_function(*id) as usize
            };
            let size = self.sizes.get(name).copied().unwrap_or(0);
            out.push((name.clone(), addr, size));
        }
        out
    }

    pub fn call(&mut self, name: &str, args: &[i64]) -> Result<i64, Diagnostic> {
        runtime::guard::guard_install();
        let id = self.funcs.get(name).copied().ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("unknown function `{name}`"))
        })?;
        if self.vec_rets.contains(name) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("jit entry `{name}` must return Int, not a vector"),
            ));
        }
        let ptr = if self.hot {
            let slot = self.slots.get(name).copied().ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("unknown function `{name}`"))
            })?;
            let addr = self
                .table
                .as_ref()
                .ok_or_else(|| Diagnostic::new(Code::E108, "hot JIT has no dispatch table"))?[
                slot as usize
            ]
                .load(Ordering::Acquire);
            addr as *const u8
        } else {
            self.module.get_finalized_function(id)
        };
        if args.len() > 4 {
            return Err(Diagnostic::new(Code::E108, "jit arity > 4 pending"));
        }
        let v = unsafe {
            match args.len() {
                0 => {
                    let f: unsafe extern "C" fn() -> i64 = std::mem::transmute(ptr);
                    f()
                }
                1 => {
                    let f: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute(ptr);
                    f(args[0])
                }
                2 => {
                    let f: unsafe extern "C" fn(i64, i64) -> i64 = std::mem::transmute(ptr);
                    f(args[0], args[1])
                }
                3 => {
                    let f: unsafe extern "C" fn(i64, i64, i64) -> i64 =
                        std::mem::transmute(ptr);
                    f(args[0], args[1], args[2])
                }
                _ => {
                    let f: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 =
                        std::mem::transmute(ptr);
                    f(args[0], args[1], args[2], args[3])
                }
            }
        };
        if let Some(msg) = runtime::native::take_uncaught_message() {
            return Err(Diagnostic::new(Code::E108, format!("uncaught error: {msg}")));
        }
        Ok(v)
    }
}

fn obj_local(lir: &LirModule, f: &LirFunction, l: Local) -> Option<(usize, usize)> {
    match f.locals.get(l as usize) {
        Some(LirType::Obj(name)) => {
            let ci = *lir.class_index.get(name)?;
            Some((ci, l as usize))
        }
        _ => None,
    }
}

fn named_field(lir: &LirModule, f: &LirFunction, obj: Local, field: &str) -> Option<(usize, usize)> {
    let (ci, _) = obj_local(lir, f, obj)?;
    let fi = *lir.classes[ci].field_index.get(field)?;
    Some((ci, fi))
}

fn array_capturable(inner: &LirType) -> bool {
    matches!(
        inner,
        LirType::I64
            | LirType::Bool
            | LirType::F64(_)
            | LirType::Null
            | LirType::Str
            | LirType::Closure
            | LirType::Obj(_)
            | LirType::Enum(_)
            | LirType::Any
    )
}


pub fn native_flags() -> settings::Flags {
    settings::Flags::new(settings::builder())
}