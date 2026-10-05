pub(super) use diagnostics::{Code, Diagnostic};
pub(super) use inkwell::builder::Builder;
pub(super) use inkwell::context::Context;
pub(super) use inkwell::module::Module as LlModule;
pub(super) use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum};
pub(super) use inkwell::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum, FloatValue, FunctionValue, IntValue, PointerValue, ValueKind, VectorValue};
pub(super) use inkwell::{FloatPredicate, IntPredicate};
pub(super) use inkwell::targets::TargetMachine;
pub(super) use lir::instr::*;
pub(super) use std::collections::{BTreeMap, BTreeSet};

mod lower_fn;
mod instr;
pub mod stdlib_prebuilt;

use self::instr::{FnCx, lower_instr};
use self::lower_fn::{build_module, check_supported};


pub struct Jit<'ctx> {
    module: LlModule<'ctx>,
    funcs: BTreeMap<String, FunctionValue<'ctx>>,
    engine: inkwell::execution_engine::ExecutionEngine<'ctx>,
    vec_rets: BTreeSet<String>,
}

type JitFn0 = unsafe extern "C" fn() -> i64;
type JitFn1 = unsafe extern "C" fn(i64) -> i64;
type JitFn2 = unsafe extern "C" fn(i64, i64) -> i64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OptLevel {
    Dev,
    Release,
}

#[derive(Clone, Debug, Default)]
pub struct DebugInfo {
    pub file: String,
    pub dir: String,
    pub lines: BTreeMap<String, (u32, u32)>,
}

const DW_ATE_SIGNED: u32 = 5;

struct ActiveDebug<'ctx> {
    builder: inkwell::debug_info::DebugInfoBuilder<'ctx>,
    subs: BTreeMap<String, inkwell::debug_info::DISubprogram<'ctx>>,
}

fn start_debug<'ctx>(
    context: &'ctx Context,
    module: &LlModule<'ctx>,
    lir: &Module,
    info: &DebugInfo,
    funcs: &BTreeMap<String, FunctionValue<'ctx>>,
) -> Result<ActiveDebug<'ctx>, Diagnostic> {
    use inkwell::debug_info::{
        AsDIScope, DIFlags, DIFlagsConstants, DWARFEmissionKind, DWARFSourceLanguage,
    };
    module.add_basic_value_flag(
        "Debug Info Version",
        inkwell::module::FlagBehavior::Warning,
        context.i32_type().const_int(3, false),
    );
    let (dibuilder, cu) = module.create_debug_info_builder(
        false,
        DWARFSourceLanguage::C,
        &info.file,
        &info.dir,
        "rnx",
        false,
        "",
        0,
        "",
        DWARFEmissionKind::Full,
        0,
        false,
        false,
        "",
        "",
    );
    let file = cu.get_file();
    let i64di = dibuilder
        .create_basic_type("long", 64, DW_ATE_SIGNED, DIFlags::ZERO)
        .map_err(|e| Diagnostic::new(Code::E108, format!("llvm debug type: {e}")))?;
    let sub_ty =
        dibuilder.create_subroutine_type(file, Some(i64di.as_type()), &[], DIFlags::ZERO);
    let mut subs = BTreeMap::new();
    for f in &lir.functions {
        let (line, _) = info.lines.get(&f.name).copied().unwrap_or((1, 1));
        let sp = dibuilder.create_function(
            cu.as_debug_info_scope(),
            &f.name,
            None,
            file,
            line,
            sub_ty,
            true,
            true,
            line,
            DIFlags::ZERO,
            false,
        );
        if let Some(fv) = funcs.get(&f.name) {
            fv.set_subprogram(sp);
        }
        subs.insert(f.name.clone(), sp);
    }
    Ok(ActiveDebug { builder: dibuilder, subs })
}

pub type PerfMap = (i64, Vec<PerfEntry>);

pub type PerfEntry = (String, usize, usize);

static LLVM_JIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn jit_lock() -> std::sync::MutexGuard<'static, ()> {
    LLVM_JIT_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn execute_with_map(lir: &Module, entry: &str) -> Result<PerfMap, Diagnostic> {
    let context = Context::create();
    let jit = Jit::compile(&context, lir, "rnx_test")?;
    let mut addrs: Vec<(String, usize)> = {
        let _guard = jit_lock();
        let mut addrs: Vec<(String, usize)> = Vec::new();
        for name in jit.funcs.keys() {
            let addr = jit
                .engine
                .get_function_address(name)
                .map_err(|e| Diagnostic::new(Code::E108, format!("addr {name}: {e}")))?;
            addrs.push((name.clone(), addr));
        }
        addrs
    };
    addrs.sort_by_key(|(_, addr)| *addr);
    let mut entries = Vec::with_capacity(addrs.len());
    for (i, (name, addr)) in addrs.iter().enumerate() {
        let size = if i + 1 < addrs.len() {
            addrs[i + 1].1.saturating_sub(*addr)
        } else {
            0
        };
        entries.push((name.clone(), *addr, size));
    }
    let value = jit.call(entry, &[])?;
    Ok((value, entries))
}

impl<'ctx> Jit<'ctx> {

