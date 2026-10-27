use crate::ast::{ImportClause, ImportDecl, ImportSource, ImportSpecifier, NativeFnSig, Param, Type};
use crate::capabilities::Sink;
use diagnostics::Span;

pub struct RuntimeDecl {
    pub name: &'static str,
    pub module: &'static str,
    pub params: &'static [&'static str],
    pub ret: &'static str,
    pub sinks: &'static [Sink],
}

pub const RUNTIME_MODULES: &[&str] = &[
    "bytes",
    "core",
    "env",
    "fs",
    "io",
    "json",
    "math",
    "net",
    "os",
    "process",
    "random",
    "sync",
    "testing",
    "time",
];

pub fn is_runtime_module(module: &str) -> bool {
    RUNTIME_MODULES.contains(&module)
}

const fn d(
    name: &'static str,
    module: &'static str,
    params: &'static [&'static str],
    ret: &'static str,
    sinks: &'static [Sink],
) -> RuntimeDecl {
    RuntimeDecl { name, module, params, ret, sinks }
}

pub fn lookup(name: &str) -> Option<&'static RuntimeDecl> {
    RUNTIME_TABLE.iter().find(|e| e.name == name)
}

pub fn is_runtime_symbol(name: &str) -> bool {
    lookup(name).is_some()
}

fn manifest_ty(name: &str) -> Type {
    if let Some(inner) = name
        .strip_suffix('>')
        .and_then(|s| s.strip_prefix("Array<"))
    {
        Type {
            path: vec!["Array".to_string()],
            args: vec![manifest_ty(inner)],
            nullable: false,
            fn_sig: None,
            tuple: Vec::new(),
        }
    } else {
        Type::named(name)
    }
}

pub fn import_decl(symbol: &str) -> Option<crate::ast::Spanned<crate::ast::Decl>> {
    let decl = lookup(symbol)?;
    let params = decl
        .params
        .iter()
        .enumerate()
        .map(|(i, t)| Param {
            name: format!("p{i}"),
            span: Span { start: 0, end: 0 },
            ty: Some(manifest_ty(t)),
            default: None,
            promote: false,
        })
        .collect();
    let sig = NativeFnSig {
        name: symbol.to_string(),
        params,
        ret: Some(manifest_ty(decl.ret)),
        alias: None,
    };
    Some(crate::ast::Spanned {
        node: crate::ast::Decl::Import(ImportDecl {
            clause: ImportClause::Named(vec![ImportSpecifier {
                name: symbol.to_string(),
                alias: None,
                is_export: false,
                native_fn: Some(sig),
            }]),
            source: ImportSource::Runtime(decl.module.to_string()),
            source_span: Span { start: 0, end: 0 },
        }),
        span: Span { start: 0, end: 0 },
    })
}

pub fn ensure_import(module: &mut crate::ast::Module, symbol: &str) {
    for decl in &module.decls {
        if let crate::ast::Decl::Import(d) = &decl.node
            && let ImportSource::Runtime(_) = &d.source
            && let ImportClause::Named(specs) = &d.clause
            && specs.iter().any(|s| {
                s.native_fn.as_ref().is_some_and(|sig| {
                    sig.name == symbol || sig.alias.as_deref() == Some(symbol)
                })
            })
        {
            return;
        }
    }
    if let Some(import) = import_decl(symbol) {
        module.decls.push(import);
    }
}

pub fn owner_hint(module: &str) -> String {
    if module == "core" {
        "the owning `@std/*` wrapper".to_string()
    } else {
        format!("the owning `@std/{module}` wrapper")
    }
}

