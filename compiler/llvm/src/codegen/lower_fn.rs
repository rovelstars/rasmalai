use super::*;
use super::instr::{FnCx, lower_instr};

pub(super) fn build_module<'ctx>(    context: &'ctx Context,
    lir: &Module,
    name: &str,
    release: bool,
    debug: Option<&DebugInfo>,
) -> Result<(LlModule<'ctx>, BTreeMap<String, FunctionValue<'ctx>>), Diagnostic> {
    let module = context.create_module(name);
    let builder = context.create_builder();
    let mut funcs: BTreeMap<String, FunctionValue<'ctx>> = BTreeMap::new();
    for f in &lir.functions {
        let i64t = context.i64_type();
        let v4f32 = context.f32_type().vec_type(4);
        let v4i32 = context.i32_type().vec_type(4);
        let meta_ty = |t: &LirType| -> BasicMetadataTypeEnum {
            match t {
                LirType::Vec4f => v4f32.into(),
                LirType::Vec4i => v4i32.into(),
                _ => i64t.into(),
            }
        };
        let params: Vec<_> = f.params.iter().map(meta_ty).collect();
        let fty = match &f.ret {
            LirType::Vec4f => v4f32.fn_type(&params, false),
            LirType::Vec4i => v4i32.fn_type(&params, false),
            LirType::Tuple(_) => {
                let fields: Vec<BasicTypeEnum> = lir::instr::flat_sig(&f.ret)
                    .iter()
                    .map(|t| match t {
                        LirType::Vec4f => v4f32.into(),
                        LirType::Vec4i => v4i32.into(),
                        _ => i64t.into(),
                    })
                    .collect();
                context.struct_type(&fields, false).fn_type(&params, false)
            }
            _ => i64t.fn_type(&params, false),
        };
        let fv = module.add_function(&f.name, fty, None);
        // Spec 04_NUMERICS: strict `Float` rounds in two steps, hence
        // `fp-contract=off`. The attribute only sets the default for bare
        // instructions: strict ops are emitted bare and never fuse, while
        // `FastFloat` mul+add pairs are fused up front by LIR Fma
        // synthesis, so both backends lower the identical single-rounding
        // operation. Caveat, learned the hard way: LLVM-only rewrites
        // under reassoc/reciprocal (folding `x/C`, reordering add
        // chains) still fork bitwise agreement, so exact-checksum
        // FastFloat workloads must be canonicalized in LIR first (Fma
        // plus add-of-sub reassociation) with strict prologues; see
        // benches/README.md for the mandelbrot instance.
        let contract = context.create_string_attribute("fp-contract", "off");
        fv.add_attribute(inkwell::attributes::AttributeLoc::Function, contract);
        if release {
            fv.set_section(Some(&format!(".text.{}", f.name)));
        }
        funcs.insert(f.name.clone(), fv);
    }
    for f in &lir.foreign {
        if funcs.contains_key(&f.symbol) {
            continue;
        }
        let i64t = context.i64_type();
        let i8t = context.i8_type();
        let f64t = context.f64_type();
        let ptr_t = context.ptr_type(inkwell::AddressSpace::default());
        let void_t = context.void_type();
        let mut cparams: Vec<BasicMetadataTypeEnum> = Vec::with_capacity(f.params.len());
        for p in &f.params {
            cparams.push(match p {
                LirType::I64 => i64t.into(),
                LirType::I8 | LirType::Bool => i8t.into(),
                LirType::F64(_) => f64t.into(),
                LirType::Pointer(_) => ptr_t.into(),
                _ => {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!("foreign fn `{}` has non-C-ABI param", f.symbol),
                    ));
                }
            });
        }
        let cty = match &f.ret {
            LirType::Void => void_t.fn_type(&cparams, false),
            LirType::I64 => i64t.fn_type(&cparams, false),
            LirType::I8 | LirType::Bool => i8t.fn_type(&cparams, false),
            LirType::F64(_) => f64t.fn_type(&cparams, false),
            LirType::Pointer(_) => ptr_t.fn_type(&cparams, false),
            _ => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("foreign fn `{}` has non-C-ABI return", f.symbol),
                ));
            }
        };
        // External C declaration: default LLVM calling convention is C,
        // linkage external, so the host linker resolves the symbol from -l libs.
        let fv = module.add_function(&f.symbol, cty, None);
        funcs.insert(f.symbol.clone(), fv);
    }
    let ptr_t = context.ptr_type(inkwell::AddressSpace::default());
    let i64t = context.i64_type();
    let void_t = context.void_type();
    module.add_function("rnx_alloc", ptr_t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_free", void_t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_print_str", void_t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_print_i64", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_panic", void_t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_retain", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_string_concat",
        ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_string_split",
        ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_string_eq",
        context.bool_type().fn_type(&[ptr_t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_string_cmp",
        i64t.fn_type(&[ptr_t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_print_val",
        void_t.fn_type(&[i64t.into(), context.i32_type().into()], false),
        None,
    );
    module.add_function("rnx_print_str", void_t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_int_to_str", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_float_to_str", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_bool_to_str", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_any_to_str", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_eq_any_str", i64t.fn_type(&[i64t.into(), ptr_t.into()], false), None);
    module.add_function("rnx_eq_any", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_release_str", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_array_new",
        ptr_t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_array_push",
        void_t.fn_type(&[ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_array_get",
        i64t.fn_type(&[ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_array_get_unchecked",
        i64t.fn_type(&[ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_array_set_unchecked",
        void_t.fn_type(&[ptr_t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_array_set",
        void_t.fn_type(&[ptr_t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_array_len", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_string_len", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_string_slice",
        ptr_t.fn_type(&[ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_string_index_of",
        i64t.fn_type(&[ptr_t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_string_index_of_from",
        i64t.fn_type(&[ptr_t.into(), ptr_t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_string_trim", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_string_char_code_at",
        i64t.fn_type(&[ptr_t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_string_from_char_code", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_array_pop",
        i64t.fn_type(&[ptr_t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_any_box",
        i64t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_any_unbox",
        i64t.fn_type(&[i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_any_unbox_heap",
        i64t.fn_type(&[i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_any_retain",
        void_t.fn_type(&[i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_any_release_box",
        void_t.fn_type(&[i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_any_release",
        void_t.fn_type(&[i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_heap_track",
        void_t.fn_type(&[i64t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_closure_new",
        ptr_t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_closure_set",
        void_t.fn_type(
            &[
                ptr_t.into(),
                i64t.into(),
                i64t.into(),
                i64t.into(),
                i64t.into(),
                ptr_t.into(),
                i64t.into(),
                ptr_t.into(),
            ],
            false,
        ),
        None,
    );
    module.add_function("rnx_closure_release", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_thread_spawn_closure",
        i64t.fn_type(&[ptr_t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_thread_join_val", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_thread_join_err", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_task_await_val", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_task_await_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_pool_new", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_thread_pool_submit_handle",
        ptr_t.fn_type(&[i64t.into(), i64t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_thread_pool_submit_closure",
        ptr_t.fn_type(&[i64t.into(), i64t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_thread_pool_parallel_closure",
        void_t.fn_type(&[i64t.into(), ptr_t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_closure_register",
        void_t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_panic_str", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_fatal_span",
        void_t.fn_type(&[ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_report_fatal",
        context.i32_type().fn_type(&[], false),
        None,
    );
    module.add_function(
        "rnx_release_array",
        void_t.fn_type(&[ptr_t.into(), i64t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_thread_spawn",
        i64t.fn_type(&[ptr_t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_thread_join",
        i64t.fn_type(&[i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_clock_monotonic_nanos",
        i64t.fn_type(&[], false),
        None,
    );
    module.add_function("rnx_sleep_nanos", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_crypto_random_u64", i64t.fn_type(&[], false), None);
    module.add_function(
        "rnx_prng_seed",
        void_t.fn_type(&[ptr_t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_prng_next", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_file_open",
        ptr_t.fn_type(&[ptr_t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_file_close", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_file_read_text", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_file_read_text_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_file_write_text",
        i64t.fn_type(&[ptr_t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function("rnx_file_flush", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_file_seek",
        i64t.fn_type(&[ptr_t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_file_tell", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_file_from_handle",
        i64t.fn_type(&[i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_io_is_tty", context.bool_type().fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_io_winsize", ptr_t.fn_type(&[], false), None);
    module.add_function(
        "rnx_io_set_raw",
        ptr_t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    let bytes_2i = i64t.fn_type(&[ptr_t.into(), i64t.into()], false);
    let bytes_3i = void_t.fn_type(&[ptr_t.into(), i64t.into(), i64t.into()], false);
    for name in BYTES_READS {
        module.add_function(name, bytes_2i, None);
    }
    for name in BYTES_WRITES {
        module.add_function(name, bytes_3i, None);
    }
    module.add_function("rnx_bytes_alloc", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_bytes_free", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_bytes_len", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_bytes_data", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_bytes_cap", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_bytes_copy_within",
        void_t.fn_type(&[ptr_t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_bytes_read_string",
        ptr_t.fn_type(&[ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_bytes_write_string",
        i64t.fn_type(&[ptr_t.into(), i64t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_file_read_bytes",
        i64t.fn_type(&[ptr_t.into(), ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_file_write_bytes",
        i64t.fn_type(&[ptr_t.into(), ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_path_exists", context.bool_type().fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_path_remove", context.bool_type().fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_exists", context.bool_type().fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_is_file", context.bool_type().fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_is_dir", context.bool_type().fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_stat", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_stat_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_read_dir", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_read_dir_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_glob", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_glob_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_read_link", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_read_link_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_remove", context.bool_type().fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_remove_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_remove_all", context.bool_type().fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_remove_all_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_mkdir_err", ptr_t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_fs_copy_err", ptr_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_fs_move_err", ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false), None);
    module.add_function("rnx_fs_rename_err", ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false), None);
    module.add_function("rnx_fs_truncate_err", ptr_t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_fs_chmod_err", ptr_t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_fs_symlink_err", ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false), None);
    module.add_function("rnx_fs_fsync_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_read_text", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_read_text_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_write_text", i64t.fn_type(&[ptr_t.into(), ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_fs_write_text_err", ptr_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_fs_read_bytes", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_read_bytes_err", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_write_bytes", i64t.fn_type(&[ptr_t.into(), ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_fs_write_bytes_err", ptr_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_fs_mmap", ptr_t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_fs_mmap_err", ptr_t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_fs_mmap_anon", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_fs_mmap_anon_err", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_fs_mmap_addr", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_mmap_len", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_mmap_flush", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_fs_mmap_close", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_assert", void_t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_test_check", context.bool_type().fn_type(&[], false), None);
    module.add_function("rnx_env_args_count", i64t.fn_type(&[], false), None);
    module.add_function("rnx_env_args_get", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_env_get", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_env_set",
        void_t.fn_type(&[ptr_t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function("rnx_env_cwd", ptr_t.fn_type(&[], false), None);
    module.add_function("rnx_host_version", ptr_t.fn_type(&[], false), None);
    module.add_function("rnx_env_exit", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_net_connect_start", i64t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_net_take_error", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_net_connect_wait", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_net_recv_or_wait", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_net_send_or_wait", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_net_recv_get", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_net_error_text", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_net_close", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_net_listener_bind", i64t.fn_type(&[ptr_t.into(), i64t.into()], false), None);
    module.add_function("rnx_net_listener_port", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_net_listener_accept_start", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_net_listener_accept_wait", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_net_listener_close", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_tls_connect_start", i64t.fn_type(&[i64t.into(), ptr_t.into()], false), None);
    module.add_function("rnx_tls_handshake_start", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_tls_handshake_wait", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_tls_recv_or_wait", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_tls_send_or_wait", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_tls_recv_get", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_tls_error_text", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_tls_close", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_process_pid", i64t.fn_type(&[], false), None);
    module.add_function("rnx_process_remove_env", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_process_all_env_count", i64t.fn_type(&[], false), None);
    module.add_function("rnx_process_all_env_get", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_process_chdir", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function(
        "rnx_process_spawn",
        i64t.fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into(), ptr_t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_process_run",
        i64t.fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into(), ptr_t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_process_pid_of", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_process_write_stdin",
        i64t.fn_type(&[i64t.into(), ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_process_read_stdout",
        i64t.fn_type(&[i64t.into(), ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_process_read_stderr",
        i64t.fn_type(&[i64t.into(), ptr_t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_process_close_stdin", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_process_wait", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_process_try_wait", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_process_kill", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_process_take_stdout", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_process_take_stderr", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_process_exit_code", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_process_forget", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_os_platform", ptr_t.fn_type(&[], false), None);
    module.add_function("rnx_os_arch", ptr_t.fn_type(&[], false), None);
    module.add_function("rnx_os_hostname", ptr_t.fn_type(&[], false), None);
    module.add_function("rnx_os_tmpdir", ptr_t.fn_type(&[], false), None);
    module.add_function("rnx_os_homedir", ptr_t.fn_type(&[], false), None);
    module.add_function("rnx_os_cpu_count", i64t.fn_type(&[], false), None);
    module.add_function("rnx_os_uptime", i64t.fn_type(&[], false), None);
    module.add_function("rnx_map_new", ptr_t.fn_type(&[], false), None);
    module.add_function(
        "rnx_map_set",
        void_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_map_get", i64t.fn_type(&[ptr_t.into(), ptr_t.into()], false), None);
    module.add_function(
        "rnx_map_has",
        context.bool_type().fn_type(&[ptr_t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_map_delete",
        context.bool_type().fn_type(&[ptr_t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function("rnx_map_len", i64t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_map_clear", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_map_keys", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_map_values", ptr_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_gmap_new", ptr_t.fn_type(&[], false), None);
    module.add_function("rnx_gmap_free", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_gmap_set",
        void_t.fn_type(&[i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_gmap_get",
        i64t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_gmap_has",
        context.bool_type().fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_gmap_delete",
        context.bool_type().fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_gmap_len", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_gmap_clear", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_gmap_keys", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_gmap_values", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_json_parse", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_json_parse_typed",
        ptr_t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_json_stringify_into",
        i64t.fn_type(&[i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_json_stringify", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_json_unwrap", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_dns_lookup_start", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_dns_lookup_wait", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_dns_lookup_get", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_dns_lookup_error", ptr_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_release_map", void_t.fn_type(&[ptr_t.into()], false), None);
    module.add_function("rnx_sync_atomic_get", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_black_box_i64", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_sync_atomic_set",
        void_t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_sync_atomic_fetch_add",
        i64t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_sync_atomic_cas",
        context.bool_type().fn_type(&[i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_sync_channel_send",
        void_t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_sync_channel_recv", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_sync_channel_try_recv", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_sync_channel_len", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_fs_pool_depth", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_sync_channel_drop", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_debug_live_count", i64t.fn_type(&[], false), None);
    module.add_function("rnx_defer_push", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_defer_pop", i64t.fn_type(&[], false), None);
    module.add_function("rnx_defer_len", i64t.fn_type(&[], false), None);
    module.add_function("rnx_any_tag", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_obj_class", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_error_set", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_error_take", i64t.fn_type(&[], false), None);
    module.add_function("rnx_error_class", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_error_unbox", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_error_str", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_error_release", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_report_uncaught", context.i32_type().fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_note_type", void_t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_note_fields", void_t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_note_namespace", void_t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function(
        "rnx_note_array_kind",
        void_t.fn_type(&[i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_note_enum",
        void_t.fn_type(&[i64t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_io_pretty", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_type_name", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_typeof_any", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_array_slice", i64t.fn_type(&[i64t.into(), i64t.into(), i64t.into(), i64t.into(), i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_mutex_lock", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_mutex_unlock", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_mutex_try_lock",
        context.bool_type().fn_type(&[i64t.into()], false),
        None,
    );
    module.add_function("rnx_rwlock_read_lock", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_rwlock_read_unlock", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_rwlock_write_lock", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_rwlock_write_unlock", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_rwlock_try_read_lock",
        context.bool_type().fn_type(&[i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_rwlock_try_write_lock",
        context.bool_type().fn_type(&[i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_condvar_wait",
        void_t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_condvar_wait_timeout",
        context.bool_type().fn_type(&[i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_condvar_notify_one", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_condvar_notify_all", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_barrier_wait",
        context.bool_type().fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    let v4f32 = context.f32_type().vec_type(4);
    module.add_function("llvm.minnum.v4f32", v4f32.fn_type(&[v4f32.into(), v4f32.into()], false), None);
    module.add_function("llvm.maxnum.v4f32", v4f32.fn_type(&[v4f32.into(), v4f32.into()], false), None);
    module.add_function("llvm.sqrt.v4f32", v4f32.fn_type(&[v4f32.into()], false), None);
    module.add_function(
        "rnx_thread_pool_init",
        void_t.fn_type(&[i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_thread_pool_submit",
        void_t.fn_type(&[i64t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function(
        "rnx_thread_pool_parallel_for",
        void_t.fn_type(&[i64t.into(), i64t.into(), i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_thread_pool_join", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_thread_pool_shutdown", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_sync_channel_send_str",
        void_t.fn_type(&[i64t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_sync_channel_send_obj",
        void_t.fn_type(&[i64t.into(), ptr_t.into(), i64t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function(
        "rnx_sync_channel_send_array",
        void_t.fn_type(&[i64t.into(), ptr_t.into(), i64t.into(), ptr_t.into()], false),
        None,
    );
    module.add_function("rnx_math_sqrt", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_math_sin", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_math_cos", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_math_tan", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_math_atan2", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_math_pow", i64t.fn_type(&[i64t.into(), i64t.into()], false), None);
    module.add_function("rnx_math_floor", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_math_ceil", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_math_round", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_math_log", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_float_nan", i64t.fn_type(&[], false), None);
    module.add_function("rnx_float_to_bits", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_float_from_bits", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_float_fma",
        i64t.fn_type(&[i64t.into(), i64t.into(), i64t.into()], false),
        None,
    );
    module.add_function("rnx_genref_create", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_genref_get", i64t.fn_type(&[i64t.into()], false), None);
    module.add_function("rnx_genref_invalidate", void_t.fn_type(&[i64t.into()], false), None);
    module.add_function(
        "rnx_release",
        void_t.fn_type(&[ptr_t.into(), i64t.into(), ptr_t.into()], false),
        None,
    );
    mark_pure_runtime_fns(context, &module);
    let debug = if release { None } else { debug };
    let mut active: Option<ActiveDebug<'ctx>> = None;
    if let Some(info) = debug {
        active = Some(start_debug(context, &module, lir, info, &funcs)?);
    }
    for f in &lir.functions {
        let loc = if let Some(dbg) = active.as_ref() {
            match dbg.subs.get(&f.name) {
                Some(sp) => {
                    use inkwell::debug_info::AsDIScope;
                    let (line, col) =
                        debug.and_then(|d| d.lines.get(&f.name).copied()).unwrap_or((1, 1));
                    Some(dbg.builder.create_debug_location(
                        context,
                        line,
                        col,
                        sp.as_debug_info_scope(),
                        None,
                    ))
                }
                None => None,
            }
        } else {
            None
        };
        lower_fn(context, &builder, &module, lir, &funcs, f, release, loc)?;
    }
    if let Some(dbg) = active {
        dbg.builder.finalize();
    }
    Ok((module, funcs))
}

pub(super) fn check_supported(lir: &Module, f: &Function) -> Result<(), Diagnostic> {
    let bad = |what: &str| Diagnostic::new(Code::E108, format!("llvm subset: {what} in `{}`", f.name));
    for t in f.params.iter().chain(f.locals.iter()) {
        if !matches!(t, LirType::I64 | LirType::Bool | LirType::I8 | LirType::F64(_) | LirType::Null | LirType::Obj(_) | LirType::GenRef(_) | LirType::Pointer(_) | LirType::Pool | LirType::Vec4f | LirType::Vec4i | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Any | LirType::Closure | LirType::Tuple(_) | LirType::Range | LirType::Error) {
            return Err(bad("non-scalar local"));
        }
    }
    if !matches!(f.ret, LirType::I64 | LirType::Bool | LirType::I8 | LirType::F64(_) | LirType::Any | LirType::Void | LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Vec4f | LirType::Vec4i | LirType::Closure | LirType::Tuple(_) | LirType::Range | LirType::Error) {
        return Err(bad("non-scalar return"));
    }
    for b in &f.blocks {
        for ins in &b.instrs {
            match ins {
                Instr::Const { lit, .. } => match lit {
                    Lit::Int(_) | Lit::Bool(_) | Lit::Null | Lit::Str(_) | Lit::Float(..) => {}
                },
                Instr::Concat { .. } | Instr::ToStr { .. } | Instr::Convert { .. } => {},
                Instr::Cast { .. } => {}
                Instr::Copy { dst, src , ..} => {
                    let dinfo = f.locals.get(*dst as usize);
                    let sinfo = f.locals.get(*src as usize);
                    match (dinfo, sinfo) {
                        (Some(LirType::Obj(_)), Some(LirType::Obj(_))) => {}
                        (Some(LirType::Str), Some(LirType::Str)) => {}
                        (Some(LirType::Array(_)), Some(LirType::Array(_))) => {}
                        (Some(LirType::Enum(_)), Some(LirType::Enum(_))) => {}
                        (Some(LirType::Obj(_)), Some(LirType::Any))
                        | (Some(LirType::Any), Some(LirType::Obj(_)))
                        | (Some(LirType::Array(_)), Some(LirType::Any))
                        | (Some(LirType::Any), Some(LirType::Array(_)))
                        | (Some(LirType::Enum(_)), Some(LirType::Any))
                        | (Some(LirType::Any), Some(LirType::Enum(_)))
                        | (Some(LirType::Obj(_)), Some(LirType::Null))
                        | (Some(LirType::Array(_)), Some(LirType::Null)) => {}
                        (Some(LirType::Obj(_)), _) | (_, Some(LirType::Obj(_))) => {
                            return Err(bad("mixed object copy"));
                        }
                        (Some(LirType::Array(_)), _) | (_, Some(LirType::Array(_))) => {
                            return Err(bad("mixed array copy"));
                        }
                        (Some(LirType::Enum(_)), _) | (_, Some(LirType::Enum(_))) => {
                            return Err(bad("mixed enum copy"));
                        }
                        _ => {}
                    }
                }
                Instr::Arith { .. }
                | Instr::Fma { .. }
                | Instr::Cmp { .. }
                | Instr::Not { .. }
                | Instr::Neg { .. } => {}
                Instr::ObjNew { class, instance_size: size, .. } => {
                    match lir.classes.get(*class) {
                        Some(c) if *size == instance_size(c.fields.len()) => {}
                        _ => return Err(bad("bad class alloc")),
                    }
                }
                Instr::StackAlloc { class, instance_size: size, .. } => {
                    match lir.classes.get(*class) {
                        Some(c) if *size == instance_size(c.fields.len()) => {}
                        _ => return Err(bad("bad class alloc")),
                    }
                }
                Instr::GetField { obj, field, .. } => {
                    match obj_local(lir, f, *obj) {
                        Some((ci, _)) if *field < lir.classes[ci].fields.len() => {}
                        _ => return Err(bad("bad field load")),
                    }
                }
                Instr::SetField { obj, field, .. } => {
                    match obj_local(lir, f, *obj) {
                        Some((ci, _)) if *field < lir.classes[ci].fields.len() => {}
                        _ => return Err(bad("bad field store")),
                    }
                }
                Instr::GetFieldByName { obj, field, .. } | Instr::SetFieldByName { obj, field, .. } => {
                    if named_field(lir, f, *obj, field).is_none() {
                        return Err(bad("unresolved field"));
                    }
                }
                Instr::Retain { obj , ..} => {
                    if obj_local(lir, f, *obj).is_none() && !matches!(f.locals.get(*obj as usize), Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Any) | Some(LirType::Error) | Some(LirType::Obj(_))) {
                        return Err(bad("retain of non-object"));
                    }
                }
                Instr::Release { obj , ..} => {
                    if obj_local(lir, f, *obj).is_none() && !matches!(f.locals.get(*obj as usize), Some(LirType::Str) | Some(LirType::Array(_)) | Some(LirType::Enum(_)) | Some(LirType::Any) | Some(LirType::Error) | Some(LirType::Obj(_))) {
                        return Err(bad("release of non-object"));
                    }
                }
                Instr::ReleaseAs { obj: _, class, .. } => {
                    if lir.classes.get(*class).is_none() {
                        return Err(bad("release of unknown class"));
                    }
                }
                Instr::ArrayNew { .. } | Instr::ArrayPush { .. } | Instr::ArrayGet { .. } | Instr::ArraySet { .. } | Instr::ArrayLen { .. } => {}
                Instr::PtrLoad { dst, .. } => {
                    match f.locals.get(*dst as usize) {
                        Some(LirType::I64) | Some(LirType::I8) | Some(LirType::F64(_)) | Some(LirType::Bool) => {}
                        _ => return Err(bad("pointer load needs an Int, Float, or Bool slot")),
                    }
                }
                Instr::PtrStore { val, .. } => {
                    match f.locals.get(*val as usize) {
                        Some(LirType::I64) | Some(LirType::I8) | Some(LirType::F64(_)) | Some(LirType::Bool) => {}
                        _ => return Err(bad("pointer store needs an Int, Float, or Bool value")),
                    }
                }
                Instr::ClosureNew { captures, decay, .. } => {
                    if *decay {
                        return Err(bad("decayed closure"));
                    }
                    for c in captures {
                        match f.locals.get(*c as usize) {
                            Some(LirType::I64)
                            | Some(LirType::Bool)
                            | Some(LirType::Str)
                            | Some(LirType::Closure)
                            | Some(LirType::Null) => {}
                            Some(LirType::Obj(_)) => {}
                            Some(LirType::Array(inner)) if llvm_array_capturable(inner) => {}
                            _ => return Err(bad("closure captures unsupported value")),
                        }
                    }
                }
                Instr::ReleaseField { obj, field , ..} => {
                    match obj_local(lir, f, *obj) {
                        Some((ci, _)) if *field < lir.classes[ci].fields.len() => {}
                        _ => return Err(bad("bad field release")),
                    }
                }
                Instr::GenRefOf { obj, .. } | Instr::GenRefInvalidate { obj , ..} => {
                    if obj_local(lir, f, *obj).is_none() {
                        return Err(bad("genref of non-object"));
                    }
                }
                Instr::GenRefEmpty { .. } | Instr::GenRefGet { .. } => {}
                Instr::ThreadSpawn { .. } | Instr::ThreadJoin { .. } => {}
                Instr::VecNew { .. }
                | Instr::VecSplat { .. }
                | Instr::VecExtract { .. }
                | Instr::VecInsert { .. }
                | Instr::VecArith { .. }
                | Instr::VecUnary { .. }
                | Instr::VecDot { .. } => {}
                Instr::PoolInit { .. }
                | Instr::PoolSubmit { .. }
                | Instr::PoolParallelFor { .. }
                | Instr::PoolJoin { .. }
                | Instr::PoolShutdown { .. } => {}
                Instr::EnumNew { dst: _, enu, variant, payload , ..} => {
                    match lir.enums.get(*enu) {
                        Some(d) => match d.variants.get(*variant) {
                            Some(v) if v.payload.len() == payload.len() => {}
                            _ => return Err(bad("bad enum variant")),
                        },
                        None => return Err(bad("unknown enum")),
                    }
                }
                Instr::EnumPayload { scrut, index, .. } => {
                    match f.locals.get(*scrut as usize) {
                        Some(LirType::Enum(ei)) => match lir.enums.get(*ei) {
                            Some(d) if *index < enum_max_payloads(d) => {}
                            _ => return Err(bad("bad enum payload")),
                        },
                        _ => return Err(bad("payload of non-enum")),
                    }
                }
                Instr::EnumTag { scrut, .. } => {
                    if !matches!(f.locals.get(*scrut as usize), Some(LirType::Enum(_))) {
                        return Err(bad("tag of non-enum"));
                    }
                }
                Instr::Call { target, span, .. } => {
                    let supported = matches!(target, CallTarget::Fn(_))
                        || matches!(target, CallTarget::Foreign { .. })
                        || matches!(target, CallTarget::Value(_))
                        || matches!(target, CallTarget::Builtin(n) if n == "print" || n == "streq" || n == "strcmp" || n == "__rnx_clock_mono" || n == "__rnx_crypto_random_u64" || n == "__rnx_prng_seed" || n == "__rnx_prng_next" || n == "__rnx_file_open" || n == "__rnx_file_close" || n == "__rnx_file_read_text" || n == "__rnx_file_read_text_err" || n == "__rnx_file_write_text" || n == "__rnx_file_flush" || n == "__rnx_file_seek" || n == "__rnx_file_tell" || n == "__rnx_file_from_handle" || n == "__rnx_io_is_tty" || n == "__rnx_io_pretty" || n == "__rnx_io_winsize" || n == "__rnx_io_set_raw" || n.starts_with("__rnx_bytes_") || n == "__rnx_file_read_bytes" || n == "__rnx_file_write_bytes" || n == "assert" || n == "__testCheck" || n == "__rnx_path_exists" || n == "__rnx_path_remove" || n.starts_with("__rnx_fs_") || n == "__rnx_env_args_count" || n == "__rnx_env_args_get" || n == "__rnx_env_get" || n == "__rnx_env_set" || n == "__rnx_env_cwd" || n == "__rnx_host_version" || n == "__rnx_env_exit" || n == "__rnx_net_connect_start" || n == "__rnx_net_take_error" || n == "__rnx_net_recv_get" || n == "__rnx_net_error_text" || n == "__rnx_net_close" || n == "__rnx_net_connect_wait" || n == "__rnx_net_recv_or_wait" || n == "__rnx_net_send_or_wait" || n == "__rnx_net_listener_bind" || n == "__rnx_net_listener_port" || n == "__rnx_net_listener_accept_start" || n == "__rnx_net_listener_accept_wait" || n == "__rnx_net_listener_close" || n == "__rnx_tls_connect_start" || n == "__rnx_tls_handshake_start" || n == "__rnx_tls_handshake_wait" || n == "__rnx_tls_recv_or_wait" || n == "__rnx_tls_send_or_wait" || n == "__rnx_tls_recv_get" || n == "__rnx_tls_error_text" || n == "__rnx_tls_close" || n == "__rnx_process_pid" || n == "__rnx_process_remove_env" || n == "__rnx_process_all_env_count" || n == "__rnx_process_all_env_get" || n == "__rnx_process_chdir" || n == "__rnx_process_spawn" || n == "__rnx_process_run" || n == "__rnx_process_pid_of" || n == "__rnx_process_write_stdin" || n == "__rnx_process_read_stdout" || n == "__rnx_process_read_stderr" || n == "__rnx_process_close_stdin" || n == "__rnx_process_wait" || n == "__rnx_process_try_wait" || n == "__rnx_process_kill" || n == "__rnx_process_take_stdout" || n == "__rnx_process_take_stderr" || n == "__rnx_process_exit_code" || n == "__rnx_process_forget" || n == "__rnx_os_platform" || n == "__rnx_os_arch" || n == "__rnx_os_hostname" || n == "__rnx_os_tmpdir" || n == "__rnx_os_homedir" || n == "__rnx_os_cpu_count" || n == "__rnx_os_uptime" || n.starts_with("__rnx_math_") || n.starts_with("__rnx_float_") || n == "__rnx_map_new" || n == "__rnx_map_set" || n == "__rnx_map_get" || n == "__rnx_map_has" || n == "__rnx_map_delete" || n == "__rnx_map_len" || n == "__rnx_string_len" || n == "__rnx_string_slice" || n == "__rnx_string_index_of" || n == "__rnx_string_index_of_from" || n == "__rnx_string_trim" || n == "__rnx_string_concat" || n == "__rnx_string_split" || n == "__rnx_string_char_code_at" || n == "__rnx_string_from_char_code" || n == "__rnx_int_to_str" || n == "__rnx_float_to_str" || n == "__rnx_bool_to_str" || n == "__rnx_array_pop" || n == "__rnx_array_len" || n == "__rnx_map_clear" || n == "__rnx_map_keys" || n == "__rnx_map_values" || n.starts_with("__rnx_gmap_") || n == "__rnx_json_parse" || n == "__rnx_json_parse_typed" || n == "__rnx_json_stringify" || n == "__rnx_json_stringify_into" || n == "__rnx_json_unwrap" || n == "__rnx_dns_lookup_start" || n == "__rnx_dns_lookup_wait" || n == "__rnx_dns_lookup_get" || n == "__rnx_dns_lookup_error" || n == "__rnx_black_box" || n == "__rnx_sync_atomic_get" || n == "__rnx_sync_atomic_set" || n == "__rnx_sync_atomic_fetch_add" || n == "__rnx_sync_atomic_cas" || n == "__rnx_sync_channel_send" || n == "__rnx_sync_channel_send_str" || n == "__rnx_sync_channel_send_obj" || n == "__rnx_sync_channel_send_array" || n == "__rnx_sync_channel_recv" || n == "__rnx_sync_channel_try_recv" || n == "__rnx_sync_channel_len" || n == "__rnx_sync_channel_drop" || n == "__rnx_debug_live_count" || n == "__rnx_mutex_lock" || n == "__rnx_mutex_unlock" || n == "__rnx_mutex_try_lock" || n == "__rnx_rwlock_read_lock" || n == "__rnx_rwlock_read_unlock" || n == "__rnx_rwlock_write_lock" || n == "__rnx_rwlock_write_unlock" || n == "__rnx_rwlock_try_read_lock" || n == "__rnx_rwlock_try_write_lock" || n == "__rnx_condvar_wait" || n == "__rnx_condvar_wait_timeout" || n == "__rnx_condvar_notify_one" || n == "__rnx_condvar_notify_all" || n == "__rnx_barrier_wait" || n == "__rnx_thread_join_val" || n == "__rnx_thread_join_err" || n == "__rnx_task_await_val" || n == "__rnx_task_await_err" || n == "__rnx_pool_new" || n == "__rnx_any_box" || n == "__rnx_any_unbox" || n == "__rnx_any_unbox_heap" || n == "__rnx_any_retain" || n == "__rnx_any_release_box" || n == "__rnx_any_to_str" || n == "__rnx_eq_any" || n == "__rnx_eq_any_str" || n == "__rnx_any_tag" || n == "__rnx_obj_class" || n == "__rnx_error_set" || n == "__rnx_error_take" || n == "__rnx_error_class" || n == "__rnx_error_unbox" || n == "__rnx_error_str" || n == "__rnx_error_release" || n == "__rnx_note_type" || n == "__rnx_type_name" || n == "__rnx_typeof_any" || n == "__rnx_array_slice");
                    if !supported {
                        return Err(match target {
                            CallTarget::Dyn { method, .. } => Diagnostic::new(
                                Code::E108,
                                format!("method `{method}` on a value of unknown type is not supported in native builds"),
                            )
                            .with_span(*span)
                            .with_hint("annotate the receiver type, e.g. `let hs: Array<Thread> = []`, so the call resolves statically"),
                            _ => bad("indirect call"),
                        });
                    }
                }
                Instr::Defer { .. } | Instr::RunDefers { .. } => {}
                Instr::AddrOf { .. } => {
                    return Err(bad("address of a stack local is not supported in native codegen; use `Pointer.fromAddress` with a real address"));
                }
                _ => return Err(bad("unsupported instr")),
            }
        }
        match &b.term {
            Terminator::Ret(_) | Terminator::Br(_) | Terminator::BrIf { .. } => {}
            Terminator::Throw { src, catch, .. } => {
                if catch.is_some() {
                    match f.locals.get(*src as usize) {
                        Some(LirType::Str) | Some(LirType::Error) | Some(LirType::I64) | Some(LirType::I8) | Some(LirType::F64(_)) | Some(LirType::Bool) | Some(LirType::Obj(_)) => {}
                        _ => return Err(bad("throw payload")),
                    }
                }
            }
            Terminator::BrErr { .. } => {}            Terminator::Switch { cases, .. } => {
                for (pat, _) in cases {
                    match pat {
                        SwitchPat::Int(_) | SwitchPat::Enum { .. } | SwitchPat::Is { .. } => {}
                        _ => return Err(bad("switch pattern")),
                    }
                }
            }
            _ => return Err(bad("unsupported terminator")),
        }
    }
    Ok(())
}

fn declare_strings<'a>(
    context: &'a Context,
    module: &LlModule<'a>,
    lir: &Module,
    release: bool,
) -> Result<BTreeMap<String, inkwell::values::GlobalValue<'a>>, Diagnostic> {
    use std::collections::BTreeSet;
    let mut texts = BTreeSet::new();
    texts.insert(" ".to_string());
    texts.insert("\n".to_string());
    texts.insert("vector lane out of range".to_string());
    texts.insert("division by zero".to_string());
    texts.insert("got null or a value of the wrong type".to_string());
    texts.insert("len of null".to_string());
    texts.insert("field of null".to_string());
    texts.insert("field-set on null".to_string());
    for f in &lir.functions {
        for b in &f.blocks {
            lir::instr::walk_instrs(&b.instrs, &mut |ins| {
                if let Instr::Const { lit: Lit::Str(s), .. } = ins {
                    texts.insert(s.clone());
                }
            });
        }
    }
    for c in &lir.classes {
        texts.insert(c.name.rsplit('.').next().unwrap_or(&c.name).to_string());
        texts.insert(lir::instr::pretty_field_desc(c));
    }
    for (name, ns) in &lir.namespaces {
        let _ = name;
        texts.insert(lir::instr::pretty_namespace_desc(ns));
    }
    for e in &lir.enums {
        for v in &e.variants {
            texts.insert(lir::instr::pretty_variant_desc(v));
        }
    }
    let i32t = context.i32_type();
    let i64t = context.i64_type();
    let i8t = context.i8_type();
    let mut out = BTreeMap::new();
    for (n, text) in texts.into_iter().enumerate() {
        let bytes = text.as_bytes();
        let arr_vals: Vec<_> = bytes
            .iter()
            .chain(std::iter::once(&0u8))
            .map(|b| i8t.const_int(*b as u64, false))
            .collect();
        let arr = i8t.const_array(&arr_vals);
        let ty = context.struct_type(
            &[
                i32t.into(),
                i32t.into(),
                i64t.into(),
                i64t.into(),
                i64t.into(),
                arr.get_type().into(),
            ],
            false,
        );
        let init = ty.const_named_struct(&[
            i32t.const_int(0xFFFF_FFFF, false).into(),
            i32t.const_int(if text.is_ascii() { 1 } else { 2 }, false).into(),
            i64t.const_zero().into(),
            i64t.const_int(bytes.len() as u64, false).into(),
            i64t.const_int(text.chars().count() as u64 + 1, false).into(),
            arr.into(),
        ]);
        let gv = module.add_global(ty, None, &format!("rnx_str_{n}"));
        if release {
            gv.set_section(Some(&format!(".rodata.rnx_str_{n}")));
        }
        gv.set_initializer(&init);
        gv.set_linkage(inkwell::module::Linkage::Private);
        out.insert(text, gv);
    }
    Ok(out)
}

fn stack_locals(f: &Function) -> BTreeSet<Local> {
    let mut set: BTreeSet<Local> = BTreeSet::new();
    for block in &f.blocks {
        for ins in &block.instrs {
            if let Instr::StackAlloc { dst, .. } = ins {
                set.insert(*dst);
            }
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for block in &f.blocks {
            for ins in &block.instrs {
                if let Instr::Copy { dst, src , ..} | Instr::Cast { dst, src , ..} = ins {
                    if set.contains(src) && set.insert(*dst) {
                        changed = true;
                    }
                    if set.contains(dst) && set.insert(*src) {
                        changed = true;
                    }
                }
            }
        }
    }
    set
}

#[allow(clippy::too_many_arguments)]
const BYTES_READS: [&str; 16] = [
    "rnx_bytes_read_u8",
    "rnx_bytes_read_i8",
    "rnx_bytes_read_u16le",
    "rnx_bytes_read_u16be",
    "rnx_bytes_read_i16le",
    "rnx_bytes_read_i16be",
    "rnx_bytes_read_u32le",
    "rnx_bytes_read_u32be",
    "rnx_bytes_read_i32le",
    "rnx_bytes_read_i32be",
    "rnx_bytes_read_i64le",
    "rnx_bytes_read_i64be",
    "rnx_bytes_read_f32le",
    "rnx_bytes_read_f32be",
    "rnx_bytes_read_f64le",
    "rnx_bytes_read_f64be",
];

const BYTES_WRITES: [&str; 11] = [
    "rnx_bytes_write_u8",
    "rnx_bytes_write_u16le",
    "rnx_bytes_write_u16be",
    "rnx_bytes_write_u32le",
    "rnx_bytes_write_u32be",
    "rnx_bytes_write_u64le",
    "rnx_bytes_write_u64be",
    "rnx_bytes_write_f32le",
    "rnx_bytes_write_f32be",
    "rnx_bytes_write_f64le",
    "rnx_bytes_write_f64be",
];

const BYTES_SINGLE: [&str; 10] = [
    "rnx_bytes_alloc",
    "rnx_bytes_free",
    "rnx_bytes_len",
    "rnx_bytes_cap",
    "rnx_bytes_data",
    "rnx_bytes_copy_within",
    "rnx_bytes_read_string",
    "rnx_bytes_write_string",
    "rnx_file_read_bytes",
    "rnx_file_write_bytes",
];

const FLOAT_FNS: [&str; 4] = [
    "rnx_float_nan",
    "rnx_float_to_bits",
    "rnx_float_from_bits",
    "rnx_float_fma",
];

fn bytes_fns_map<'ctx>(module: &LlModule<'ctx>) -> BTreeMap<String, FunctionValue<'ctx>> {
    let mut out = BTreeMap::new();
    for name in BYTES_READS.into_iter().chain(BYTES_WRITES).chain(BYTES_SINGLE).chain(FLOAT_FNS) {
        if let Some(fv) = module.get_function(name) {
            out.insert(name.to_string(), fv);
        }
    }
    out
}

fn lower_fn<'a>(
    context: &'a Context,
    _builder: &Builder,
    module: &LlModule<'a>,
    lir: &Module,
    funcs: &BTreeMap<String, FunctionValue<'a>>,
    f_raw: &Function,
    is_release: bool,
    debug_loc: Option<inkwell::debug_info::DILocation<'a>>,
) -> Result<(), Diagnostic> {
    let normalized;
    let f = {
        let rets = f_raw.blocks.iter().filter(|b| matches!(b.term, Terminator::Ret(_))).count();
        if rets >= 2 {
            normalized = lir::opt::single_exit(f_raw);
            &normalized
        } else {
            f_raw
        }
    };
    let fv = funcs[&f.name];
    let i64t = context.i64_type();
    let entry = context.append_basic_block(fv, "entry");
    let runtime_fn = |name: &str| {
        module.get_function(name).ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("llvm jit: missing runtime symbol `{name}`"))
        })
    };
    let alloc = runtime_fn("rnx_alloc")?;
    let release = runtime_fn("rnx_release")?;
    let genref_create = runtime_fn("rnx_genref_create")?;
    let genref_get = runtime_fn("rnx_genref_get")?;
    let genref_invalidate = runtime_fn("rnx_genref_invalidate")?;
    let retain = runtime_fn("rnx_retain")?;
    let concat = runtime_fn("rnx_string_concat")?;
    let string_split = runtime_fn("rnx_string_split")?;
    let streq = runtime_fn("rnx_string_eq")?;
    let strcmp = runtime_fn("rnx_string_cmp")?;
    let print_val = runtime_fn("rnx_print_val")?;
    let print_str = runtime_fn("rnx_print_str")?;
    let int_to_str = runtime_fn("rnx_int_to_str")?;
    let float_to_str = runtime_fn("rnx_float_to_str")?;
    let bool_to_str = runtime_fn("rnx_bool_to_str")?;
    let any_to_str = runtime_fn("rnx_any_to_str")?;
    let eq_any_str = runtime_fn("rnx_eq_any_str")?;
    let eq_any = runtime_fn("rnx_eq_any")?;
    let release_str = runtime_fn("rnx_release_str")?;
    let array_new = runtime_fn("rnx_array_new")?;
    let array_push = runtime_fn("rnx_array_push")?;
    let array_get = runtime_fn("rnx_array_get")?;
    let array_set = runtime_fn("rnx_array_set")?;
    let array_get_unchecked = runtime_fn("rnx_array_get_unchecked")?;
    let array_set_unchecked = runtime_fn("rnx_array_set_unchecked")?;
    let array_len = runtime_fn("rnx_array_len")?;
    let string_len = runtime_fn("rnx_string_len")?;
    let string_slice = runtime_fn("rnx_string_slice")?;
    let string_index_of = runtime_fn("rnx_string_index_of")?;
    let string_index_of_from = runtime_fn("rnx_string_index_of_from")?;
    let string_trim = runtime_fn("rnx_string_trim")?;
    let string_char_code_at = runtime_fn("rnx_string_char_code_at")?;
    let string_from_char_code = runtime_fn("rnx_string_from_char_code")?;
    let array_pop_fn = runtime_fn("rnx_array_pop")?;
    let any_box = runtime_fn("rnx_any_box")?;
    let any_unbox = runtime_fn("rnx_any_unbox")?;
    let any_unbox_heap = runtime_fn("rnx_any_unbox_heap")?;
    let any_retain = runtime_fn("rnx_any_retain")?;
    let any_release_box = runtime_fn("rnx_any_release_box")?;
    let any_release = runtime_fn("rnx_any_release")?;
    let heap_track = runtime_fn("rnx_heap_track")?;
    let closure_new = runtime_fn("rnx_closure_new")?;
    let closure_set = runtime_fn("rnx_closure_set")?;
    let closure_release = runtime_fn("rnx_closure_release")?;
    let spawn_closure = runtime_fn("rnx_thread_spawn_closure")?;
    let join_val = runtime_fn("rnx_thread_join_val")?;
    let join_err = runtime_fn("rnx_thread_join_err")?;
    let task_val = runtime_fn("rnx_task_await_val")?;
    let task_err = runtime_fn("rnx_task_await_err")?;
    let pool_new = runtime_fn("rnx_pool_new")?;
    let submit_handle = runtime_fn("rnx_thread_pool_submit_handle")?;
    let submit_closure = runtime_fn("rnx_thread_pool_submit_closure")?;
    let parallel_closure = runtime_fn("rnx_thread_pool_parallel_closure")?;
    let panic_str = runtime_fn("rnx_panic_str")?;
    let fatal_span = runtime_fn("rnx_fatal_span")?;
    let release_array = runtime_fn("rnx_release_array")?;
    let thread_spawn = runtime_fn("rnx_thread_spawn")?;
    let thread_join = runtime_fn("rnx_thread_join")?;
    let clock_mono = runtime_fn("rnx_clock_monotonic_nanos")?;
    let crypto_random = runtime_fn("rnx_crypto_random_u64")?;
    let prng_seed = runtime_fn("rnx_prng_seed")?;
    let prng_next = runtime_fn("rnx_prng_next")?;
    let file_open = runtime_fn("rnx_file_open")?;
    let file_close = runtime_fn("rnx_file_close")?;
    let file_read = runtime_fn("rnx_file_read_text")?;
    let file_read_err = runtime_fn("rnx_file_read_text_err")?;
    let file_write = runtime_fn("rnx_file_write_text")?;
    let file_flush = runtime_fn("rnx_file_flush")?;
    let file_seek = runtime_fn("rnx_file_seek")?;
    let file_tell = runtime_fn("rnx_file_tell")?;
    let file_from_handle = runtime_fn("rnx_file_from_handle")?;
    let io_is_tty = runtime_fn("rnx_io_is_tty")?;
    let io_winsize = runtime_fn("rnx_io_winsize")?;
    let io_set_raw = runtime_fn("rnx_io_set_raw")?;
    let path_exists = runtime_fn("rnx_path_exists")?;
    let path_remove = runtime_fn("rnx_path_remove")?;
    let fs_exists = runtime_fn("rnx_fs_exists")?;
    let fs_is_file = runtime_fn("rnx_fs_is_file")?;
    let fs_is_dir = runtime_fn("rnx_fs_is_dir")?;
    let fs_stat = runtime_fn("rnx_fs_stat")?;
    let fs_stat_err = runtime_fn("rnx_fs_stat_err")?;
    let fs_read_dir = runtime_fn("rnx_fs_read_dir")?;
    let fs_read_dir_err = runtime_fn("rnx_fs_read_dir_err")?;
    let fs_glob = runtime_fn("rnx_fs_glob")?;
    let fs_glob_err = runtime_fn("rnx_fs_glob_err")?;
    let fs_read_link = runtime_fn("rnx_fs_read_link")?;
    let fs_read_link_err = runtime_fn("rnx_fs_read_link_err")?;
    let fs_remove = runtime_fn("rnx_fs_remove")?;
    let fs_remove_err = runtime_fn("rnx_fs_remove_err")?;
    let fs_remove_all = runtime_fn("rnx_fs_remove_all")?;
    let fs_remove_all_err = runtime_fn("rnx_fs_remove_all_err")?;
    let fs_mkdir_err = runtime_fn("rnx_fs_mkdir_err")?;
    let fs_copy_err = runtime_fn("rnx_fs_copy_err")?;
    let fs_move_err = runtime_fn("rnx_fs_move_err")?;
    let fs_rename_err = runtime_fn("rnx_fs_rename_err")?;
    let fs_truncate_err = runtime_fn("rnx_fs_truncate_err")?;
    let fs_chmod_err = runtime_fn("rnx_fs_chmod_err")?;
    let fs_symlink_err = runtime_fn("rnx_fs_symlink_err")?;
    let fs_fsync_err = runtime_fn("rnx_fs_fsync_err")?;
    let fs_read_text = runtime_fn("rnx_fs_read_text")?;
    let fs_read_text_err = runtime_fn("rnx_fs_read_text_err")?;
    let fs_write_text = runtime_fn("rnx_fs_write_text")?;
    let fs_write_text_err = runtime_fn("rnx_fs_write_text_err")?;
    let fs_read_bytes = runtime_fn("rnx_fs_read_bytes")?;
    let fs_read_bytes_err = runtime_fn("rnx_fs_read_bytes_err")?;
    let fs_write_bytes = runtime_fn("rnx_fs_write_bytes")?;
    let fs_write_bytes_err = runtime_fn("rnx_fs_write_bytes_err")?;
    let fs_mmap = runtime_fn("rnx_fs_mmap")?;
    let fs_mmap_err = runtime_fn("rnx_fs_mmap_err")?;
    let fs_mmap_anon = runtime_fn("rnx_fs_mmap_anon")?;
    let fs_mmap_anon_err = runtime_fn("rnx_fs_mmap_anon_err")?;
    let fs_mmap_addr = runtime_fn("rnx_fs_mmap_addr")?;
    let fs_mmap_len = runtime_fn("rnx_fs_mmap_len")?;
    let fs_mmap_flush = runtime_fn("rnx_fs_mmap_flush")?;
    let fs_mmap_close = runtime_fn("rnx_fs_mmap_close")?;
    let test_assert = runtime_fn("rnx_assert")?;
    let test_check = runtime_fn("rnx_test_check")?;
    let env_count = runtime_fn("rnx_env_args_count")?;
    let env_args_get = runtime_fn("rnx_env_args_get")?;
    let env_get = runtime_fn("rnx_env_get")?;
    let env_set = runtime_fn("rnx_env_set")?;
    let env_cwd = runtime_fn("rnx_env_cwd")?;
    let host_version = runtime_fn("rnx_host_version")?;
    let env_exit = runtime_fn("rnx_env_exit")?;
    let net_connect = runtime_fn("rnx_net_connect_start")?;
    let net_take_error = runtime_fn("rnx_net_take_error")?;
    let net_connect_wait = runtime_fn("rnx_net_connect_wait")?;
    let net_recv_wait = runtime_fn("rnx_net_recv_or_wait")?;
    let net_send_wait = runtime_fn("rnx_net_send_or_wait")?;
    let net_recv_get = runtime_fn("rnx_net_recv_get")?;
    let net_error_text = runtime_fn("rnx_net_error_text")?;
    let net_close = runtime_fn("rnx_net_close")?;
    let net_listener_bind = runtime_fn("rnx_net_listener_bind")?;
    let net_listener_port = runtime_fn("rnx_net_listener_port")?;
    let net_listener_accept_start = runtime_fn("rnx_net_listener_accept_start")?;
    let net_listener_accept_wait = runtime_fn("rnx_net_listener_accept_wait")?;
    let net_listener_close = runtime_fn("rnx_net_listener_close")?;
    let tls_connect_start = runtime_fn("rnx_tls_connect_start")?;
    let tls_handshake_start = runtime_fn("rnx_tls_handshake_start")?;
    let tls_handshake_wait = runtime_fn("rnx_tls_handshake_wait")?;
    let tls_recv_wait = runtime_fn("rnx_tls_recv_or_wait")?;
    let tls_send_wait = runtime_fn("rnx_tls_send_or_wait")?;
    let tls_recv_get = runtime_fn("rnx_tls_recv_get")?;
    let tls_error_text = runtime_fn("rnx_tls_error_text")?;
    let tls_close = runtime_fn("rnx_tls_close")?;
    let proc_pid = runtime_fn("rnx_process_pid")?;
    let proc_remove_env = runtime_fn("rnx_process_remove_env")?;
    let proc_all_env_count = runtime_fn("rnx_process_all_env_count")?;
    let proc_all_env_get = runtime_fn("rnx_process_all_env_get")?;
    let proc_chdir = runtime_fn("rnx_process_chdir")?;
    let proc_spawn = runtime_fn("rnx_process_spawn")?;
    let proc_run = runtime_fn("rnx_process_run")?;
    let proc_pid_of = runtime_fn("rnx_process_pid_of")?;
    let proc_write_stdin = runtime_fn("rnx_process_write_stdin")?;
    let proc_read_stdout = runtime_fn("rnx_process_read_stdout")?;
    let proc_read_stderr = runtime_fn("rnx_process_read_stderr")?;
    let proc_close_stdin = runtime_fn("rnx_process_close_stdin")?;
    let proc_wait = runtime_fn("rnx_process_wait")?;
    let proc_try_wait = runtime_fn("rnx_process_try_wait")?;
    let proc_kill = runtime_fn("rnx_process_kill")?;
    let proc_take_stdout = runtime_fn("rnx_process_take_stdout")?;
    let proc_take_stderr = runtime_fn("rnx_process_take_stderr")?;
    let proc_exit_code = runtime_fn("rnx_process_exit_code")?;
    let proc_forget = runtime_fn("rnx_process_forget")?;
    let os_platform = runtime_fn("rnx_os_platform")?;
    let os_arch = runtime_fn("rnx_os_arch")?;
    let os_hostname = runtime_fn("rnx_os_hostname")?;
    let os_tmpdir = runtime_fn("rnx_os_tmpdir")?;
    let os_homedir = runtime_fn("rnx_os_homedir")?;
    let os_cpu_count = runtime_fn("rnx_os_cpu_count")?;
    let os_uptime = runtime_fn("rnx_os_uptime")?;
    let map_new = runtime_fn("rnx_map_new")?;
    let map_set = runtime_fn("rnx_map_set")?;
    let map_get = runtime_fn("rnx_map_get")?;
    let map_has = runtime_fn("rnx_map_has")?;
    let map_delete = runtime_fn("rnx_map_delete")?;
    let map_len = runtime_fn("rnx_map_len")?;
    let map_clear = runtime_fn("rnx_map_clear")?;
    let map_keys = runtime_fn("rnx_map_keys")?;
    let map_values = runtime_fn("rnx_map_values")?;
    let gmap_new = runtime_fn("rnx_gmap_new")?;
    let gmap_free = runtime_fn("rnx_gmap_free")?;
    let gmap_set = runtime_fn("rnx_gmap_set")?;
    let gmap_get = runtime_fn("rnx_gmap_get")?;
    let gmap_has = runtime_fn("rnx_gmap_has")?;
    let gmap_delete = runtime_fn("rnx_gmap_delete")?;
    let gmap_len = runtime_fn("rnx_gmap_len")?;
    let gmap_clear = runtime_fn("rnx_gmap_clear")?;
    let gmap_keys = runtime_fn("rnx_gmap_keys")?;
    let gmap_values = runtime_fn("rnx_gmap_values")?;
    let json_parse = runtime_fn("rnx_json_parse")?;
    let json_parse_typed = runtime_fn("rnx_json_parse_typed")?;
    let json_stringify_into = runtime_fn("rnx_json_stringify_into")?;
    let json_stringify = runtime_fn("rnx_json_stringify")?;
    let json_unwrap = runtime_fn("rnx_json_unwrap")?;
    let dns_lookup_start = runtime_fn("rnx_dns_lookup_start")?;
    let dns_lookup_wait = runtime_fn("rnx_dns_lookup_wait")?;
    let dns_lookup_get = runtime_fn("rnx_dns_lookup_get")?;
    let dns_lookup_error = runtime_fn("rnx_dns_lookup_error")?;
    let sync_atomic_get = runtime_fn("rnx_sync_atomic_get")?;
    let black_box = runtime_fn("rnx_black_box_i64")?;
    let sync_atomic_set = runtime_fn("rnx_sync_atomic_set")?;
    let sync_atomic_fetch_add = runtime_fn("rnx_sync_atomic_fetch_add")?;
    let sync_atomic_cas = runtime_fn("rnx_sync_atomic_cas")?;
    let sync_channel_send = runtime_fn("rnx_sync_channel_send")?;
    let sync_channel_send_str = runtime_fn("rnx_sync_channel_send_str")?;
    let sync_channel_send_obj = runtime_fn("rnx_sync_channel_send_obj")?;
    let sync_channel_send_array = runtime_fn("rnx_sync_channel_send_array")?;
    let sync_channel_recv = runtime_fn("rnx_sync_channel_recv")?;
    let sync_channel_try_recv = runtime_fn("rnx_sync_channel_try_recv")?;
    let sync_channel_len = runtime_fn("rnx_sync_channel_len")?;
    let fs_pool_depth = runtime_fn("rnx_fs_pool_depth")?;
    let sync_channel_drop = runtime_fn("rnx_sync_channel_drop")?;
    let debug_live = runtime_fn("rnx_debug_live_count")?;
    let defer_push = runtime_fn("rnx_defer_push")?;
    let defer_pop = runtime_fn("rnx_defer_pop")?;
    let defer_len = runtime_fn("rnx_defer_len")?;
    let any_tag = runtime_fn("rnx_any_tag")?;
    let obj_class = runtime_fn("rnx_obj_class")?;
    let error_set = runtime_fn("rnx_error_set")?;
    let error_take = runtime_fn("rnx_error_take")?;
    let error_class = runtime_fn("rnx_error_class")?;
    let error_unbox = runtime_fn("rnx_error_unbox")?;
    let error_str = runtime_fn("rnx_error_str")?;
    let error_release = runtime_fn("rnx_error_release")?;
    let note_type = runtime_fn("rnx_note_type")?;
    let type_name = runtime_fn("rnx_type_name")?;
    let typeof_any = runtime_fn("rnx_typeof_any")?;
    let io_pretty = runtime_fn("rnx_io_pretty")?;
    let note_array_kind = runtime_fn("rnx_note_array_kind")?;
    let note_fields = runtime_fn("rnx_note_fields")?;
    let note_enum = runtime_fn("rnx_note_enum")?;
    let note_namespace = runtime_fn("rnx_note_namespace")?;
    let array_slice = runtime_fn("rnx_array_slice")?;
    let mutex_lock = runtime_fn("rnx_mutex_lock")?;
    let mutex_unlock = runtime_fn("rnx_mutex_unlock")?;
    let mutex_try_lock = runtime_fn("rnx_mutex_try_lock")?;
    let rwlock_read_lock = runtime_fn("rnx_rwlock_read_lock")?;
    let rwlock_read_unlock = runtime_fn("rnx_rwlock_read_unlock")?;
    let rwlock_write_lock = runtime_fn("rnx_rwlock_write_lock")?;
    let rwlock_write_unlock = runtime_fn("rnx_rwlock_write_unlock")?;
    let rwlock_try_read_lock = runtime_fn("rnx_rwlock_try_read_lock")?;
    let rwlock_try_write_lock = runtime_fn("rnx_rwlock_try_write_lock")?;
    let condvar_wait = runtime_fn("rnx_condvar_wait")?;
    let condvar_wait_timeout = runtime_fn("rnx_condvar_wait_timeout")?;
    let condvar_notify_one = runtime_fn("rnx_condvar_notify_one")?;
    let condvar_notify_all = runtime_fn("rnx_condvar_notify_all")?;
    let barrier_wait = runtime_fn("rnx_barrier_wait")?;
    let vec_minnum = runtime_fn("llvm.minnum.v4f32")?;
    let vec_maxnum = runtime_fn("llvm.maxnum.v4f32")?;
    let vec_sqrt = runtime_fn("llvm.sqrt.v4f32")?;
    let pool_init = runtime_fn("rnx_thread_pool_init")?;
    let pool_parallel_for = runtime_fn("rnx_thread_pool_parallel_for")?;
    let pool_join = runtime_fn("rnx_thread_pool_join")?;
    let pool_shutdown = runtime_fn("rnx_thread_pool_shutdown")?;
    let math_sqrt = runtime_fn("rnx_math_sqrt")?;
    let math_sin = runtime_fn("rnx_math_sin")?;
    let math_cos = runtime_fn("rnx_math_cos")?;
    let math_tan = runtime_fn("rnx_math_tan")?;
    let math_atan2 = runtime_fn("rnx_math_atan2")?;
    let math_pow = runtime_fn("rnx_math_pow")?;
    let math_floor = runtime_fn("rnx_math_floor")?;
    let math_ceil = runtime_fn("rnx_math_ceil")?;
    let math_round = runtime_fn("rnx_math_round")?;
    let math_log = runtime_fn("rnx_math_log")?;
    let strings = declare_strings(context, module, lir, is_release)?;
    let mut cx = FnCx {
        context,
        sqrt_fn: {
            use inkwell::intrinsics::Intrinsic;
            Intrinsic::find("llvm.sqrt").and_then(|i| {
                i.get_declaration(module, &[context.f64_type().into()])
            })
        },
        fma_fn: {
            use inkwell::intrinsics::Intrinsic;
            Intrinsic::find("llvm.fma").and_then(|i| {
                i.get_declaration(module, &[context.f64_type().into()])
            })
        },
        tbaa_kind: context.get_kind_id("tbaa"),
        tbaa_header: {
            let zero = context.i64_type().const_int(0, false);
            let root = context.metadata_node(&[context.metadata_string("rnx").into()]);
            let ty = context.metadata_node(&[
                context.metadata_string("rnx.header").into(),
                root.into(),
            ]);
            context.metadata_node(&[ty.into(), ty.into(), zero.into()])
        },
        tbaa_payload: {
            let zero = context.i64_type().const_int(0, false);
            let root = context.metadata_node(&[context.metadata_string("rnx").into()]);
            let ty = context.metadata_node(&[
                context.metadata_string("rnx.payload").into(),
                root.into(),
            ]);
            context.metadata_node(&[ty.into(), ty.into(), zero.into()])
        },
        builder: context.create_builder(),
        funcs: funcs.clone(),
        alloc,
        release,
        genref_create,
        genref_get,
        genref_invalidate,
        retain,
        concat,
        string_split,
        streq,
        strcmp,
        print_val,
        print_str,
        int_to_str,
        float_to_str,
        bool_to_str,
        any_to_str,
        eq_any_str,
        eq_any,
        release_str,
        array_new,
        array_push,
        array_get,
        array_set,
        array_get_unchecked,
        array_set_unchecked,
        array_len,
        string_len,
        string_slice,
        string_index_of,
        string_index_of_from,
        string_trim,
        string_char_code_at,
        string_from_char_code,
        array_pop_fn,
        any_box,
        any_unbox,
        any_unbox_heap,
        any_retain,
        any_release_box,
        any_release,
        heap_track,
        closure_new,
        closure_set,
        closure_release,
        spawn_closure,
        join_val,
        join_err,
        task_val,
        task_err,
        pool_new,
        submit_handle,
        submit_closure,
        parallel_closure,
        panic_str,
        fatal_span,
        release_array,
        thread_spawn,
        thread_join,
        clock_mono,
        crypto_random,
        prng_seed,
        prng_next,
        file_open,
        file_close,
        file_read,
        file_read_err,
        file_write,
        file_flush,
        file_seek,
        file_tell,
        file_from_handle,
        io_is_tty,
        io_winsize,
        io_set_raw,
        path_exists,
        path_remove,
        fs_exists,
        fs_is_file,
        fs_is_dir,
        fs_stat,
        fs_stat_err,
        fs_read_dir,
        fs_read_dir_err,
        fs_glob,
        fs_glob_err,
        fs_read_link,
        fs_read_link_err,
        fs_remove,
        fs_remove_err,
        fs_remove_all,
        fs_remove_all_err,
        fs_mkdir_err,
        fs_copy_err,
        fs_move_err,
        fs_rename_err,
        fs_truncate_err,
        fs_chmod_err,
        fs_symlink_err,
        fs_fsync_err,
        fs_read_text,
        fs_read_text_err,
        fs_write_text,
        fs_write_text_err,
        fs_read_bytes,
        fs_read_bytes_err,
        fs_write_bytes,
        fs_write_bytes_err,
        fs_mmap,
        fs_mmap_err,
        fs_mmap_anon,
        fs_mmap_anon_err,
        fs_mmap_addr,
        fs_mmap_len,
        fs_mmap_flush,
        fs_mmap_close,
        test_assert,
        test_check,
        env_count,
        env_args_get,
        env_get,
        env_set,
        env_cwd,
        host_version,
        env_exit,
        net_connect,
        net_take_error,
        net_connect_wait,
        net_recv_wait,
        net_send_wait,
        net_recv_get,
        net_error_text,
        net_close,
        net_listener_bind,
        net_listener_port,
        net_listener_accept_start,
        net_listener_accept_wait,
        net_listener_close,
        tls_connect_start,
        tls_handshake_start,
        tls_handshake_wait,
        tls_recv_wait,
        tls_send_wait,
        tls_recv_get,
        tls_error_text,
        tls_close,
        proc_pid,
        proc_remove_env,
        proc_all_env_count,
        proc_all_env_get,
        proc_chdir,
        proc_spawn,
        proc_run,
        proc_pid_of,
        proc_write_stdin,
        proc_read_stdout,
        proc_read_stderr,
        proc_close_stdin,
        proc_wait,
        proc_try_wait,
        proc_kill,
        proc_take_stdout,
        proc_take_stderr,
        proc_exit_code,
        proc_forget,
        os_platform,
        os_arch,
        os_hostname,
        os_tmpdir,
        os_homedir,
        os_cpu_count,
        os_uptime,
        math_sqrt,
        math_sin,
        math_cos,
        math_tan,
        math_atan2,
        math_pow,
        math_floor,
        math_ceil,
        math_round,
        math_log,
        map_new,
        map_set,
        map_get,
        map_has,
        map_delete,
        map_len,
        map_clear,
        map_keys,
        map_values,
        gmap_new,
        gmap_free,
        gmap_set,
        gmap_get,
        gmap_has,
        gmap_delete,
        gmap_len,
        gmap_clear,
        gmap_keys,
        gmap_values,
        json_parse,
        json_parse_typed,
        json_stringify_into,
        json_stringify,
        json_unwrap,
        dns_lookup_start,
        dns_lookup_wait,
        dns_lookup_get,
        dns_lookup_error,
        sync_atomic_get,
        black_box,
        sync_atomic_set,
        sync_atomic_fetch_add,
        sync_atomic_cas,
        sync_channel_send,
        sync_channel_send_str,
        sync_channel_send_obj,
        sync_channel_send_array,
        sync_channel_recv,
        sync_channel_try_recv,
        sync_channel_len,
        fs_pool_depth,
        sync_channel_drop,
        debug_live,
        defer_push,
        defer_pop,
        defer_len,
        any_tag,
        obj_class,
        error_set,
        error_take,
        error_class,
        error_unbox,
        error_str,
        error_release,
        note_type,
        type_name,
        typeof_any,
        io_pretty,
        note_array_kind,
        note_fields,
        note_enum,
        note_namespace,
        array_slice,
        mutex_lock,
        mutex_unlock,
        mutex_try_lock,
        rwlock_read_lock,
        rwlock_read_unlock,
        rwlock_write_lock,
        rwlock_write_unlock,
        rwlock_try_read_lock,
        rwlock_try_write_lock,
        condvar_wait,
        condvar_wait_timeout,
        condvar_notify_one,
        condvar_notify_all,
        barrier_wait,
        vec_minnum,
        vec_maxnum,
        vec_sqrt,
        pool_init,
        pool_parallel_for,
        pool_join,
        pool_shutdown,
        strings,
        bytes_fns: bytes_fns_map(&module),
        locals: Vec::with_capacity(f.locals.len()),
        blocks: Vec::with_capacity(f.blocks.len()),
        stack_slots: Vec::new(),
        stack_next: 0,
        stack: stack_locals(f),
        ftypes: f.locals.clone(),
        ret_slots: lir::instr::flat_sig(&f.ret).len().max(1),
        owned: BTreeSet::new(),
        ever_owned: BTreeSet::new(),
        nborrowed: f.params.len(),
        dom: lir::licm::compute_dominators(f),
        cur_block: 0,
        last_write: std::cell::RefCell::new(BTreeMap::new()),
        in_enum_dtor: f_raw.name.starts_with("__enum_dtor_"),
        defers: Vec::new(),
        defer_base: None,
    };
    for b in &f.blocks {
        lir::instr::walk_instrs(&b.instrs, &mut |ins| {
            if let Instr::Defer { body, .. } = ins {
                if !cx.defers.iter().any(|x| x == body) {
                    cx.defers.push(body.clone());
                }
            }
        });
    }
    for _ in &f.blocks {
        cx.blocks.push(context.append_basic_block(fv, "bb"));
    }
    cx.builder.position_at_end(entry);
    if let Some(loc) = debug_loc {
        cx.builder.set_current_debug_location(loc);
    }
    if !cx.defers.is_empty() {
        let slot = cx.builder.build_alloca(i64t, "defer_base").map_err(err)?;
        cx.defer_base = Some(slot);
    }
    for t in f.locals.iter() {
        let aty = local_alloca_ty(context, t);
        let slot = cx.builder.build_alloca(aty, "l").map_err(err)?;
        cx.locals.push(slot);
    }
    for block in &f.blocks {
        for ins in &block.instrs {
            if let Instr::StackAlloc { instance_size: size, .. } = ins {
                let arrty = i64t.array_type(((*size as u64 + 7) / 8) as u32);
                let slot = cx.builder.build_alloca(arrty, "stk").map_err(err)?;
                cx.stack_slots.push(slot);
            }
        }
    }
    for (i, param) in fv.get_param_iter().enumerate() {
        cx.builder.build_store(cx.locals[i], param).map_err(err)?;
    }
    for (n, t) in f.locals.iter().enumerate() {
        if matches!(t, LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Error | LirType::Any | LirType::Closure)
            && n >= cx.nborrowed
        {
            cx.builder
                .build_store(cx.locals[n], i64t.const_zero())
                .map_err(err)?;
        }
    }
    if let Some(slot) = cx.defer_base {
        let site = cx.builder.build_call(cx.defer_len, &[], "").map_err(err)?;
        match site.try_as_basic_value() {
            ValueKind::Basic(v) => {
                cx.builder.build_store(slot, v).map_err(err)?;
            }
            ValueKind::Instruction(_) => {
                return Err(Diagnostic::new(Code::E108, "llvm subset: defer base"));
            }
        }
    }
    cx.builder.build_unconditional_branch(cx.blocks[0]).map_err(err)?;
    for i in lir::opt::rpo_order(f) {
        let block = &f.blocks[i];
        cx.builder.position_at_end(cx.blocks[i]);
        cx.cur_block = i;
        for ins in &block.instrs {
            lower_instr(&mut cx, lir, ins, &f.name)?;
        }
        lower_term(&mut cx, lir, &block.term)?;
    }
    tag_loop_unrolls(&cx, f)?;
    if fv.verify(true) {
        Ok(())
    } else {
        Err(Diagnostic::new(Code::E108, format!("llvm verify failed in `{}`", f.name)))
    }
}

fn lower_term(cx: &mut FnCx, lir: &Module, term: &Terminator) -> Result<(), Diagnostic> {
    match term {
        Terminator::Ret(v) => {
            let ret_locals = v.clone();
            // Releases must not drain tracking: later returns need the full set.
            let saved_owned = cx.owned.clone();
            let saved_ever = cx.ever_owned.clone();
            let live: Vec<Local> = cx.ever_owned.iter().copied().collect();
            for l in live {
                if ret_locals.contains(&l) {
                    continue;
                }
                release_any(cx, lir, l, None)?;
            }
            cx.owned = saved_owned;
            cx.ever_owned = saved_ever;
            let r = match ret_locals.as_slice() {
                [] => cx.context.i64_type().const_zero().into(),
                [l] => load_value(cx, *l)?,
                ls => {
                    let fields: Vec<BasicTypeEnum> = ls
                        .iter()
                        .map(|l| match cx.ftypes.get(*l as usize) {
                            Some(LirType::Vec4f) => cx.context.f32_type().vec_type(4).into(),
                            Some(LirType::Vec4i) => cx.context.i32_type().vec_type(4).into(),
                            _ => cx.context.i64_type().into(),
                        })
                        .collect();
                    let st = cx.context.struct_type(&fields, false);
                    let mut agg = st.get_undef();
                    for (i, l) in ls.iter().enumerate() {
                        let val = load_value(cx, *l)?;
                        agg = cx.builder.build_insert_value(agg, val, i as u32, "tup").map_err(err)?.into_struct_value();
                    }
                    agg.into()
                }
            };
            cx.builder.build_return(Some(&r)).map_err(err)?;
        }
        Terminator::Br(bb) => {
            cx.builder.build_unconditional_branch(cx.blocks[*bb]).map_err(err)?;
        }
        Terminator::Throw { src, catch, .. } => {
            let st = cx.ftypes.get(*src as usize).cloned().unwrap_or(LirType::Any);
            let payload: inkwell::values::IntValue = match st {
                LirType::Str | LirType::Error => {
                    let v = load(cx, *src)?;
                    let ptr = as_ptr(cx, v)?;
                    cx.builder.build_call(cx.retain, &[ptr.into()], "").map_err(err)?;
                    v
                }
                LirType::Obj(_) => {
                    let v = load(cx, *src)?;
                    let ptr = as_ptr(cx, v)?;
                    cx.builder.build_call(cx.retain, &[ptr.into()], "").map_err(err)?;
                    let one = cx.context.i64_type().const_int(1, false);
                    cx.builder.build_or(v, one, "errtag").map_err(err)?
                }
                LirType::I64 | LirType::I8 => {
                    let v = load(cx, *src)?;
                    let site = cx.builder.build_call(cx.int_to_str, &[v.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(x) => {
                            let ptr = x.into_pointer_value();
                            cx.builder.build_ptr_to_int(ptr, cx.context.i64_type(), "").map_err(err)?
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: throw int convert"));
                        }
                    }
                }
                LirType::F64(_) => {
                    let v = load(cx, *src)?;
                    let site = cx.builder.build_call(cx.float_to_str, &[v.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(x) => {
                            let ptr = x.into_pointer_value();
                            cx.builder.build_ptr_to_int(ptr, cx.context.i64_type(), "").map_err(err)?
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: throw float convert"));
                        }
                    }
                }
                LirType::Bool => {
                    let v = load(cx, *src)?;
                    let site = cx.builder.build_call(cx.bool_to_str, &[v.into()], "").map_err(err)?;
                    match site.try_as_basic_value() {
                        ValueKind::Basic(x) => {
                            let ptr = x.into_pointer_value();
                            cx.builder.build_ptr_to_int(ptr, cx.context.i64_type(), "").map_err(err)?
                        }
                        ValueKind::Instruction(_) => {
                            return Err(Diagnostic::new(Code::E108, "llvm subset: throw bool convert"));
                        }
                    }
                }
                _ => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: throw payload"));
                }
            };
            if let Some((bb, bind, _depth)) = catch {
                store(cx, *bind, payload.into())?;
                if !cx.stack.contains(bind) {
                    own(cx, *bind);
                }
                cx.builder.build_unconditional_branch(cx.blocks[*bb]).map_err(err)?;
                return Ok(());
            }
            cx.builder.build_call(cx.error_set, &[payload.into()], "").map_err(err)?;
            if cx.ret_slots <= 1 {
                let z = cx.context.i64_type().const_zero();
                cx.builder.build_return(Some(&z)).map_err(err)?;
            } else {
                let fields: Vec<inkwell::types::BasicTypeEnum> =
                    (0..cx.ret_slots).map(|_| cx.context.i64_type().into()).collect();
                let st = cx.context.struct_type(&fields, false);
                let mut agg = st.get_undef();
                for i in 0..cx.ret_slots {
                    let z = cx.context.i64_type().const_zero();
                    agg = cx.builder.build_insert_value(agg, z, i as u32, "errz").map_err(err)?.into_struct_value();
                }
                cx.builder.build_return(Some(&agg)).map_err(err)?;
            }
        }
        Terminator::BrErr { catch_bb, catch_bind, next_bb, depth, .. } => {
            emit_run_defers(cx, lir, *depth)?;
            let site = cx.builder.build_call(cx.error_take, &[], "errtake").map_err(err)?;
            let w = match site.try_as_basic_value() {
                ValueKind::Basic(x) => x.into_int_value(),
                ValueKind::Instruction(_) => {
                    return Err(Diagnostic::new(Code::E108, "llvm subset: error take"));
                }
            };
            store(cx, *catch_bind, w.into())?;
            if !cx.stack.contains(catch_bind) {
                own(cx, *catch_bind);
            }
            let zero = cx.context.i64_type().const_zero();
            let is_err = cx.builder.build_int_compare(inkwell::IntPredicate::NE, w, zero, "iserr").map_err(err)?;
            cx.builder.build_conditional_branch(is_err, cx.blocks[*catch_bb], cx.blocks[*next_bb]).map_err(err)?;
        }
        Terminator::BrIf { cond, then_bb, else_bb, .. } => {
            let c = load(cx, *cond)?;
            let zero = cx.context.i64_type().const_zero();
            let t = cx
                .builder
                .build_int_compare(inkwell::IntPredicate::NE, c, zero, "br")
                .map_err(err)?;
            cx.builder
                .build_conditional_branch(t, cx.blocks[*then_bb], cx.blocks[*else_bb])
                .map_err(err)?;
        }
        Terminator::Switch { scrut, cases, default, .. } => {
            let disc = match cx.ftypes.get(*scrut as usize) {
                Some(LirType::Enum(_)) => {
                    let base = load(cx, *scrut)?;
                    let at = byte_ptr(cx, base, 16)?;
                    cx.builder.build_load(cx.context.i64_type(), at, "tag").map_err(err)?.into_int_value()
                }
                _ => load(cx, *scrut)?,
            };
            enum Arm<'x> {
                Int(i64),
                Check(inkwell::values::IntValue<'x>),
            }
            let mut arms: Vec<Arm> = Vec::with_capacity(cases.len());
            for (pat, _) in cases.iter() {
                match pat {
                    SwitchPat::Int(n) => arms.push(Arm::Int(*n)),
                    SwitchPat::Enum { variant, .. } => arms.push(Arm::Int(*variant as i64)),
                    SwitchPat::Is { source, check, .. } => {
                        use lir::instr::IsDecision;
                        let cond: Option<inkwell::values::IntValue> = match check {
                            IsDecision::Const(false) => None,
                            IsDecision::Const(true) => {
                                let one = cx.context.i64_type().const_int(1, false);
                                Some(cx.builder.build_int_compare(inkwell::IntPredicate::EQ, one, one, "sw_true").map_err(err)?)
                            }
                            IsDecision::Tag(pt) => {
                                let v = load(cx, *source)?;
                                let site = cx.builder.build_call(cx.any_tag, &[v.into()], "").map_err(err)?;
                                let t = match site.try_as_basic_value() {
                                    ValueKind::Basic(x) => x.into_int_value(),
                                    ValueKind::Instruction(_) => {
                                        return Err(Diagnostic::new(Code::E108, "llvm subset: switch tag call"));
                                    }
                                };
                                let want = cx.context.i64_type().const_int(*pt as u64, true);
                                Some(cx.builder.build_int_compare(inkwell::IntPredicate::EQ, t, want, "sw_is").map_err(err)?)
                            }
                            IsDecision::Class(ci) => {
                                let class_fn = if matches!(cx.ftypes.get(*source as usize), Some(LirType::Error)) {
                                    cx.error_class
                                } else {
                                    cx.obj_class
                                };
                                let v = load(cx, *source)?;
                                let site = cx.builder.build_call(class_fn, &[v.into()], "").map_err(err)?;
                                let t = match site.try_as_basic_value() {
                                    ValueKind::Basic(x) => x.into_int_value(),
                                    ValueKind::Instruction(_) => {
                                        return Err(Diagnostic::new(Code::E108, "llvm subset: switch class call"));
                                    }
                                };
                                let mut acc: Option<inkwell::values::IntValue> = None;
                                for sub in std::iter::once(*ci).chain(lir::instr::subclasses_of(lir, *ci)) {
                                    let want = cx.context.i64_type().const_int(sub as u64 + 1, false);
                                    let eq = cx.builder.build_int_compare(inkwell::IntPredicate::EQ, t, want, "sw_is").map_err(err)?;
                                    acc = Some(match acc {
                                        Some(prev) => cx.builder.build_or(prev, eq, "sw_or").map_err(err)?,
                                        None => eq,
                                    });
                                }
                                Some(acc.ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: empty class hierarchy in is-check"))?)
                            }
                            IsDecision::Iface(ii) => {
                                let class_fn = if matches!(cx.ftypes.get(*source as usize), Some(LirType::Error)) {
                                    cx.error_class
                                } else {
                                    cx.obj_class
                                };
                                let v = load(cx, *source)?;
                                let site = cx.builder.build_call(class_fn, &[v.into()], "").map_err(err)?;
                                let t = match site.try_as_basic_value() {
                                    ValueKind::Basic(x) => x.into_int_value(),
                                    ValueKind::Instruction(_) => {
                                        return Err(Diagnostic::new(Code::E108, "llvm subset: switch iface call"));
                                    }
                                };
                                let mut acc: Option<inkwell::values::IntValue> = None;
                                for (ci, c) in lir.classes.iter().enumerate() {
                                    if !c.ifaces.contains(ii) {
                                        continue;
                                    }
                                    let want = cx.context.i64_type().const_int(ci as u64 + 1, false);
                                    let eq = cx.builder.build_int_compare(inkwell::IntPredicate::EQ, t, want, "sw_iface").map_err(err)?;
                                    acc = Some(match acc {
                                        Some(prev) => cx.builder.build_or(prev, eq, "sw_or").map_err(err)?,
                                        None => eq,
                                    });
                                }
                                Some(match acc {
                                    Some(v) => v,
                                    None => {
                                        let z = cx.context.i64_type().const_zero();
                                        cx.builder.build_int_compare(inkwell::IntPredicate::EQ, t, z, "sw_never").map_err(err)?
                                    }
                                })
                            }
                        };
                        if let Some(c) = cond {
                            arms.push(Arm::Check(c));
                        }
                    }
                    _ => return Err(Diagnostic::new(Code::E108, "llvm subset: switch pattern")),
                }
            }
            if arms.is_empty() {
                cx.builder.build_unconditional_branch(cx.blocks[*default]).map_err(err)?;
                return Ok(());
            }
            let fv = cx.builder.get_insert_block().and_then(|b| b.get_parent()).ok_or_else(|| {
                Diagnostic::new(Code::E108, "llvm subset: switch outside function")
            })?;
            let mut nexts = Vec::with_capacity(arms.len().saturating_sub(1));
            for _ in 1..arms.len() {
                nexts.push(cx.context.append_basic_block(fv, "sw_next"));
            }
            for (i, arm) in arms.iter().enumerate() {
                let t = match arm {
                    Arm::Int(n) => {
                        let want = cx.context.i64_type().const_int(*n as u64, true);
                        cx.builder.build_int_compare(inkwell::IntPredicate::EQ, disc, want, "sw_eq").map_err(err)?
                    }
                    Arm::Check(c) => *c,
                };
                let else_bb = if i + 1 < arms.len() { nexts[i] } else { cx.blocks[*default] };
                cx.builder.build_conditional_branch(t, cx.blocks[cases[i].1], else_bb).map_err(err)?;
                if i + 1 < arms.len() {
                    cx.builder.position_at_end(nexts[i]);
                }
            }
        }
        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unsupported terminator")),
    }
    Ok(())
}