    pub fn compile(
        context: &'ctx Context,
        lir: &Module,
        name: &str,
    ) -> Result<Jit<'ctx>, Diagnostic> {
        let _guard = jit_lock();
        runtime::native::rnx_set_closure_epoch(runtime::native::rnx_claim_closure_epoch());
        for f in &lir.functions {
            check_supported(lir, f)?;
        }
        let (module, funcs) = build_module(context, lir, name, false, None)?;
        let engine = module
            .create_execution_engine()
            .map_err(|e| Diagnostic::new(Code::E108, format!("llvm jit: {e}")))?;
        for (name, addr) in [
            ("rnx_alloc", runtime::native::rnx_alloc as *const () as usize),
            ("rnx_free", runtime::native::rnx_free as *const () as usize),
            ("rnx_print_str", runtime::native::rnx_print_str as *const () as usize),
            ("rnx_print_i64", runtime::native::rnx_print_i64 as *const () as usize),
            ("rnx_panic", runtime::native::rnx_panic as *const () as usize),
            ("rnx_retain", runtime::native::rnx_retain as *const () as usize),
            ("rnx_string_concat", runtime::native::rnx_string_concat as *const () as usize),
            ("rnx_string_split", runtime::native::rnx_string_split as *const () as usize),
            ("rnx_string_eq", runtime::native::rnx_string_eq as *const () as usize),
            ("rnx_string_cmp", runtime::native::rnx_string_cmp as *const () as usize),
            ("rnx_print_val", runtime::native::rnx_print_val as *const () as usize),
            ("rnx_print_str", runtime::native::rnx_print_str as *const () as usize),
            ("rnx_int_to_str", runtime::native::rnx_int_to_str as *const () as usize),
            ("rnx_bool_to_str", runtime::native::rnx_bool_to_str as *const () as usize),
            ("rnx_any_to_str", runtime::native::rnx_any_to_str as *const () as usize),
            ("rnx_eq_any_str", runtime::native::rnx_eq_any_str as *const () as usize),
            ("rnx_eq_any", runtime::native::rnx_eq_any as *const () as usize),
            ("rnx_float_to_str", runtime::native::rnx_float_to_str as *const () as usize),
            ("rnx_release_str", runtime::native::rnx_release_str as *const () as usize),
            ("rnx_array_new", runtime::native::rnx_array_new as *const () as usize),
            ("rnx_array_push", runtime::native::rnx_array_push as *const () as usize),
            ("rnx_array_get", runtime::native::rnx_array_get as *const () as usize),
            ("rnx_array_set", runtime::native::rnx_array_set as *const () as usize),
            ("rnx_array_get_unchecked", runtime::native::rnx_array_get_unchecked as *const () as usize),
            ("rnx_array_set_unchecked", runtime::native::rnx_array_set_unchecked as *const () as usize),
            ("rnx_array_len", runtime::native::rnx_array_len as *const () as usize),
            ("rnx_string_len", runtime::native::rnx_string_len as *const () as usize),
            ("rnx_string_slice", runtime::native::rnx_string_slice as *const () as usize),
            ("rnx_string_index_of", runtime::native::rnx_string_index_of as *const () as usize),
            ("rnx_string_index_of_from", runtime::native::rnx_string_index_of_from as *const () as usize),
            ("rnx_string_trim", runtime::native::rnx_string_trim as *const () as usize),
            ("rnx_string_char_code_at", runtime::native::rnx_string_char_code_at as *const () as usize),
            ("rnx_string_from_char_code", runtime::native::rnx_string_from_char_code as *const () as usize),
            ("rnx_array_pop", runtime::native::rnx_array_pop as *const () as usize),
            ("rnx_any_box", runtime::native::rnx_any_box as *const () as usize),
            ("rnx_any_unbox", runtime::native::rnx_any_unbox as *const () as usize),
            ("rnx_any_unbox_heap", runtime::native::rnx_any_unbox_heap as *const () as usize),
            ("rnx_any_retain", runtime::native::rnx_any_retain as *const () as usize),
            ("rnx_any_release_box", runtime::native::rnx_any_release_box as *const () as usize),
            ("rnx_any_release", runtime::native::rnx_any_release as *const () as usize),
            ("rnx_heap_track", runtime::native::rnx_heap_track as *const () as usize),
            ("rnx_closure_new", runtime::native::rnx_closure_new as *const () as usize),
            ("rnx_closure_set", runtime::native::rnx_closure_set as *const () as usize),
            ("rnx_closure_release", runtime::native::rnx_closure_release as *const () as usize),
            ("rnx_thread_spawn_closure", runtime::native::rnx_thread_spawn_closure as *const () as usize),
            ("rnx_thread_join_val", runtime::native::rnx_thread_join_val as *const () as usize),
            ("rnx_thread_join_err", runtime::native::rnx_thread_join_err as *const () as usize),
            ("rnx_task_await_val", runtime::native::rnx_task_await_val as *const () as usize),
            ("rnx_task_await_err", runtime::native::rnx_task_await_err as *const () as usize),
            ("rnx_pool_new", runtime::native::rnx_pool_new as *const () as usize),
            ("rnx_thread_pool_submit_handle", runtime::native::rnx_thread_pool_submit_handle as *const () as usize),
            ("rnx_thread_pool_submit_closure", runtime::native::rnx_thread_pool_submit_closure as *const () as usize),
            ("rnx_thread_pool_parallel_closure", runtime::native::rnx_thread_pool_parallel_closure as *const () as usize),
            ("rnx_closure_register", runtime::native::rnx_closure_register as *const () as usize),
            ("rnx_panic_str", runtime::native::rnx_panic_str as *const () as usize),
            ("rnx_fatal_span", runtime::native::rnx_fatal_span as *const () as usize),
            ("rnx_release_array", runtime::native::rnx_release_array as *const () as usize),
            ("rnx_thread_spawn", runtime::native::rnx_thread_spawn as *const () as usize),
            ("rnx_thread_join", runtime::native::rnx_thread_join as *const () as usize),
            ("rnx_clock_monotonic_nanos", runtime::native::rnx_clock_monotonic_nanos as *const () as usize),
            ("rnx_sleep_nanos", runtime::native::rnx_sleep_nanos as *const () as usize),
            ("rnx_crypto_random_u64", runtime::native::rnx_crypto_random_u64 as *const () as usize),
            ("rnx_prng_seed", runtime::native::rnx_prng_seed as *const () as usize),
            ("rnx_prng_next", runtime::native::rnx_prng_next as *const () as usize),
            ("rnx_file_open", runtime::native::rnx_file_open as *const () as usize),
            ("rnx_file_close", runtime::native::rnx_file_close as *const () as usize),
            ("rnx_file_read_text", runtime::native::rnx_file_read_text as *const () as usize),
            ("rnx_file_read_text_err", runtime::native::rnx_file_read_text_err as *const () as usize),
            ("rnx_file_write_text", runtime::native::rnx_file_write_text as *const () as usize),
            ("rnx_file_flush", runtime::native::rnx_file_flush as *const () as usize),
            ("rnx_file_seek", runtime::native::rnx_file_seek as *const () as usize),
            ("rnx_file_tell", runtime::native::rnx_file_tell as *const () as usize),
            ("rnx_file_from_handle", runtime::native::rnx_file_from_handle as *const () as usize),
            ("rnx_io_is_tty", runtime::native::rnx_io_is_tty as *const () as usize),
            ("rnx_io_winsize", runtime::native::rnx_io_winsize as *const () as usize),
            ("rnx_io_set_raw", runtime::native::rnx_io_set_raw as *const () as usize),
            ("rnx_file_read_bytes", runtime::native::rnx_file_read_bytes as *const () as usize),
            ("rnx_file_write_bytes", runtime::native::rnx_file_write_bytes as *const () as usize),
            ("rnx_bytes_alloc", runtime::native::rnx_bytes_alloc as *const () as usize),
            ("rnx_bytes_free", runtime::native::rnx_bytes_free as *const () as usize),
            ("rnx_bytes_len", runtime::native::rnx_bytes_len as *const () as usize),
            ("rnx_bytes_cap", runtime::native::rnx_bytes_cap as *const () as usize),
            ("rnx_bytes_data", runtime::native::rnx_bytes_data as *const () as usize),
            ("rnx_bytes_copy_within", runtime::native::rnx_bytes_copy_within as *const () as usize),
            ("rnx_bytes_read_u8", runtime::native::rnx_bytes_read_u8 as *const () as usize),
            ("rnx_bytes_read_i8", runtime::native::rnx_bytes_read_i8 as *const () as usize),
            ("rnx_bytes_read_u16le", runtime::native::rnx_bytes_read_u16le as *const () as usize),
            ("rnx_bytes_read_u16be", runtime::native::rnx_bytes_read_u16be as *const () as usize),
            ("rnx_bytes_read_i16le", runtime::native::rnx_bytes_read_i16le as *const () as usize),
            ("rnx_bytes_read_i16be", runtime::native::rnx_bytes_read_i16be as *const () as usize),
            ("rnx_bytes_read_u32le", runtime::native::rnx_bytes_read_u32le as *const () as usize),
            ("rnx_bytes_read_u32be", runtime::native::rnx_bytes_read_u32be as *const () as usize),
            ("rnx_bytes_read_i32le", runtime::native::rnx_bytes_read_i32le as *const () as usize),
            ("rnx_bytes_read_i32be", runtime::native::rnx_bytes_read_i32be as *const () as usize),
            ("rnx_bytes_read_i64le", runtime::native::rnx_bytes_read_i64le as *const () as usize),
            ("rnx_bytes_read_i64be", runtime::native::rnx_bytes_read_i64be as *const () as usize),
            ("rnx_bytes_write_u8", runtime::native::rnx_bytes_write_u8 as *const () as usize),
            ("rnx_bytes_write_u16le", runtime::native::rnx_bytes_write_u16le as *const () as usize),
            ("rnx_bytes_write_u16be", runtime::native::rnx_bytes_write_u16be as *const () as usize),
            ("rnx_bytes_write_u32le", runtime::native::rnx_bytes_write_u32le as *const () as usize),
            ("rnx_bytes_write_u32be", runtime::native::rnx_bytes_write_u32be as *const () as usize),
            ("rnx_bytes_write_u64le", runtime::native::rnx_bytes_write_u64le as *const () as usize),
            ("rnx_bytes_write_u64be", runtime::native::rnx_bytes_write_u64be as *const () as usize),
            ("rnx_bytes_read_f32le", runtime::native::rnx_bytes_read_f32le as *const () as usize),
            ("rnx_bytes_read_f32be", runtime::native::rnx_bytes_read_f32be as *const () as usize),
            ("rnx_bytes_read_f64le", runtime::native::rnx_bytes_read_f64le as *const () as usize),
            ("rnx_bytes_read_f64be", runtime::native::rnx_bytes_read_f64be as *const () as usize),
            ("rnx_bytes_write_f32le", runtime::native::rnx_bytes_write_f32le as *const () as usize),
            ("rnx_bytes_write_f32be", runtime::native::rnx_bytes_write_f32be as *const () as usize),
            ("rnx_bytes_write_f64le", runtime::native::rnx_bytes_write_f64le as *const () as usize),
            ("rnx_bytes_write_f64be", runtime::native::rnx_bytes_write_f64be as *const () as usize),
            ("rnx_bytes_read_string", runtime::native::rnx_bytes_read_string as *const () as usize),
            ("rnx_bytes_write_string", runtime::native::rnx_bytes_write_string as *const () as usize),
            ("rnx_path_exists", runtime::native::rnx_path_exists as *const () as usize),
            ("rnx_path_remove", runtime::native::rnx_path_remove as *const () as usize),
            ("rnx_fs_exists", runtime::native::rnx_fs_exists as *const () as usize),
            ("rnx_fs_is_file", runtime::native::rnx_fs_is_file as *const () as usize),
            ("rnx_fs_is_dir", runtime::native::rnx_fs_is_dir as *const () as usize),
            ("rnx_fs_stat", runtime::native::rnx_fs_stat as *const () as usize),
            ("rnx_fs_stat_err", runtime::native::rnx_fs_stat_err as *const () as usize),
            ("rnx_fs_read_dir", runtime::native::rnx_fs_read_dir as *const () as usize),
            ("rnx_fs_read_dir_err", runtime::native::rnx_fs_read_dir_err as *const () as usize),
            ("rnx_fs_glob", runtime::native::rnx_fs_glob as *const () as usize),
            ("rnx_fs_glob_err", runtime::native::rnx_fs_glob_err as *const () as usize),
            ("rnx_fs_read_link", runtime::native::rnx_fs_read_link as *const () as usize),
            ("rnx_fs_read_link_err", runtime::native::rnx_fs_read_link_err as *const () as usize),
            ("rnx_fs_remove", runtime::native::rnx_fs_remove as *const () as usize),
            ("rnx_fs_remove_err", runtime::native::rnx_fs_remove_err as *const () as usize),
            ("rnx_fs_remove_all", runtime::native::rnx_fs_remove_all as *const () as usize),
            ("rnx_fs_remove_all_err", runtime::native::rnx_fs_remove_all_err as *const () as usize),
            ("rnx_fs_mkdir_err", runtime::native::rnx_fs_mkdir_err as *const () as usize),
            ("rnx_fs_copy_err", runtime::native::rnx_fs_copy_err as *const () as usize),
            ("rnx_fs_move_err", runtime::native::rnx_fs_move_err as *const () as usize),
            ("rnx_fs_rename_err", runtime::native::rnx_fs_rename_err as *const () as usize),
            ("rnx_fs_truncate_err", runtime::native::rnx_fs_truncate_err as *const () as usize),
            ("rnx_fs_chmod_err", runtime::native::rnx_fs_chmod_err as *const () as usize),
            ("rnx_fs_symlink_err", runtime::native::rnx_fs_symlink_err as *const () as usize),
            ("rnx_fs_fsync_err", runtime::native::rnx_fs_fsync_err as *const () as usize),
            ("rnx_fs_read_text", runtime::native::rnx_fs_read_text as *const () as usize),
            ("rnx_fs_read_text_err", runtime::native::rnx_fs_read_text_err as *const () as usize),
            ("rnx_fs_write_text", runtime::native::rnx_fs_write_text as *const () as usize),
            ("rnx_fs_write_text_err", runtime::native::rnx_fs_write_text_err as *const () as usize),
            ("rnx_fs_read_bytes", runtime::native::rnx_fs_read_bytes as *const () as usize),
            ("rnx_fs_read_bytes_err", runtime::native::rnx_fs_read_bytes_err as *const () as usize),
            ("rnx_fs_write_bytes", runtime::native::rnx_fs_write_bytes as *const () as usize),
            ("rnx_fs_write_bytes_err", runtime::native::rnx_fs_write_bytes_err as *const () as usize),
            ("rnx_fs_mmap", runtime::native::rnx_fs_mmap as *const () as usize),
            ("rnx_fs_mmap_err", runtime::native::rnx_fs_mmap_err as *const () as usize),
            ("rnx_fs_mmap_anon", runtime::native::rnx_fs_mmap_anon as *const () as usize),
            ("rnx_fs_mmap_anon_err", runtime::native::rnx_fs_mmap_anon_err as *const () as usize),
            ("rnx_fs_mmap_addr", runtime::native::rnx_fs_mmap_addr as *const () as usize),
            ("rnx_fs_mmap_len", runtime::native::rnx_fs_mmap_len as *const () as usize),
            ("rnx_fs_mmap_flush", runtime::native::rnx_fs_mmap_flush as *const () as usize),
            ("rnx_fs_mmap_close", runtime::native::rnx_fs_mmap_close as *const () as usize),
            ("rnx_assert", runtime::native::rnx_assert as *const () as usize),
            ("rnx_test_check", runtime::native::rnx_test_check as *const () as usize),
            ("rnx_env_args_count", runtime::native::rnx_env_args_count as *const () as usize),
            ("rnx_env_args_get", runtime::native::rnx_env_args_get as *const () as usize),
            ("rnx_env_get", runtime::native::rnx_env_get as *const () as usize),
            ("rnx_env_set", runtime::native::rnx_env_set as *const () as usize),
            ("rnx_env_cwd", runtime::native::rnx_env_cwd as *const () as usize),
            ("rnx_host_version", runtime::native::rnx_host_version as *const () as usize),
            ("rnx_env_exit", runtime::native::rnx_env_exit as *const () as usize),
            ("rnx_net_connect_start", runtime::native::rnx_net_connect_start as *const () as usize),
            ("rnx_net_take_error", runtime::native::rnx_net_take_error as *const () as usize),
            ("rnx_net_connect_wait", runtime::native::rnx_net_connect_wait as *const () as usize),
            ("rnx_net_recv_or_wait", runtime::native::rnx_net_recv_or_wait as *const () as usize),
            ("rnx_net_send_or_wait", runtime::native::rnx_net_send_or_wait as *const () as usize),
            ("rnx_net_recv_get", runtime::native::rnx_net_recv_get as *const () as usize),
            ("rnx_net_error_text", runtime::native::rnx_net_error_text as *const () as usize),
            ("rnx_net_close", runtime::native::rnx_net_close as *const () as usize),
            ("rnx_net_listener_bind", runtime::native::rnx_net_listener_bind as *const () as usize),
            ("rnx_net_listener_port", runtime::native::rnx_net_listener_port as *const () as usize),
            ("rnx_net_listener_accept_start", runtime::native::rnx_net_listener_accept_start as *const () as usize),
            ("rnx_net_listener_accept_wait", runtime::native::rnx_net_listener_accept_wait as *const () as usize),
            ("rnx_net_listener_close", runtime::native::rnx_net_listener_close as *const () as usize),
            ("rnx_tls_connect_start", runtime::native::rnx_tls_connect_start as *const () as usize),
            ("rnx_tls_handshake_start", runtime::native::rnx_tls_handshake_start as *const () as usize),
            ("rnx_tls_handshake_wait", runtime::native::rnx_tls_handshake_wait as *const () as usize),
            ("rnx_tls_recv_or_wait", runtime::native::rnx_tls_recv_or_wait as *const () as usize),
            ("rnx_tls_send_or_wait", runtime::native::rnx_tls_send_or_wait as *const () as usize),
            ("rnx_tls_recv_get", runtime::native::rnx_tls_recv_get as *const () as usize),
            ("rnx_tls_error_text", runtime::native::rnx_tls_error_text as *const () as usize),
            ("rnx_tls_close", runtime::native::rnx_tls_close as *const () as usize),
            ("rnx_process_pid", runtime::native::rnx_process_pid as *const () as usize),
            ("rnx_process_remove_env", runtime::native::rnx_process_remove_env as *const () as usize),
            ("rnx_process_all_env_count", runtime::native::rnx_process_all_env_count as *const () as usize),
            ("rnx_process_all_env_get", runtime::native::rnx_process_all_env_get as *const () as usize),
            ("rnx_process_chdir", runtime::native::rnx_process_chdir as *const () as usize),
            ("rnx_process_spawn", runtime::native::rnx_process_spawn as *const () as usize),
            ("rnx_process_run", runtime::native::rnx_process_run as *const () as usize),
            ("rnx_process_pid_of", runtime::native::rnx_process_pid_of as *const () as usize),
            ("rnx_process_write_stdin", runtime::native::rnx_process_write_stdin as *const () as usize),
            ("rnx_process_read_stdout", runtime::native::rnx_process_read_stdout as *const () as usize),
            ("rnx_process_read_stderr", runtime::native::rnx_process_read_stderr as *const () as usize),
            ("rnx_process_close_stdin", runtime::native::rnx_process_close_stdin as *const () as usize),
            ("rnx_process_wait", runtime::native::rnx_process_wait as *const () as usize),
            ("rnx_process_try_wait", runtime::native::rnx_process_try_wait as *const () as usize),
            ("rnx_process_kill", runtime::native::rnx_process_kill as *const () as usize),
            ("rnx_process_take_stdout", runtime::native::rnx_process_take_stdout as *const () as usize),
            ("rnx_process_take_stderr", runtime::native::rnx_process_take_stderr as *const () as usize),
            ("rnx_process_exit_code", runtime::native::rnx_process_exit_code as *const () as usize),
            ("rnx_process_forget", runtime::native::rnx_process_forget as *const () as usize),
            ("rnx_os_platform", runtime::native::rnx_os_platform as *const () as usize),
            ("rnx_os_arch", runtime::native::rnx_os_arch as *const () as usize),
            ("rnx_os_hostname", runtime::native::rnx_os_hostname as *const () as usize),
            ("rnx_os_tmpdir", runtime::native::rnx_os_tmpdir as *const () as usize),
            ("rnx_os_homedir", runtime::native::rnx_os_homedir as *const () as usize),
            ("rnx_os_cpu_count", runtime::native::rnx_os_cpu_count as *const () as usize),
            ("rnx_os_uptime", runtime::native::rnx_os_uptime as *const () as usize),
            ("rnx_map_new", runtime::native::rnx_map_new as *const () as usize),
            ("rnx_map_set", runtime::native::rnx_map_set as *const () as usize),
            ("rnx_map_get", runtime::native::rnx_map_get as *const () as usize),
            ("rnx_map_has", runtime::native::rnx_map_has as *const () as usize),
            ("rnx_map_delete", runtime::native::rnx_map_delete as *const () as usize),
            ("rnx_map_len", runtime::native::rnx_map_len as *const () as usize),
            ("rnx_map_clear", runtime::native::rnx_map_clear as *const () as usize),
            ("rnx_map_keys", runtime::native::rnx_map_keys as *const () as usize),
            ("rnx_map_values", runtime::native::rnx_map_values as *const () as usize),
            ("rnx_gmap_new", runtime::native::rnx_gmap_new as *const () as usize),
            ("rnx_gmap_free", runtime::native::rnx_gmap_free as *const () as usize),
            ("rnx_gmap_set", runtime::native::rnx_gmap_set as *const () as usize),
            ("rnx_gmap_get", runtime::native::rnx_gmap_get as *const () as usize),
            ("rnx_gmap_has", runtime::native::rnx_gmap_has as *const () as usize),
            ("rnx_gmap_delete", runtime::native::rnx_gmap_delete as *const () as usize),
            ("rnx_gmap_len", runtime::native::rnx_gmap_len as *const () as usize),
            ("rnx_gmap_clear", runtime::native::rnx_gmap_clear as *const () as usize),
            ("rnx_gmap_keys", runtime::native::rnx_gmap_keys as *const () as usize),
            ("rnx_gmap_values", runtime::native::rnx_gmap_values as *const () as usize),
            ("rnx_json_parse", runtime::native::rnx_json_parse as *const () as usize),
            ("rnx_json_parse_typed", runtime::native::rnx_json_parse_typed as *const () as usize),
            ("rnx_json_stringify_into", runtime::native::rnx_json_stringify_into as *const () as usize),
            ("rnx_json_stringify", runtime::native::rnx_json_stringify as *const () as usize),
            ("rnx_json_unwrap", runtime::native::rnx_json_unwrap as *const () as usize),
            ("rnx_dns_lookup_start", runtime::native::rnx_dns_lookup_start as *const () as usize),
            ("rnx_dns_lookup_wait", runtime::native::rnx_dns_lookup_wait as *const () as usize),
            ("rnx_dns_lookup_get", runtime::native::rnx_dns_lookup_get as *const () as usize),
            ("rnx_dns_lookup_error", runtime::native::rnx_dns_lookup_error as *const () as usize),
            ("rnx_release_map", runtime::native::rnx_release_map as *const () as usize),
            ("rnx_sync_atomic_get", runtime::native::rnx_sync_atomic_get as *const () as usize),
            ("rnx_black_box_i64", runtime::native::rnx_black_box_i64 as *const () as usize),
            ("rnx_sync_atomic_set", runtime::native::rnx_sync_atomic_set as *const () as usize),
            ("rnx_sync_atomic_fetch_add", runtime::native::rnx_sync_atomic_fetch_add as *const () as usize),
            ("rnx_sync_atomic_cas", runtime::native::rnx_sync_atomic_cas as *const () as usize),
            ("rnx_sync_channel_send", runtime::native::rnx_sync_channel_send as *const () as usize),
            ("rnx_sync_channel_send_str", runtime::native::rnx_sync_channel_send_str as *const () as usize),
            ("rnx_sync_channel_send_obj", runtime::native::rnx_sync_channel_send_obj as *const () as usize),
            ("rnx_sync_channel_send_array", runtime::native::rnx_sync_channel_send_array as *const () as usize),
            ("rnx_sync_channel_recv", runtime::native::rnx_sync_channel_recv as *const () as usize),
            ("rnx_sync_channel_try_recv", runtime::native::rnx_sync_channel_try_recv as *const () as usize),
            ("rnx_sync_channel_len", runtime::native::rnx_sync_channel_len as *const () as usize),
            ("rnx_fs_pool_depth", runtime::native::rnx_fs_pool_depth as *const () as usize),
            ("rnx_sync_channel_drop", runtime::native::rnx_sync_channel_drop as *const () as usize),
            ("rnx_debug_live_count", runtime::native::rnx_debug_live_count as *const () as usize),
            ("rnx_any_tag", runtime::native::rnx_any_tag as *const () as usize),
            ("rnx_obj_class", runtime::native::rnx_obj_class as *const () as usize),
            ("rnx_error_set", runtime::native::rnx_error_set as *const () as usize),
            ("rnx_error_take", runtime::native::rnx_error_take as *const () as usize),
            ("rnx_error_class", runtime::native::rnx_error_class as *const () as usize),
            ("rnx_error_unbox", runtime::native::rnx_error_unbox as *const () as usize),
            ("rnx_error_str", runtime::native::rnx_error_str as *const () as usize),
            ("rnx_error_release", runtime::native::rnx_error_release as *const () as usize),
            ("rnx_error_set", runtime::native::rnx_error_set as *const () as usize),
            ("rnx_error_take", runtime::native::rnx_error_take as *const () as usize),
            ("rnx_error_class", runtime::native::rnx_error_class as *const () as usize),
            ("rnx_error_unbox", runtime::native::rnx_error_unbox as *const () as usize),
            ("rnx_error_str", runtime::native::rnx_error_str as *const () as usize),
            ("rnx_error_release", runtime::native::rnx_error_release as *const () as usize),
            ("rnx_note_type", runtime::native::rnx_note_type as *const () as usize),
            ("rnx_note_fields", runtime::native::rnx_note_fields as *const () as usize),
            ("rnx_note_namespace", runtime::native::rnx_note_namespace as *const () as usize),
            ("rnx_note_array_kind", runtime::native::rnx_note_array_kind as *const () as usize),
            ("rnx_note_enum", runtime::native::rnx_note_enum as *const () as usize),
            ("rnx_io_pretty", runtime::native::rnx_io_pretty as *const () as usize),
            ("rnx_type_name", runtime::native::rnx_type_name as *const () as usize),
            ("rnx_typeof_any", runtime::native::rnx_typeof_any as *const () as usize),
            ("rnx_array_slice", runtime::native::rnx_array_slice as *const () as usize),
            ("rnx_defer_push", runtime::native::rnx_defer_push as *const () as usize),
            ("rnx_defer_pop", runtime::native::rnx_defer_pop as *const () as usize),
            ("rnx_defer_len", runtime::native::rnx_defer_len as *const () as usize),
            ("rnx_mutex_lock", runtime::native::rnx_mutex_lock as *const () as usize),
            ("rnx_mutex_unlock", runtime::native::rnx_mutex_unlock as *const () as usize),
            ("rnx_mutex_try_lock", runtime::native::rnx_mutex_try_lock as *const () as usize),
            ("rnx_rwlock_read_lock", runtime::native::rnx_rwlock_read_lock as *const () as usize),
            ("rnx_rwlock_read_unlock", runtime::native::rnx_rwlock_read_unlock as *const () as usize),
            ("rnx_rwlock_write_lock", runtime::native::rnx_rwlock_write_lock as *const () as usize),
            ("rnx_rwlock_write_unlock", runtime::native::rnx_rwlock_write_unlock as *const () as usize),
            ("rnx_rwlock_try_read_lock", runtime::native::rnx_rwlock_try_read_lock as *const () as usize),
            ("rnx_rwlock_try_write_lock", runtime::native::rnx_rwlock_try_write_lock as *const () as usize),
            ("rnx_condvar_wait", runtime::native::rnx_condvar_wait as *const () as usize),
            ("rnx_condvar_wait_timeout", runtime::native::rnx_condvar_wait_timeout as *const () as usize),
            ("rnx_condvar_notify_one", runtime::native::rnx_condvar_notify_one as *const () as usize),
            ("rnx_condvar_notify_all", runtime::native::rnx_condvar_notify_all as *const () as usize),
            ("rnx_barrier_wait", runtime::native::rnx_barrier_wait as *const () as usize),
            ("rnx_thread_pool_init", runtime::native::rnx_thread_pool_init as *const () as usize),
            ("rnx_thread_pool_submit", runtime::native::rnx_thread_pool_submit as *const () as usize),
            ("rnx_thread_pool_parallel_for", runtime::native::rnx_thread_pool_parallel_for as *const () as usize),
            ("rnx_thread_pool_join", runtime::native::rnx_thread_pool_join as *const () as usize),
            ("rnx_thread_pool_shutdown", runtime::native::rnx_thread_pool_shutdown as *const () as usize),
            ("rnx_math_sqrt", runtime::native::rnx_math_sqrt as *const () as usize),
            ("rnx_math_sin", runtime::native::rnx_math_sin as *const () as usize),
            ("rnx_math_cos", runtime::native::rnx_math_cos as *const () as usize),
            ("rnx_math_tan", runtime::native::rnx_math_tan as *const () as usize),
            ("rnx_math_atan2", runtime::native::rnx_math_atan2 as *const () as usize),
            ("rnx_math_pow", runtime::native::rnx_math_pow as *const () as usize),
            ("rnx_math_floor", runtime::native::rnx_math_floor as *const () as usize),
            ("rnx_math_ceil", runtime::native::rnx_math_ceil as *const () as usize),
            ("rnx_math_round", runtime::native::rnx_math_round as *const () as usize),
            ("rnx_math_log", runtime::native::rnx_math_log as *const () as usize),
            ("rnx_float_nan", runtime::native::rnx_float_nan as *const () as usize),
            ("rnx_float_to_bits", runtime::native::rnx_float_to_bits as *const () as usize),
            ("rnx_float_from_bits", runtime::native::rnx_float_from_bits as *const () as usize),
            ("rnx_float_fma", runtime::native::rnx_float_fma as *const () as usize),
            ("rnx_genref_create", runtime::native::rnx_genref_create as *const () as usize),
            ("rnx_genref_get", runtime::native::rnx_genref_get as *const () as usize),
            (
                "rnx_genref_invalidate",
                runtime::native::rnx_genref_invalidate as *const () as usize,
            ),
            ("rnx_release", runtime::native::rnx_release as *const () as usize),
        ] {
            let fv: FunctionValue<'ctx> = module.get_function(name).ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("llvm jit: missing runtime symbol `{name}`"))
            })?;
            engine.add_global_mapping(&fv, addr);
        }
        for f in &lir.foreign {
            let fv: FunctionValue<'ctx> = module.get_function(&f.symbol).ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("llvm jit: missing foreign symbol `{}`", f.symbol))
            })?;
            let addr = runtime::native::resolve_foreign(&f.lib, &f.symbol).ok_or_else(|| {
                Diagnostic::new(
                    Code::E108,
                    format!("unknown foreign symbol `{}` in native lib `{}`", f.symbol, f.lib),
                )
            })? as usize;
            engine.add_global_mapping(&fv, addr);
        }
        for (i, f) in lir.functions.iter().enumerate() {
            if !f.is_closure {
                continue;
            }
            let fv: FunctionValue<'ctx> = funcs.get(&f.name).copied().ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("llvm jit: missing closure `{}`", f.name))
            })?;
            let addr = engine
                .get_function_address(&fv.get_name().to_string_lossy())
                .map_err(|e| Diagnostic::new(Code::E108, format!("llvm jit closure addr: {e}")))?;
            runtime::native::rnx_closure_register(runtime::native::rnx_closure_tag(i as u64), addr);
        }
        Ok(Jit {
            module,
            funcs,
            engine,
            vec_rets: lir
                .functions
                .iter()
                .filter(|f| matches!(f.ret, LirType::Vec4f | LirType::Vec4i))
                .map(|f| f.name.clone())
                .collect(),
        })
    }

    pub fn call(&self, name: &str, args: &[i64]) -> Result<i64, Diagnostic> {
        runtime::guard::guard_install();
        let fv = self.funcs.get(name).copied().ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("unknown function `{name}`"))
        })?;
        if self.vec_rets.contains(name) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("llvm entry `{name}` must return Int, not a vector"),
            ));
        }
        unsafe {
            let v = match args.len() {
                0 => {
                    let addr = {
                        let _guard = jit_lock();
                        self.engine.get_function_address(&fv.get_name().to_string_lossy()).map_err(|e| {
                            Diagnostic::new(Code::E108, format!("addr: {e}"))
                        })?
                    };
                    let f: JitFn0 = std::mem::transmute(addr);
                    f()
                }
                1 => {
                    let addr = {
                        let _guard = jit_lock();
                        self.engine.get_function_address(&fv.get_name().to_string_lossy()).map_err(|e| {
                            Diagnostic::new(Code::E108, format!("addr: {e}"))
                        })?
                    };
                    let f: JitFn1 = std::mem::transmute(addr);
                    f(args[0])
                }
                2 => {
                    let addr = {
                        let _guard = jit_lock();
                        self.engine.get_function_address(&fv.get_name().to_string_lossy()).map_err(|e| {
                            Diagnostic::new(Code::E108, format!("addr: {e}"))
                        })?
                    };
                    let f: JitFn2 = std::mem::transmute(addr);
                    f(args[0], args[1])
                }
                _ => return Err(Diagnostic::new(Code::E108, "llvm jit arity > 2 pending")),
            };
            if let Some((msg, start, end)) = runtime::native::take_fatal() {
                let span = diagnostics::Span { start, end };
                return Err(Diagnostic::new(Code::E108, format!("fatal: {msg}")).with_span(span));
            }
            if let Some(msg) = runtime::native::take_uncaught_message() {
                return Err(Diagnostic::new(Code::E108, format!("uncaught error: {msg}")));
            }
            Ok(v)
        }
    }

    #[allow(dead_code)]
    pub fn ir(&self) -> String {
        self.module.print_to_string().to_string()
    }
}

