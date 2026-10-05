use super::*;

impl Jit {
    pub(super) fn compile_inner_timed(
        lir: &LirModule,
        hot: bool,
        spare: usize,
    ) -> Result<(Jit, CodegenTimings), Diagnostic> {
        use std::time::Instant;
        let mut timings = CodegenTimings::default();
        let mut step = Instant::now();
        let mut module = Self::jit_module_for(lir)?;
        timings.init = step.elapsed();
        step = Instant::now();
        let (ids, sigs, rt, bytes_fns, statics, foreign_ids) = Self::declare_imports(&mut module, lir)?;
        timings.declare = step.elapsed();
        step = Instant::now();
        timings.fn_count = lir.functions.len();
        let mut ctx = FunctionBuilderContext::new();
        let (slots, slot_names, table, sizes) = Self::define_functions(&mut module, lir, &ids, &sigs, &foreign_ids, &rt, &bytes_fns, &statics, &mut ctx, hot, spare, &mut timings)?;
        timings.define = step.elapsed();
        step = Instant::now();
        let vec_rets = Self::finalize_module(&mut module, lir, &ids, &table)?;
        timings.finalize = step.elapsed();
        Ok((Jit { module, funcs: ids, sizes, vec_rets, hot, slots, slot_names, table, versions: BTreeMap::new(), sigs, rt, bytes_fns, statics, foreign_ids }, timings))
    }