const RUNTIME_TABLE: &[RuntimeDecl] = &[
    d("__rnx_any_box", "core", &["Int", "Any"], "Any", &[]),
    d("__rnx_any_release_box", "core", &["Any"], "Void", &[]),
    d("__rnx_any_retain", "core", &["Any"], "Void", &[]),
    d("__rnx_any_tag", "core", &["Any"], "Int", &[]),
    d("__rnx_any_to_str", "core", &["Any"], "String", &[]),
    d("__rnx_any_unbox", "core", &["Any"], "Any", &[]),
    d("__rnx_any_unbox_heap", "core", &["Any"], "Any", &[]),
    d("__rnx_array_len", "core", &["Array"], "Int", &[]),
    d("__rnx_array_pop", "core", &["Array", "Any"], "Any", &[]),
    d("__rnx_array_slice", "core", &["Array", "Int", "Int", "Int", "Int", "Int"], "Array", &[]),
    d("__rnx_barrier_wait", "sync", &["Int", "Int"], "Bool", &[]),
    d("__rnx_black_box", "core", &["Any"], "Any", &[]),
    d("__rnx_bool_to_str", "core", &["Bool"], "String", &[]),
    d("__rnx_bytes_alloc", "bytes", &["Int"], "Int", &[]),
    d("__rnx_bytes_cap", "bytes", &["Int"], "Int", &[]),
    d("__rnx_bytes_copy_within", "bytes", &["Int", "Int", "Int", "Int"], "Void", &[]),
    d("__rnx_bytes_data", "bytes", &["Int"], "Int", &[]),
    d("__rnx_bytes_free", "core", &["Int"], "Void", &[]),
    d("__rnx_bytes_len", "bytes", &["Int"], "Int", &[]),
    d("__rnx_bytes_read_f32be", "bytes", &["Int", "Int"], "Float", &[]),
    d("__rnx_bytes_read_f32le", "bytes", &["Int", "Int"], "Float", &[]),
    d("__rnx_bytes_read_f64be", "bytes", &["Int", "Int"], "Float", &[]),
    d("__rnx_bytes_read_f64le", "bytes", &["Int", "Int"], "Float", &[]),
    d("__rnx_bytes_read_i16be", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_i16le", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_i32be", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_i32le", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_i64be", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_i64le", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_i8", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_string", "bytes", &["Int", "Int", "Int"], "String", &[]),
    d("__rnx_bytes_read_u16be", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_u16le", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_u32be", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_u32le", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_read_u8", "bytes", &["Int", "Int"], "Int", &[]),
    d("__rnx_bytes_write_f32be", "bytes", &["Int", "Int", "Float"], "Void", &[]),
    d("__rnx_bytes_write_f32le", "bytes", &["Int", "Int", "Float"], "Void", &[]),
    d("__rnx_bytes_write_f64be", "bytes", &["Int", "Int", "Float"], "Void", &[]),
    d("__rnx_bytes_write_f64le", "bytes", &["Int", "Int", "Float"], "Void", &[]),
    d("__rnx_bytes_write_string", "bytes", &["Int", "Int", "String"], "Int", &[]),
    d("__rnx_bytes_write_u16be", "bytes", &["Int", "Int", "Int"], "Void", &[]),
    d("__rnx_bytes_write_u16le", "bytes", &["Int", "Int", "Int"], "Void", &[]),
    d("__rnx_bytes_write_u32be", "bytes", &["Int", "Int", "Int"], "Void", &[]),
    d("__rnx_bytes_write_u32le", "bytes", &["Int", "Int", "Int"], "Void", &[]),
    d("__rnx_bytes_write_u64be", "bytes", &["Int", "Int", "Int"], "Void", &[]),
    d("__rnx_bytes_write_u64le", "bytes", &["Int", "Int", "Int"], "Void", &[]),
    d("__rnx_bytes_write_u8", "bytes", &["Int", "Int", "Int"], "Void", &[]),
    d("__rnx_clock_mono", "time", &[], "Int", &[]),
    d("__rnx_condvar_notify_all", "sync", &["Int"], "Void", &[]),
    d("__rnx_condvar_notify_one", "sync", &["Int"], "Void", &[]),
    d("__rnx_condvar_wait", "sync", &["Int", "Int"], "Void", &[]),
    d("__rnx_condvar_wait_timeout", "sync", &["Int", "Int", "Int"], "Bool", &[]),
    d("__rnx_crypto_random_u64", "random", &[], "Int", &[]),
    d("__rnx_debug_live_count", "testing", &[], "Int", &[]),
    d("__rnx_dns_lookup_error", "net", &["Int"], "String", &[]),
    d("__rnx_dns_lookup_get", "net", &["Int"], "String", &[]),
    d("__rnx_dns_lookup_start", "net", &["String"], "Int", &[]),
    d("__rnx_dns_lookup_wait", "net", &["Int"], "Int", &[]),
    d("__rnx_env_args_count", "env", &[], "Int", &[]),
    d("__rnx_env_args_get", "env", &["Int"], "String", &[]),
    d("__rnx_env_cwd", "env", &[], "String", &[]),
    d("__rnx_env_exit", "env", &["Int"], "Void", &[]),
    d("__rnx_env_get", "env", &["String"], "String", &[Sink::EnvReadArg(0)]),
    d("__rnx_env_set", "env", &["String", "String"], "Void", &[]),
    d("__rnx_eq_any", "core", &["Any", "Any"], "Bool", &[]),
    d("__rnx_eq_any_str", "core", &["Any", "String"], "Bool", &[]),
    d("__rnx_error_class", "core", &["Any"], "Int", &[]),
    d("__rnx_error_release", "core", &["Int"], "Void", &[]),
    d("__rnx_error_set", "core", &["Int"], "Void", &[]),
    d("__rnx_error_str", "core", &["Any"], "String", &[]),
    d("__rnx_error_take", "core", &[], "Int", &[]),
    d("__rnx_error_unbox", "core", &["Any"], "Any", &[]),
    d("__rnx_file_close", "fs", &["Int"], "Void", &[]),
    d("__rnx_file_flush", "fs", &["Int"], "Void", &[]),
    d("__rnx_file_from_handle", "fs", &["Int", "Int", "Int"], "Int", &[Sink::TermReadWrite]),
    d("__rnx_file_open", "fs", &["String", "Int"], "Int", &[Sink::FsOpen]),
    d(
        "__rnx_file_read_bytes",
        "fs",
        &["Int", "Int", "Int", "Int"],
        "Int",
        &[Sink::FsReadArg(0)],
    ),
    d("__rnx_file_read_text", "fs", &["Int"], "String", &[Sink::FsReadArg(0)]),
    d("__rnx_file_read_text_err", "fs", &["Int"], "String", &[Sink::FsReadArg(0)]),
    d("__rnx_file_seek", "fs", &["Int", "Int"], "Int", &[]),
    d("__rnx_file_tell", "fs", &["Int"], "Int", &[]),
    d(
        "__rnx_file_write_bytes",
        "fs",
        &["Int", "Int", "Int", "Int"],
        "Int",
        &[Sink::FsWriteArg(0)],
    ),
    d("__rnx_file_write_text", "fs", &["Int", "String"], "Int", &[Sink::FsWriteArg(0)]),
    d("__rnx_float_fma", "math", &["Float", "Float", "Float"], "Float", &[]),
    d("__rnx_float_from_bits", "math", &["Int"], "Float", &[]),
    d("__rnx_float_nan", "math", &[], "Float", &[]),
    d("__rnx_float_to_bits", "math", &["Float"], "Int", &[]),
    d("__rnx_float_to_str", "core", &["Float"], "String", &[]),
    d("__rnx_fs_chmod_err", "fs", &["String", "Int"], "String", &[Sink::FsWriteArg(0)]),
    d(
        "__rnx_fs_copy_err",
        "fs",
        &["String", "String", "Int"],
        "String",
        &[Sink::FsReadArg(0), Sink::FsWriteArg(1)],
    ),
    d("__rnx_fs_exists", "fs", &["String"], "Bool", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_fsync_err", "fs", &["String"], "String", &[Sink::FsWriteArg(0)]),
    d("__rnx_fs_glob", "fs", &["String"], "Array<String>", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_glob_err", "fs", &["String"], "String", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_is_dir", "fs", &["String"], "Bool", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_is_file", "fs", &["String"], "Bool", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_mkdir_err", "fs", &["String", "Int"], "String", &[Sink::FsWriteArg(0)]),
    d("__rnx_fs_mmap", "fs", &["String", "Int"], "Int", &[Sink::FsOpen]),
    d("__rnx_fs_mmap_addr", "fs", &["Int"], "Int", &[]),
    d("__rnx_fs_mmap_anon", "fs", &["Int"], "Int", &[]),
    d("__rnx_fs_mmap_anon_err", "fs", &["Int"], "String", &[]),
    d("__rnx_fs_mmap_close", "fs", &["Int"], "Void", &[]),
    d("__rnx_fs_mmap_err", "fs", &["String", "Int"], "String", &[Sink::FsOpen]),
    d("__rnx_fs_mmap_flush", "fs", &["Int"], "Void", &[]),
    d("__rnx_fs_mmap_len", "fs", &["Int"], "Int", &[]),
    d(
        "__rnx_fs_move_err",
        "fs",
        &["String", "String"],
        "String",
        &[Sink::FsReadArg(0), Sink::FsWriteArg(1)],
    ),
    d("__rnx_fs_pool_depth", "fs", &["Int"], "Int", &[]),
    d("__rnx_fs_read_bytes", "fs", &["String"], "Int", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_read_bytes_err", "fs", &["String"], "String", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_read_dir", "fs", &["String"], "Array<String>", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_read_dir_err", "fs", &["String"], "String", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_read_link", "fs", &["String"], "String", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_read_link_err", "fs", &["String"], "String", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_read_text", "fs", &["String"], "String", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_read_text_err", "fs", &["String"], "String", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_remove", "fs", &["String"], "Bool", &[Sink::FsWriteArg(0)]),
    d("__rnx_fs_remove_all", "fs", &["String"], "Bool", &[Sink::FsWriteArg(0)]),
    d("__rnx_fs_remove_all_err", "fs", &["String"], "String", &[Sink::FsWriteArg(0)]),
    d("__rnx_fs_remove_err", "fs", &["String"], "String", &[Sink::FsWriteArg(0)]),
    d(
        "__rnx_fs_rename_err",
        "fs",
        &["String", "String"],
        "String",
        &[Sink::FsReadArg(0), Sink::FsWriteArg(1)],
    ),
    d("__rnx_fs_stat", "fs", &["String"], "Array<Int>", &[Sink::FsReadArg(0)]),
    d("__rnx_fs_stat_err", "fs", &["String"], "String", &[Sink::FsReadArg(0)]),
    d(
        "__rnx_fs_symlink_err",
        "fs",
        &["String", "String"],
        "String",
        &[Sink::FsReadArg(0), Sink::FsWriteArg(1)],
    ),
    d("__rnx_fs_truncate_err", "fs", &["String", "Int"], "String", &[Sink::FsWriteArg(0)]),
    d(
        "__rnx_fs_write_bytes",
        "fs",
        &["String", "Int", "Int"],
        "Int",
        &[Sink::FsWriteArg(0)],
    ),
    d(
        "__rnx_fs_write_bytes_err",
        "fs",
        &["String", "Int", "Int"],
        "String",
        &[Sink::FsWriteArg(0)],
    ),
    d("__rnx_fs_write_text", "fs", &["String", "String", "Int"], "Int", &[Sink::FsWriteArg(0)]),
    d(
        "__rnx_fs_write_text_err",
        "fs",
        &["String", "String", "Int"],
        "String",
        &[Sink::FsWriteArg(0)],
    ),
    d("__rnx_gmap_clear", "core", &["Int"], "Void", &[]),
    d("__rnx_gmap_delete", "core", &["Int", "Any"], "Bool", &[]),
    d("__rnx_gmap_free", "core", &["Int"], "Void", &[]),
    d("__rnx_gmap_get", "core", &["Int", "Any"], "Any", &[]),
    d("__rnx_gmap_has", "core", &["Int", "Any"], "Bool", &[]),
    d("__rnx_gmap_keys", "core", &["Int"], "Array<Any>", &[]),
    d("__rnx_gmap_len", "core", &["Int"], "Int", &[]),
    d("__rnx_gmap_new", "core", &[], "Int", &[]),
    d("__rnx_gmap_set", "core", &["Int", "Any", "Any"], "Void", &[]),
    d("__rnx_gmap_values", "core", &["Int"], "Array<Any>", &[]),
    d("__rnx_host_version", "core", &[], "String", &[]),
    d("__rnx_int_to_str", "core", &["Int"], "String", &[]),
    d("__rnx_io_is_tty", "io", &["Int"], "Bool", &[Sink::TermRead]),
    d("__rnx_io_pretty", "io", &["Any", "Int"], "String", &[Sink::TermWrite]),
    d("__rnx_io_set_raw", "io", &["Int", "Int"], "String", &[Sink::TermRaw]),
    d("__rnx_io_winsize", "io", &[], "Array<Int>", &[Sink::TermRead]),
    d("__rnx_json_parse", "json", &["String"], "Any", &[]),
    d("__rnx_json_parse_typed", "json", &["String", "String"], "Array", &[]),
    d("__rnx_json_stringify", "json", &["Any"], "String", &[]),
    d("__rnx_json_stringify_into", "json", &["Any", "Int", "Int"], "Int", &[]),
    d("__rnx_json_unwrap", "json", &["Int"], "Int", &[]),
    d("__rnx_map_clear", "core", &["Int"], "Void", &[]),
    d("__rnx_map_delete", "core", &["Int", "String"], "Bool", &[]),
    d("__rnx_map_get", "core", &["Int", "String"], "Int", &[]),
    d("__rnx_map_has", "core", &["Int", "String"], "Bool", &[]),
    d("__rnx_map_keys", "core", &["Int"], "Array<String>", &[]),
    d("__rnx_map_len", "core", &["Int"], "Int", &[]),
    d("__rnx_map_new", "core", &[], "Int", &[]),
    d("__rnx_map_set", "core", &["Int", "String", "Int"], "Void", &[]),
    d("__rnx_map_values", "core", &["Int"], "Array<Int>", &[]),
    d("__rnx_math_atan2", "math", &["Float", "Float"], "Float", &[]),
    d("__rnx_math_ceil", "math", &["Float"], "Float", &[]),
    d("__rnx_math_cos", "math", &["Float"], "Float", &[]),
    d("__rnx_math_floor", "math", &["Float"], "Float", &[]),
    d("__rnx_math_log", "math", &["Float"], "Float", &[]),
    d("__rnx_math_pow", "math", &["Float", "Float"], "Float", &[]),
    d("__rnx_math_round", "math", &["Float"], "Float", &[]),
    d("__rnx_math_sin", "math", &["Float"], "Float", &[]),
    d("__rnx_math_sqrt", "math", &["Float"], "Float", &[]),
    d("__rnx_math_tan", "math", &["Float"], "Float", &[]),
    d("__rnx_mutex_lock", "sync", &["Int"], "Void", &[]),
    d("__rnx_mutex_try_lock", "sync", &["Int"], "Bool", &[]),
    d("__rnx_mutex_unlock", "sync", &["Int"], "Void", &[]),
    d("__rnx_net_close", "net", &["Int"], "Int", &[]),
    d("__rnx_net_connect_start", "net", &["String", "Int"], "Int", &[]),
    d("__rnx_net_connect_wait", "net", &["Int"], "Int", &[]),
    d("__rnx_net_error_text", "net", &["Int"], "String", &[]),
    d("__rnx_net_listener_accept_start", "net", &["Int"], "Int", &[]),
    d("__rnx_net_listener_accept_wait", "net", &["Int"], "Int", &[]),
    d("__rnx_net_listener_bind", "net", &["String", "Int"], "Int", &[]),
    d("__rnx_net_listener_close", "net", &["Int"], "Int", &[]),
    d("__rnx_net_listener_port", "net", &["Int"], "Int", &[]),
    d("__rnx_net_recv_get", "net", &["Int", "Int"], "Int", &[]),
    d("__rnx_net_recv_or_wait", "net", &["Int", "Int"], "Int", &[]),
    d("__rnx_net_send_or_wait", "net", &["Int", "Int"], "Int", &[]),
    d("__rnx_net_take_error", "net", &["Int"], "Int", &[]),
    d("__rnx_note_type", "core", &["Any"], "Void", &[]),
    d("__rnx_obj_class", "core", &["Any"], "Int", &[]),
    d("__rnx_os_arch", "os", &[], "String", &[]),
    d("__rnx_os_cpu_count", "os", &[], "Int", &[]),
    d("__rnx_os_homedir", "os", &[], "String", &[]),
    d("__rnx_os_hostname", "os", &[], "String", &[]),
    d("__rnx_os_platform", "os", &[], "String", &[]),
    d("__rnx_os_tmpdir", "os", &[], "String", &[]),
    d("__rnx_os_uptime", "os", &[], "Float", &[]),
    d("__rnx_path_exists", "fs", &["String"], "Bool", &[Sink::FsReadArg(0)]),
    d("__rnx_path_remove", "fs", &["String"], "Bool", &[Sink::FsWriteArg(0)]),
    d("__rnx_pool_new", "sync", &["Int"], "Int", &[]),
    d("__rnx_prng_next", "random", &["Array<Int>"], "Int", &[]),
    d("__rnx_prng_seed", "random", &["Array<Int>", "Int"], "Void", &[]),
    d("__rnx_process_all_env_count", "process", &[], "Int", &[]),
    d("__rnx_process_all_env_get", "process", &["Int"], "String", &[Sink::EnvDump]),
    d("__rnx_process_chdir", "process", &["String"], "Int", &[]),
    d("__rnx_process_close_stdin", "process", &["Int"], "Void", &[]),
    d("__rnx_process_exit_code", "process", &["Int"], "Int", &[]),
    d("__rnx_process_forget", "process", &["Int"], "Void", &[]),
    d("__rnx_process_kill", "process", &["Int", "Int"], "Int", &[]),
    d("__rnx_process_pid", "process", &[], "Int", &[]),
    d("__rnx_process_pid_of", "process", &["Int"], "Int", &[]),
    d(
        "__rnx_process_read_stderr",
        "process",
        &["Int", "Int", "Int", "Int"],
        "Int",
        &[],
    ),
    d(
        "__rnx_process_read_stdout",
        "process",
        &["Int", "Int", "Int", "Int"],
        "Int",
        &[],
    ),
    d("__rnx_process_remove_env", "process", &["String"], "Void", &[]),
    d(
        "__rnx_process_run",
        "process",
        &["String", "Array<String>", "String", "Array<String>", "Int", "Int", "Int"],
        "Int",
        &[Sink::SysExecArg(0)],
    ),
    d(
        "__rnx_process_spawn",
        "process",
        &["String", "Array<String>", "String", "Array<String>", "Int", "Int", "Int"],
        "Int",
        &[Sink::SysExecArg(0)],
    ),
    d("__rnx_process_take_stderr", "process", &["Int"], "Int", &[]),
    d("__rnx_process_take_stdout", "process", &["Int"], "Int", &[]),
    d("__rnx_process_try_wait", "process", &["Int"], "Int", &[]),
    d("__rnx_process_wait", "process", &["Int"], "Int", &[]),
    d(
        "__rnx_process_write_stdin",
        "process",
        &["Int", "Int", "Int", "Int"],
        "Int",
        &[],
    ),
    d("__rnx_rwlock_read_lock", "sync", &["Int"], "Void", &[]),
    d("__rnx_rwlock_read_unlock", "sync", &["Int"], "Void", &[]),
    d("__rnx_rwlock_try_read_lock", "sync", &["Int"], "Bool", &[]),
    d("__rnx_rwlock_try_write_lock", "sync", &["Int"], "Bool", &[]),
    d("__rnx_rwlock_write_lock", "sync", &["Int"], "Void", &[]),
    d("__rnx_rwlock_write_unlock", "sync", &["Int"], "Void", &[]),
    d("__rnx_string_char_code_at", "core", &["String", "Int"], "Int", &[]),
    d("__rnx_string_concat", "core", &["String", "String"], "String", &[]),
    d("__rnx_string_from_char_code", "core", &["Int"], "String", &[]),
    d("__rnx_string_index_of", "core", &["String", "String"], "Int", &[]),
    d(
        "__rnx_string_index_of_from",
        "core",
        &["String", "String", "Int"],
        "Int",
        &[],
    ),
    d("__rnx_string_len", "core", &["String"], "Int", &[]),
    d("__rnx_string_slice", "core", &["String", "Int", "Int"], "String", &[]),
    d("__rnx_string_split", "core", &["String", "String"], "Array<String>", &[]),
    d("__rnx_string_trim", "core", &["String"], "String", &[]),
    d("__rnx_sync_atomic_cas", "sync", &["Int", "Int", "Int"], "Bool", &[]),
    d("__rnx_sync_atomic_fetch_add", "sync", &["Int", "Int"], "Int", &[]),
    d("__rnx_sync_atomic_get", "sync", &["Int"], "Int", &[]),
    d("__rnx_sync_atomic_set", "sync", &["Int", "Int"], "Void", &[]),
    d("__rnx_sync_channel_drop", "sync", &["Int"], "Void", &[]),
    d("__rnx_sync_channel_len", "sync", &["Int"], "Int", &[]),
    d("__rnx_sync_channel_recv", "sync", &["Int"], "Any", &[]),
    d("__rnx_sync_channel_send", "sync", &["Int", "Any"], "Void", &[]),
    d("__rnx_sync_channel_send_array", "sync", &["Int", "Array"], "Void", &[]),
    d("__rnx_sync_channel_send_obj", "sync", &["Int", "Any"], "Void", &[]),
    d("__rnx_sync_channel_send_str", "sync", &["Int", "String"], "Void", &[]),
    d("__rnx_sync_channel_try_recv", "sync", &["Int"], "Int", &[]),
    d("__rnx_thread_join_err", "sync", &["Int"], "String", &[]),
    d("__rnx_thread_join_val", "sync", &["Int"], "Any", &[]),
    d("__rnx_thread_pool_parallel_closure", "sync", &["Int", "Int", "Int", "Int", "Int"], "Void", &[]),
    d(
        "__rnx_thread_pool_submit_closure",
        "sync",
        &["Int", "Int", "Int", "Int", "Int"],
        "Int",
        &[],
    ),
    d(
        "__rnx_thread_pool_submit_handle",
        "sync",
        &["Int", "Int", "Int", "Int", "Int"],
        "Int",
        &[],
    ),
    d("__rnx_thread_spawn_closure", "sync", &["Int", "Int"], "Int", &[]),
    d("__rnx_task_await_err", "sync", &["Int"], "String", &[]),
    d("__rnx_task_await_val", "sync", &["Int"], "Any", &[]),
    d("__rnx_tls_close", "net", &["Int"], "Int", &[]),
    d("__rnx_tls_connect_start", "net", &["Int", "String"], "Int", &[]),
    d("__rnx_tls_error_text", "net", &["Int"], "String", &[]),
    d("__rnx_tls_handshake_start", "net", &["Int"], "Int", &[]),
    d("__rnx_tls_handshake_wait", "net", &["Int"], "Int", &[]),
    d("__rnx_tls_recv_get", "net", &["Int", "Int"], "Int", &[]),
    d("__rnx_tls_recv_or_wait", "net", &["Int", "Int"], "Int", &[]),
    d("__rnx_tls_send_or_wait", "net", &["Int", "Int"], "Int", &[]),
    d("__rnx_type_name", "core", &["Int"], "String", &[]),
    d("__rnx_typeof_any", "core", &["Any"], "String", &[]),
];