pub fn execute(lir: &Module, entry: &str) -> Result<i64, Diagnostic> {
    Ok(execute_with_map(lir, entry)?.0)
}

fn obj_local(lir: &Module, f: &Function, l: Local) -> Option<(usize, usize)> {
    match f.locals.get(l as usize) {
        Some(LirType::Obj(name)) => {
            let ci = *lir.class_index.get(name)?;
            Some((ci, l as usize))
        }
        _ => None,
    }
}

fn named_field(lir: &Module, f: &Function, obj: Local, field: &str) -> Option<(usize, usize)> {
    let (ci, _) = obj_local(lir, f, obj)?;
    let fi = *lir.classes[ci].field_index.get(field)?;
    Some((ci, fi))
}

fn mark_pure_runtime_fns(context: &Context, module: &LlModule) {
    use inkwell::attributes::{Attribute, AttributeLoc};
    let flag = |name: &str| context.create_enum_attribute(Attribute::get_named_enum_kind_id(name), 0);
    let nounwind = flag("nounwind");
    for name in [
        "rnx_array_len",
        "rnx_array_get",
        "rnx_array_get_unchecked",
        "rnx_array_set",
        "rnx_array_set_unchecked",
        "rnx_math_sqrt",
        "rnx_math_sin",
        "rnx_math_cos",
        "rnx_math_tan",
        "rnx_math_atan2",
        "rnx_math_pow",
        "rnx_math_floor",
        "rnx_math_ceil",
        "rnx_math_round",
        "rnx_math_log",
    ] {
        if let Some(fv) = module.get_function(name) {
            fv.add_attribute(AttributeLoc::Function, nounwind);
        }
    }
}