    fn jit_module_for(lir: &LirModule) -> Result<JITModule, Diagnostic> {
        runtime::native::rnx_set_closure_epoch(runtime::native::rnx_claim_closure_epoch());
        let mut builder = JITBuilder::with_flags(
            &[("enable_nan_canonicalization", "false")],
            default_libcall_names(),
        )
        .map_err(|e| Diagnostic::new(Code::E108, format!("jit init: {e}")))?;
        builder.symbol("rnx_alloc", runtime::native::rnx_alloc as *const u8);
        builder.symbol("rnx_free", runtime::native::rnx_free as *const u8);
        builder.symbol("rnx_print_str", runtime::native::rnx_print_str as *const u8);
        builder.symbol("rnx_print_i64", runtime::native::rnx_print_i64 as *const u8);
        builder.symbol("rnx_panic", runtime::native::rnx_panic as *const u8);
        builder.symbol("rnx_retain", runtime::native::rnx_retain as *const u8);
        builder.symbol("rnx_release", runtime::native::rnx_release as *const u8);
        builder.symbol("rnx_genref_create", runtime::native::rnx_genref_create as *const u8);
        builder.symbol("rnx_genref_get", runtime::native::rnx_genref_get as *const u8);
        builder.symbol(
            "rnx_genref_invalidate",
            runtime::native::rnx_genref_invalidate as *const u8,
        );
        builder.symbol("rnx_string_concat", runtime::native::rnx_string_concat as *const u8);
        builder.symbol("rnx_string_split", runtime::native::rnx_string_split as *const u8);
        builder.symbol("rnx_string_eq", runtime::native::rnx_string_eq as *const u8);
        builder.symbol("rnx_string_cmp", runtime::native::rnx_string_cmp as *const u8);
        builder.symbol("rnx_print_val", runtime::native::rnx_print_val as *const u8);
        builder.symbol("rnx_int_to_str", runtime::native::rnx_int_to_str as *const u8);
        builder.symbol("rnx_bool_to_str", runtime::native::rnx_bool_to_str as *const u8);
        builder.symbol("rnx_any_to_str", runtime::native::rnx_any_to_str as *const u8);
        builder.symbol("rnx_eq_any_str", runtime::native::rnx_eq_any_str as *const u8);
        builder.symbol("rnx_eq_any", runtime::native::rnx_eq_any as *const u8);
        builder.symbol("rnx_float_to_str", runtime::native::rnx_float_to_str as *const u8);
        builder.symbol("rnx_float_nan", runtime::native::rnx_float_nan as *const u8);
        builder.symbol("rnx_float_to_bits", runtime::native::rnx_float_to_bits as *const u8);
        builder.symbol("rnx_float_from_bits", runtime::native::rnx_float_from_bits as *const u8);
        builder.symbol("rnx_float_fma", runtime::native::rnx_float_fma as *const u8);
        builder.symbol("rnx_release_str", runtime::native::rnx_release_str as *const u8);
        builder.symbol("rnx_print_str", runtime::native::rnx_print_str as *const u8);
        builder.symbol("rnx_array_new", runtime::native::rnx_array_new as *const u8);
        builder.symbol("rnx_array_push", runtime::native::rnx_array_push as *const u8);
        builder.symbol("rnx_array_get", runtime::native::rnx_array_get as *const u8);
        builder.symbol("rnx_array_set", runtime::native::rnx_array_set as *const u8);
        builder.symbol("rnx_array_get_unchecked", runtime::native::rnx_array_get_unchecked as *const u8);
        builder.symbol("rnx_array_set_unchecked", runtime::native::rnx_array_set_unchecked as *const u8);
        builder.symbol("rnx_array_len", runtime::native::rnx_array_len as *const u8);
        builder.symbol("rnx_string_len", runtime::native::rnx_string_len as *const u8);
        builder.symbol("rnx_string_slice", runtime::native::rnx_string_slice as *const u8);
        builder.symbol("rnx_string_index_of", runtime::native::rnx_string_index_of as *const u8);
        builder.symbol("rnx_string_index_of_from", runtime::native::rnx_string_index_of_from as *const u8);
        builder.symbol("rnx_string_trim", runtime::native::rnx_string_trim as *const u8);
        builder.symbol("rnx_string_char_code_at", runtime::native::rnx_string_char_code_at as *const u8);
        builder.symbol("rnx_string_from_char_code", runtime::native::rnx_string_from_char_code as *const u8);
        builder.symbol("rnx_array_pop", runtime::native::rnx_array_pop as *const u8);
        builder.symbol("rnx_any_box", runtime::native::rnx_any_box as *const u8);
        builder.symbol("rnx_any_unbox", runtime::native::rnx_any_unbox as *const u8);
        builder.symbol("rnx_any_unbox_heap", runtime::native::rnx_any_unbox_heap as *const u8);
        builder.symbol("rnx_any_retain", runtime::native::rnx_any_retain as *const u8);
        builder.symbol("rnx_any_release_box", runtime::native::rnx_any_release_box as *const u8);
        builder.symbol("rnx_any_release", runtime::native::rnx_any_release as *const u8);
        builder.symbol("rnx_heap_track", runtime::native::rnx_heap_track as *const u8);
        builder.symbol("rnx_closure_new", runtime::native::rnx_closure_new as *const u8);
        builder.symbol("rnx_closure_set", runtime::native::rnx_closure_set as *const u8);
        builder.symbol("rnx_closure_release", runtime::native::rnx_closure_release as *const u8);
        builder.symbol("rnx_closure_register", runtime::native::rnx_closure_register as *const u8);
        builder.symbol("rnx_thread_spawn_closure", runtime::native::rnx_thread_spawn_closure as *const u8);
        builder.symbol("rnx_thread_join_val", runtime::native::rnx_thread_join_val as *const u8);
        builder.symbol("rnx_thread_join_err", runtime::native::rnx_thread_join_err as *const u8);
        builder.symbol("rnx_task_await_val", runtime::native::rnx_task_await_val as *const u8);
        builder.symbol("rnx_task_await_err", runtime::native::rnx_task_await_err as *const u8);
        builder.symbol("rnx_pool_new", runtime::native::rnx_pool_new as *const u8);
        builder.symbol("rnx_thread_pool_submit_handle", runtime::native::rnx_thread_pool_submit_handle as *const u8);
        builder.symbol("rnx_thread_pool_submit_closure", runtime::native::rnx_thread_pool_submit_closure as *const u8);
        builder.symbol("rnx_thread_pool_parallel_closure", runtime::native::rnx_thread_pool_parallel_closure as *const u8);
        builder.symbol("rnx_panic_str", runtime::native::rnx_panic_str as *const u8);
        builder.symbol("rnx_fatal_span", runtime::native::rnx_fatal_span as *const u8);
        builder.symbol("rnx_release_array", runtime::native::rnx_release_array as *const u8);
        builder.symbol("rnx_thread_spawn", runtime::native::rnx_thread_spawn as *const u8);
        builder.symbol("rnx_thread_join", runtime::native::rnx_thread_join as *const u8);
        builder.symbol("rnx_clock_monotonic_nanos", runtime::native::rnx_clock_monotonic_nanos as *const u8);
        builder.symbol("rnx_sleep_nanos", runtime::native::rnx_sleep_nanos as *const u8);
        builder.symbol("rnx_crypto_random_u64", runtime::native::rnx_crypto_random_u64 as *const u8);
        builder.symbol("rnx_prng_seed", runtime::native::rnx_prng_seed as *const u8);
        builder.symbol("rnx_prng_next", runtime::native::rnx_prng_next as *const u8);
        builder.symbol("rnx_file_open", runtime::native::rnx_file_open as *const u8);
        builder.symbol("rnx_file_close", runtime::native::rnx_file_close as *const u8);
        builder.symbol("rnx_file_read_text", runtime::native::rnx_file_read_text as *const u8);
        builder.symbol("rnx_file_read_text_err", runtime::native::rnx_file_read_text_err as *const u8);
        builder.symbol("rnx_file_write_text", runtime::native::rnx_file_write_text as *const u8);
        builder.symbol("rnx_file_flush", runtime::native::rnx_file_flush as *const u8);
        builder.symbol("rnx_file_seek", runtime::native::rnx_file_seek as *const u8);
        builder.symbol("rnx_file_tell", runtime::native::rnx_file_tell as *const u8);
        builder.symbol("rnx_file_from_handle", runtime::native::rnx_file_from_handle as *const u8);
        builder.symbol("rnx_io_is_tty", runtime::native::rnx_io_is_tty as *const u8);
        builder.symbol("rnx_io_pretty", runtime::native::rnx_io_pretty as *const u8);
        builder.symbol("rnx_note_array_kind", runtime::native::rnx_note_array_kind as *const u8);
        builder.symbol("rnx_note_fields", runtime::native::rnx_note_fields as *const u8);
        builder.symbol("rnx_note_enum", runtime::native::rnx_note_enum as *const u8);
        builder.symbol("rnx_io_winsize", runtime::native::rnx_io_winsize as *const u8);
        builder.symbol("rnx_io_set_raw", runtime::native::rnx_io_set_raw as *const u8);
        for (name, fptr) in [
            ("rnx_gmap_new", runtime::native::rnx_gmap_new as *const u8),
            ("rnx_gmap_free", runtime::native::rnx_gmap_free as *const u8),
            ("rnx_gmap_set", runtime::native::rnx_gmap_set as *const u8),
            ("rnx_gmap_get", runtime::native::rnx_gmap_get as *const u8),
            ("rnx_gmap_has", runtime::native::rnx_gmap_has as *const u8),
            ("rnx_gmap_delete", runtime::native::rnx_gmap_delete as *const u8),
            ("rnx_gmap_len", runtime::native::rnx_gmap_len as *const u8),
            ("rnx_gmap_clear", runtime::native::rnx_gmap_clear as *const u8),
            ("rnx_gmap_keys", runtime::native::rnx_gmap_keys as *const u8),
            ("rnx_gmap_values", runtime::native::rnx_gmap_values as *const u8),
            ("rnx_json_parse", runtime::native::rnx_json_parse as *const u8),
            ("rnx_json_parse_typed", runtime::native::rnx_json_parse_typed as *const u8),
            ("rnx_json_stringify_into", runtime::native::rnx_json_stringify_into as *const u8),
            ("rnx_json_stringify", runtime::native::rnx_json_stringify as *const u8),
            ("rnx_json_unwrap", runtime::native::rnx_json_unwrap as *const u8),
            ("rnx_dns_lookup_start", runtime::native::rnx_dns_lookup_start as *const u8),
            ("rnx_dns_lookup_wait", runtime::native::rnx_dns_lookup_wait as *const u8),
            ("rnx_dns_lookup_get", runtime::native::rnx_dns_lookup_get as *const u8),
            ("rnx_dns_lookup_error", runtime::native::rnx_dns_lookup_error as *const u8),
            ("rnx_fs_exists", runtime::native::rnx_fs_exists as *const u8),
            ("rnx_fs_is_file", runtime::native::rnx_fs_is_file as *const u8),
            ("rnx_fs_is_dir", runtime::native::rnx_fs_is_dir as *const u8),
            ("rnx_fs_stat", runtime::native::rnx_fs_stat as *const u8),
            ("rnx_fs_stat_err", runtime::native::rnx_fs_stat_err as *const u8),
            ("rnx_fs_read_dir", runtime::native::rnx_fs_read_dir as *const u8),
            ("rnx_fs_read_dir_err", runtime::native::rnx_fs_read_dir_err as *const u8),
            ("rnx_fs_glob", runtime::native::rnx_fs_glob as *const u8),
            ("rnx_fs_glob_err", runtime::native::rnx_fs_glob_err as *const u8),
            ("rnx_fs_read_link", runtime::native::rnx_fs_read_link as *const u8),
            ("rnx_fs_read_link_err", runtime::native::rnx_fs_read_link_err as *const u8),
            ("rnx_fs_remove", runtime::native::rnx_fs_remove as *const u8),
            ("rnx_fs_remove_err", runtime::native::rnx_fs_remove_err as *const u8),
            ("rnx_fs_remove_all", runtime::native::rnx_fs_remove_all as *const u8),
            ("rnx_fs_remove_all_err", runtime::native::rnx_fs_remove_all_err as *const u8),
            ("rnx_fs_mkdir_err", runtime::native::rnx_fs_mkdir_err as *const u8),
            ("rnx_fs_copy_err", runtime::native::rnx_fs_copy_err as *const u8),
            ("rnx_fs_move_err", runtime::native::rnx_fs_move_err as *const u8),
            ("rnx_fs_rename_err", runtime::native::rnx_fs_rename_err as *const u8),
            ("rnx_fs_truncate_err", runtime::native::rnx_fs_truncate_err as *const u8),
            ("rnx_fs_chmod_err", runtime::native::rnx_fs_chmod_err as *const u8),
            ("rnx_fs_symlink_err", runtime::native::rnx_fs_symlink_err as *const u8),
            ("rnx_fs_fsync_err", runtime::native::rnx_fs_fsync_err as *const u8),
            ("rnx_fs_read_text", runtime::native::rnx_fs_read_text as *const u8),
            ("rnx_fs_read_text_err", runtime::native::rnx_fs_read_text_err as *const u8),
            ("rnx_fs_write_text", runtime::native::rnx_fs_write_text as *const u8),
            ("rnx_fs_write_text_err", runtime::native::rnx_fs_write_text_err as *const u8),
            ("rnx_fs_read_bytes", runtime::native::rnx_fs_read_bytes as *const u8),
            ("rnx_fs_read_bytes_err", runtime::native::rnx_fs_read_bytes_err as *const u8),
            ("rnx_fs_write_bytes", runtime::native::rnx_fs_write_bytes as *const u8),
            ("rnx_fs_write_bytes_err", runtime::native::rnx_fs_write_bytes_err as *const u8),
            ("rnx_fs_mmap", runtime::native::rnx_fs_mmap as *const u8),
            ("rnx_fs_mmap_err", runtime::native::rnx_fs_mmap_err as *const u8),
            ("rnx_fs_mmap_anon", runtime::native::rnx_fs_mmap_anon as *const u8),
            ("rnx_fs_mmap_anon_err", runtime::native::rnx_fs_mmap_anon_err as *const u8),
            ("rnx_fs_mmap_addr", runtime::native::rnx_fs_mmap_addr as *const u8),
            ("rnx_fs_mmap_len", runtime::native::rnx_fs_mmap_len as *const u8),
            ("rnx_fs_mmap_flush", runtime::native::rnx_fs_mmap_flush as *const u8),
            ("rnx_fs_mmap_close", runtime::native::rnx_fs_mmap_close as *const u8),
        ] {
            builder.symbol(name, fptr);
        }
        builder.symbol("rnx_file_read_bytes", runtime::native::rnx_file_read_bytes as *const u8);
        builder.symbol("rnx_file_write_bytes", runtime::native::rnx_file_write_bytes as *const u8);
        for (name, fptr) in [
            ("rnx_bytes_alloc", runtime::native::rnx_bytes_alloc as *const u8),
            ("rnx_bytes_free", runtime::native::rnx_bytes_free as *const u8),
            ("rnx_bytes_len", runtime::native::rnx_bytes_len as *const u8),
            ("rnx_bytes_cap", runtime::native::rnx_bytes_cap as *const u8),
            ("rnx_bytes_data", runtime::native::rnx_bytes_data as *const u8),
            ("rnx_bytes_copy_within", runtime::native::rnx_bytes_copy_within as *const u8),
            ("rnx_bytes_read_u8", runtime::native::rnx_bytes_read_u8 as *const u8),
            ("rnx_bytes_read_i8", runtime::native::rnx_bytes_read_i8 as *const u8),
            ("rnx_bytes_read_u16le", runtime::native::rnx_bytes_read_u16le as *const u8),
            ("rnx_bytes_read_u16be", runtime::native::rnx_bytes_read_u16be as *const u8),
            ("rnx_bytes_read_i16le", runtime::native::rnx_bytes_read_i16le as *const u8),
            ("rnx_bytes_read_i16be", runtime::native::rnx_bytes_read_i16be as *const u8),
            ("rnx_bytes_read_u32le", runtime::native::rnx_bytes_read_u32le as *const u8),
            ("rnx_bytes_read_u32be", runtime::native::rnx_bytes_read_u32be as *const u8),
            ("rnx_bytes_read_i32le", runtime::native::rnx_bytes_read_i32le as *const u8),
            ("rnx_bytes_read_i32be", runtime::native::rnx_bytes_read_i32be as *const u8),
            ("rnx_bytes_read_i64le", runtime::native::rnx_bytes_read_i64le as *const u8),
            ("rnx_bytes_read_i64be", runtime::native::rnx_bytes_read_i64be as *const u8),
            ("rnx_bytes_write_u8", runtime::native::rnx_bytes_write_u8 as *const u8),
            ("rnx_bytes_write_u16le", runtime::native::rnx_bytes_write_u16le as *const u8),
            ("rnx_bytes_write_u16be", runtime::native::rnx_bytes_write_u16be as *const u8),
            ("rnx_bytes_write_u32le", runtime::native::rnx_bytes_write_u32le as *const u8),
            ("rnx_bytes_write_u32be", runtime::native::rnx_bytes_write_u32be as *const u8),
            ("rnx_bytes_write_u64le", runtime::native::rnx_bytes_write_u64le as *const u8),
            ("rnx_bytes_write_u64be", runtime::native::rnx_bytes_write_u64be as *const u8),
            ("rnx_bytes_read_f32le", runtime::native::rnx_bytes_read_f32le as *const u8),
            ("rnx_bytes_read_f32be", runtime::native::rnx_bytes_read_f32be as *const u8),
            ("rnx_bytes_read_f64le", runtime::native::rnx_bytes_read_f64le as *const u8),
            ("rnx_bytes_read_f64be", runtime::native::rnx_bytes_read_f64be as *const u8),
            ("rnx_bytes_write_f32le", runtime::native::rnx_bytes_write_f32le as *const u8),
            ("rnx_bytes_write_f32be", runtime::native::rnx_bytes_write_f32be as *const u8),
            ("rnx_bytes_write_f64le", runtime::native::rnx_bytes_write_f64le as *const u8),
            ("rnx_bytes_write_f64be", runtime::native::rnx_bytes_write_f64be as *const u8),
            ("rnx_bytes_read_string", runtime::native::rnx_bytes_read_string as *const u8),
            ("rnx_bytes_write_string", runtime::native::rnx_bytes_write_string as *const u8),
        ] {
            builder.symbol(name, fptr);
        }
        builder.symbol("rnx_path_exists", runtime::native::rnx_path_exists as *const u8);
        builder.symbol("rnx_path_remove", runtime::native::rnx_path_remove as *const u8);
        builder.symbol("rnx_assert", runtime::native::rnx_assert as *const u8);
        builder.symbol("rnx_test_check", runtime::native::rnx_test_check as *const u8);
        builder.symbol("rnx_env_args_count", runtime::native::rnx_env_args_count as *const u8);
        builder.symbol("rnx_env_args_get", runtime::native::rnx_env_args_get as *const u8);
        builder.symbol("rnx_env_get", runtime::native::rnx_env_get as *const u8);
        builder.symbol("rnx_env_set", runtime::native::rnx_env_set as *const u8);
        builder.symbol("rnx_env_cwd", runtime::native::rnx_env_cwd as *const u8);
        builder.symbol("rnx_host_version", runtime::native::rnx_host_version as *const u8);
        builder.symbol("rnx_env_exit", runtime::native::rnx_env_exit as *const u8);
        builder.symbol("rnx_net_connect_start", runtime::native::rnx_net_connect_start as *const u8);
        builder.symbol("rnx_net_take_error", runtime::native::rnx_net_take_error as *const u8);
        builder.symbol("rnx_net_connect_wait", runtime::native::rnx_net_connect_wait as *const u8);
        builder.symbol("rnx_net_recv_or_wait", runtime::native::rnx_net_recv_or_wait as *const u8);
        builder.symbol("rnx_net_send_or_wait", runtime::native::rnx_net_send_or_wait as *const u8);
        builder.symbol("rnx_net_recv_get", runtime::native::rnx_net_recv_get as *const u8);
        builder.symbol("rnx_net_error_text", runtime::native::rnx_net_error_text as *const u8);
        builder.symbol("rnx_net_close", runtime::native::rnx_net_close as *const u8);
        builder.symbol("rnx_net_listener_bind", runtime::native::rnx_net_listener_bind as *const u8);
        builder.symbol("rnx_net_listener_port", runtime::native::rnx_net_listener_port as *const u8);
        builder.symbol("rnx_net_listener_accept_start", runtime::native::rnx_net_listener_accept_start as *const u8);
        builder.symbol("rnx_net_listener_accept_wait", runtime::native::rnx_net_listener_accept_wait as *const u8);
        builder.symbol("rnx_net_listener_close", runtime::native::rnx_net_listener_close as *const u8);
        builder.symbol("rnx_tls_connect_start", runtime::native::rnx_tls_connect_start as *const u8);
        builder.symbol("rnx_tls_handshake_start", runtime::native::rnx_tls_handshake_start as *const u8);
        builder.symbol("rnx_tls_handshake_wait", runtime::native::rnx_tls_handshake_wait as *const u8);
        builder.symbol("rnx_tls_recv_or_wait", runtime::native::rnx_tls_recv_or_wait as *const u8);
        builder.symbol("rnx_tls_send_or_wait", runtime::native::rnx_tls_send_or_wait as *const u8);
        builder.symbol("rnx_tls_recv_get", runtime::native::rnx_tls_recv_get as *const u8);
        builder.symbol("rnx_tls_error_text", runtime::native::rnx_tls_error_text as *const u8);
        builder.symbol("rnx_tls_close", runtime::native::rnx_tls_close as *const u8);
        builder.symbol("rnx_process_pid", runtime::native::rnx_process_pid as *const u8);
        builder.symbol("rnx_process_remove_env", runtime::native::rnx_process_remove_env as *const u8);
        builder.symbol("rnx_process_all_env_count", runtime::native::rnx_process_all_env_count as *const u8);
        builder.symbol("rnx_process_all_env_get", runtime::native::rnx_process_all_env_get as *const u8);
        builder.symbol("rnx_process_chdir", runtime::native::rnx_process_chdir as *const u8);
        builder.symbol("rnx_process_spawn", runtime::native::rnx_process_spawn as *const u8);
        builder.symbol("rnx_process_run", runtime::native::rnx_process_run as *const u8);
        builder.symbol("rnx_process_pid_of", runtime::native::rnx_process_pid_of as *const u8);
        builder.symbol("rnx_process_write_stdin", runtime::native::rnx_process_write_stdin as *const u8);
        builder.symbol("rnx_process_read_stdout", runtime::native::rnx_process_read_stdout as *const u8);
        builder.symbol("rnx_process_read_stderr", runtime::native::rnx_process_read_stderr as *const u8);
        builder.symbol("rnx_process_close_stdin", runtime::native::rnx_process_close_stdin as *const u8);
        builder.symbol("rnx_process_wait", runtime::native::rnx_process_wait as *const u8);
        builder.symbol("rnx_process_try_wait", runtime::native::rnx_process_try_wait as *const u8);
        builder.symbol("rnx_process_kill", runtime::native::rnx_process_kill as *const u8);
        builder.symbol("rnx_process_take_stdout", runtime::native::rnx_process_take_stdout as *const u8);
        builder.symbol("rnx_process_take_stderr", runtime::native::rnx_process_take_stderr as *const u8);
        builder.symbol("rnx_process_exit_code", runtime::native::rnx_process_exit_code as *const u8);
        builder.symbol("rnx_process_forget", runtime::native::rnx_process_forget as *const u8);
        builder.symbol("rnx_os_platform", runtime::native::rnx_os_platform as *const u8);
        builder.symbol("rnx_os_arch", runtime::native::rnx_os_arch as *const u8);
        builder.symbol("rnx_os_hostname", runtime::native::rnx_os_hostname as *const u8);
        builder.symbol("rnx_os_tmpdir", runtime::native::rnx_os_tmpdir as *const u8);
        builder.symbol("rnx_os_homedir", runtime::native::rnx_os_homedir as *const u8);
        builder.symbol("rnx_os_cpu_count", runtime::native::rnx_os_cpu_count as *const u8);
        builder.symbol("rnx_os_uptime", runtime::native::rnx_os_uptime as *const u8);
        builder.symbol("rnx_map_new", runtime::native::rnx_map_new as *const u8);
        builder.symbol("rnx_map_set", runtime::native::rnx_map_set as *const u8);
        builder.symbol("rnx_map_get", runtime::native::rnx_map_get as *const u8);
        builder.symbol("rnx_map_has", runtime::native::rnx_map_has as *const u8);
        builder.symbol("rnx_map_delete", runtime::native::rnx_map_delete as *const u8);
        builder.symbol("rnx_map_len", runtime::native::rnx_map_len as *const u8);
        builder.symbol("rnx_map_clear", runtime::native::rnx_map_clear as *const u8);
        builder.symbol("rnx_map_keys", runtime::native::rnx_map_keys as *const u8);
        builder.symbol("rnx_map_values", runtime::native::rnx_map_values as *const u8);
        builder.symbol("rnx_release_map", runtime::native::rnx_release_map as *const u8);
        builder.symbol("rnx_sync_atomic_get", runtime::native::rnx_sync_atomic_get as *const u8);
        builder.symbol("rnx_black_box_i64", runtime::native::rnx_black_box_i64 as *const u8);
        builder.symbol("rnx_sync_atomic_set", runtime::native::rnx_sync_atomic_set as *const u8);
        builder.symbol("rnx_sync_atomic_fetch_add", runtime::native::rnx_sync_atomic_fetch_add as *const u8);
        builder.symbol("rnx_sync_atomic_cas", runtime::native::rnx_sync_atomic_cas as *const u8);
        builder.symbol("rnx_sync_channel_send", runtime::native::rnx_sync_channel_send as *const u8);
        builder.symbol("rnx_sync_channel_send_str", runtime::native::rnx_sync_channel_send_str as *const u8);
        builder.symbol("rnx_sync_channel_send_obj", runtime::native::rnx_sync_channel_send_obj as *const u8);
        builder.symbol("rnx_sync_channel_send_array", runtime::native::rnx_sync_channel_send_array as *const u8);
        builder.symbol("rnx_sync_channel_recv", runtime::native::rnx_sync_channel_recv as *const u8);
        builder.symbol("rnx_sync_channel_try_recv", runtime::native::rnx_sync_channel_try_recv as *const u8);
        builder.symbol("rnx_sync_channel_len", runtime::native::rnx_sync_channel_len as *const u8);
        builder.symbol("rnx_fs_pool_depth", runtime::native::rnx_fs_pool_depth as *const u8);
        builder.symbol("rnx_sync_channel_drop", runtime::native::rnx_sync_channel_drop as *const u8);
        builder.symbol("rnx_debug_live_count", runtime::native::rnx_debug_live_count as *const u8);
        builder.symbol("rnx_defer_push", runtime::native::rnx_defer_push as *const u8);
        builder.symbol("rnx_defer_pop", runtime::native::rnx_defer_pop as *const u8);
        builder.symbol("rnx_defer_len", runtime::native::rnx_defer_len as *const u8);
        builder.symbol("rnx_any_tag", runtime::native::rnx_any_tag as *const u8);
        builder.symbol("rnx_obj_class", runtime::native::rnx_obj_class as *const u8);
        builder.symbol("rnx_error_set", runtime::native::rnx_error_set as *const u8);
        builder.symbol("rnx_error_take", runtime::native::rnx_error_take as *const u8);
        builder.symbol("rnx_error_class", runtime::native::rnx_error_class as *const u8);
        builder.symbol("rnx_error_unbox", runtime::native::rnx_error_unbox as *const u8);
        builder.symbol("rnx_error_str", runtime::native::rnx_error_str as *const u8);
        builder.symbol("rnx_error_release", runtime::native::rnx_error_release as *const u8);
        builder.symbol("rnx_note_type", runtime::native::rnx_note_type as *const u8);
        builder.symbol("rnx_type_name", runtime::native::rnx_type_name as *const u8);
        builder.symbol("rnx_typeof_any", runtime::native::rnx_typeof_any as *const u8);
        builder.symbol("rnx_array_slice", runtime::native::rnx_array_slice as *const u8);
        builder.symbol("rnx_mutex_lock", runtime::native::rnx_mutex_lock as *const u8);
        builder.symbol("rnx_mutex_unlock", runtime::native::rnx_mutex_unlock as *const u8);
        builder.symbol("rnx_mutex_try_lock", runtime::native::rnx_mutex_try_lock as *const u8);
        builder.symbol("rnx_rwlock_read_lock", runtime::native::rnx_rwlock_read_lock as *const u8);
        builder.symbol("rnx_rwlock_read_unlock", runtime::native::rnx_rwlock_read_unlock as *const u8);
        builder.symbol("rnx_rwlock_write_lock", runtime::native::rnx_rwlock_write_lock as *const u8);
        builder.symbol("rnx_rwlock_write_unlock", runtime::native::rnx_rwlock_write_unlock as *const u8);
        builder.symbol("rnx_rwlock_try_read_lock", runtime::native::rnx_rwlock_try_read_lock as *const u8);
        builder.symbol("rnx_rwlock_try_write_lock", runtime::native::rnx_rwlock_try_write_lock as *const u8);
        builder.symbol("rnx_condvar_wait", runtime::native::rnx_condvar_wait as *const u8);
        builder.symbol("rnx_condvar_wait_timeout", runtime::native::rnx_condvar_wait_timeout as *const u8);
        builder.symbol("rnx_condvar_notify_one", runtime::native::rnx_condvar_notify_one as *const u8);
        builder.symbol("rnx_condvar_notify_all", runtime::native::rnx_condvar_notify_all as *const u8);
        builder.symbol("rnx_barrier_wait", runtime::native::rnx_barrier_wait as *const u8);
        builder.symbol("rnx_thread_pool_init", runtime::native::rnx_thread_pool_init as *const u8);
        builder.symbol("rnx_thread_pool_submit", runtime::native::rnx_thread_pool_submit as *const u8);
        builder.symbol("rnx_thread_pool_parallel_for", runtime::native::rnx_thread_pool_parallel_for as *const u8);
        builder.symbol("rnx_thread_pool_join", runtime::native::rnx_thread_pool_join as *const u8);
        builder.symbol("rnx_thread_pool_shutdown", runtime::native::rnx_thread_pool_shutdown as *const u8);
        builder.symbol("rnx_math_sqrt", runtime::native::rnx_math_sqrt as *const u8);
        builder.symbol("rnx_math_sin", runtime::native::rnx_math_sin as *const u8);
        builder.symbol("rnx_math_cos", runtime::native::rnx_math_cos as *const u8);
        builder.symbol("rnx_math_tan", runtime::native::rnx_math_tan as *const u8);
        builder.symbol("rnx_math_atan2", runtime::native::rnx_math_atan2 as *const u8);
        builder.symbol("rnx_math_pow", runtime::native::rnx_math_pow as *const u8);
        builder.symbol("rnx_math_floor", runtime::native::rnx_math_floor as *const u8);
        builder.symbol("rnx_math_ceil", runtime::native::rnx_math_ceil as *const u8);
        builder.symbol("rnx_math_round", runtime::native::rnx_math_round as *const u8);
        builder.symbol("rnx_math_log", runtime::native::rnx_math_log as *const u8);
        for f in &lir.foreign {
            let addr = runtime::native::resolve_foreign(&f.lib, &f.symbol).ok_or_else(|| {
                Diagnostic::new(
                    Code::E108,
                    format!("unknown foreign symbol `{}` in native lib `{}`", f.symbol, f.lib),
                )
            })?;
            builder.symbol(&f.symbol, addr);
        }
        // Prebuilt release stdlib: dlopen the cached artifact when fresh and
        // register its symbols through JITBuilder::symbol. Program functions
        // declare Linkage::Local, so these registrations stay inert until a
        // later change marks stdlib functions Import; a missing or stale
        // artifact falls back to JIT-compiling stdlib inline as today.
        let _ = super::try_register_prebuilt_stdlib(&mut builder, lir);
        let module = JITModule::new(builder);
        Ok(module)
    }