pub fn emit_object(
    lir: &Module,
    name: &str,
    entry: &str,
    opt: OptLevel,
    target: Option<&str>,
) -> Result<Vec<u8>, Diagnostic> {
    emit_object_with_debug(lir, name, entry, opt, target, None)
}

pub fn emit_object_with_debug(
    lir: &Module,
    name: &str,
    entry: &str,
    opt: OptLevel,
    target: Option<&str>,
    debug: Option<&DebugInfo>,
) -> Result<Vec<u8>, Diagnostic> {
    runtime::native::rnx_set_closure_epoch(0);
    for f in &lir.functions {
        check_supported(lir, f)?;
        if f.name == "main" && entry != "main" || f.name == "rnx_entry_main" || f.name == "rnx_init" {
            return Err(Diagnostic::new(
                Code::E108,
                format!("build reserves `{}`", f.name),
            ));
        }
    }
    let target_entry = lir
        .functions
        .iter()
        .find(|f| f.name == entry)
        .ok_or_else(|| Diagnostic::new(Code::E108, format!("unknown entry `{entry}`")))?;
    if !target_entry.params.is_empty() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("build entry `{entry}` takes no arguments"),
        ));
    }
    if matches!(target_entry.ret, LirType::Vec4f | LirType::Vec4i) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("build entry `{entry}` must return Int, not a vector"),
        ));
    }
    let context = Context::create();
    let release = opt == OptLevel::Release;
    let (module, funcs) = build_module(&context, lir, name, release, debug)?;
    let entry_fn = funcs[entry];
    if entry == "main" {
        entry_fn.as_global_value().set_name("__rnx_user_main");
    }
    let i64t = context.i64_type();
    let i32t = context.i32_type();
    let ptr_t = context.ptr_type(inkwell::AddressSpace::default());
    let wrapper = module.add_function("rnx_entry_main", i64t.fn_type(&[], false), None);
    let wb = context.create_builder();
    let wbb = context.append_basic_block(wrapper, "entry");
    wb.position_at_end(wbb);
    let site = wb.build_call(entry_fn, &[], "r").map_err(err)?;
    match site.try_as_basic_value() {
        ValueKind::Basic(v) => {
            wb.build_return(Some(&v)).map_err(err)?;
        }
        ValueKind::Instruction(_) => {
            return Err(Diagnostic::new(Code::E108, "build: entry returned void"));
        }
    }
    module.add_function("rnx_init", context.void_type().fn_type(&[i32t.into(), ptr_t.into()], false), None);
    let init = module.get_function("rnx_init").expect("declared");
    let strict_fn = module.add_function(
        "rnx_set_assert_strict",
        context.void_type().fn_type(&[context.bool_type().into()], false),
        None,
    );
    let reg_fn = module.get_function("rnx_closure_register").expect("declared");
    let clo_init = module.add_function("__rnx_closure_init", context.void_type().fn_type(&[], false), None);
    let cbb = context.append_basic_block(clo_init, "entry");
    wb.position_at_end(cbb);
    for (i, f) in lir.functions.iter().enumerate() {
        if !f.is_closure {
            continue;
        }
        let fv = funcs.get(&f.name).copied().ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("build: missing closure `{}`", f.name))
        })?;
        let addr = wb.build_ptr_to_int(fv.as_global_value().as_pointer_value(), i64t, "").map_err(err)?;
        let fid = i64t.const_int(runtime::native::rnx_closure_tag(i as u64), false);
        wb.build_call(reg_fn, &[fid.into(), addr.into()], "").map_err(err)?;
    }
    wb.build_return(None).map_err(err)?;
    let main_fn = module.add_function(
        "main",
        i32t.fn_type(&[i32t.into(), ptr_t.into()], false),
        None,
    );
    let mbb = context.append_basic_block(main_fn, "entry");
    wb.position_at_end(mbb);
    let mut main_params = main_fn.get_param_iter();
    let argc = main_params.next().map(|v| v.into_int_value());
    let argv = main_params.next().map(|v| v.into_pointer_value());
    match (argc, argv) {
        (Some(argc), Some(argv)) => {
            wb.build_call(init, &[argc.into(), argv.into()], "").map_err(err)?;
        }
        _ => {
            return Err(Diagnostic::new(Code::E108, "build: main needs (argc, argv)"));
        }
    }
    wb.build_call(clo_init, &[], "").map_err(err)?;
    let strict_on = context.bool_type().const_int(1, false);
    wb.build_call(strict_fn, &[strict_on.into()], "").map_err(err)?;
    let site = wb.build_call(wrapper, &[], "r").map_err(err)?;
    match site.try_as_basic_value() {
        ValueKind::Basic(v) => {
            let code = wb
                .build_int_truncate(v.into_int_value(), i32t, "code")
                .map_err(err)?;
            let take_fn = module.get_function("rnx_error_take").expect("declared");
            let report_fn = module.get_function("rnx_report_uncaught").expect("declared");
            let fatal_fn = module.get_function("rnx_report_fatal").expect("declared");
            let fatal_site = wb.build_call(fatal_fn, &[], "fatalrep").map_err(err)?;
            let fatal_code = match fatal_site.try_as_basic_value() {
                ValueKind::Basic(x) => x.into_int_value(),
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "build: fatal report returned void"));
                }
            };
            let has_fatal = wb.build_int_compare(
                inkwell::IntPredicate::NE,
                fatal_code,
                i32t.const_zero(),
                "hasfatal",
            ).map_err(err)?;
            let fatal_bb = context.append_basic_block(main_fn, "fatal");
            let nofatal_bb = context.append_basic_block(main_fn, "nofatal");
            wb.build_conditional_branch(has_fatal, fatal_bb, nofatal_bb).map_err(err)?;
            wb.position_at_end(fatal_bb);
            let one = i32t.const_int(1, false);
            wb.build_return(Some(&one)).map_err(err)?;
            wb.position_at_end(nofatal_bb);
            let take_site = wb.build_call(take_fn, &[], "errtake").map_err(err)?;
            let pending = match take_site.try_as_basic_value() {
                ValueKind::Basic(x) => x.into_int_value(),
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "build: error take returned void"));
                }
            };
            let has_err = wb.build_int_compare(
                inkwell::IntPredicate::NE,
                pending,
                i64t.const_zero(),
                "haserr",
            ).map_err(err)?;
            let err_bb = context.append_basic_block(main_fn, "uncaught");
            let ok_bb = context.append_basic_block(main_fn, "ok");
            wb.build_conditional_branch(has_err, err_bb, ok_bb).map_err(err)?;
            wb.position_at_end(err_bb);
            let rep_site = wb.build_call(report_fn, &[pending.into()], "rep").map_err(err)?;
            match rep_site.try_as_basic_value() {
                ValueKind::Basic(x) => {
                    wb.build_return(Some(&x)).map_err(err)?;
                }
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "build: report returned void"));
                }
            }
            wb.position_at_end(ok_bb);
            wb.build_return(Some(&code)).map_err(err)?;
        }
        ValueKind::Instruction(_) => {
            return Err(Diagnostic::new(Code::E108, "build: entry returned void"));
        }
    }
    if release {
        wrapper.set_section(Some(".text.rnx_entry_main"));
        main_fn.set_section(Some(".text.main"));
        init.set_section(Some(".text.rnx_init"));
        clo_init.set_section(Some(".text.rnx_closure_init"));
    }
    module.verify().map_err(|e| {
        Diagnostic::new(Code::E108, format!("llvm verify failed for `{name}`: {e}"))
    })?;
    finish_object(&module, name, opt, target)
}

fn cross_cpu(triple: &str) -> String {
    if triple.starts_with("x86_64") {
        "x86-64".to_string()
    } else {
        "generic".to_string()
    }
}

fn finish_object(
    module: &LlModule,
    name: &str,
    opt: OptLevel,
    triple: Option<&str>,
) -> Result<Vec<u8>, Diagnostic> {
    use inkwell::targets::{
        CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple,
    };
    use inkwell::OptimizationLevel;
    let release = opt == OptLevel::Release;
    Target::initialize_x86(&InitializationConfig::default());
    Target::initialize_aarch64(&InitializationConfig::default());
    let default_triple = TargetMachine::get_default_triple();
    let triple_ref = match triple {
        Some(t) => TargetTriple::create(t),
        None => default_triple,
    };
    let target = Target::from_triple(&triple_ref)
        .map_err(|e| Diagnostic::new(Code::E108, format!("llvm target: {e}")))?;
    let level = match opt {
        OptLevel::Dev => OptimizationLevel::None,
        OptLevel::Release => OptimizationLevel::Aggressive,
    };
    let (cpu, features) = match triple {
        Some(t) => (cross_cpu(t), String::new()),
        None => (
            TargetMachine::get_host_cpu_name().to_string(),
            TargetMachine::get_host_cpu_features().to_string(),
        ),
    };
    let machine = target
        .create_target_machine(
            &triple_ref,
            &cpu,
            &features,
            level,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| Diagnostic::new(Code::E108, "llvm: no target machine"))?;
    module.set_triple(&triple_ref);
    module.set_data_layout(&machine.get_target_data().get_data_layout());
    if release {
        use inkwell::passes::PassBuilderOptions;
        module
            .run_passes("default<O3>", &machine, PassBuilderOptions::create())
            .map_err(|e| Diagnostic::new(Code::E108, format!("llvm O3 passes: {e}")))?;
        module.verify().map_err(|e| {
            Diagnostic::new(Code::E108, format!("llvm verify after O3 for `{name}`: {e}"))
        })?;
    }
    let buffer = machine
        .write_to_memory_buffer(&module, FileType::Object)
        .map_err(|e| Diagnostic::new(Code::E108, format!("llvm object emit: {e}")))?;
    Ok(buffer.as_slice().to_vec())
}

pub fn emit_library(lir: &Module, name: &str, opt: OptLevel, target: Option<&str>) -> Result<Vec<u8>, Diagnostic> {
    emit_library_with_debug(lir, name, opt, target, None)
}

pub fn emit_library_with_debug(lir: &Module, name: &str, opt: OptLevel, target: Option<&str>, debug: Option<&DebugInfo>) -> Result<Vec<u8>, Diagnostic> {
    runtime::native::rnx_set_closure_epoch(0);
    for f in &lir.functions {
        check_supported(lir, f)?;
        if f.name == "main" || f.name == "rnx_entry_main" || f.name == "rnx_init" {
            return Err(Diagnostic::new(
                Code::E108,
                format!("build reserves `{}`", f.name),
            ));
        }
    }
    let release = opt == OptLevel::Release;
    let context = Context::create();
    let (module, funcs) = build_module(&context, lir, name, release, debug)?;
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (fi, f) in lir.functions.iter().enumerate() {
        if !f.is_pub {
            continue;
        }
        let short = f.name.rsplit('.').next().unwrap_or(&f.name).to_string();
        if seen.insert(short.clone(), fi).is_some() {
            return Err(Diagnostic::new(
                Code::E108,
                format!("duplicate C export `{short}`"),
            ));
        }
        validate_c_export(f, &short)?;
        let internal = funcs[&f.name];
        let sanitized: String =
            short.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        internal.as_global_value().set_name(&format!("__rnx_impl_{fi}_{sanitized}"));
        internal.set_linkage(inkwell::module::Linkage::Internal);
        emit_c_wrapper(&context, &module, internal, &short, &f.params, &f.ret, release)?;
    }
    for (fi, f) in lir.functions.iter().enumerate() {
        if f.is_pub {
            continue;
        }
        if let Some(fv) = funcs.get(&f.name) {
            let _ = fi;
            fv.set_linkage(inkwell::module::Linkage::Internal);
        }
    }
    module.verify().map_err(|e| {
        Diagnostic::new(Code::E108, format!("llvm verify failed for `{name}`: {e}"))
    })?;
    finish_object(&module, name, opt, target)
}

fn validate_c_export(f: &Function, short: &str) -> Result<(), Diagnostic> {
    if f.throws {
        return Err(Diagnostic::new(
            Code::E108,
            format!("pub fn `{short}` cannot throw in --lib mode"),
        ));
    }
    if f.method_self {
        return Err(Diagnostic::new(
            Code::E108,
            format!("pub methods cannot be C exports (`{short}`)"),
        ));
    }
    for (i, p) in f.params.iter().enumerate() {
        if !matches!(p, LirType::I64 | LirType::I8 | LirType::F64(_) | LirType::Bool | LirType::Str) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("pub fn `{short}` param {i} has non-C-ABI type"),
            ));
        }
    }
    if !matches!(f.ret, LirType::I64 | LirType::I8 | LirType::F64(_) | LirType::Bool | LirType::Void) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("pub fn `{short}` has non-C-ABI return type"),
        ));
    }
    Ok(())
}

fn emit_c_wrapper<'ctx>(
    context: &'ctx Context,
    module: &LlModule<'ctx>,
    internal: FunctionValue<'ctx>,
    short: &str,
    params: &[LirType],
    ret: &LirType,
    release: bool,
) -> Result<(), Diagnostic> {
    let i64t = context.i64_type();
    let f64t = context.f64_type();
    let i8t = context.i8_type();
    let ptr_t = context.ptr_type(inkwell::AddressSpace::default());
    let void_t = context.void_type();
    let mut cparams: Vec<BasicMetadataTypeEnum> = Vec::with_capacity(params.len());
    for p in params {
        cparams.push(match p {
            LirType::I64 | LirType::I8 => i64t.into(),
            LirType::F64(_) => f64t.into(),
            LirType::Bool => i8t.into(),
            LirType::Str => ptr_t.into(),
            _ => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("pub fn `{short}` has non-C-ABI param"),
                ));
            }
        });
    }
    let cty = match ret {
        LirType::I64 | LirType::I8 => i64t.fn_type(&cparams, false),
        LirType::F64(_) => f64t.fn_type(&cparams, false),
        LirType::Bool => i8t.fn_type(&cparams, false),
        LirType::Void => void_t.fn_type(&cparams, false),
        _ => {
            return Err(Diagnostic::new(
                Code::E108,
                format!("pub fn `{short}` has non-C-ABI return"),
            ));
        }
    };
    let wrap = module.add_function(short, cty, None);
    if release {
        wrap.set_section(Some(&format!(".text.{short}")));
    }
    let from_cstr = if params.iter().any(|p| matches!(p, LirType::Str)) {
        Some(module.add_function(
            "rnx_string_from_cstr",
            ptr_t.fn_type(&[ptr_t.into()], false),
            None,
        ))
    } else {
        None
    };
    let release_str = if from_cstr.is_some() {
        Some(module.add_function("rnx_release_str", void_t.fn_type(&[ptr_t.into()], false), None))
    } else {
        None
    };
    let builder = context.create_builder();
    let bb = context.append_basic_block(wrap, "entry");
    builder.position_at_end(bb);
    let mut args: Vec<BasicMetadataValueEnum> = Vec::with_capacity(params.len());
    let mut strings: Vec<PointerValue> = Vec::new();
    for (i, p) in params.iter().enumerate() {
        let pv = wrap.get_nth_param(i as u32).ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("pub fn `{short}` missing param {i}"))
        })?;
        match p {
            LirType::I64 | LirType::I8 => args.push(pv.into()),
            LirType::F64(_) => {
                let b = builder.build_bit_cast(pv.into_float_value(), i64t, "b").map_err(err)?;
                args.push(b.into());
            }
            LirType::Bool => {
                let w =
                    builder.build_int_z_extend(pv.into_int_value(), i64t, "w").map_err(err)?;
                args.push(w.into());
            }
            LirType::Str => {
                let site = builder
                    .build_call(from_cstr.expect("str converter"), &[pv.into()], "")
                    .map_err(err)?;
                let ptr = match site.try_as_basic_value() {
                    ValueKind::Basic(v) => v.into_pointer_value(),
                    ValueKind::Instruction(_) => {
                        return Err(Diagnostic::new(
                            Code::E108,
                            "llvm subset: string convert returned void",
                        ));
                    }
                };
                let n = builder.build_ptr_to_int(ptr, i64t, "s").map_err(err)?;
                args.push(n.into());
                strings.push(ptr);
            }
            _ => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("pub fn `{short}` has non-C-ABI param"),
                ));
            }
        }
    }
    let site = builder.build_call(internal, &args, "r").map_err(err)?;
    if let Some(free) = release_str {
        for s in &strings {
            builder.build_call(free, &[(*s).into()], "").map_err(err)?;
        }
    }
    match ret {
        LirType::I64 | LirType::I8 => {
            let v = match site.try_as_basic_value() {
                ValueKind::Basic(v) => v.into_int_value(),
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: call returned void"));
                }
            };
            builder.build_return(Some(&v)).map_err(err)?;
        }
        LirType::F64(_) => {
            let v = match site.try_as_basic_value() {
                ValueKind::Basic(v) => v.into_int_value(),
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: call returned void"));
                }
            };
            let d = builder.build_bit_cast(v, f64t, "d").map_err(err)?;
            builder.build_return(Some(&d)).map_err(err)?;
        }
        LirType::Bool => {
            let v = match site.try_as_basic_value() {
                ValueKind::Basic(v) => v.into_int_value(),
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: call returned void"));
                }
            };
            let b = builder.build_int_truncate(v, i8t, "b").map_err(err)?;
            builder.build_return(Some(&b)).map_err(err)?;
        }
        _ => {
            builder.build_return(None).map_err(err)?;
        }
    }
    Ok(())
}

fn zero_value<'a>(context: &'a Context, ty: BasicTypeEnum<'a>) -> Result<BasicValueEnum<'a>, Diagnostic> {
    match ty {
        BasicTypeEnum::IntType(t) => Ok(t.const_zero().into()),
        BasicTypeEnum::FloatType(t) => Ok(t.const_zero().into()),
        BasicTypeEnum::PointerType(t) => Ok(t.const_null().into()),
        BasicTypeEnum::VectorType(t) => Ok(t.const_zero().into()),
        BasicTypeEnum::StructType(t) => {
            let mut vals = Vec::with_capacity(t.count_fields() as usize);
            for ft in t.get_field_types() {
                vals.push(zero_value(context, ft)?);
            }
            Ok(context.const_struct(&vals, false).into())
        }
        BasicTypeEnum::ArrayType(t) => Ok(t.const_zero().into()),
        BasicTypeEnum::ScalableVectorType(_) => Err(Diagnostic::new(
            Code::E108,
            "llvm subset: scalable vector trap return",
        )),
    }
}

fn loop_unroll_id<'a>(cx: &'a FnCx<'a>, count: u32) -> inkwell::values::MetadataValue<'a> {
    use inkwell::values::BasicMetadataValueEnum as M;
    let i32t = cx.context.i32_type();
    let hint = cx.context.metadata_node(&[
        M::MetadataValue(cx.context.metadata_string("llvm.loop.unroll.count")),
        M::IntValue(i32t.const_int(count as u64, false)),
    ]);
    unsafe {
        use inkwell::context::AsContextRef;
        use inkwell::values::AsValueRef;
        let ctx_ref = cx.context.as_ctx_ref();
        let hint_ref = llvm_sys::core::LLVMValueAsMetadata(hint.as_value_ref());
        let mut alone = [hint_ref];
        let temp = llvm_sys::debuginfo::LLVMTemporaryMDNode(ctx_ref, alone.as_mut_ptr(), alone.len());
        let mut pair = [temp, hint_ref];
        let node = llvm_sys::core::LLVMMDNodeInContext2(ctx_ref, pair.as_mut_ptr(), pair.len());
        llvm_sys::debuginfo::LLVMMetadataReplaceAllUsesWith(temp, node);
        let val = llvm_sys::core::LLVMMetadataAsValue(ctx_ref, node);
        inkwell::values::MetadataValue::new(val)
    }
}

fn tag_loop_unrolls(cx: &FnCx, f: &lir::instr::Function) -> Result<(), Diagnostic> {
    let loops = lir::licm::find_natural_loops(f, &cx.dom);
    for lp in &loops {
        let mut calls = false;
        for &bi in lp.blocks.iter() {
            for ins in &f.blocks[bi].instrs {
                if matches!(ins, Instr::Call { .. }) {
                    calls = true;
                    break;
                }
            }
            if calls {
                break;
            }
        }
        if calls {
            continue;
        }
        let id = loop_unroll_id(cx, 4);
        let kind = cx.context.get_kind_id("llvm.loop");
        for &bi in lp.blocks.iter() {
            let is_latch = match &f.blocks[bi].term {
                Terminator::Br(t) => *t == lp.header,
                Terminator::BrIf { then_bb, else_bb, .. } => *then_bb == lp.header || *else_bb == lp.header,
                _ => false,
            };
            if !is_latch {
                continue;
            }
            if let Some(term) = cx.blocks[bi].get_terminator() {
                term.set_metadata(id, kind)
                    .map_err(|e| Diagnostic::new(Code::E108, format!("llvm build: {e}")))?;
            }
        }
    }
    Ok(())
}