    fn declare_imports(module: &mut JITModule, lir: &LirModule) -> Result<(BTreeMap<String, FuncId>, BTreeMap<FuncId, Signature>, RtIds, BTreeMap<String, FuncId>, BTreeMap<String, DataId>, BTreeMap<(String, String), FuncId>), Diagnostic> {
        let mut ids: BTreeMap<String, FuncId> = BTreeMap::new();
        let mut sigs: BTreeMap<FuncId, Signature> = BTreeMap::new();
        for f in &lir.functions {
            check_supported(lir, f)?;
            let sig = fn_sig(f);
            let id = module
                .declare_function(&f.name, Linkage::Local, &sig)
                .map_err(|e| Diagnostic::new(Code::E108, format!("declare {}: {e}", f.name)))?;
            ids.insert(f.name.clone(), id);
            sigs.insert(id, sig);
        }
        let mut alloc_sig = Signature::new(CallConv::SystemV);
        alloc_sig.params.push(AbiParam::new(types::I64));
        alloc_sig.params.push(AbiParam::new(types::I64));
        alloc_sig.returns.push(AbiParam::new(types::I64));
        let alloc = module
            .declare_function("rnx_alloc", Linkage::Import, &alloc_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_alloc: {e}")))?;
        let mut release_sig = Signature::new(CallConv::SystemV);
        release_sig.params.push(AbiParam::new(types::I64));
        release_sig.params.push(AbiParam::new(types::I64));
        release_sig.params.push(AbiParam::new(types::I64));
        let release = module
            .declare_function("rnx_release", Linkage::Import, &release_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_release: {e}")))?;
        let mut genref_create_sig = Signature::new(CallConv::SystemV);
        genref_create_sig.params.push(AbiParam::new(types::I64));
        genref_create_sig.returns.push(AbiParam::new(types::I64));
        let genref_create = module
            .declare_function("rnx_genref_create", Linkage::Import, &genref_create_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_genref_create: {e}")))?;
        let mut genref_get_sig = Signature::new(CallConv::SystemV);
        genref_get_sig.params.push(AbiParam::new(types::I64));
        genref_get_sig.returns.push(AbiParam::new(types::I64));
        let genref_get = module
            .declare_function("rnx_genref_get", Linkage::Import, &genref_get_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_genref_get: {e}")))?;
        let mut genref_inv_sig = Signature::new(CallConv::SystemV);
        genref_inv_sig.params.push(AbiParam::new(types::I64));
        let genref_invalidate = module
            .declare_function("rnx_genref_invalidate", Linkage::Import, &genref_inv_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_genref_invalidate: {e}")))?;
        let mut retain_sig = Signature::new(CallConv::SystemV);
        retain_sig.params.push(AbiParam::new(types::I64));
        let retain = module
            .declare_function("rnx_retain", Linkage::Import, &retain_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_retain: {e}")))?;
        let mut concat_sig = Signature::new(CallConv::SystemV);
        concat_sig.params.push(AbiParam::new(types::I64));
        concat_sig.params.push(AbiParam::new(types::I64));
        concat_sig.returns.push(AbiParam::new(types::I64));
        let concat = module
            .declare_function("rnx_string_concat", Linkage::Import, &concat_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_string_concat: {e}")))?;
        let mut eq_sig = Signature::new(CallConv::SystemV);
        eq_sig.params.push(AbiParam::new(types::I64));
        eq_sig.params.push(AbiParam::new(types::I64));
        eq_sig.returns.push(AbiParam::new(types::I8));
        let streq = module
            .declare_function("rnx_string_eq", Linkage::Import, &eq_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_string_eq: {e}")))?;
        let mut cmp_sig = Signature::new(CallConv::SystemV);
        cmp_sig.params.push(AbiParam::new(types::I64));
        cmp_sig.params.push(AbiParam::new(types::I64));
        cmp_sig.returns.push(AbiParam::new(types::I64));
        let strcmp = module
            .declare_function("rnx_string_cmp", Linkage::Import, &cmp_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_string_cmp: {e}")))?;
        let mut print_val_sig = Signature::new(CallConv::SystemV);
        print_val_sig.params.push(AbiParam::new(types::I64));
        print_val_sig.params.push(AbiParam::new(types::I32));
        let print_val = module
            .declare_function("rnx_print_val", Linkage::Import, &print_val_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_print_val: {e}")))?;
        let mut print_str_sig = Signature::new(CallConv::SystemV);
        print_str_sig.params.push(AbiParam::new(types::I64));
        print_str_sig.params.push(AbiParam::new(types::I64));
        let print_str = module
            .declare_function("rnx_print_str", Linkage::Import, &print_str_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_print_str: {e}")))?;
        let mut int_to_str_sig = Signature::new(CallConv::SystemV);
        int_to_str_sig.params.push(AbiParam::new(types::I64));
        int_to_str_sig.returns.push(AbiParam::new(types::I64));
        let int_to_str = module
            .declare_function("rnx_int_to_str", Linkage::Import, &int_to_str_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_int_to_str: {e}")))?;
        let mut float_to_str_sig = Signature::new(CallConv::SystemV);
        float_to_str_sig.params.push(AbiParam::new(types::I64));
        float_to_str_sig.returns.push(AbiParam::new(types::I64));
        let float_to_str = module
            .declare_function("rnx_float_to_str", Linkage::Import, &float_to_str_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_float_to_str: {e}")))?;
        let mut bool_to_str_sig = Signature::new(CallConv::SystemV);
        bool_to_str_sig.params.push(AbiParam::new(types::I64));
        bool_to_str_sig.returns.push(AbiParam::new(types::I64));
        let bool_to_str = module
            .declare_function("rnx_bool_to_str", Linkage::Import, &bool_to_str_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_bool_to_str: {e}")))?;
        let mut any_to_str_sig = Signature::new(CallConv::SystemV);
        any_to_str_sig.params.push(AbiParam::new(types::I64));
        any_to_str_sig.returns.push(AbiParam::new(types::I64));
        let any_to_str = module
            .declare_function("rnx_any_to_str", Linkage::Import, &any_to_str_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_any_to_str: {e}")))?;
        let mut eq_any_str_sig = Signature::new(CallConv::SystemV);
        eq_any_str_sig.params.push(AbiParam::new(types::I64));
        eq_any_str_sig.params.push(AbiParam::new(types::I64));
        eq_any_str_sig.returns.push(AbiParam::new(types::I64));
        let eq_any_str = module
            .declare_function("rnx_eq_any_str", Linkage::Import, &eq_any_str_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_eq_any_str: {e}")))?;
        let mut eq_any_sig = Signature::new(CallConv::SystemV);
        eq_any_sig.params.push(AbiParam::new(types::I64));
        eq_any_sig.params.push(AbiParam::new(types::I64));
        eq_any_sig.returns.push(AbiParam::new(types::I64));
        let eq_any = module
            .declare_function("rnx_eq_any", Linkage::Import, &eq_any_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_eq_any: {e}")))?;
        let mut release_str_sig = Signature::new(CallConv::SystemV);
        release_str_sig.params.push(AbiParam::new(types::I64));
        let release_str = module
            .declare_function("rnx_release_str", Linkage::Import, &release_str_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_release_str: {e}")))?;
        let mut array_new_sig = Signature::new(CallConv::SystemV);
        array_new_sig.params.push(AbiParam::new(types::I64));
        array_new_sig.params.push(AbiParam::new(types::I64));
        array_new_sig.returns.push(AbiParam::new(types::I64));
        let array_new = module
            .declare_function("rnx_array_new", Linkage::Import, &array_new_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_array_new: {e}")))?;
        let mut array_push_sig = Signature::new(CallConv::SystemV);
        array_push_sig.params.push(AbiParam::new(types::I64));
        array_push_sig.params.push(AbiParam::new(types::I64));
        array_push_sig.params.push(AbiParam::new(types::I64));
        let array_push = module
            .declare_function("rnx_array_push", Linkage::Import, &array_push_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_array_push: {e}")))?;
        let mut array_get_sig = Signature::new(CallConv::SystemV);
        array_get_sig.params.push(AbiParam::new(types::I64));
        array_get_sig.params.push(AbiParam::new(types::I64));
        array_get_sig.params.push(AbiParam::new(types::I64));
        array_get_sig.returns.push(AbiParam::new(types::I64));
        let array_get = module
            .declare_function("rnx_array_get", Linkage::Import, &array_get_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_array_get: {e}")))?;
        let mut array_set_sig = Signature::new(CallConv::SystemV);
        array_set_sig.params.push(AbiParam::new(types::I64));
        array_set_sig.params.push(AbiParam::new(types::I64));
        array_set_sig.params.push(AbiParam::new(types::I64));
        array_set_sig.params.push(AbiParam::new(types::I64));
        let array_set = module
            .declare_function("rnx_array_set", Linkage::Import, &array_set_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_array_set: {e}")))?;
        let array_get_unchecked = module
            .declare_function("rnx_array_get_unchecked", Linkage::Import, &array_get_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_array_get_unchecked: {e}")))?;
        let array_set_unchecked = module
            .declare_function("rnx_array_set_unchecked", Linkage::Import, &array_set_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_array_set_unchecked: {e}")))?;
        let mut array_len_sig = Signature::new(CallConv::SystemV);
        array_len_sig.params.push(AbiParam::new(types::I64));
        array_len_sig.returns.push(AbiParam::new(types::I64));
        let array_len = module
            .declare_function("rnx_array_len", Linkage::Import, &array_len_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_array_len: {e}")))?;

        let mut str1_sig = Signature::new(CallConv::SystemV);
        str1_sig.params.push(AbiParam::new(types::I64));
        str1_sig.returns.push(AbiParam::new(types::I64));
        let mut str2_sig = Signature::new(CallConv::SystemV);
        str2_sig.params.push(AbiParam::new(types::I64));
        str2_sig.params.push(AbiParam::new(types::I64));
        str2_sig.returns.push(AbiParam::new(types::I64));
        let mut str3_sig = Signature::new(CallConv::SystemV);
        str3_sig.params.push(AbiParam::new(types::I64));
        str3_sig.params.push(AbiParam::new(types::I64));
        str3_sig.params.push(AbiParam::new(types::I64));
        str3_sig.returns.push(AbiParam::new(types::I64));
        let decl_isig = |module: &mut JITModule, name: &str, sig: &Signature| {
            module
                .declare_function(name, Linkage::Import, sig)
                .map_err(|e| Diagnostic::new(Code::E108, format!("declare {name}: {e}")))
        };
        let string_len = decl_isig(&mut *module, "rnx_string_len", &str1_sig)?;
        let string_trim = decl_isig(&mut *module, "rnx_string_trim", &str1_sig)?;
        let string_concat = decl_isig(&mut *module, "rnx_string_concat", &str2_sig)?;
        let string_split = decl_isig(&mut *module, "rnx_string_split", &str2_sig)?;
        let string_index_of = decl_isig(&mut *module, "rnx_string_index_of", &str2_sig)?;
        let string_index_of_from = decl_isig(&mut *module, "rnx_string_index_of_from", &str3_sig)?;
        let string_char_code_at = decl_isig(&mut *module, "rnx_string_char_code_at", &str2_sig)?;
        let string_from_char_code = decl_isig(&mut *module, "rnx_string_from_char_code", &str1_sig)?;
        let string_slice = decl_isig(&mut *module, "rnx_string_slice", &str3_sig)?;
        let array_pop_fn = decl_isig(&mut *module, "rnx_array_pop", &str2_sig)?;
        let any_box = decl_isig(&mut *module, "rnx_any_box", &str2_sig)?;
        let any_unbox = decl_isig(&mut *module, "rnx_any_unbox", &str1_sig)?;
        let any_unbox_heap = decl_isig(&mut *module, "rnx_any_unbox_heap", &str1_sig)?;
        let mut void1_sig = Signature::new(CallConv::SystemV);
        void1_sig.params.push(AbiParam::new(types::I64));
        let any_retain = decl_isig(&mut *module, "rnx_any_retain", &void1_sig)?;
        let any_release_box = decl_isig(&mut *module, "rnx_any_release_box", &void1_sig)?;
        let any_release = decl_isig(&mut *module, "rnx_any_release", &void1_sig)?;
        let mut heap_track_sig = Signature::new(CallConv::SystemV);
        for _ in 0..4 {
            heap_track_sig.params.push(AbiParam::new(types::I64));
        }
        let heap_track = decl_isig(&mut *module, "rnx_heap_track", &heap_track_sig)?;
        let closure_new = decl_isig(&mut *module, "rnx_closure_new", &str2_sig)?;
        let closure_set = {
            let mut sig = Signature::new(CallConv::SystemV);
            for _ in 0..8 {
                sig.params.push(AbiParam::new(types::I64));
            }
            decl_isig(&mut *module, "rnx_closure_set", &sig)?
        };
        let closure_release = decl_isig(&mut *module, "rnx_closure_release", &str1_sig)?;
        let panic_str = decl_isig(&mut *module, "rnx_panic_str", &str1_sig)?;
        let fatal_span = {
            let mut sig = Signature::new(CallConv::SystemV);
            for _ in 0..3 {
                sig.params.push(AbiParam::new(types::I64));
            }
            decl_isig(&mut *module, "rnx_fatal_span", &sig)?
        };
        let spawn_closure = {
            let mut sig = Signature::new(CallConv::SystemV);
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            decl_isig(&mut *module, "rnx_thread_spawn_closure", &sig)?
        };
        let join_val = decl_isig(&mut *module, "rnx_thread_join_val", &str1_sig)?;
        let join_err = decl_isig(&mut *module, "rnx_thread_join_err", &str1_sig)?;
        let task_val = decl_isig(&mut *module, "rnx_task_await_val", &str1_sig)?;
        let task_err = decl_isig(&mut *module, "rnx_task_await_err", &str1_sig)?;
        let pool_new = decl_isig(&mut *module, "rnx_pool_new", &str1_sig)?;
        let submit_handle = {
            let mut sig = Signature::new(CallConv::SystemV);
            for _ in 0..5 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            decl_isig(&mut *module, "rnx_thread_pool_submit_handle", &sig)?
        };
        let submit_closure = {
            let mut sig = Signature::new(CallConv::SystemV);
            for _ in 0..5 {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));
            decl_isig(&mut *module, "rnx_thread_pool_submit_closure", &sig)?
        };
        let parallel_closure = {
            let mut sig = Signature::new(CallConv::SystemV);
            for _ in 0..5 {
                sig.params.push(AbiParam::new(types::I64));
            }
            decl_isig(&mut *module, "rnx_thread_pool_parallel_closure", &sig)?
        };
        let mut release_array_sig = Signature::new(CallConv::SystemV);
        release_array_sig.params.push(AbiParam::new(types::I64));
        release_array_sig.params.push(AbiParam::new(types::I64));
        release_array_sig.params.push(AbiParam::new(types::I64));
        let release_array = module
            .declare_function("rnx_release_array", Linkage::Import, &release_array_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_release_array: {e}")))?;
        let mut thread_spawn_sig = Signature::new(CallConv::SystemV);
        thread_spawn_sig.params.push(AbiParam::new(types::I64));
        thread_spawn_sig.params.push(AbiParam::new(types::I64));
        thread_spawn_sig.returns.push(AbiParam::new(types::I64));
        let thread_spawn = module
            .declare_function("rnx_thread_spawn", Linkage::Import, &thread_spawn_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_thread_spawn: {e}")))?;
        let mut thread_join_sig = Signature::new(CallConv::SystemV);
        thread_join_sig.params.push(AbiParam::new(types::I64));
        thread_join_sig.returns.push(AbiParam::new(types::I64));
        let thread_join = module
            .declare_function("rnx_thread_join", Linkage::Import, &thread_join_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_thread_join: {e}")))?;
        let mut clock_mono_sig = Signature::new(CallConv::SystemV);
        clock_mono_sig.returns.push(AbiParam::new(types::I64));
        let clock_mono = module
            .declare_function("rnx_clock_monotonic_nanos", Linkage::Import, &clock_mono_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_clock_monotonic_nanos: {e}")))?;
        let mut crypto_random_sig = Signature::new(CallConv::SystemV);
        crypto_random_sig.returns.push(AbiParam::new(types::I64));
        let crypto_random = module
            .declare_function("rnx_crypto_random_u64", Linkage::Import, &crypto_random_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_crypto_random_u64: {e}")))?;
        let mut prng_seed_sig = Signature::new(CallConv::SystemV);
        prng_seed_sig.params.push(AbiParam::new(types::I64));
        prng_seed_sig.params.push(AbiParam::new(types::I64));
        let prng_seed = module
            .declare_function("rnx_prng_seed", Linkage::Import, &prng_seed_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_prng_seed: {e}")))?;
        let mut prng_next_sig = Signature::new(CallConv::SystemV);
        prng_next_sig.params.push(AbiParam::new(types::I64));
        prng_next_sig.returns.push(AbiParam::new(types::I64));
        let prng_next = module
            .declare_function("rnx_prng_next", Linkage::Import, &prng_next_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_prng_next: {e}")))?;
        let mut file_open_sig = Signature::new(CallConv::SystemV);
        file_open_sig.params.push(AbiParam::new(types::I64));
        file_open_sig.params.push(AbiParam::new(types::I64));
        file_open_sig.returns.push(AbiParam::new(types::I64));
        let file_open = module
            .declare_function("rnx_file_open", Linkage::Import, &file_open_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_file_open: {e}")))?;
        let mut file_close_sig = Signature::new(CallConv::SystemV);
        file_close_sig.params.push(AbiParam::new(types::I64));
        let file_close = module
            .declare_function("rnx_file_close", Linkage::Import, &file_close_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_file_close: {e}")))?;
        let mut file_read_sig = Signature::new(CallConv::SystemV);
        file_read_sig.params.push(AbiParam::new(types::I64));
        file_read_sig.returns.push(AbiParam::new(types::I64));
        let file_read = module
            .declare_function("rnx_file_read_text", Linkage::Import, &file_read_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_file_read_text: {e}")))?;
        let mut file_write_sig = Signature::new(CallConv::SystemV);
        file_write_sig.params.push(AbiParam::new(types::I64));
        file_write_sig.params.push(AbiParam::new(types::I64));
        file_write_sig.returns.push(AbiParam::new(types::I64));
        let file_write = module
            .declare_function("rnx_file_write_text", Linkage::Import, &file_write_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_file_write_text: {e}")))?;
        let mut file_flush_sig = Signature::new(CallConv::SystemV);
        file_flush_sig.params.push(AbiParam::new(types::I64));
        let file_flush = module
            .declare_function("rnx_file_flush", Linkage::Import, &file_flush_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_file_flush: {e}")))?;
        let mut file_seek_sig = Signature::new(CallConv::SystemV);
        file_seek_sig.params.push(AbiParam::new(types::I64));
        file_seek_sig.params.push(AbiParam::new(types::I64));
        file_seek_sig.returns.push(AbiParam::new(types::I64));
        let file_seek = module
            .declare_function("rnx_file_seek", Linkage::Import, &file_seek_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_file_seek: {e}")))?;
        let mut file_tell_sig = Signature::new(CallConv::SystemV);
        file_tell_sig.params.push(AbiParam::new(types::I64));
        file_tell_sig.returns.push(AbiParam::new(types::I64));
        let file_tell = module
            .declare_function("rnx_file_tell", Linkage::Import, &file_tell_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_file_tell: {e}")))?;
        let mut file_from_handle_sig = Signature::new(CallConv::SystemV);
        file_from_handle_sig.params.push(AbiParam::new(types::I64));
        file_from_handle_sig.params.push(AbiParam::new(types::I64));
        file_from_handle_sig.params.push(AbiParam::new(types::I64));
        file_from_handle_sig.returns.push(AbiParam::new(types::I64));
        let file_from_handle = module
            .declare_function("rnx_file_from_handle", Linkage::Import, &file_from_handle_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_file_from_handle: {e}")))?;
        let mut io_is_tty_sig = Signature::new(CallConv::SystemV);
        io_is_tty_sig.params.push(AbiParam::new(types::I64));
        io_is_tty_sig.returns.push(AbiParam::new(types::I8));
        let io_is_tty = module
            .declare_function("rnx_io_is_tty", Linkage::Import, &io_is_tty_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_io_is_tty: {e}")))?;
        let mut io_winsize_sig = Signature::new(CallConv::SystemV);
        io_winsize_sig.returns.push(AbiParam::new(types::I64));
        let io_winsize = module
            .declare_function("rnx_io_winsize", Linkage::Import, &io_winsize_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_io_winsize: {e}")))?;
        let mut io_set_raw_sig = Signature::new(CallConv::SystemV);
        io_set_raw_sig.params.push(AbiParam::new(types::I64));
        io_set_raw_sig.params.push(AbiParam::new(types::I64));
        io_set_raw_sig.returns.push(AbiParam::new(types::I64));
        let io_set_raw = module
            .declare_function("rnx_io_set_raw", Linkage::Import, &io_set_raw_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_io_set_raw: {e}")))?;
        let mut gmap_new_sig = Signature::new(CallConv::SystemV);
        gmap_new_sig.returns.push(AbiParam::new(types::I64));
        let mut gmap_bool2_sig = Signature::new(CallConv::SystemV);
        gmap_bool2_sig.params.push(AbiParam::new(types::I64));
        gmap_bool2_sig.params.push(AbiParam::new(types::I64));
        gmap_bool2_sig.returns.push(AbiParam::new(types::I8));
        let mut bytes_fns: BTreeMap<String, FuncId> = BTreeMap::new();
        let mut bytes4_sig = Signature::new(CallConv::SystemV);
        bytes4_sig.params.push(AbiParam::new(types::I64));
        bytes4_sig.params.push(AbiParam::new(types::I64));
        bytes4_sig.params.push(AbiParam::new(types::I64));
        bytes4_sig.params.push(AbiParam::new(types::I64));
        let mut bytes4r_sig = Signature::new(CallConv::SystemV);
        bytes4r_sig.params.push(AbiParam::new(types::I64));
        bytes4r_sig.params.push(AbiParam::new(types::I64));
        bytes4r_sig.params.push(AbiParam::new(types::I64));
        bytes4r_sig.params.push(AbiParam::new(types::I64));
        bytes4r_sig.returns.push(AbiParam::new(types::I64));
        for (name, sig) in [
            ("rnx_bytes_alloc", &str1_sig),
            ("rnx_bytes_len", &str1_sig),
            ("rnx_bytes_cap", &str1_sig),
            ("rnx_bytes_data", &str1_sig),
            ("rnx_bytes_free", &file_close_sig),
            ("rnx_bytes_read_u8", &str2_sig),
            ("rnx_bytes_read_i8", &str2_sig),
            ("rnx_bytes_read_u16le", &str2_sig),
            ("rnx_bytes_read_u16be", &str2_sig),
            ("rnx_bytes_read_i16le", &str2_sig),
            ("rnx_bytes_read_i16be", &str2_sig),
            ("rnx_bytes_read_u32le", &str2_sig),
            ("rnx_bytes_read_u32be", &str2_sig),
            ("rnx_bytes_read_i32le", &str2_sig),
            ("rnx_bytes_read_i32be", &str2_sig),
            ("rnx_bytes_read_i64le", &str2_sig),
            ("rnx_bytes_read_i64be", &str2_sig),
            ("rnx_bytes_read_f32le", &str2_sig),
            ("rnx_bytes_read_f32be", &str2_sig),
            ("rnx_bytes_read_f64le", &str2_sig),
            ("rnx_bytes_read_f64be", &str2_sig),
            ("rnx_bytes_write_u8", &array_push_sig),
            ("rnx_bytes_write_u16le", &array_push_sig),
            ("rnx_bytes_write_u16be", &array_push_sig),
            ("rnx_bytes_write_u32le", &array_push_sig),
            ("rnx_bytes_write_u32be", &array_push_sig),
            ("rnx_bytes_write_u64le", &array_push_sig),
            ("rnx_bytes_write_u64be", &array_push_sig),
            ("rnx_bytes_write_f32le", &array_push_sig),
            ("rnx_bytes_write_f32be", &array_push_sig),
            ("rnx_bytes_write_f64le", &array_push_sig),
            ("rnx_bytes_write_f64be", &array_push_sig),
            ("rnx_bytes_copy_within", &bytes4_sig),
            ("rnx_file_read_bytes", &bytes4r_sig),
            ("rnx_file_write_bytes", &bytes4r_sig),
            ("rnx_gmap_new", &gmap_new_sig),
            ("rnx_gmap_free", &file_close_sig),
            ("rnx_gmap_set", &array_push_sig),
            ("rnx_gmap_get", &str2_sig),
            ("rnx_gmap_has", &gmap_bool2_sig),
            ("rnx_gmap_delete", &gmap_bool2_sig),
            ("rnx_gmap_len", &str1_sig),
            ("rnx_gmap_clear", &file_close_sig),
            ("rnx_gmap_keys", &str1_sig),
            ("rnx_gmap_values", &str1_sig),
            ("rnx_json_parse", &str1_sig),
            ("rnx_json_parse_typed", &str2_sig),
            ("rnx_json_stringify_into", &str3_sig),
            ("rnx_json_stringify", &str1_sig),
            ("rnx_json_unwrap", &str1_sig),
            ("rnx_dns_lookup_start", &str1_sig),
            ("rnx_dns_lookup_wait", &str1_sig),
            ("rnx_dns_lookup_get", &str1_sig),
            ("rnx_dns_lookup_error", &str1_sig),
            ("rnx_fs_stat", &str1_sig),
            ("rnx_fs_stat_err", &str1_sig),
            ("rnx_fs_read_dir", &str1_sig),
            ("rnx_fs_read_dir_err", &str1_sig),
            ("rnx_fs_glob", &str1_sig),
            ("rnx_fs_glob_err", &str1_sig),
            ("rnx_fs_read_link", &str1_sig),
            ("rnx_fs_read_link_err", &str1_sig),
            ("rnx_fs_remove_err", &str1_sig),
            ("rnx_fs_remove_all_err", &str1_sig),
            ("rnx_fs_fsync_err", &str1_sig),
            ("rnx_fs_read_text", &str1_sig),
            ("rnx_fs_read_text_err", &str1_sig),
            ("rnx_fs_read_bytes", &str1_sig),
            ("rnx_fs_read_bytes_err", &str1_sig),
            ("rnx_fs_mkdir_err", &str2_sig),
            ("rnx_fs_move_err", &str2_sig),
            ("rnx_fs_rename_err", &str2_sig),
            ("rnx_fs_symlink_err", &str2_sig),
            ("rnx_fs_truncate_err", &str2_sig),
            ("rnx_fs_chmod_err", &str2_sig),
            ("rnx_fs_copy_err", &str3_sig),
            ("rnx_fs_write_text", &str3_sig),
            ("rnx_fs_write_text_err", &str3_sig),
            ("rnx_fs_write_bytes", &str3_sig),
            ("rnx_fs_write_bytes_err", &str3_sig),
            ("rnx_fs_mmap", &str2_sig),
            ("rnx_fs_mmap_err", &str2_sig),
            ("rnx_fs_mmap_anon", &str1_sig),
            ("rnx_fs_mmap_anon_err", &str1_sig),
            ("rnx_fs_mmap_addr", &str1_sig),
            ("rnx_fs_mmap_len", &str1_sig),
            ("rnx_fs_mmap_flush", &file_close_sig),
            ("rnx_fs_mmap_close", &file_close_sig),
            ("rnx_bytes_read_string", &str3_sig),
            ("rnx_bytes_write_string", &str3_sig),
            ("rnx_file_read_text_err", &str1_sig),
            ("rnx_float_nan", &gmap_new_sig),
            ("rnx_float_to_bits", &file_read_sig),
            ("rnx_float_from_bits", &file_read_sig),
            ("rnx_float_fma", &str3_sig),
        ] {
            let fid = module
                .declare_function(name, Linkage::Import, sig)
                .map_err(|e| Diagnostic::new(Code::E108, format!("declare {name}: {e}")))?;
            bytes_fns.insert(name.to_string(), fid);
        }
        let mut path_bool_sig = Signature::new(CallConv::SystemV);
        path_bool_sig.params.push(AbiParam::new(types::I64));
        path_bool_sig.returns.push(AbiParam::new(types::I8));
        let path_exists = module
            .declare_function("rnx_path_exists", Linkage::Import, &path_bool_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_path_exists: {e}")))?;
        let path_remove = module
            .declare_function("rnx_path_remove", Linkage::Import, &path_bool_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_path_remove: {e}")))?;
        for name in ["rnx_fs_exists", "rnx_fs_is_file", "rnx_fs_is_dir", "rnx_fs_remove", "rnx_fs_remove_all"] {
            let fid = module
                .declare_function(name, Linkage::Import, &path_bool_sig)
                .map_err(|e| Diagnostic::new(Code::E108, format!("declare {name}: {e}")))?;
            bytes_fns.insert(name.to_string(), fid);
        }
        let mut test_assert_sig = Signature::new(CallConv::SystemV);
        test_assert_sig.params.push(AbiParam::new(types::I64));
        test_assert_sig.params.push(AbiParam::new(types::I64));
        let test_assert = module
            .declare_function("rnx_assert", Linkage::Import, &test_assert_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_assert: {e}")))?;
        let mut test_check_sig = Signature::new(CallConv::SystemV);
        test_check_sig.returns.push(AbiParam::new(types::I8));
        let test_check = module
            .declare_function("rnx_test_check", Linkage::Import, &test_check_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_test_check: {e}")))?;
        let mut env_count_sig = Signature::new(CallConv::SystemV);
        env_count_sig.returns.push(AbiParam::new(types::I64));
        let env_count = module
            .declare_function("rnx_env_args_count", Linkage::Import, &env_count_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_env_args_count: {e}")))?;
        let mut env_get_arg_sig = Signature::new(CallConv::SystemV);
        env_get_arg_sig.params.push(AbiParam::new(types::I64));
        env_get_arg_sig.returns.push(AbiParam::new(types::I64));
        let env_args_get = module
            .declare_function("rnx_env_args_get", Linkage::Import, &env_get_arg_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_env_args_get: {e}")))?;
        let mut env_str_sig = Signature::new(CallConv::SystemV);
        env_str_sig.params.push(AbiParam::new(types::I64));
        env_str_sig.returns.push(AbiParam::new(types::I64));
        let env_get = module
            .declare_function("rnx_env_get", Linkage::Import, &env_str_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_env_get: {e}")))?;
        let mut env_set_sig = Signature::new(CallConv::SystemV);
        env_set_sig.params.push(AbiParam::new(types::I64));
        env_set_sig.params.push(AbiParam::new(types::I64));
        let env_set = module
            .declare_function("rnx_env_set", Linkage::Import, &env_set_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_env_set: {e}")))?;
        let mut env_cwd_sig = Signature::new(CallConv::SystemV);
        env_cwd_sig.returns.push(AbiParam::new(types::I64));
        let env_cwd = module
            .declare_function("rnx_env_cwd", Linkage::Import, &env_cwd_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_env_cwd: {e}")))?;
        let mut host_version_sig = Signature::new(CallConv::SystemV);
        host_version_sig.returns.push(AbiParam::new(types::I64));
        let host_version = module
            .declare_function("rnx_host_version", Linkage::Import, &host_version_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_host_version: {e}")))?;
        let mut env_exit_sig = Signature::new(CallConv::SystemV);
        env_exit_sig.params.push(AbiParam::new(types::I64));
        let env_exit = module
            .declare_function("rnx_env_exit", Linkage::Import, &env_exit_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_env_exit: {e}")))?;
        let net_connect = module
            .declare_function("rnx_net_connect_start", Linkage::Import, &file_open_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_connect_start: {e}")))?;
        let mut net_i64_sig = Signature::new(CallConv::SystemV);
        net_i64_sig.params.push(AbiParam::new(types::I64));
        net_i64_sig.returns.push(AbiParam::new(types::I64));
        let net_take_error = module
            .declare_function("rnx_net_take_error", Linkage::Import, &net_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_take_error: {e}")))?;
        let net_connect_wait = module
            .declare_function("rnx_net_connect_wait", Linkage::Import, &net_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_connect_wait: {e}")))?;
        let net_recv_wait = module
            .declare_function("rnx_net_recv_or_wait", Linkage::Import, &file_open_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_recv_or_wait: {e}")))?;
        let net_send_wait = module
            .declare_function("rnx_net_send_or_wait", Linkage::Import, &file_open_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_send_or_wait: {e}")))?;
        let net_recv_get = module
            .declare_function("rnx_net_recv_get", Linkage::Import, &file_open_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_recv_get: {e}")))?;
        let net_error_text = module
            .declare_function("rnx_net_error_text", Linkage::Import, &env_get_arg_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_error_text: {e}")))?;
        let net_close = module
            .declare_function("rnx_net_close", Linkage::Import, &net_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_close: {e}")))?;
        let net_listener_bind = module
            .declare_function("rnx_net_listener_bind", Linkage::Import, &file_open_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_listener_bind: {e}")))?;
        let net_listener_port = module
            .declare_function("rnx_net_listener_port", Linkage::Import, &net_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_listener_port: {e}")))?;
        let net_listener_accept_start = module
            .declare_function("rnx_net_listener_accept_start", Linkage::Import, &net_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_listener_accept_start: {e}")))?;
        let net_listener_accept_wait = module
            .declare_function("rnx_net_listener_accept_wait", Linkage::Import, &net_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_listener_accept_wait: {e}")))?;
        let net_listener_close = module
            .declare_function("rnx_net_listener_close", Linkage::Import, &net_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_net_listener_close: {e}")))?;
        let tls_connect_start = module
            .declare_function("rnx_tls_connect_start", Linkage::Import, &file_open_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_tls_connect_start: {e}")))?;
        let tls_handshake_start = module
            .declare_function("rnx_tls_handshake_start", Linkage::Import, &net_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_tls_handshake_start: {e}")))?;
        let tls_handshake_wait = module
            .declare_function("rnx_tls_handshake_wait", Linkage::Import, &net_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_tls_handshake_wait: {e}")))?;
        let tls_recv_wait = module
            .declare_function("rnx_tls_recv_or_wait", Linkage::Import, &file_open_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_tls_recv_or_wait: {e}")))?;
        let tls_send_wait = module
            .declare_function("rnx_tls_send_or_wait", Linkage::Import, &file_open_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_tls_send_or_wait: {e}")))?;
        let tls_recv_get = module
            .declare_function("rnx_tls_recv_get", Linkage::Import, &file_open_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_tls_recv_get: {e}")))?;
        let tls_error_text = module
            .declare_function("rnx_tls_error_text", Linkage::Import, &env_get_arg_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_tls_error_text: {e}")))?;
        let tls_close = module
            .declare_function("rnx_tls_close", Linkage::Import, &net_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_tls_close: {e}")))?;
        let mut proc0_sig = Signature::new(CallConv::SystemV);
        proc0_sig.returns.push(AbiParam::new(types::I64));
        let mut proc4_sig = Signature::new(CallConv::SystemV);
        for _ in 0..4 {
            proc4_sig.params.push(AbiParam::new(types::I64));
        }
        proc4_sig.returns.push(AbiParam::new(types::I64));
        let mut proc7_sig = Signature::new(CallConv::SystemV);
        for _ in 0..7 {
            proc7_sig.params.push(AbiParam::new(types::I64));
        }
        proc7_sig.returns.push(AbiParam::new(types::I64));
        let proc_decl = |module: &mut JITModule, name: &str, sig: &Signature| {
            module
                .declare_function(name, Linkage::Import, sig)
                .map_err(|e| Diagnostic::new(Code::E108, format!("declare {name}: {e}")))
        };
        let proc_pid = proc_decl(&mut *module, "rnx_process_pid", &proc0_sig)?;
        let proc_remove_env = proc_decl(&mut *module, "rnx_process_remove_env", &void1_sig)?;
        let proc_all_env_count = proc_decl(&mut *module, "rnx_process_all_env_count", &proc0_sig)?;
        let proc_all_env_get = proc_decl(&mut *module, "rnx_process_all_env_get", &str1_sig)?;
        let proc_chdir = proc_decl(&mut *module, "rnx_process_chdir", &str1_sig)?;
        let proc_spawn = proc_decl(&mut *module, "rnx_process_spawn", &proc7_sig)?;
        let proc_run = proc_decl(&mut *module, "rnx_process_run", &proc7_sig)?;
        let proc_pid_of = proc_decl(&mut *module, "rnx_process_pid_of", &str1_sig)?;
        let proc_write_stdin = proc_decl(&mut *module, "rnx_process_write_stdin", &proc4_sig)?;
        let proc_read_stdout = proc_decl(&mut *module, "rnx_process_read_stdout", &proc4_sig)?;
        let proc_read_stderr = proc_decl(&mut *module, "rnx_process_read_stderr", &proc4_sig)?;
        let proc_close_stdin = proc_decl(&mut *module, "rnx_process_close_stdin", &void1_sig)?;
        let proc_wait = proc_decl(&mut *module, "rnx_process_wait", &str1_sig)?;
        let proc_try_wait = proc_decl(&mut *module, "rnx_process_try_wait", &str1_sig)?;
        let proc_kill = proc_decl(&mut *module, "rnx_process_kill", &str2_sig)?;
        let proc_take_stdout = proc_decl(&mut *module, "rnx_process_take_stdout", &str1_sig)?;
        let proc_take_stderr = proc_decl(&mut *module, "rnx_process_take_stderr", &str1_sig)?;
        let proc_exit_code = proc_decl(&mut *module, "rnx_process_exit_code", &str1_sig)?;
        let proc_forget = proc_decl(&mut *module, "rnx_process_forget", &void1_sig)?;
        let os_platform = proc_decl(&mut *module, "rnx_os_platform", &proc0_sig)?;
        let os_arch = proc_decl(&mut *module, "rnx_os_arch", &proc0_sig)?;
        let os_hostname = proc_decl(&mut *module, "rnx_os_hostname", &proc0_sig)?;
        let os_tmpdir = proc_decl(&mut *module, "rnx_os_tmpdir", &proc0_sig)?;
        let os_homedir = proc_decl(&mut *module, "rnx_os_homedir", &proc0_sig)?;
        let os_cpu_count = proc_decl(&mut *module, "rnx_os_cpu_count", &proc0_sig)?;
        let os_uptime = proc_decl(&mut *module, "rnx_os_uptime", &proc0_sig)?;
        let mut map_new_sig = Signature::new(CallConv::SystemV);
        map_new_sig.returns.push(AbiParam::new(types::I64));
        let map_new = module
            .declare_function("rnx_map_new", Linkage::Import, &map_new_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_map_new: {e}")))?;
        let mut map_set_sig = Signature::new(CallConv::SystemV);
        map_set_sig.params.push(AbiParam::new(types::I64));
        map_set_sig.params.push(AbiParam::new(types::I64));
        map_set_sig.params.push(AbiParam::new(types::I64));
        let map_set = module
            .declare_function("rnx_map_set", Linkage::Import, &map_set_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_map_set: {e}")))?;
        let mut map_get_sig = Signature::new(CallConv::SystemV);
        map_get_sig.params.push(AbiParam::new(types::I64));
        map_get_sig.params.push(AbiParam::new(types::I64));
        map_get_sig.returns.push(AbiParam::new(types::I64));
        let map_get = module
            .declare_function("rnx_map_get", Linkage::Import, &map_get_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_map_get: {e}")))?;
        let mut map_bool_sig = Signature::new(CallConv::SystemV);
        map_bool_sig.params.push(AbiParam::new(types::I64));
        map_bool_sig.params.push(AbiParam::new(types::I64));
        map_bool_sig.returns.push(AbiParam::new(types::I8));
        let map_has = module
            .declare_function("rnx_map_has", Linkage::Import, &map_bool_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_map_has: {e}")))?;
        let map_delete = module
            .declare_function("rnx_map_delete", Linkage::Import, &map_bool_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_map_delete: {e}")))?;
        let mut map_len_sig = Signature::new(CallConv::SystemV);
        map_len_sig.params.push(AbiParam::new(types::I64));
        map_len_sig.returns.push(AbiParam::new(types::I64));
        let map_len = module
            .declare_function("rnx_map_len", Linkage::Import, &map_len_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_map_len: {e}")))?;
        let mut map_clear_sig = Signature::new(CallConv::SystemV);
        map_clear_sig.params.push(AbiParam::new(types::I64));
        let map_clear = module
            .declare_function("rnx_map_clear", Linkage::Import, &map_clear_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_map_clear: {e}")))?;
        let mut map_arr_sig = Signature::new(CallConv::SystemV);
        map_arr_sig.params.push(AbiParam::new(types::I64));
        map_arr_sig.returns.push(AbiParam::new(types::I64));
        let map_keys = module
            .declare_function("rnx_map_keys", Linkage::Import, &map_arr_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_map_keys: {e}")))?;
        let map_values = module
            .declare_function("rnx_map_values", Linkage::Import, &map_arr_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_map_values: {e}")))?;
        let mut sync_i64_sig = Signature::new(CallConv::SystemV);
        sync_i64_sig.params.push(AbiParam::new(types::I64));
        sync_i64_sig.returns.push(AbiParam::new(types::I64));
        let sync_atomic_get = module
            .declare_function("rnx_sync_atomic_get", Linkage::Import, &sync_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_atomic_get: {e}")))?;
        let black_box = module
            .declare_function("rnx_black_box_i64", Linkage::Import, &sync_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_black_box_i64: {e}")))?;
        let sync_channel_recv = module
            .declare_function("rnx_sync_channel_recv", Linkage::Import, &sync_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_channel_recv: {e}")))?;
        let sync_channel_try_recv = module
            .declare_function("rnx_sync_channel_try_recv", Linkage::Import, &sync_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_channel_try_recv: {e}")))?;
        let sync_channel_len = module
            .declare_function("rnx_sync_channel_len", Linkage::Import, &sync_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_channel_len: {e}")))?;
        let fs_pool_depth = module
            .declare_function("rnx_fs_pool_depth", Linkage::Import, &sync_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_fs_pool_depth: {e}")))?;
        let mut sync_i64_i64_sig = Signature::new(CallConv::SystemV);
        sync_i64_i64_sig.params.push(AbiParam::new(types::I64));
        sync_i64_i64_sig.params.push(AbiParam::new(types::I64));
        sync_i64_i64_sig.returns.push(AbiParam::new(types::I64));
        let sync_atomic_fetch_add = module
            .declare_function("rnx_sync_atomic_fetch_add", Linkage::Import, &sync_i64_i64_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_atomic_fetch_add: {e}")))?;
        let mut sync_void_sig = Signature::new(CallConv::SystemV);
        sync_void_sig.params.push(AbiParam::new(types::I64));
        sync_void_sig.params.push(AbiParam::new(types::I64));
        let sync_atomic_set = module
            .declare_function("rnx_sync_atomic_set", Linkage::Import, &sync_void_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_atomic_set: {e}")))?;
        let sync_channel_send = module
            .declare_function("rnx_sync_channel_send", Linkage::Import, &sync_void_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_channel_send: {e}")))?;
        let mut sync_void1_sig = Signature::new(CallConv::SystemV);
        sync_void1_sig.params.push(AbiParam::new(types::I64));
        let sync_channel_drop = module
            .declare_function("rnx_sync_channel_drop", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_channel_drop: {e}")))?;
        let mutex_lock = module
            .declare_function("rnx_mutex_lock", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_mutex_lock: {e}")))?;
        let mutex_unlock = module
            .declare_function("rnx_mutex_unlock", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_mutex_unlock: {e}")))?;
        let rwlock_read_lock = module
            .declare_function("rnx_rwlock_read_lock", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_rwlock_read_lock: {e}")))?;
        let rwlock_read_unlock = module
            .declare_function("rnx_rwlock_read_unlock", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_rwlock_read_unlock: {e}")))?;
        let rwlock_write_lock = module
            .declare_function("rnx_rwlock_write_lock", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_rwlock_write_lock: {e}")))?;
        let rwlock_write_unlock = module
            .declare_function("rnx_rwlock_write_unlock", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_rwlock_write_unlock: {e}")))?;
        let mut sync_bool_sig = Signature::new(CallConv::SystemV);
        sync_bool_sig.params.push(AbiParam::new(types::I64));
        sync_bool_sig.returns.push(AbiParam::new(types::I8));
        let mutex_try_lock = module
            .declare_function("rnx_mutex_try_lock", Linkage::Import, &sync_bool_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_mutex_try_lock: {e}")))?;
        let rwlock_try_read_lock = module
            .declare_function("rnx_rwlock_try_read_lock", Linkage::Import, &sync_bool_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_rwlock_try_read_lock: {e}")))?;
        let rwlock_try_write_lock = module
            .declare_function("rnx_rwlock_try_write_lock", Linkage::Import, &sync_bool_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_rwlock_try_write_lock: {e}")))?;
        let condvar_wait = module
            .declare_function("rnx_condvar_wait", Linkage::Import, &sync_void_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_condvar_wait: {e}")))?;
        let condvar_notify_one = module
            .declare_function("rnx_condvar_notify_one", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_condvar_notify_one: {e}")))?;
        let condvar_notify_all = module
            .declare_function("rnx_condvar_notify_all", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_condvar_notify_all: {e}")))?;
        let mut sync_bool2_sig = Signature::new(CallConv::SystemV);
        sync_bool2_sig.params.push(AbiParam::new(types::I64));
        sync_bool2_sig.params.push(AbiParam::new(types::I64));
        sync_bool2_sig.returns.push(AbiParam::new(types::I8));
        let barrier_wait = module
            .declare_function("rnx_barrier_wait", Linkage::Import, &sync_bool2_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_barrier_wait: {e}")))?;
        let mut sync_bool3_sig = Signature::new(CallConv::SystemV);
        sync_bool3_sig.params.push(AbiParam::new(types::I64));
        sync_bool3_sig.params.push(AbiParam::new(types::I64));
        sync_bool3_sig.params.push(AbiParam::new(types::I64));
        sync_bool3_sig.returns.push(AbiParam::new(types::I8));
        let condvar_wait_timeout = module
            .declare_function("rnx_condvar_wait_timeout", Linkage::Import, &sync_bool3_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_condvar_wait_timeout: {e}")))?;
        let mut pool_void2_sig = Signature::new(CallConv::SystemV);
        pool_void2_sig.params.push(AbiParam::new(types::I64));
        pool_void2_sig.params.push(AbiParam::new(types::I64));
        let pool_init = module
            .declare_function("rnx_thread_pool_init", Linkage::Import, &pool_void2_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_thread_pool_init: {e}")))?;
        let pool_join = module
            .declare_function("rnx_thread_pool_join", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_thread_pool_join: {e}")))?;
        let pool_shutdown = module
            .declare_function("rnx_thread_pool_shutdown", Linkage::Import, &sync_void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_thread_pool_shutdown: {e}")))?;
        let mut pool_for_sig = Signature::new(CallConv::SystemV);
        pool_for_sig.params.push(AbiParam::new(types::I64));
        pool_for_sig.params.push(AbiParam::new(types::I64));
        pool_for_sig.params.push(AbiParam::new(types::I64));
        pool_for_sig.params.push(AbiParam::new(types::I64));
        pool_for_sig.params.push(AbiParam::new(types::I64));
        let pool_parallel_for = module
            .declare_function("rnx_thread_pool_parallel_for", Linkage::Import, &pool_for_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_thread_pool_parallel_for: {e}")))?;
        let mut debug_sig = Signature::new(CallConv::SystemV);
        debug_sig.returns.push(AbiParam::new(types::I64));
        let debug_live = module
            .declare_function("rnx_debug_live_count", Linkage::Import, &debug_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_debug_live_count: {e}")))?;
        let mut defer_push_sig = Signature::new(CallConv::SystemV);
        defer_push_sig.params.push(AbiParam::new(types::I64));
        let defer_push = module
            .declare_function("rnx_defer_push", Linkage::Import, &defer_push_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_defer_push: {e}")))?;
        let mut defer_pop_sig = Signature::new(CallConv::SystemV);
        defer_pop_sig.returns.push(AbiParam::new(types::I64));
        let defer_pop = module
            .declare_function("rnx_defer_pop", Linkage::Import, &defer_pop_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_defer_pop: {e}")))?;
        let mut defer_len_sig = Signature::new(CallConv::SystemV);
        defer_len_sig.returns.push(AbiParam::new(types::I64));
        let defer_len = module
            .declare_function("rnx_defer_len", Linkage::Import, &defer_len_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_defer_len: {e}")))?;
        let mut u64_fn_sig = Signature::new(CallConv::SystemV);
        u64_fn_sig.params.push(AbiParam::new(types::I64));
        u64_fn_sig.returns.push(AbiParam::new(types::I64));
        let any_tag = module
            .declare_function("rnx_any_tag", Linkage::Import, &u64_fn_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_any_tag: {e}")))?;
        let obj_class = module
            .declare_function("rnx_obj_class", Linkage::Import, &u64_fn_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_obj_class: {e}")))?;
        let error_set = module
            .declare_function("rnx_error_set", Linkage::Import, &void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_error_set: {e}")))?;
        let mut take_sig = Signature::new(CallConv::SystemV);
        take_sig.returns.push(AbiParam::new(types::I64));
        let error_take = module
            .declare_function("rnx_error_take", Linkage::Import, &take_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_error_take: {e}")))?;
        let error_class = module
            .declare_function("rnx_error_class", Linkage::Import, &u64_fn_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_error_class: {e}")))?;
        let error_unbox = module
            .declare_function("rnx_error_unbox", Linkage::Import, &u64_fn_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_error_unbox: {e}")))?;
        let error_str = module
            .declare_function("rnx_error_str", Linkage::Import, &u64_fn_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_error_str: {e}")))?;
        let error_release = module
            .declare_function("rnx_error_release", Linkage::Import, &void1_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_error_release: {e}")))?;
        let type_name = module
            .declare_function("rnx_type_name", Linkage::Import, &u64_fn_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_type_name: {e}")))?;
        let typeof_any = module
            .declare_function("rnx_typeof_any", Linkage::Import, &u64_fn_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_typeof_any: {e}")))?;
        let mut note_sig = Signature::new(CallConv::SystemV);
        note_sig.params.push(AbiParam::new(types::I64));
        note_sig.params.push(AbiParam::new(types::I64));
        let note_type = module
            .declare_function("rnx_note_type", Linkage::Import, &note_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_note_type: {e}")))?;
        let mut pretty_sig = Signature::new(CallConv::SystemV);
        pretty_sig.params.push(AbiParam::new(types::I64));
        pretty_sig.params.push(AbiParam::new(types::I64));
        pretty_sig.returns.push(AbiParam::new(types::I64));
        let io_pretty = module
            .declare_function("rnx_io_pretty", Linkage::Import, &pretty_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_io_pretty: {e}")))?;
        let mut note3_sig = Signature::new(CallConv::SystemV);
        note3_sig.params.push(AbiParam::new(types::I64));
        note3_sig.params.push(AbiParam::new(types::I64));
        note3_sig.params.push(AbiParam::new(types::I64));
        let note_array_kind = module
            .declare_function("rnx_note_array_kind", Linkage::Import, &note3_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_note_array_kind: {e}")))?;
        let note_fields = module
            .declare_function("rnx_note_fields", Linkage::Import, &note_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_note_fields: {e}")))?;
        let mut note4_sig = Signature::new(CallConv::SystemV);
        note4_sig.params.push(AbiParam::new(types::I64));
        note4_sig.params.push(AbiParam::new(types::I64));
        note4_sig.params.push(AbiParam::new(types::I64));
        note4_sig.params.push(AbiParam::new(types::I64));
        let note_enum = module
            .declare_function("rnx_note_enum", Linkage::Import, &note4_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_note_enum: {e}")))?;
        let mut slice_sig = Signature::new(CallConv::SystemV);
        for _ in 0..6 {
            slice_sig.params.push(AbiParam::new(types::I64));
        }
        slice_sig.returns.push(AbiParam::new(types::I64));
        let array_slice = module
            .declare_function("rnx_array_slice", Linkage::Import, &slice_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_array_slice: {e}")))?;
        let mut sync_obj_sig = Signature::new(CallConv::SystemV);
        sync_obj_sig.params.push(AbiParam::new(types::I64));
        sync_obj_sig.params.push(AbiParam::new(types::I64));
        sync_obj_sig.params.push(AbiParam::new(types::I64));
        sync_obj_sig.params.push(AbiParam::new(types::I64));
        let sync_channel_send_str = module
            .declare_function("rnx_sync_channel_send_str", Linkage::Import, &sync_void_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_channel_send_str: {e}")))?;
        let sync_channel_send_obj = module
            .declare_function("rnx_sync_channel_send_obj", Linkage::Import, &sync_obj_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_channel_send_obj: {e}")))?;
        let sync_channel_send_array = module
            .declare_function("rnx_sync_channel_send_array", Linkage::Import, &sync_obj_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_channel_send_array: {e}")))?;

        let mut sync_cas_sig = Signature::new(CallConv::SystemV);
        sync_cas_sig.params.push(AbiParam::new(types::I64));
        sync_cas_sig.params.push(AbiParam::new(types::I64));
        sync_cas_sig.params.push(AbiParam::new(types::I64));
        sync_cas_sig.returns.push(AbiParam::new(types::I8));
        let sync_atomic_cas = module
            .declare_function("rnx_sync_atomic_cas", Linkage::Import, &sync_cas_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_sync_atomic_cas: {e}")))?;
        let mut math_sqrt_sig = Signature::new(CallConv::SystemV);
        math_sqrt_sig.params.push(AbiParam::new(types::I64));
        math_sqrt_sig.returns.push(AbiParam::new(types::I64));
        let math_sqrt = module
            .declare_function("rnx_math_sqrt", Linkage::Import, &math_sqrt_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_math_sqrt: {e}")))?;
        let mut math_sin_sig = Signature::new(CallConv::SystemV);
        math_sin_sig.params.push(AbiParam::new(types::I64));
        math_sin_sig.returns.push(AbiParam::new(types::I64));
        let math_sin = module
            .declare_function("rnx_math_sin", Linkage::Import, &math_sin_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_math_sin: {e}")))?;
        let mut math_cos_sig = Signature::new(CallConv::SystemV);
        math_cos_sig.params.push(AbiParam::new(types::I64));
        math_cos_sig.returns.push(AbiParam::new(types::I64));
        let math_cos = module
            .declare_function("rnx_math_cos", Linkage::Import, &math_cos_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_math_cos: {e}")))?;
        let mut math_tan_sig = Signature::new(CallConv::SystemV);
        math_tan_sig.params.push(AbiParam::new(types::I64));
        math_tan_sig.returns.push(AbiParam::new(types::I64));
        let math_tan = module
            .declare_function("rnx_math_tan", Linkage::Import, &math_tan_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_math_tan: {e}")))?;
        let mut math_atan2_sig = Signature::new(CallConv::SystemV);
        math_atan2_sig.params.push(AbiParam::new(types::I64));
        math_atan2_sig.params.push(AbiParam::new(types::I64));
        math_atan2_sig.returns.push(AbiParam::new(types::I64));
        let math_atan2 = module
            .declare_function("rnx_math_atan2", Linkage::Import, &math_atan2_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_math_atan2: {e}")))?;
        let mut math_pow_sig = Signature::new(CallConv::SystemV);
        math_pow_sig.params.push(AbiParam::new(types::I64));
        math_pow_sig.params.push(AbiParam::new(types::I64));
        math_pow_sig.returns.push(AbiParam::new(types::I64));
        let math_pow = module
            .declare_function("rnx_math_pow", Linkage::Import, &math_pow_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_math_pow: {e}")))?;
        let mut math_floor_sig = Signature::new(CallConv::SystemV);
        math_floor_sig.params.push(AbiParam::new(types::I64));
        math_floor_sig.returns.push(AbiParam::new(types::I64));
        let math_floor = module
            .declare_function("rnx_math_floor", Linkage::Import, &math_floor_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_math_floor: {e}")))?;
        let mut math_ceil_sig = Signature::new(CallConv::SystemV);
        math_ceil_sig.params.push(AbiParam::new(types::I64));
        math_ceil_sig.returns.push(AbiParam::new(types::I64));
        let math_ceil = module
            .declare_function("rnx_math_ceil", Linkage::Import, &math_ceil_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_math_ceil: {e}")))?;
        let mut math_round_sig = Signature::new(CallConv::SystemV);
        math_round_sig.params.push(AbiParam::new(types::I64));
        math_round_sig.returns.push(AbiParam::new(types::I64));
        let math_round = module
            .declare_function("rnx_math_round", Linkage::Import, &math_round_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_math_round: {e}")))?;
        let mut math_log_sig = Signature::new(CallConv::SystemV);
        math_log_sig.params.push(AbiParam::new(types::I64));
        math_log_sig.returns.push(AbiParam::new(types::I64));
        let math_log = module
            .declare_function("rnx_math_log", Linkage::Import, &math_log_sig)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare rnx_math_log: {e}")))?;
        let mut foreign_ids: BTreeMap<(String, String), FuncId> = BTreeMap::new();
        for f in &lir.foreign {
            let sig = foreign_sig(f);
            let id = module
                .declare_function(&f.symbol, Linkage::Import, &sig)
                .map_err(|e| Diagnostic::new(Code::E108, format!("declare {}: {e}", f.symbol)))?;
            foreign_ids.insert((f.lib.clone(), f.symbol.clone()), id);
        }
        let statics = declare_statics(&mut *module, lir)?;
        let rt = RtIds {
            alloc,
            release,
            genref_create,
            genref_get,
            genref_invalidate,
            retain,
            concat,
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
            map_new,
            map_set,
            map_get,
            map_has,
            map_delete,
            map_len,
            map_clear,
            map_keys,
            map_values,
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
            pool_init,
            pool_parallel_for,
            pool_join,
            pool_shutdown,
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
            string_len,
            string_trim,
            string_concat,
            string_split,
            string_index_of,
            string_index_of_from,
            string_char_code_at,
            string_from_char_code,
            string_slice,
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
            panic_str,
            fatal_span,
            spawn_closure,
            join_val,
            join_err,
            task_val,
            task_err,
            pool_new,
            submit_handle,
            submit_closure,
            parallel_closure,
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
            array_slice,
        };
        Ok((ids, sigs, rt, bytes_fns, statics, foreign_ids))
    }

    fn define_functions(module: &mut JITModule, lir: &LirModule, ids: &BTreeMap<String, FuncId>, sigs: &BTreeMap<FuncId, Signature>, foreign_ids: &BTreeMap<(String, String), FuncId>, rt: &RtIds, bytes_fns: &BTreeMap<String, FuncId>, statics: &BTreeMap<String, DataId>, ctx: &mut FunctionBuilderContext, hot: bool, spare: usize, timings: &mut CodegenTimings) -> Result<(BTreeMap<String, FunctionSlotId>, Vec<String>, Option<Box<[AtomicU64]>>, BTreeMap<String, usize>), Diagnostic> {
        use std::time::Instant;
        let mut sizes: BTreeMap<String, usize> = BTreeMap::new();
        let (slots, slot_names, table) = if hot {
            let mut slots = BTreeMap::new();
            let mut slot_names = Vec::with_capacity(lir.functions.len() + spare);
            for (i, f) in lir.functions.iter().enumerate() {
                slots.insert(f.name.clone(), i as FunctionSlotId);
                slot_names.push(f.name.clone());
            }
            let table: Box<[AtomicU64]> = (0..lir.functions.len() + spare)
                .map(|_| AtomicU64::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice();
            (slots, slot_names, Some(table))
        } else {
            (BTreeMap::new(), Vec::new(), None)
        };
        let table_base = table.as_ref().map(|t| t.as_ptr() as usize).unwrap_or(0);
        let hot_ctx = HotCtx { sigs: &sigs, slots: &slots, table_base };
        for f in &lir.functions {
            let id = ids[&f.name];
            let mut codegen_ctx = module.make_context();
            codegen_ctx.func =
                ClFunction::with_name_signature(UserFuncName::user(0, id.as_u32()), sigs[&id].clone());
            let inner = Instant::now();
            lower_fn(lir, f, ids, foreign_ids, rt, hot.then_some(hot_ctx), bytes_fns, statics, module, &mut codegen_ctx.func, ctx)?;
            timings.clif_lower += inner.elapsed();
            let inner = Instant::now();
            module
                .define_function(id, &mut codegen_ctx)
                .map_err(|e| Diagnostic::new(Code::E108, format!("define {}: {e}", f.name)))?;
            timings.clif_backend += inner.elapsed();
            let size = codegen_ctx
                .compiled_code()
                .map(|c| c.code_info().total_size as usize)
                .unwrap_or(0);
            sizes.insert(f.name.clone(), size);
        }
        Ok((slots, slot_names, table, sizes))
    }

    fn finalize_module(module: &mut JITModule, lir: &LirModule, ids: &BTreeMap<String, FuncId>, table: &Option<Box<[AtomicU64]>>) -> Result<BTreeSet<String>, Diagnostic> {
        module.finalize_definitions().map_err(|e| {
            Diagnostic::new(Code::E108, format!("finalize: {e}"))
        })?;
            for (i, f) in lir.functions.iter().enumerate() {
            if f.is_closure {
                if let Some(id) = ids.get(&f.name) {
                    let addr = module.get_finalized_function(*id) as usize;
                    runtime::native::rnx_closure_register(runtime::native::rnx_closure_tag(i as u64), addr);
                }
            }
        }
        if let Some(t) = table.as_ref() {
            for (i, f) in lir.functions.iter().enumerate() {
                let addr = module.get_finalized_function(ids[&f.name]) as u64;
                t[i].store(addr, Ordering::Release);
            }
        }
        let mut vec_rets = BTreeSet::new();
        for f in &lir.functions {
            if matches!(f.ret, LirType::Vec4f | LirType::Vec4i) {
                vec_rets.insert(f.name.clone());
            }
        }
        Ok(vec_rets)
    }
}

pub(super) fn check_supported(lir: &LirModule, f: &LirFunction) -> Result<(), Diagnostic> {
    let bad = |what: &str| Diagnostic::new(Code::E108, format!("jit subset: {what} in `{}`", f.name));
    for t in f.params.iter().chain(f.locals.iter()) {
        if !matches!(t, LirType::I64 | LirType::I8 | LirType::Bool | LirType::F64(_) | LirType::Null | LirType::Obj(_) | LirType::GenRef(_) | LirType::Pointer(_) | LirType::Pool | LirType::Vec4f | LirType::Vec4i | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Any | LirType::Closure | LirType::Tuple(_) | LirType::Range | LirType::Error) {
            return Err(bad("non-scalar local"));
        }
    }
    if !matches!(f.ret, LirType::I64 | LirType::I8 | LirType::Bool | LirType::F64(_) | LirType::Any | LirType::Void | LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Vec4f | LirType::Vec4i | LirType::Closure | LirType::Tuple(_) | LirType::Range | LirType::Error) {
        return Err(bad("non-scalar return"));
    }
    for b in &f.blocks {
        for ins in &b.instrs {
            match ins {
                Instr::Const { lit, .. } => match lit {
                    Lit::Int(_) | Lit::Bool(_) | Lit::Null | Lit::Str(_) | Lit::Float(..) => {}
                },
                Instr::Concat { .. } | Instr::ToStr { .. } | Instr::Convert { .. } => {},
                Instr::Copy { dst, src , ..} => {
                    let dinfo = f.locals.get(*dst as usize);
                    let sinfo = f.locals.get(*src as usize);
                    match (dinfo, sinfo) {
                        (Some(LirType::Obj(_)), Some(LirType::Obj(_))) => {}
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
                Instr::Cast { .. } => {}
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
                        _ => return Err(bad("pointer load needs an Int, Float, Byte, or Bool slot")),
                    }
                }
                Instr::PtrStore { val, .. } => {
                    match f.locals.get(*val as usize) {
                        Some(LirType::I64) | Some(LirType::I8) | Some(LirType::F64(_)) | Some(LirType::Bool) => {}
                        _ => return Err(bad("pointer store needs an Int, Float, Byte, or Bool value")),
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
                            Some(LirType::Array(inner)) if array_capturable(inner) => {}
                            _ => return Err(bad("closure captures unsupported value")),
                        }
                    }
                }
                Instr::EnumNew { .. } | Instr::EnumPayload { .. } | Instr::EnumTag { .. } => {}
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
                Instr::PoolInit { .. }
                | Instr::PoolSubmit { .. }
                | Instr::PoolParallelFor { .. }
                | Instr::PoolJoin { .. }
                | Instr::PoolShutdown { .. } => {}
                Instr::VecNew { .. }
                | Instr::VecSplat { .. }
                | Instr::VecExtract { .. }
                | Instr::VecInsert { .. }
                | Instr::VecArith { .. }
                | Instr::VecUnary { .. }
                | Instr::VecDot { .. } => {}
                Instr::Defer { .. } | Instr::RunDefers { .. } => {}
                Instr::AddrOf { .. } => {
                    return Err(bad("address of a stack local is not supported in native codegen; use `Pointer.fromAddress` with a real address"));
                }
                Instr::Call { target, span, .. } => {
                    let supported = matches!(target, CallTarget::Fn(_))
                        || matches!(target, CallTarget::Foreign { .. })
                        || matches!(target, CallTarget::Value(_))
                        || matches!(target, CallTarget::Builtin(n) if n == "print" || n == "streq" || n == "strcmp" || n == "__rnx_clock_mono" || n == "__rnx_crypto_random_u64" || n == "__rnx_prng_seed" || n == "__rnx_prng_next" || n == "__rnx_file_open" || n == "__rnx_file_close" || n == "__rnx_file_read_text" || n == "__rnx_file_read_text_err" || n == "__rnx_file_write_text" || n == "__rnx_file_flush" || n == "__rnx_file_seek" || n == "__rnx_file_tell" || n == "__rnx_file_from_handle" || n == "__rnx_io_is_tty" || n == "__rnx_io_pretty" || n == "__rnx_io_winsize" || n == "__rnx_io_set_raw" || n.starts_with("__rnx_bytes_") || n == "__rnx_file_read_bytes" || n == "__rnx_file_write_bytes" || n == "assert" || n == "__testCheck" || n == "__rnx_path_exists" || n == "__rnx_path_remove" || n.starts_with("__rnx_fs_") || n == "__rnx_env_args_count" || n == "__rnx_env_args_get" || n == "__rnx_env_get" || n == "__rnx_env_set" || n == "__rnx_env_cwd" || n == "__rnx_host_version" || n == "__rnx_env_exit" || n == "__rnx_net_connect_start" || n == "__rnx_net_take_error" || n == "__rnx_net_recv_get" || n == "__rnx_net_error_text" || n == "__rnx_net_close" || n == "__rnx_net_connect_wait" || n == "__rnx_net_recv_or_wait" || n == "__rnx_net_send_or_wait" || n == "__rnx_net_listener_bind" || n == "__rnx_net_listener_port" || n == "__rnx_net_listener_accept_start" || n == "__rnx_net_listener_accept_wait" || n == "__rnx_net_listener_close" || n == "__rnx_tls_connect_start" || n == "__rnx_tls_handshake_start" || n == "__rnx_tls_handshake_wait" || n == "__rnx_tls_recv_or_wait" || n == "__rnx_tls_send_or_wait" || n == "__rnx_tls_recv_get" || n == "__rnx_tls_error_text" || n == "__rnx_tls_close" || n == "__rnx_process_pid" || n == "__rnx_process_remove_env" || n == "__rnx_process_all_env_count" || n == "__rnx_process_all_env_get" || n == "__rnx_process_chdir" || n == "__rnx_process_spawn" || n == "__rnx_process_run" || n == "__rnx_process_pid_of" || n == "__rnx_process_write_stdin" || n == "__rnx_process_read_stdout" || n == "__rnx_process_read_stderr" || n == "__rnx_process_close_stdin" || n == "__rnx_process_wait" || n == "__rnx_process_try_wait" || n == "__rnx_process_kill" || n == "__rnx_process_take_stdout" || n == "__rnx_process_take_stderr" || n == "__rnx_process_exit_code" || n == "__rnx_process_forget" || n == "__rnx_os_platform" || n == "__rnx_os_arch" || n == "__rnx_os_hostname" || n == "__rnx_os_tmpdir" || n == "__rnx_os_homedir" || n == "__rnx_os_cpu_count" || n == "__rnx_os_uptime" || n == "__rnx_math_sqrt" || n == "__rnx_math_sin" || n == "__rnx_math_cos" || n == "__rnx_math_tan" || n == "__rnx_math_atan2" || n == "__rnx_math_pow" || n == "__rnx_math_floor" || n == "__rnx_math_ceil" || n == "__rnx_math_round" || n == "__rnx_math_log" || n.starts_with("__rnx_float_") || n == "__rnx_map_new" || n == "__rnx_map_set" || n == "__rnx_map_get" || n == "__rnx_map_has" || n == "__rnx_map_delete" || n == "__rnx_map_len" || n == "__rnx_string_len" || n == "__rnx_string_slice" || n == "__rnx_string_index_of" || n == "__rnx_string_index_of_from" || n == "__rnx_string_trim"                             || n == "__rnx_string_concat" || n == "__rnx_string_split"
                            || n == "__rnx_string_char_code_at" || n == "__rnx_string_from_char_code" || n == "__rnx_int_to_str" || n == "__rnx_float_to_str" || n == "__rnx_bool_to_str" || n == "__rnx_array_pop" || n == "__rnx_array_len" || n == "__rnx_map_clear" || n == "__rnx_map_keys" || n == "__rnx_map_values" || n.starts_with("__rnx_gmap_") || n == "__rnx_json_parse" || n == "__rnx_json_parse_typed" || n == "__rnx_json_stringify" || n == "__rnx_json_stringify_into" || n == "__rnx_json_unwrap" || n == "__rnx_dns_lookup_start" || n == "__rnx_dns_lookup_wait" || n == "__rnx_dns_lookup_get" || n == "__rnx_dns_lookup_error" || n == "__rnx_black_box" || n == "__rnx_sync_atomic_get" || n == "__rnx_sync_atomic_set" || n == "__rnx_sync_atomic_fetch_add" || n == "__rnx_sync_atomic_cas" || n == "__rnx_sync_channel_send" || n == "__rnx_sync_channel_send_str" || n == "__rnx_sync_channel_send_obj" || n == "__rnx_sync_channel_send_array" || n == "__rnx_sync_channel_recv" || n == "__rnx_sync_channel_try_recv" || n == "__rnx_sync_channel_len" || n == "__rnx_sync_channel_drop" || n == "__rnx_debug_live_count" || n == "__rnx_mutex_lock" || n == "__rnx_mutex_unlock" || n == "__rnx_mutex_try_lock" || n == "__rnx_rwlock_read_lock" || n == "__rnx_rwlock_read_unlock" || n == "__rnx_rwlock_write_lock" || n == "__rnx_rwlock_write_unlock" || n == "__rnx_rwlock_try_read_lock" || n == "__rnx_rwlock_try_write_lock" || n == "__rnx_condvar_wait" || n == "__rnx_condvar_wait_timeout" || n == "__rnx_condvar_notify_one" || n == "__rnx_condvar_notify_all" || n == "__rnx_barrier_wait" || n == "__rnx_thread_spawn_closure" || n == "__rnx_thread_join_val" || n == "__rnx_thread_join_err" || n == "__rnx_task_await_val" || n == "__rnx_task_await_err" || n == "__rnx_pool_new" || n == "__rnx_thread_pool_submit_handle" || n == "__rnx_thread_pool_submit_closure" || n == "__rnx_thread_pool_parallel_closure" || n == "__rnx_any_box" || n == "__rnx_any_unbox" || n == "__rnx_any_unbox_heap" || n == "__rnx_any_retain" || n == "__rnx_any_release_box" || n == "__rnx_any_to_str" || n == "__rnx_eq_any" || n == "__rnx_eq_any_str" || n == "__rnx_any_tag" || n == "__rnx_obj_class" || n == "__rnx_error_set" || n == "__rnx_error_take" || n == "__rnx_error_class" || n == "__rnx_error_unbox" || n == "__rnx_error_str" || n == "__rnx_error_release" || n == "__rnx_note_type" || n == "__rnx_type_name" || n == "__rnx_typeof_any" || n == "__rnx_array_slice");
                    if !supported {
                        return Err(match target {
                            CallTarget::Dyn { method, .. } => Diagnostic::new(
                                Code::E108,
                                format!("method `{method}` on a value of unknown type is not supported in native builds [in {}]", f.name),
                            )
                            .with_span(*span)
                            .with_hint("annotate the receiver type, e.g. `let hs: Array<Thread> = []`, so the call resolves statically"),
                            _ => bad("indirect call"),
                        });
                    }
                }
                _ => return Err(bad("unsupported instr")),
            }
        }
        match &b.term {
            Terminator::Ret(_) | Terminator::Br(_) | Terminator::BrIf { .. } => {}
            Terminator::Throw { src, catch, .. } => {
                if catch.is_some() {
                    match f.locals.get(*src as usize) {
                        Some(LirType::Str) | Some(LirType::Error) | Some(LirType::I64) | Some(LirType::F64(_)) | Some(LirType::Bool) | Some(LirType::Obj(_)) => {}
                        _ => return Err(bad("throw payload")),
                    }
                }
            }
            Terminator::BrErr { .. } => {}
            Terminator::Switch { cases, .. } => {
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

fn declare_statics(module: &mut JITModule, lir: &LirModule) -> Result<BTreeMap<String, DataId>, Diagnostic> {
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
    for e in &lir.enums {
        for v in &e.variants {
            texts.insert(lir::instr::pretty_variant_desc(v));
        }
    }
    let mut out = BTreeMap::new();
    for (n, text) in texts.into_iter().enumerate() {
        let bytes = text.as_bytes();
        let mut contents = Vec::with_capacity(32 + bytes.len() + 1);
        contents.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        contents.extend_from_slice(&if text.is_ascii() { 1u32 } else { 2u32 }.to_le_bytes());
        contents.extend_from_slice(&0u64.to_le_bytes());
        contents.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        contents.extend_from_slice(&(text.chars().count() as u64 + 1).to_le_bytes());
        contents.extend_from_slice(bytes);
        contents.push(0);
        let id: DataId = module
            .declare_data(&format!("rnx_str_{n}"), Linkage::Local, false, false)
            .map_err(|e| Diagnostic::new(Code::E108, format!("declare static str: {e}")))?;
        let mut desc = DataDescription::new();
        desc.init = cranelift_module::Init::Bytes { contents: contents.into() };
        desc.align = Some(8);
        module
            .define_data(id, &desc)
            .map_err(|e| Diagnostic::new(Code::E108, format!("define static str: {e}")))?;
        out.insert(text, id);
    }
    Ok(out)
}

fn stack_locals(f: &LirFunction) -> BTreeSet<Local> {
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

pub(super) fn lower_fn(
    lir: &LirModule,
    f_raw: &LirFunction,
    ids: &BTreeMap<String, FuncId>,
    foreign: &BTreeMap<(String, String), FuncId>,
    rt: &RtIds,
    hot: Option<HotCtx<'_>>,
    bytes_fns: &BTreeMap<String, FuncId>,
    statics: &BTreeMap<String, DataId>,
    module: &mut JITModule,
    func: &mut ClFunction,
    ctx: &mut FunctionBuilderContext,
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
    let mut b = FunctionBuilder::new(func, ctx);
    let mut blocks: Vec<cranelift_codegen::ir::Block> = Vec::with_capacity(f.blocks.len());
    for _ in &f.blocks {
        blocks.push(b.create_block());
    }
    let mut vars = Vec::with_capacity(f.locals.len());
    for (n, t) in f.locals.iter().enumerate() {
        let ty = if n >= f.params.len() {
            match t {
                LirType::Vec4f => types::F32X4,
                LirType::Vec4i => types::I32X4,
                _ => types::I64,
            }
        } else {
            match f.params.get(n) {
                Some(LirType::Vec4f) => types::F32X4,
                Some(LirType::Vec4i) => types::I32X4,
                _ => types::I64,
            }
        };
        vars.push(b.declare_var(ty));
    }
    let defer_base = b.declare_var(types::I64);
    b.append_block_params_for_function_params(blocks[0]);
    let mut lower = FnLower {
        lir,
        ids,
        foreign,
        hot_sigs: hot.as_ref().map(|h| h.sigs),
        hot_slots: hot.as_ref().map(|h| h.slots),
        hot_base: hot.map(|h| h.table_base).unwrap_or(0),
        alloc: rt.alloc,
        release: rt.release,
        genref_create: rt.genref_create,
        genref_get: rt.genref_get,
        genref_invalidate: rt.genref_invalidate,
        retain: rt.retain,
        concat: rt.concat,
        streq: rt.streq,
        strcmp: rt.strcmp,
        print_val: rt.print_val,
        print_str: rt.print_str,
        int_to_str: rt.int_to_str,
        float_to_str: rt.float_to_str,
        bool_to_str: rt.bool_to_str,
        any_to_str: rt.any_to_str,
        eq_any_str: rt.eq_any_str,
        eq_any: rt.eq_any,
        release_str: rt.release_str,
        array_new: rt.array_new,
        array_push: rt.array_push,
        array_get: rt.array_get,
        array_set: rt.array_set,
        array_get_unchecked: rt.array_get_unchecked,
        array_set_unchecked: rt.array_set_unchecked,
        array_len: rt.array_len,
        release_array: rt.release_array,
        thread_spawn: rt.thread_spawn,
        thread_join: rt.thread_join,
        clock_mono: rt.clock_mono,
        crypto_random: rt.crypto_random,
        prng_seed: rt.prng_seed,
        prng_next: rt.prng_next,
        file_open: rt.file_open,
        file_close: rt.file_close,
        file_read: rt.file_read,
        file_write: rt.file_write,
        file_flush: rt.file_flush,
        file_seek: rt.file_seek,
        file_tell: rt.file_tell,
        file_from_handle: rt.file_from_handle,
        io_is_tty: rt.io_is_tty,
        io_winsize: rt.io_winsize,
        io_set_raw: rt.io_set_raw,
        path_exists: rt.path_exists,
        path_remove: rt.path_remove,
        test_assert: rt.test_assert,
        test_check: rt.test_check,
        env_count: rt.env_count,
        env_args_get: rt.env_args_get,
        env_get: rt.env_get,
        env_set: rt.env_set,
        env_cwd: rt.env_cwd,
        host_version: rt.host_version,
        env_exit: rt.env_exit,
        net_connect: rt.net_connect,
        net_take_error: rt.net_take_error,
        net_connect_wait: rt.net_connect_wait,
        net_recv_wait: rt.net_recv_wait,
        net_send_wait: rt.net_send_wait,
        net_recv_get: rt.net_recv_get,
        net_error_text: rt.net_error_text,
        net_close: rt.net_close,
        net_listener_bind: rt.net_listener_bind,
        net_listener_port: rt.net_listener_port,
        net_listener_accept_start: rt.net_listener_accept_start,
        net_listener_accept_wait: rt.net_listener_accept_wait,
        net_listener_close: rt.net_listener_close,
        tls_connect_start: rt.tls_connect_start,
        tls_handshake_start: rt.tls_handshake_start,
        tls_handshake_wait: rt.tls_handshake_wait,
        tls_recv_wait: rt.tls_recv_wait,
        tls_send_wait: rt.tls_send_wait,
        tls_recv_get: rt.tls_recv_get,
        tls_error_text: rt.tls_error_text,
        tls_close: rt.tls_close,
        proc_pid: rt.proc_pid,
        proc_remove_env: rt.proc_remove_env,
        proc_all_env_count: rt.proc_all_env_count,
        proc_all_env_get: rt.proc_all_env_get,
        proc_chdir: rt.proc_chdir,
        proc_spawn: rt.proc_spawn,
        proc_run: rt.proc_run,
        proc_pid_of: rt.proc_pid_of,
        proc_write_stdin: rt.proc_write_stdin,
        proc_read_stdout: rt.proc_read_stdout,
        proc_read_stderr: rt.proc_read_stderr,
        proc_close_stdin: rt.proc_close_stdin,
        proc_wait: rt.proc_wait,
        proc_try_wait: rt.proc_try_wait,
        proc_kill: rt.proc_kill,
        proc_take_stdout: rt.proc_take_stdout,
        proc_take_stderr: rt.proc_take_stderr,
        proc_exit_code: rt.proc_exit_code,
        proc_forget: rt.proc_forget,
        os_platform: rt.os_platform,
        os_arch: rt.os_arch,
        os_hostname: rt.os_hostname,
        os_tmpdir: rt.os_tmpdir,
        os_homedir: rt.os_homedir,
        os_cpu_count: rt.os_cpu_count,
        os_uptime: rt.os_uptime,
        map_new: rt.map_new,
        map_set: rt.map_set,
        map_get: rt.map_get,
        map_has: rt.map_has,
        map_delete: rt.map_delete,
        map_len: rt.map_len,
        map_clear: rt.map_clear,
        map_keys: rt.map_keys,
        map_values: rt.map_values,
        sync_atomic_get: rt.sync_atomic_get,
        black_box: rt.black_box,
        sync_atomic_set: rt.sync_atomic_set,
        sync_atomic_fetch_add: rt.sync_atomic_fetch_add,
        sync_atomic_cas: rt.sync_atomic_cas,
        sync_channel_send: rt.sync_channel_send,
        sync_channel_send_str: rt.sync_channel_send_str,
        sync_channel_send_obj: rt.sync_channel_send_obj,
        sync_channel_send_array: rt.sync_channel_send_array,
        sync_channel_recv: rt.sync_channel_recv,
        sync_channel_try_recv: rt.sync_channel_try_recv,
        sync_channel_len: rt.sync_channel_len,
        fs_pool_depth: rt.fs_pool_depth,
        sync_channel_drop: rt.sync_channel_drop,
        debug_live: rt.debug_live,
        mutex_lock: rt.mutex_lock,
        mutex_unlock: rt.mutex_unlock,
        mutex_try_lock: rt.mutex_try_lock,
        rwlock_read_lock: rt.rwlock_read_lock,
        rwlock_read_unlock: rt.rwlock_read_unlock,
        rwlock_write_lock: rt.rwlock_write_lock,
        rwlock_write_unlock: rt.rwlock_write_unlock,
        rwlock_try_read_lock: rt.rwlock_try_read_lock,
        rwlock_try_write_lock: rt.rwlock_try_write_lock,
        condvar_wait: rt.condvar_wait,
        condvar_wait_timeout: rt.condvar_wait_timeout,
        condvar_notify_one: rt.condvar_notify_one,
        condvar_notify_all: rt.condvar_notify_all,
        barrier_wait: rt.barrier_wait,
        pool_init: rt.pool_init,
        pool_parallel_for: rt.pool_parallel_for,
        pool_join: rt.pool_join,
        pool_shutdown: rt.pool_shutdown,
        math_sqrt: rt.math_sqrt,
        math_sin: rt.math_sin,
        math_cos: rt.math_cos,
        math_tan: rt.math_tan,
        math_atan2: rt.math_atan2,
        math_pow: rt.math_pow,
        math_floor: rt.math_floor,
        math_ceil: rt.math_ceil,
        math_round: rt.math_round,
        math_log: rt.math_log,
        string_len: rt.string_len,
        string_trim: rt.string_trim,
        string_concat: rt.string_concat,
        string_split: rt.string_split,
        string_index_of: rt.string_index_of,
        string_index_of_from: rt.string_index_of_from,
        string_char_code_at: rt.string_char_code_at,
        string_from_char_code: rt.string_from_char_code,
        string_slice: rt.string_slice,
        array_pop_fn: rt.array_pop_fn,
        any_box: rt.any_box,
        any_unbox: rt.any_unbox,
        any_unbox_heap: rt.any_unbox_heap,
        any_retain: rt.any_retain,
        any_release_box: rt.any_release_box,
        any_release: rt.any_release,
        heap_track: rt.heap_track,
        closure_new: rt.closure_new,
        closure_set: rt.closure_set,
        closure_release: rt.closure_release,
        panic_str: rt.panic_str,
        fatal_span: rt.fatal_span,
        spawn_closure: rt.spawn_closure,
        join_val: rt.join_val,
        join_err: rt.join_err,
        task_val: rt.task_val,
        task_err: rt.task_err,
        pool_new: rt.pool_new,
        submit_handle: rt.submit_handle,
        submit_closure: rt.submit_closure,
        parallel_closure: rt.parallel_closure,
        defer_push: rt.defer_push,
        defer_pop: rt.defer_pop,
        defer_len: rt.defer_len,
        any_tag: rt.any_tag,
        obj_class: rt.obj_class,
        error_set: rt.error_set,
        error_take: rt.error_take,
        error_class: rt.error_class,
        error_unbox: rt.error_unbox,
        error_str: rt.error_str,
        error_release: rt.error_release,
        note_type: rt.note_type,
        type_name: rt.type_name,
        typeof_any: rt.typeof_any,
        io_pretty: rt.io_pretty,
        note_array_kind: rt.note_array_kind,
        note_fields: rt.note_fields,
        note_enum: rt.note_enum,
        array_slice: rt.array_slice,
        bytes_fns,
        statics,
        module,
        vars,
        ftypes: f.locals.clone(),
        ret_slots: lir::instr::flat_sig(&f.ret).len().max(1),
        stack: stack_locals(f),
        owned: BTreeSet::new(),
        ever_owned: BTreeSet::new(),
        nborrowed: f.params.len(),
        in_enum_dtor: f_raw.name.starts_with("__enum_dtor_"),
        defers: Vec::new(),
        defer_base,
        dom: lir::licm::compute_dominators(f),
        cur_block: 0,
        last_write: BTreeMap::new(),
    };
    for b in &f.blocks {
        lir::instr::walk_instrs(&b.instrs, &mut |ins| {
            if let Instr::Defer { body, .. } = ins {
                if !lower.defers.iter().any(|x| x == body) {
                    lower.defers.push(body.clone());
                }
            }
        });
    }
    for i in lir::opt::rpo_order(f) {
        let block = &f.blocks[i];
        b.switch_to_block(blocks[i]);
        lower.cur_block = i;
        if i == 0 {
            for (p, param) in b.block_params(blocks[0]).to_vec().iter().enumerate() {
                b.def_var(lower.vars[p], *param);
            }
            for (n, t) in f.locals.iter().enumerate() {
                if matches!(t, LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Error | LirType::Any | LirType::Closure)
                    && n >= lower.nborrowed
                {
                    let z = b.ins().iconst(types::I64, 0);
                    b.def_var(lower.vars[n], z);
                }
            }
            if !lower.defers.is_empty() {
                let callee = lower.module.declare_func_in_func(lower.defer_len, &mut b.func);
                let inst = b.ins().call(callee, &[]);
                let base = b.inst_results(inst)[0];
                b.def_var(lower.defer_base, base);
            }
        }
        for ins in &block.instrs {
            lower.instr(&mut b, ins).map_err(|e| {
                Diagnostic::new(Code::E108, format!("{}: {e}", f.name)).with_span(ins.span())
            })?;
        }
        lower.term(&mut b, &block.term, &mut blocks)?;
    }
    for bb in &blocks {
        b.seal_block(*bb);
    }
    let config = module.isa().frontend_config();
    b.finalize(config);
    Ok(())
}

impl FnLower<'_> {
    fn term(
        &mut self,
        b: &mut FunctionBuilder<'_>,
        term: &Terminator,
        blocks: &mut Vec<cranelift_codegen::ir::Block>,
    ) -> Result<(), Diagnostic> {
        match term {
            Terminator::Ret(v) => {
                let ret_locals = v.clone();
                // Releases must not drain tracking: later returns need the full set.
                let saved_owned = self.owned.clone();
                let saved_ever = self.ever_owned.clone();
                let live: Vec<Local> = self.ever_owned.iter().copied().collect();
                for l in live {
                    if ret_locals.contains(&l) {
                        continue;
                    }
                    self.release_any(b, l, None).map_err(|e| {
                        Diagnostic::new(Code::E108, format!("release: {e}"))
                    })?;
                }
                self.owned = saved_owned;
                self.ever_owned = saved_ever;
                let mut rs: Vec<cranelift_codegen::ir::Value> = Vec::with_capacity(ret_locals.len().max(1));
                if ret_locals.is_empty() {
                    rs.push(b.ins().iconst(types::I64, 0));
                } else {
                    for l in &ret_locals {
                        rs.push(self.val(b, *l));
                    }
                }
                b.ins().return_(&rs);
            }
            Terminator::Br(bb) => {
                b.ins().jump(blocks[*bb], &[]);
            }
            Terminator::Throw { src, catch, .. } => {
                let st = self.ftypes.get(*src as usize).cloned().unwrap_or(LirType::Any);
                let payload = match st {
                    LirType::Str | LirType::Error => {
                        let v = self.val(b, *src);
                        let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(callee, &[v]);
                        v
                    }
                    LirType::Obj(_) => {
                        let v = self.val(b, *src);
                        let callee = self.module.declare_func_in_func(self.retain, &mut b.func);
                        b.ins().call(callee, &[v]);
                        let one = b.ins().iconst(types::I64, 1);
                        b.ins().bor(v, one)
                    }
                    LirType::I64 => {
                        let callee = self.module.declare_func_in_func(self.int_to_str, &mut b.func);
                        let v = self.val(b, *src);
                        let inst = b.ins().call(callee, &[v]);
                        b.inst_results(inst)[0]
                    }
                    LirType::F64(_) => {
                        let callee = self.module.declare_func_in_func(self.float_to_str, &mut b.func);
                        let v = self.val(b, *src);
                        let inst = b.ins().call(callee, &[v]);
                        b.inst_results(inst)[0]
                    }
                    LirType::Bool => {
                        let callee = self.module.declare_func_in_func(self.bool_to_str, &mut b.func);
                        let v = self.val(b, *src);
                        let inst = b.ins().call(callee, &[v]);
                        b.inst_results(inst)[0]
                    }
                    _ => {
                        return Err(Diagnostic::new(Code::E108, "jit subset: throw payload"));
                    }
                };
                if let Some((bb, bind, _depth)) = catch {
                    self.set(b, *bind, payload);
                    if !self.stack.contains(bind) {
                        self.own(*bind);
                    }
                    b.ins().jump(blocks[*bb], &[]);
                    return Ok(());
                }
                let callee = self.module.declare_func_in_func(self.error_set, &mut b.func);
                b.ins().call(callee, &[payload]);
                let mut rs = Vec::with_capacity(self.ret_slots);
                for _ in 0..self.ret_slots {
                    rs.push(b.ins().iconst(types::I64, 0));
                }
                b.ins().return_(&rs);
            }
            Terminator::BrErr { err: _, catch_bb, catch_bind, next_bb, depth, .. } => {
                self.emit_run_defers(b, *depth).map_err(|e| {
                    Diagnostic::new(Code::E108, format!("defers: {e}"))
                })?;
                let callee = self.module.declare_func_in_func(self.error_take, &mut b.func);
                let inst = b.ins().call(callee, &[]);
                let w = b.inst_results(inst)[0];
                self.set(b, *catch_bind, w);
                if !self.stack.contains(catch_bind) {
                    self.own(*catch_bind);
                }
                let is_err = b.ins().icmp_imm_s(
                    cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                    w,
                    0,
                );
                b.ins().brif(is_err, blocks[*catch_bb], &[], blocks[*next_bb], &[]);
            }
            Terminator::BrIf { cond, then_bb, else_bb, .. } => {
                let c = self.val(b, *cond);
                let t = b.ins().icmp_imm_s(
                    cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                    c,
                    0,
                );
                b.ins().brif(t, blocks[*then_bb], &[], blocks[*else_bb], &[]);
            }
            Terminator::Switch { scrut, cases, default, .. } => {
                let is_enum = matches!(self.ftypes.get(*scrut as usize), Some(LirType::Enum(_)));
                enum Arm {
                    Int(i64),
                    IsCheck { source: Local, check: lir::instr::IsDecision },
                }
                let mut pairs: Vec<(Arm, usize)> = Vec::with_capacity(cases.len());
                for (pat, bb) in cases.iter() {
                    match pat {
                        SwitchPat::Int(n) => pairs.push((Arm::Int(*n), *bb)),
                        SwitchPat::Enum { variant, .. } => pairs.push((Arm::Int(*variant as i64), *bb)),
                        SwitchPat::Is { source, check, .. } => {
                            use lir::instr::IsDecision;
                            match check {
                                IsDecision::Const(false) => {}
                                _ => pairs.push((Arm::IsCheck { source: *source, check: *check }, *bb)),
                            }
                        }
                        _ => {
                            return Err(Diagnostic::new(Code::E108, "jit subset: switch pattern"));
                        }
                    }
                }
                if pairs.is_empty() {
                    b.ins().jump(blocks[*default], &[]);
                    return Ok(());
                }
                let mut nexts: Vec<cranelift_codegen::ir::Block> = Vec::with_capacity(pairs.len());
                for _ in &pairs {
                    nexts.push(b.create_block());
                }
                for (i, (arm, bb)) in pairs.iter().enumerate() {
                    if i > 0 {
                        b.switch_to_block(nexts[i - 1]);
                    }
                    let t = match arm {
                        Arm::Int(n) => {
                            let sv = self.val(b, *scrut);
                            let disc = if is_enum {
                                b.ins().load(types::I64, MemFlagsData::trusted(), sv, 16)
                            } else {
                                sv
                            };
                            b.ins().icmp_imm_s(
                                cranelift_codegen::ir::condcodes::IntCC::Equal,
                                disc,
                                *n,
                            )
                        }
                        Arm::IsCheck { source, check } => {
                            use lir::instr::IsDecision;
                            match check {
                                IsDecision::Const(false) => {
                                    return Err(Diagnostic::new(Code::E108, "jit subset: surviving Const(false) is-check"));
                                }
                                IsDecision::Const(true) => {
                                    let sv = self.val(b, *scrut);
                                    b.ins().icmp(
                                        cranelift_codegen::ir::condcodes::IntCC::Equal,
                                        sv,
                                        sv,
                                    )
                                }
                                IsDecision::Tag(pt) => {
                                    let callee = self.module.declare_func_in_func(self.any_tag, &mut b.func);
                                    let v = self.val(b, *source);
                                    let inst = b.ins().call(callee, &[v]);
                                    let t = b.inst_results(inst)[0];
                                    let k = b.ins().iconst(types::I64, *pt);
                                    b.ins().icmp(
                                        cranelift_codegen::ir::condcodes::IntCC::Equal,
                                        t,
                                        k,
                                    )
                                }
                                IsDecision::Class(ci) => {
                                    let class_fn = if matches!(self.ftypes.get(*source as usize), Some(LirType::Error)) {
                                        self.error_class
                                    } else {
                                        self.obj_class
                                    };
                                    let callee = self.module.declare_func_in_func(class_fn, &mut b.func);
                                    let v = self.val(b, *source);
                                    let inst = b.ins().call(callee, &[v]);
                                    let t = b.inst_results(inst)[0];
                                    let mut acc = None;
                                    for sub in std::iter::once(*ci).chain(lir::instr::subclasses_of(self.lir, *ci)) {
                                        let k = b.ins().iconst(types::I64, sub as i64 + 1);
                                        let eq = b.ins().icmp(
                                            cranelift_codegen::ir::condcodes::IntCC::Equal,
                                            t,
                                            k,
                                        );
                                        acc = Some(match acc {
                                            Some(prev) => b.ins().bor(prev, eq),
                                            None => eq,
                                        });
                                    }
                                    acc.ok_or_else(|| Diagnostic::new(Code::E108, "jit subset: empty class hierarchy in is-check"))?
                                }
                                IsDecision::Iface(ii) => {
                                    let class_fn = if matches!(self.ftypes.get(*source as usize), Some(LirType::Error)) {
                                        self.error_class
                                    } else {
                                        self.obj_class
                                    };
                                    let callee = self.module.declare_func_in_func(class_fn, &mut b.func);
                                    let v = self.val(b, *source);
                                    let inst = b.ins().call(callee, &[v]);
                                    let t = b.inst_results(inst)[0];
                                    let mut acc = None;
                                    for (ci, c) in self.lir.classes.iter().enumerate() {
                                        if !c.ifaces.contains(ii) {
                                            continue;
                                        }
                                        let k = b.ins().iconst(types::I64, ci as i64 + 1);
                                        let eq = b.ins().icmp(
                                            cranelift_codegen::ir::condcodes::IntCC::Equal,
                                            t,
                                            k,
                                        );
                                        acc = Some(match acc {
                                            Some(prev) => b.ins().bor(prev, eq),
                                            None => eq,
                                        });
                                    }
                                    match acc {
                                        Some(v) => v,
                                        None => {
                                            let z = b.ins().iconst(types::I64, 0);
                                            b.ins().icmp(
                                                cranelift_codegen::ir::condcodes::IntCC::Equal,
                                                t,
                                                z,
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    };
                    let else_bb = if i + 1 < pairs.len() { nexts[i] } else { blocks[*default] };
                    b.ins().brif(t, blocks[*bb], &[], else_bb, &[]);
                    blocks.push(nexts[i]);
                }
            }
            _ => {
                return Err(Diagnostic::new(Code::E108, "jit subset: unsupported terminator"));
            }
        }
        Ok(())
    }
}