fn emit_run_defers(cx: &mut FnCx, lir: &Module, keep: usize) -> Result<(), Diagnostic> {
    if cx.defers.is_empty() {
        return Ok(());
    }
    let base_slot = cx
        .defer_base
        .ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: defer without base"))?;
    let bodies = cx.defers.clone();
    let i64t = cx.context.i64_type();
    let func = cx
        .builder
        .get_insert_block()
        .and_then(|bb| bb.get_parent())
        .ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: defer outside block"))?;
    let head = cx.context.append_basic_block(func, "defer_head");
    let pop_bb = cx.context.append_basic_block(func, "defer_pop");
    let done = cx.context.append_basic_block(func, "defer_done");
    let resume_next = cx.context.append_basic_block(func, "defer_resume");
    let mut dispatch: Vec<inkwell::basic_block::BasicBlock> = Vec::with_capacity(bodies.len());
    let mut work: Vec<inkwell::basic_block::BasicBlock> = Vec::with_capacity(bodies.len());
    for (n, _) in bodies.iter().enumerate() {
        dispatch.push(cx.context.append_basic_block(func, &format!("defer_chk{n}")));
        work.push(cx.context.append_basic_block(func, &format!("defer_do{n}")));
    }
    cx.builder.build_unconditional_branch(head).map_err(err)?;
    cx.builder.position_at_end(head);
    let site = cx.builder.build_call(cx.defer_len, &[], "").map_err(err)?;
    let len = match site.try_as_basic_value() {
        ValueKind::Basic(v) => v.into_int_value(),
        ValueKind::Instruction(_) => {
            return Err(Diagnostic::new(Code::E108, "llvm subset: defer len"));
        }
    };
    let base: inkwell::values::IntValue = cx
        .builder
        .build_load(i64t, base_slot, "defer_base")
        .map_err(err)?
        .into_int_value();
    let target = cx
        .builder
        .build_int_add(base, i64t.const_int(keep as u64, false), "defer_target")
        .map_err(err)?;
    let finished = cx
        .builder
        .build_int_compare(inkwell::IntPredicate::ULE, len, target, "defer_finished")
        .map_err(err)?;
    cx.builder
        .build_conditional_branch(finished, done, pop_bb)
        .map_err(err)?;
    cx.builder.position_at_end(pop_bb);
    let site = cx.builder.build_call(cx.defer_pop, &[], "").map_err(err)?;
    let id = match site.try_as_basic_value() {
        ValueKind::Basic(v) => v.into_int_value(),
        ValueKind::Instruction(_) => {
            return Err(Diagnostic::new(Code::E108, "llvm subset: defer pop"));
        }
    };
    cx.builder
        .build_unconditional_branch(dispatch[0])
        .map_err(err)?;
    for (n, body) in bodies.iter().enumerate() {
        cx.builder.position_at_end(dispatch[n]);
        let want = i64t.const_int(n as u64, false);
        let is_mine = cx
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, id, want, "defer_mine")
            .map_err(err)?;
        let next = if n + 1 < dispatch.len() { dispatch[n + 1] } else { done };
        cx.builder
            .build_conditional_branch(is_mine, work[n], next)
            .map_err(err)?;
        cx.builder.position_at_end(work[n]);
        for ins in body {
            lower_instr(cx, lir, ins, "defer")?;
        }
        cx.builder.build_unconditional_branch(head).map_err(err)?;
    }
    cx.builder.position_at_end(done);
    cx.builder.build_unconditional_branch(resume_next).map_err(err)?;
    cx.builder.position_at_end(resume_next);
    Ok(())
}

fn err<E: std::fmt::Display>(e: E) -> Diagnostic {
    Diagnostic::new(Code::E108, format!("llvm build: {e}"))
}

fn load<'a>(cx: &FnCx<'a>, l: Local) -> Result<IntValue<'a>, Diagnostic> {
    cx.builder
        .build_load(cx.context.i64_type(), cx.locals[l as usize], "v")
        .map_err(err)
        .map(|v| v.into_int_value())
}

fn load_value<'a>(cx: &FnCx<'a>, l: Local) -> Result<BasicValueEnum<'a>, Diagnostic> {
    match cx.ftypes.get(l as usize) {
        Some(LirType::Vec4f) | Some(LirType::Vec4i) => {
            load_vec(cx, l).map(|v| v.into())
        }
        _ => load(cx, l).map(|v| v.into()),
    }
}

fn double_of<'a>(cx: &FnCx<'a>, l: Local) -> Result<FloatValue<'a>, Diagnostic> {
    let bits = load(cx, l)?;
    let f64t = cx.context.f64_type();
    match cx.ftypes.get(l as usize) {
        Some(LirType::F64(_)) => cx
            .builder
            .build_bit_cast(bits, f64t, "f")
            .map_err(err)
            .map(|v| v.into_float_value()),
        _ => cx.builder.build_signed_int_to_float(bits, f64t, "f").map_err(err),
    }
}

fn store_double(cx: &FnCx, dst: Local, v: FloatValue) -> Result<(), Diagnostic> {
    let bits = cx
        .builder
        .build_bit_cast(v, cx.context.i64_type(), "b")
        .map_err(err)?
        .into_int_value();
    store(cx, dst, bits.into())
}

fn fast_flags(_cx: &FnCx, v: FloatValue, fast: bool) -> Result<(), Diagnostic> {
    if !fast {
        return Ok(());
    }
    if let Some(inst) = v.as_instruction() {
        use inkwell::values::FastMathFlags;
        // The relaxed set spec 04_NUMERICS promises for FastFloat: nnan
        // ninf nsz arcp contract reassoc. ApproxFunc stays excluded (it
        // permits approximate transcendentals, forking backends). Two
        // caveats learned from post-O3 inspection: (1) arcp folds `x/C`
        // to `x*(1/C)` and reassoc rewrites fadd chains, both LLVM-only,
        // so exact-checksum FastFloat workloads must be canonicalized in
        // LIR first (Fma synthesis plus add-of-sub reassociation below);
        // backends then lower literally and agree bitwise. (2) Strict
        // `Float` never carries these flags, so strict prologues feeding
        // a FastFloat loop stay bit-identical everywhere.
        let all = FastMathFlags::AllowReassoc
            | FastMathFlags::NoNaNs
            | FastMathFlags::NoInfs
            | FastMathFlags::NoSignedZeros
            | FastMathFlags::AllowReciprocal
            | FastMathFlags::AllowContract;
        inst.set_fast_math_flags(all)
            .map_err(|e| Diagnostic::new(Code::E108, format!("llvm build: {e}")))?;
    }
    Ok(())
}

fn store(cx: &FnCx, dst: Local, v: BasicValueEnum) -> Result<(), Diagnostic> {
    cx.builder.build_store(cx.locals[dst as usize], v).map_err(err)?;
    cx.last_write.borrow_mut().insert(dst, cx.cur_block);
    Ok(())
}

fn write_dominates(cx: &FnCx, dst: Local) -> bool {
    cx.last_write.borrow().get(&dst).is_some_and(|w| cx.dom.dominates(*w, cx.cur_block))
}

fn obj_size(lir: &Module, cx: &FnCx, l: Local) -> Result<u64, Diagnostic> {
    match cx.ftypes.get(l as usize) {
        Some(LirType::Obj(name)) => match lir.class_index.get(name) {
            Some(ci) => Ok(instance_size(lir.classes[*ci].fields.len()) as u64),
            None => Err(Diagnostic::new(Code::E108, "release of unknown class")),
        },
        Some(LirType::Enum(ei)) => match lir.enums.get(*ei) {
            Some(d) => Ok(enum_instance_size(d) as u64),
            None => Err(Diagnostic::new(Code::E108, "release of unknown enum")),
        },
        _ => Err(Diagnostic::new(Code::E108, "release of non-object")),
    }
}

fn own(cx: &mut FnCx, l: Local) {
    if (l as usize) < cx.nborrowed {
        return;
    }
    cx.owned.insert(l);
    cx.ever_owned.insert(l);
}

fn any_retain_val(cx: &FnCx, v: inkwell::values::IntValue) -> Result<(), Diagnostic> {
    cx.builder.build_call(cx.any_retain, &[v.into()], "").map_err(err)?;
    Ok(())
}

fn track_heap(
    cx: &FnCx,
    ptr: inkwell::values::IntValue,
    kind: u64,
    aux1: inkwell::values::IntValue,
    aux2: inkwell::values::IntValue,
) -> Result<(), Diagnostic> {
    let k = cx.context.i64_type().const_int(kind, false);
    cx.builder
        .build_call(cx.heap_track, &[ptr.into(), k.into(), aux1.into(), aux2.into()], "")
        .map_err(err)?;
    Ok(())
}

// True when a local's static type is an object whose class metadata is
// missing: an erased type parameter (bare `T`/`U` in generic code). The
// slot still holds the uniform value representation, so ownership must go
// through the tag-dispatched any_retain/any_release pair, never the raw
// class retain/release (those abort on inline values and misread boxes).
fn erased_obj(lir: &Module, cx: &FnCx, l: Local) -> bool {
    match cx.ftypes.get(l as usize) {
        Some(LirType::Obj(name)) => !lir.class_index.contains_key(name),
        _ => false,
    }
}

fn retain_val(cx: &FnCx, lir: &Module, l: Local, v: inkwell::values::IntValue) -> Result<(), Diagnostic> {
    if erased_obj(lir, cx, l) {
        any_retain_val(cx, v)
    } else {
        let ptr = as_ptr(cx, v)?;
        cx.builder.build_call(cx.retain, &[ptr.into()], "").map_err(err)?;
        Ok(())
    }
}

fn any_release_val(cx: &FnCx, v: inkwell::values::IntValue) -> Result<(), Diagnostic> {
    cx.builder.build_call(cx.any_release, &[v.into()], "").map_err(err)?;
    Ok(())
}

fn release_call_args(
    cx: &mut FnCx,
    lir: &Module,
    callee: Option<usize>,
    args: &[Local],
    dsts: &[Local],
    err: Option<Local>,
) -> Result<(), Diagnostic> {
    if let Some(id) = callee {
        if lir::temp_sweep::callee_may_consume(lir, id) {
            return Ok(());
        }
    } else {
        return Ok(());
    }
    for a in args.iter() {
        if !matches!(cx.ftypes.get(*a as usize), Some(LirType::Any)) {
            continue;
        }
        if dsts.contains(a) || err == Some(*a) {
            continue;
        }
        let v = load(cx, *a)?;
        any_release_val(cx, v)?;
    }
    Ok(())
}

fn as_ptr<'a>(cx: &FnCx<'a>, v: IntValue<'a>) -> Result<inkwell::values::PointerValue<'a>, Diagnostic> {
    cx.builder.build_int_to_ptr(v, cx.context.ptr_type(inkwell::AddressSpace::default()), "").map_err(err)
}

fn local_alloca_ty<'a>(context: &'a Context, t: &LirType) -> BasicTypeEnum<'a> {
    match t {
        LirType::Vec4f => context.f32_type().vec_type(4).into(),
        LirType::Vec4i => context.i32_type().vec_type(4).into(),
        _ => context.i64_type().into(),
    }
}

fn load_vec<'a>(cx: &FnCx<'a>, l: Local) -> Result<VectorValue<'a>, Diagnostic> {
    let ty = match cx.ftypes.get(l as usize) {
        Some(LirType::Vec4f) => cx.context.f32_type().vec_type(4),
        Some(LirType::Vec4i) => cx.context.i32_type().vec_type(4),
        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: vector load of scalar")),
    };
    cx.builder
        .build_load(ty, cx.locals[l as usize], "v")
        .map_err(err)
        .map(|v| v.into_vector_value())
}

fn f32_of_lane<'a>(cx: &FnCx<'a>, l: Local) -> Result<FloatValue<'a>, Diagnostic> {
    let d = double_of(cx, l)?;
    cx.builder.build_float_trunc(d, cx.context.f32_type(), "t").map_err(err)
}

fn store_f32(cx: &FnCx, dst: Local, v: FloatValue) -> Result<(), Diagnostic> {
    let d = cx.builder.build_float_ext(v, cx.context.f64_type(), "e").map_err(err)?;
    store_double(cx, dst, d)
}

fn i32_of_lane<'a>(cx: &FnCx<'a>, l: Local) -> Result<IntValue<'a>, Diagnostic> {
    let v = load(cx, l)?;
    cx.builder.build_int_truncate(v, cx.context.i32_type(), "t").map_err(err)
}

fn store_i32(cx: &FnCx, dst: Local, v: IntValue) -> Result<(), Diagnostic> {
    let w = cx.builder.build_int_s_extend(v, cx.context.i64_type(), "e").map_err(err)?;
    store(cx, dst, w.into())
}

fn str_addr<'a>(cx: &FnCx<'a>, text: &str) -> Result<IntValue<'a>, Diagnostic> {
    let gv = cx.strings.get(text).copied().ok_or_else(|| {
        Diagnostic::new(Code::E108, "llvm subset: missing static str")
    })?;
    cx.builder
        .build_ptr_to_int(gv.as_pointer_value(), cx.context.i64_type(), "")
        .map_err(err)
}

fn release_str_val(cx: &FnCx, v: IntValue) -> Result<(), Diagnostic> {
    let ptr = as_ptr(cx, v)?;
    cx.builder.build_call(cx.release_str, &[ptr.into()], "").map_err(err)?;
    Ok(())
}

fn guard_int<'a>(cx: &FnCx<'a>, ptr: IntValue<'a>, keep: Option<IntValue<'a>>) -> Result<IntValue<'a>, Diagnostic> {
    match keep {
        None => Ok(ptr),
        Some(k) => {
            let same = cx.builder.build_int_compare(IntPredicate::EQ, ptr, k, "alias").map_err(err)?;
            let zero = cx.context.i64_type().const_zero();
            cx.builder.build_select(same, zero, ptr, "unguarded").map_err(err).map(|v| v.into_int_value())
        }
    }
}

fn release_any(cx: &mut FnCx, lir: &Module, l: Local, keep: Option<IntValue>) -> Result<(), Diagnostic> {
    match cx.ftypes.get(l as usize).cloned() {
        Some(LirType::Closure) => {
            let base = load(cx, l)?;
            let raw = guard_int(cx, base, keep)?;
            let ptr = as_ptr(cx, raw)?;
            cx.builder.build_call(cx.closure_release, &[ptr.into()], "").map_err(err)?;
            cx.owned.remove(&l);
            cx.ever_owned.remove(&l);
            Ok(())
        }
        Some(LirType::Str) => {
            let base = load(cx, l)?;
            release_str_val(cx, guard_int(cx, base, keep)?)?;
            cx.owned.remove(&l);
            cx.ever_owned.remove(&l);
            Ok(())
        }
        Some(LirType::Error) => {
            let base = load(cx, l)?;
            let guarded = guard_int(cx, base, keep)?;
            cx.builder.build_call(cx.error_release, &[guarded.into()], "").map_err(err)?;
            cx.owned.remove(&l);
            cx.ever_owned.remove(&l);
            Ok(())
        }
        Some(LirType::Any) => {
            let base = load(cx, l)?;
            any_release_val(cx, guard_int(cx, base, keep)?)?;
            cx.owned.remove(&l);
            cx.ever_owned.remove(&l);
            Ok(())
        }
        Some(LirType::Array(inner)) => {
            let base = load(cx, l)?;
            release_array_val(cx, lir, guard_int(cx, base, keep)?, &inner)?;
            cx.owned.remove(&l);
            cx.ever_owned.remove(&l);
            Ok(())
        }
        Some(LirType::Enum(ei)) => {
            let size = match lir.enums.get(ei) {
                Some(d) => enum_instance_size(d) as u64,
                None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown enum")),
            };
            let raw = load(cx, l)?;
            let ptr = as_ptr(cx, guard_int(cx, raw, keep)?)?;
            let n = cx.context.i64_type().const_int(size, false);
            let dtor = enum_dtor_ptr(cx, lir, ei)?;
            cx.builder.build_call(cx.release, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
            cx.owned.remove(&l);
            cx.ever_owned.remove(&l);
            Ok(())
        }
        Some(LirType::Obj(name)) if lir.class_index.get(&name).is_none() => {
            // Erased type parameter (e.g. bare `T` in generic prelude code):
            // no class metadata exists, but the slot holds the uniform
            // value representation, so tag-dispatched release is exact for
            // boxed/inline values and a no-op leak for raw heap objects.
            // Must not error here: the interpreter frees these exactly.
            let base = load(cx, l)?;
            any_release_val(cx, guard_int(cx, base, keep)?)?;
            cx.owned.remove(&l);
            cx.ever_owned.remove(&l);
            Ok(())
        }
        _ => emit_release(cx, lir, l, keep),
    }
}

fn llvm_array_capturable(inner: &LirType) -> bool {
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

fn array_elem_dtor<'a>(cx: &FnCx<'a>, lir: &Module, elem: &LirType) -> Result<inkwell::values::PointerValue<'a>, Diagnostic> {
    let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
    match elem {
        LirType::Obj(name) => match lir.class_index.get(name) {
            Some(fci) => match lir.classes[*fci].dtor {
                Some(fid) => {
                    let fname = &lir.functions[fid].name;
                    let fv = cx.funcs.get(fname).copied().ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("llvm subset: unknown dtor `{fname}`"))
                    })?;
                    Ok(fv.as_global_value().as_pointer_value())
                }
                None => Ok(ptr_t.const_null()),
            },
            None => Ok(ptr_t.const_null()),
        },
        LirType::Str => Ok(cx.release_str.as_global_value().as_pointer_value()),
        LirType::Enum(ei) => enum_dtor_ptr(cx, lir, *ei),
        LirType::Any => Ok(cx.any_release.as_global_value().as_pointer_value()),
        LirType::Array(_) => {
            let key = lir::instr::type_key(elem);
            match lir.array_dtors.get(&key) {
                Some(fid) => {
                    let fname = &lir.functions[*fid].name;
                    let fv = cx.funcs.get(fname).copied().ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("llvm subset: unknown array dtor `{fname}`"))
                    })?;
                    Ok(fv.as_global_value().as_pointer_value())
                }
                None => Err(Diagnostic::new(Code::E108, "llvm subset: missing array dtor")),
            }
        }
        _ => Ok(ptr_t.const_null()),
    }
}

fn release_array_val(cx: &FnCx, lir: &Module, base: IntValue, elem: &LirType) -> Result<(), Diagnostic> {
    let ptr = as_ptr(cx, base)?;
    let n = cx.context.i64_type().const_int(lir::instr::elem_size(elem) as u64, false);
    let dtor = array_elem_dtor(cx, lir, elem)?;
    cx.builder.build_call(cx.release_array, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArrElem {
    I64,
    F64,
}

fn array_inline_elem(cx: &FnCx, arr: Local, elem_size: usize) -> Option<ArrElem> {
    if elem_size != 8 {
        return None;
    }
    match cx.ftypes.get(arr as usize) {
        Some(LirType::Array(inner)) => match **inner {
            LirType::I64 | LirType::I8 => Some(ArrElem::I64),
            LirType::F64(_) => Some(ArrElem::F64),
            _ => None,
        },
        _ => None,
    }
}

fn tag_last(cx: &FnCx, tag: inkwell::values::MetadataValue) {
    if let Some(inst) = cx
        .builder
        .get_insert_block()
        .and_then(|b| b.get_last_instruction())
    {
        let _ = inst.set_metadata(tag, cx.tbaa_kind);
    }
}

fn array_len_value<'a>(cx: &'a FnCx<'a>, ap: PointerValue<'a>) -> Result<IntValue<'a>, Diagnostic> {
    let i64t = cx.context.i64_type();
    let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
    let base = cx.builder.build_ptr_to_int(ap, i64t, "").map_err(err)?;
    let addr = cx.builder.build_int_add(base, i64t.const_int(16, false), "").map_err(err)?;
    let field = cx.builder.build_int_to_ptr(addr, ptr_t, "").map_err(err)?;
    let v = cx.builder.build_load(i64t, field, "").map_err(err)?.into_int_value();
    tag_last(cx, cx.tbaa_header);
    Ok(v)
}

fn array_data_ptr<'a>(cx: &'a FnCx<'a>, ap: PointerValue<'a>) -> Result<PointerValue<'a>, Diagnostic> {
    let i64t = cx.context.i64_type();
    let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
    let base = cx.builder.build_ptr_to_int(ap, i64t, "").map_err(err)?;
    let addr = cx.builder.build_int_add(base, i64t.const_int(32, false), "").map_err(err)?;
    let field = cx.builder.build_int_to_ptr(addr, ptr_t, "").map_err(err)?;
    let data = cx.builder.build_load(i64t, field, "").map_err(err)?.into_int_value();
    tag_last(cx, cx.tbaa_header);
    cx.builder.build_int_to_ptr(data, ptr_t, "").map_err(err)
}

fn array_elem_ptr<'a>(
    cx: &'a FnCx<'a>,
    datap: PointerValue<'a>,
    idx: IntValue<'a>,
) -> Result<PointerValue<'a>, Diagnostic> {
    let i64t = cx.context.i64_type();
    let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
    let base = cx.builder.build_ptr_to_int(datap, i64t, "").map_err(err)?;
    let off = cx.builder.build_int_mul(idx, i64t.const_int(8, false), "").map_err(err)?;
    let addr = cx.builder.build_int_add(base, off, "").map_err(err)?;
    cx.builder.build_int_to_ptr(addr, ptr_t, "").map_err(err)
}

fn cur_blocks<'x>(
    cx: &FnCx<'x>,
    slow: &str,
    fast: &str,
    join: &str,
) -> Result<
    (
        inkwell::basic_block::BasicBlock<'x>,
        inkwell::basic_block::BasicBlock<'x>,
        inkwell::basic_block::BasicBlock<'x>,
    ),
    Diagnostic,
> {
    let fv = cx
        .builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: no insert function"))?;
    Ok((
        cx.context.append_basic_block(fv, slow),
        cx.context.append_basic_block(fv, fast),
        cx.context.append_basic_block(fv, join),
    ))
}

fn inline_array_len(cx: &FnCx, span: diagnostics::Span, arr: Local, dst: Local) -> Result<(), Diagnostic> {
    let i64t = cx.context.i64_type();
    let a = load(cx, arr)?;
    trap_if_null(cx, span, "len of null", a)?;
    let ap = as_ptr(cx, a)?;
    let raw = array_len_value(cx, ap)?;
    let isnull = cx.builder.build_is_null(ap, "").map_err(err)?;
    let len = cx
        .builder
        .build_select(isnull, i64t.const_zero(), raw, "")
        .map_err(err)?;
    store(cx, dst, len)
}

fn trap_if_null(cx: &FnCx, span: diagnostics::Span, msg: &str, recv: IntValue) -> Result<(), Diagnostic> {
    let i64t = cx.context.i64_type();
    let zero = i64t.const_zero();
    let is_null = cx.builder.build_int_compare(IntPredicate::EQ, recv, zero, "nullrecv").map_err(err)?;
    let fv = cx.builder.get_insert_block().and_then(|b| b.get_parent()).ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: no insert function"))?;
    let trap_bb = cx.context.append_basic_block(fv, "null_recv");
    let cont_bb = cx.context.append_basic_block(fv, "recv_ok");
    cx.builder.build_conditional_branch(is_null, trap_bb, cont_bb).map_err(err)?;
    cx.builder.position_at_end(trap_bb);
    let m = str_addr(cx, msg)?;
    let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
    let p = cx.builder.build_int_to_ptr(m, ptr_t, "").map_err(err)?;
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
    Ok(())
}

fn inline_array_get<'x>(
    cx: &mut FnCx<'x>,
    lir: &Module,
    arr: Local,
    index: Local,
    dst: Local,
    elem: ArrElem,
    checked: bool,
) -> Result<(), Diagnostic> {
    let i64t = cx.context.i64_type();
    let a = load(cx, arr)?;
    let ap = as_ptr(cx, a)?;
    let i = load(cx, index)?;
    let datap = array_data_ptr(cx, ap)?;
    let ep = array_elem_ptr(cx, datap, i)?;
    let fast_bits = match elem {
        ArrElem::I64 => {
            let v = cx.builder.build_load(i64t, ep, "").map_err(err)?.into_int_value();
            tag_last(cx, cx.tbaa_payload);
            v
        }
        ArrElem::F64 => {
            let d = cx
                .builder
                .build_load(cx.context.f64_type(), ep, "")
                .map_err(err)?
                .into_float_value();
            tag_last(cx, cx.tbaa_payload);
            cx.builder.build_bit_cast(d, i64t, "").map_err(err)?.into_int_value()
        }
    };
    if !checked {
        store(cx, dst, fast_bits.into())?;
        return Ok(());
    }
    let len = array_len_value(cx, ap)?;
    let inrange = cx.builder.build_int_compare(IntPredicate::ULT, i, len, "").map_err(err)?;
    let notnull = cx.builder.build_not(cx.builder.build_is_null(ap, "").map_err(err)?, "").map_err(err)?;
    let ok = cx.builder.build_and(notnull, inrange, "").map_err(err)?;
    let (slow_bb, fast_bb, join_bb) = cur_blocks(cx, "ag_slow", "ag_fast", "ag_join")?;
    cx.builder.build_conditional_branch(ok, fast_bb, slow_bb).map_err(err)?;
    cx.builder.position_at_end(slow_bb);
    let e = i64t.const_int(8, false);
    let site = cx.builder.build_call(cx.array_get, &[ap.into(), i.into(), e.into()], "").map_err(err)?;
    let slow_bits = match site.try_as_basic_value() {
        ValueKind::Basic(v) => v.into_int_value(),
        ValueKind::Instruction(_) => {
            return Err(Diagnostic::new(Code::E108, "llvm subset: array_get returned void"));
        }
    };
    cx.builder.build_unconditional_branch(join_bb).map_err(err)?;
    cx.builder.position_at_end(fast_bb);
    cx.builder.build_unconditional_branch(join_bb).map_err(err)?;
    cx.builder.position_at_end(join_bb);
    let phi = cx.builder.build_phi(i64t, "ag").map_err(err)?;
    phi.add_incoming(&[
        (&slow_bits as &dyn inkwell::values::BasicValue, slow_bb),
        (&fast_bits as &dyn inkwell::values::BasicValue, fast_bb),
    ]);
    let bits = phi.as_basic_value().into_int_value();
    store(cx, dst, bits.into())?;
    if matches!(
        cx.ftypes.get(dst as usize),
        Some(LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_))
    ) {
        retain_val(cx, lir, dst, bits)?;
        own(cx, dst);
    }
    Ok(())
}

fn inline_array_set(
    cx: &FnCx,
    arr: Local,
    index: Local,
    value: Local,
    checked: bool,
) -> Result<(), Diagnostic> {
    let i64t = cx.context.i64_type();
    let a = load(cx, arr)?;
    let ap = as_ptr(cx, a)?;
    let i = load(cx, index)?;
    let v = load(cx, value)?;
    let datap = array_data_ptr(cx, ap)?;
    let ep = array_elem_ptr(cx, datap, i)?;
    if !checked {
        cx.builder.build_store(ep, v).map_err(err)?;
        tag_last(cx, cx.tbaa_payload);
        return Ok(());
    }
    let len = array_len_value(cx, ap)?;
    let inrange = cx.builder.build_int_compare(IntPredicate::ULT, i, len, "").map_err(err)?;
    let notnull = cx.builder.build_not(cx.builder.build_is_null(ap, "").map_err(err)?, "").map_err(err)?;
    let ok = cx.builder.build_and(notnull, inrange, "").map_err(err)?;
    let (slow_bb, fast_bb, join_bb) = cur_blocks(cx, "as_slow", "as_fast", "as_join")?;
    cx.builder.build_conditional_branch(ok, fast_bb, slow_bb).map_err(err)?;
    cx.builder.position_at_end(slow_bb);
    let e = i64t.const_int(8, false);
    cx.builder.build_call(cx.array_set, &[ap.into(), i.into(), v.into(), e.into()], "").map_err(err)?;
    cx.builder.build_unconditional_branch(join_bb).map_err(err)?;
    cx.builder.position_at_end(fast_bb);
    cx.builder.build_store(ep, v).map_err(err)?;
    cx.builder.build_unconditional_branch(join_bb).map_err(err)?;
    cx.builder.position_at_end(join_bb);
    Ok(())
}

fn array_len_slot<'a>(cx: &'a FnCx<'a>, ap: PointerValue<'a>) -> Result<PointerValue<'a>, Diagnostic> {
    let i64t = cx.context.i64_type();
    let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
    let base = cx.builder.build_ptr_to_int(ap, i64t, "").map_err(err)?;
    let addr = cx.builder.build_int_add(base, i64t.const_int(16, false), "").map_err(err)?;
    cx.builder.build_int_to_ptr(addr, ptr_t, "").map_err(err)
}

fn array_cap_value<'a>(cx: &'a FnCx<'a>, ap: PointerValue<'a>) -> Result<IntValue<'a>, Diagnostic> {
    let i64t = cx.context.i64_type();
    let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
    let base = cx.builder.build_ptr_to_int(ap, i64t, "").map_err(err)?;
    let addr = cx.builder.build_int_add(base, i64t.const_int(24, false), "").map_err(err)?;
    let field = cx.builder.build_int_to_ptr(addr, ptr_t, "").map_err(err)?;
    let v = cx.builder.build_load(i64t, field, "").map_err(err)?.into_int_value();
    tag_last(cx, cx.tbaa_header);
    Ok(v)
}

fn cold_branch(cx: &FnCx, br: inkwell::values::InstructionValue, then_w: u32, else_w: u32) {
    use inkwell::values::BasicMetadataValueEnum as M;
    let i32t = cx.context.i32_type();
    let node = cx.context.metadata_node(&[
        M::MetadataValue(cx.context.metadata_string("branch_weights")),
        M::IntValue(i32t.const_int(then_w as u64, false)),
        M::IntValue(i32t.const_int(else_w as u64, false)),
    ]);
    let _ = br.set_metadata(node, cx.context.get_kind_id("prof"));
}

fn inline_array_push(cx: &FnCx, arr: Local, value: Local) -> Result<(), Diagnostic> {
    let i64t = cx.context.i64_type();
    let a = load(cx, arr)?;
    let ap = as_ptr(cx, a)?;
    let v = load(cx, value)?;
    let len = array_len_value(cx, ap)?;
    let cap = array_cap_value(cx, ap)?;
    let full = cx.builder.build_int_compare(IntPredicate::EQ, len, cap, "").map_err(err)?;
    let isnull = cx.builder.build_is_null(ap, "").map_err(err)?;
    let slow = cx.builder.build_or(isnull, full, "").map_err(err)?;
    let (slow_bb, fast_bb, join_bb) = cur_blocks(cx, "ap_slow", "ap_fast", "ap_join")?;
    let br = cx.builder.build_conditional_branch(slow, slow_bb, fast_bb).map_err(err)?;
    cold_branch(cx, br, 1, 2000000);
    cx.builder.position_at_end(slow_bb);
    let e = i64t.const_int(8, false);
    cx.builder.build_call(cx.array_push, &[ap.into(), v.into(), e.into()], "").map_err(err)?;
    cx.builder.build_unconditional_branch(join_bb).map_err(err)?;
    cx.builder.position_at_end(fast_bb);
    let datap = array_data_ptr(cx, ap)?;
    let ep = array_elem_ptr(cx, datap, len)?;
    cx.builder.build_store(ep, v).map_err(err)?;
    tag_last(cx, cx.tbaa_payload);
    let new_len = cx.builder.build_int_add(len, i64t.const_int(1, false), "").map_err(err)?;
    let slot = array_len_slot(cx, ap)?;
    cx.builder.build_store(slot, new_len).map_err(err)?;
    tag_last(cx, cx.tbaa_header);
    cx.builder.build_unconditional_branch(join_bb).map_err(err)?;
    cx.builder.position_at_end(join_bb);
    Ok(())
}

fn try_sqrt_intrinsic<'a>(cx: &'a FnCx<'a>, arg: Local) -> Result<Option<IntValue<'a>>, Diagnostic> {
    let decl = match cx.sqrt_fn {
        Some(f) => f,
        None => return Ok(None),
    };
    let f64t = cx.context.f64_type();
    let bits = load(cx, arg)?;
    let d = cx.builder.build_bit_cast(bits, f64t, "").map_err(err)?.into_float_value();
    let site = cx.builder.build_call(decl, &[d.into()], "").map_err(err)?;
    match site.try_as_basic_value() {
        ValueKind::Basic(v) => {
            let f = v.into_float_value();
            let back = cx
                .builder
                .build_bit_cast(f, cx.context.i64_type(), "")
                .map_err(err)?
                .into_int_value();
            Ok(Some(back))
        }
        ValueKind::Instruction(_) => Ok(None),
    }
}

fn array_get_old<'a>(cx: &'a FnCx<'a>, arr: Local, index: Local, elem_size: usize, unchecked: bool) -> Result<IntValue<'a>, Diagnostic> {
    let a = load(cx, arr)?;
    let ap = as_ptr(cx, a)?;
    let i = load(cx, index)?;
    let e = cx.context.i64_type().const_int(elem_size as u64, false);
    let callee = if unchecked { cx.array_get_unchecked } else { cx.array_get };
    let site = cx.builder.build_call(callee, &[ap.into(), i.into(), e.into()], "").map_err(err)?;
    match site.try_as_basic_value() {
        ValueKind::Basic(v) => Ok(v.into_int_value()),
        ValueKind::Instruction(_) => Err(Diagnostic::new(Code::E108, "llvm subset: array_get returned void")),
    }
}

fn array_release_old(cx: &FnCx, lir: &Module, arr: Local, old: IntValue) -> Result<(), Diagnostic> {
    match cx.ftypes.get(arr as usize).cloned() {
        Some(LirType::Array(inner)) => match *inner {
            LirType::Str => release_str_val(cx, old),
            LirType::Enum(ei) => {
                let size = match lir.enums.get(ei) {
                    Some(d) => enum_instance_size(d) as u64,
                    None => return Err(Diagnostic::new(Code::E108, "llvm subset: unknown enum")),
                };
                let ptr = as_ptr(cx, old)?;
                let n = cx.context.i64_type().const_int(size, false);
                let dtor = enum_dtor_ptr(cx, lir, ei)?;
                cx.builder.build_call(cx.release, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
                Ok(())
            }
            LirType::Obj(_) | LirType::Array(_) => {
                let ptr = as_ptr(cx, old)?;
                let n = cx.context.i64_type().const_int(lir::instr::elem_size(&inner) as u64, false);
                let dtor = array_elem_dtor(cx, lir, &inner)?;
                cx.builder.build_call(cx.release_array, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
                Ok(())
            }
            LirType::Any => any_release_val(cx, old),
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

fn print_tag(ty: &LirType) -> u64 {
    match ty {
        LirType::I64 | LirType::I8 => runtime::native::TAG_INT as u64,
        LirType::Bool => runtime::native::TAG_BOOL as u64,
        LirType::F64(_) => runtime::native::TAG_FLOAT as u64,
        LirType::Str => runtime::native::TAG_STR as u64,
        LirType::Error => runtime::native::TAG_STR as u64,
        _ => runtime::native::TAG_PTR as u64,
    }
}

fn dtor_ptr<'a>(cx: &FnCx<'a>, lir: &Module, l: Local) -> Result<inkwell::values::PointerValue<'a>, Diagnostic> {
    let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
    let fid = match cx.ftypes.get(l as usize) {
        Some(LirType::Obj(name)) => {
            let ci = lir.class_index.get(name).copied().ok_or_else(|| {
                Diagnostic::new(Code::E108, "llvm subset: unknown class")
            })?;
            lir.classes[ci].dtor
        }
        Some(LirType::Enum(ei)) => {
            lir.enums.get(*ei).ok_or_else(|| {
                Diagnostic::new(Code::E108, "llvm subset: unknown enum")
            })?.dtor
        }
        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: dtor of non-object")),
    };
    match fid {
        Some(fid) => {
            let fname = &lir.functions[fid].name;
            let fv = cx.funcs.get(fname).copied().ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("llvm subset: unknown dtor `{fname}`"))
            })?;
            Ok(fv.as_global_value().as_pointer_value())
        }
        None => Ok(ptr_t.const_null()),
    }
}

fn enum_dtor_ptr<'a>(cx: &FnCx<'a>, lir: &Module, ei: usize) -> Result<inkwell::values::PointerValue<'a>, Diagnostic> {
    let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
    let dtor = lir.enums.get(ei).and_then(|e| e.dtor);
    match dtor {
        Some(fid) => {
            let fname = &lir.functions[fid].name;
            let fv = cx.funcs.get(fname).copied().ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("llvm subset: unknown enum dtor `{fname}`"))
            })?;
            Ok(fv.as_global_value().as_pointer_value())
        }
        None => Ok(ptr_t.const_null()),
    }
}

fn field_dtor_ptr<'a>(cx: &FnCx<'a>, lir: &Module, obj: Local, field: usize) -> Result<inkwell::values::PointerValue<'a>, Diagnostic> {
    let ptr_t = cx.context.ptr_type(inkwell::AddressSpace::default());
    let class_name = match cx.ftypes.get(obj as usize) {
        Some(LirType::Obj(name)) => name.clone(),
        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: field of non-object")),
    };
    let ci = lir.class_index.get(&class_name).copied().ok_or_else(|| {
        Diagnostic::new(Code::E108, "llvm subset: unknown class")
    })?;
    let fty = lir.classes[ci].fields.get(field).map(|f| f.ty.clone()).ok_or_else(|| {
        Diagnostic::new(Code::E108, "llvm subset: field out of range")
    })?;
    let target = match fty {
        LirType::Obj(name) => name,
        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: release of non-object field")),
    };
    let fci = lir.class_index.get(&target).copied().ok_or_else(|| {
        Diagnostic::new(Code::E108, "llvm subset: unknown field class")
    })?;
    match lir.classes[fci].dtor {
        Some(fid) => {
            let fname = &lir.functions[fid].name;
            let fv = cx.funcs.get(fname).copied().ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("llvm subset: unknown dtor `{fname}`"))
            })?;
            Ok(fv.as_global_value().as_pointer_value())
        }
        None => Ok(ptr_t.const_null()),
    }
}

fn emit_release(cx: &mut FnCx, lir: &Module, l: Local, keep: Option<IntValue>) -> Result<(), Diagnostic> {
    let size = obj_size(lir, cx, l)?;
    let raw = load(cx, l)?;
    let ptr = as_ptr(cx, guard_int(cx, raw, keep)?)?;
    let n = cx.context.i64_type().const_int(size, false);
    let dtor = dtor_ptr(cx, lir, l)?;
    cx.builder.build_call(cx.release, &[ptr.into(), n.into(), dtor.into()], "").map_err(err)?;
    cx.owned.remove(&l);
    cx.ever_owned.remove(&l);
    Ok(())
}

fn byte_ptr<'a>(cx: &FnCx<'a>, base: IntValue<'a>, offset: usize) -> Result<inkwell::values::PointerValue<'a>, Diagnostic> {
    let i64t = cx.context.i64_type();
    let addr = cx.builder.build_int_add(base, i64t.const_int(offset as u64, false), "").map_err(err)?;
    cx.builder.build_int_to_ptr(addr, cx.context.ptr_type(inkwell::AddressSpace::default()), "").map_err(err)
}
