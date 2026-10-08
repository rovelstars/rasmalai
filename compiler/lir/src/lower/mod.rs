use crate::instr::*;
use diagnostics::{Code, Diagnostic, Span};
use frontend::ast as A;
use std::collections::{BTreeMap, BTreeSet};

mod stmt;
mod expr;
mod calls;
mod views;
mod classes;
mod traits;
mod signatures;
mod builder;
mod closure;
mod closures;
mod parallel;

pub use parallel::lower_parallel;
use closure::*;

pub fn lower(m: &A::Module) -> Result<Module, Diagnostic> {
    let mut l = Lower {
        out: Module::default(),
        defaults: BTreeMap::new(),
        param_info: BTreeMap::new(),
        param_tys: BTreeMap::new(),
        field_opt: BTreeMap::new(),
        ret_tys: BTreeMap::new(),
        nub_fields: BTreeSet::new(),
        generic_fns: BTreeMap::new(),
        extensions: BTreeMap::new(),
        ext_self: BTreeMap::new(),
        mod_consts: BTreeMap::new(),
        diags: Vec::new(),
    };
    let expanded = l.expand_traits(m);
    let expanded = l.expand_inheritance(&expanded);
    l.classes(&expanded)?;
    l.link_parents(&expanded)?;
    l.signatures(&expanded)?;
    l.collect_foreign(&expanded)?;
    l.collect_consts(&expanded)?;
    l.collect_namespaces();
    l.link_members(&expanded)?;
    l.bodies(&expanded)?;
    l.synthesize_dtors();
    if let Some(d) = l.diags.into_iter().next() {
        return Err(d);
    }
    Ok(l.out)
}

struct Lower {
    out: Module,
    defaults: BTreeMap<String, Vec<(String, A::Spanned<A::Expr>)>>,
    param_info: BTreeMap<String, Vec<(String, Option<A::Spanned<A::Expr>>)>>,
    param_tys: BTreeMap<String, Vec<(String, A::Type)>>,
    field_opt: BTreeMap<String, LirType>,
    ret_tys: BTreeMap<String, A::Type>,
    nub_fields: BTreeSet<String>,
    generic_fns: BTreeMap<String, GenericSig>,
    extensions: BTreeMap<(String, String), usize>,
    ext_self: BTreeMap<String, A::Type>,
    mod_consts: BTreeMap<String, (Lit, LirType, Simple)>,
    diags: Vec<Diagnostic>,
}

#[derive(Clone)]
struct GenericSig {
    tparams: Vec<String>,
    params: Vec<(String, A::Type)>,
    ret: A::Type,
}

#[derive(Clone, Copy, PartialEq)]
enum Simple {
    Int,
    Strict,
    Fast,
    Other,
}

fn type_resolved(module: &Module, ty: &LirType) -> bool {
    match ty {
        LirType::Obj(n) => module.class_index.contains_key(n),
        LirType::Array(inner) => type_resolved(module, inner),
        LirType::Tuple(items) => items.iter().all(|t| type_resolved(module, t)),
        LirType::GenRef(inner) => inner.as_ref().map(|n| module.class_index.contains_key(n)).unwrap_or(true),
        _ => true,
    }
}

fn array_elem_ty(module: &Module, t: &A::Type) -> LirType {
    match t.args.first() {
        Some(a) if a.fn_sig.is_none() && !a.path.is_empty() => {
            if let Some(first) = a.path.first() {
                if short_name(first) == "Array" {
                    return LirType::Array(Box::new(array_elem_ty(module, a)));
                }
                if let Some(id) = module.enum_index.get(first) {
                    return LirType::Enum(*id);
                }
            }
            map_ty(a)
        }
        _ => LirType::Any,
    }
}

fn short_name(n: &str) -> &str {
    n.rsplit('.').next().unwrap_or(n)
}

fn ns_key_of_ty(ty: &LirType) -> Option<String> {
    match ty {
        LirType::Obj(n) => frontend::modules::parse_ns_class(n).map(|(_, key)| key),
        _ => None,
    }
}
fn nub_ty(t: &A::Type) -> bool {
    if !t.nullable {
        return false;
    }
    if !t.args.is_empty() || t.fn_sig.is_some() || !t.tuple.is_empty() {
        return false;
    }
    matches!(
        t.path.last().map(|s| short_name(s)).as_deref(),
        Some("Int") | Some("Bool") | Some("Float")
    )
}

fn removed_option_hint(n: &str) -> Option<String> {
    match short_name(n) {
        "Option" => Some(
            "`Option` was removed; use a nullable type (`T?`) and `null` for absent values".to_string(),
        ),
        "Some" => Some(
            "`Option.Some` was removed; use the value directly with a nullable type (`T?`)".to_string(),
        ),
        "None" => Some(
            "`Option.None` was removed; use `null` instead".to_string(),
        ),
        _ => None,
    }
}

fn option_arg(t: &A::Type) -> Option<&A::Type> {
    let is_opt = t.path.len() == 1 && matches!(short_name(&t.path[0]), "Result")
        || t.path.join(".") == "std.prelude.Result";
    if is_opt && t.args.len() == 1 {
        Some(&t.args[0])
    } else {
        None
    }
}

// Result tags for cross-thread transport. Values mirror
// runtime::native TAG_* (lir must not depend on runtime): 0 Int, 1 Bool,
// 2 Float, 3 String, 6 Null. Unannotated/opaque types map to Null: only
// the tag travels for untyped workers, so awaiting yields Null.
fn result_tag(ty: &LirType) -> Option<u32> {
    match ty {
        LirType::I64 => Some(0),
        LirType::Bool => Some(1),
        LirType::F64(_) => Some(2),
        LirType::Str => Some(3),
        LirType::Void | LirType::Null | LirType::Any => Some(6),
        _ => None,
    }
}

fn vec_name(n: &str) -> Option<VecKind> {
    if n == "Vec4f" || n.ends_with(".Vec4f") {
        Some(VecKind::F)
    } else if n == "Vec4i" || n.ends_with(".Vec4i") {
        Some(VecKind::I)
    } else {
        None
    }
}

fn lane_ty_name(kind: VecKind) -> &'static str {
    if kind == VecKind::F {
        "Float"
    } else {
        "Int"
    }
}

fn ty_name(ty: &LirType) -> String {
    match ty {
        LirType::F64(_) => "Float".to_string(),
        LirType::I64 => "Int".to_string(),
        LirType::Vec4f => "Vec4f".to_string(),
        LirType::Vec4i => "Vec4i".to_string(),
        _ => "other".to_string(),
    }
}

fn switch_ty_name(ty: &LirType) -> String {
    match ty {
        LirType::I64 => "Int".to_string(),
        LirType::F64(_) => "Float".to_string(),
        LirType::Bool => "Bool".to_string(),
        LirType::Str => "String".to_string(),
        LirType::Null => "Null".to_string(),
        LirType::Any => "Any".to_string(),
        LirType::Array(inner) => format!("Array<{}>", switch_ty_name(inner)),
        LirType::Obj(n) => n.clone(),
        _ => crate::instr::type_key(ty),
    }
}

fn map_ty(t: &A::Type) -> LirType {
    if t.fn_sig.is_some() {
        return LirType::Closure;
    }
    if !t.tuple.is_empty() {
        return LirType::Tuple(t.tuple.iter().map(map_ty).collect());
    }
    if t.path.len() > 1 {
        match t.path.last().map(|s| s.as_str()) {
            Some("Vec4f") => return LirType::Vec4f,
            Some("Vec4i") => return LirType::Vec4i,
            _ => {}
        }
    }
    if let Some(first) = t.path.first() {
        if first == "Vec4f" || first.ends_with(".Vec4f") {
            return LirType::Vec4f;
        }
        if first == "Vec4i" || first.ends_with(".Vec4i") {
            return LirType::Vec4i;
        }
    }
    let base = match t.path.first().map(|s| s.as_str()).map(|h| h.rsplit('.').next().unwrap_or(h)) {
        Some("Int") | Some("Int64") | Some("Int32") | Some("Int16") | Some("Int8")
        | Some("UInt") | Some("UInt64") | Some("UInt32") | Some("UInt16") | Some("UInt8")
        | Some("Short") => LirType::I64,
        Some("Byte") => LirType::I8,
        Some("Float") | Some("Float32") => LirType::F64(FloatKind::Strict),
        Some("FastFloat") | Some("FastFloat32") => LirType::F64(FloatKind::Fast),
        Some("Bool") => LirType::Bool,
        Some("Vec4f") => LirType::Vec4f,
        Some("Vec4i") => LirType::Vec4i,
        Some("String") | Some("Char") => LirType::Str,
        Some("Array") => LirType::Array(Box::new(LirType::Any)),
        Some("Range") => LirType::Range,
        Some("Any") => LirType::Any,        Some("GenRef") => genref_ty(t),
        Some("Pointer") | Some("Address") => {
            let inner = t.args.first().map(map_ty).unwrap_or(LirType::Any);
            LirType::Pointer(Box::new(inner))
        }
        Some("Void") => LirType::Void,
        Some(_) => {
            let last = t.path.last().cloned().unwrap_or_default();
            LirType::Obj(last)
        }
        None => LirType::Any,
    };
    let _ = t.nullable;
    base
}

fn is_type_param(params: &[String], t: &A::Type) -> bool {
    t.fn_sig.is_none() && t.args.is_empty() && t.path.len() == 1 && params.contains(&t.path[0])
}

fn meth_ty(module: &Module, self_class: Option<&str>, fn_tparams: &[String], t: &A::Type) -> LirType {
    if let Some(cn) = self_class {
        if let Some(ci) = module.class_index.get(cn) {
            if is_type_param(&module.classes[*ci].type_params, t) {
                return LirType::Any;
            }
        }
    }
    if is_type_param(fn_tparams, t) {
        return LirType::Any;
    }
    let class_tps: &[String] = match self_class.and_then(|cn| module.class_index.get(cn)) {
        Some(ci) => &module.classes[*ci].type_params,
        None => &[],
    };
    erase_tparams(resolve_ty(module, t), class_tps, fn_tparams)
}

fn erase_tparams(ty: LirType, class_tps: &[String], fn_tps: &[String]) -> LirType {
    match ty {
        LirType::Obj(n) => {
            let short = n.rsplit('.').next().unwrap_or(&n);
            if class_tps.iter().any(|p| p == &n || p == short)
                || fn_tps.iter().any(|p| p == &n || p == short)
            {
                LirType::Any
            } else {
                LirType::Obj(n)
            }
        }
        LirType::Array(inner) => {
            LirType::Array(Box::new(erase_tparams(*inner, class_tps, fn_tps)))
        }
        LirType::Tuple(items) => LirType::Tuple(
            items.into_iter().map(|t| erase_tparams(t, class_tps, fn_tps)).collect(),
        ),
        other => other,
    }
}

fn resolve_ty(module: &Module, t: &A::Type) -> LirType {
    if t.fn_sig.is_some() {
        return LirType::Closure;
    }
    if !t.tuple.is_empty() {
        return LirType::Tuple(t.tuple.iter().map(|x| resolve_ty(module, x)).collect());
    }
    if let Some(first) = t.path.first() {
        if let Some(id) = module.enum_index.get(first) {
            return LirType::Enum(*id);
        }
        if t.path.len() == 1 {
            let want = format!(".{first}");
            let mut hit: Option<usize> = None;
            let mut ambiguous = false;
            for (name, id) in &module.enum_index {
                if name == first || name.ends_with(want.as_str()) {
                    if hit.is_some() {
                        ambiguous = true;
                        break;
                    }
                    hit = Some(*id);
                }
            }
            if !ambiguous {
                if let Some(id) = hit {
                    return LirType::Enum(id);
                }
            }
        } else if let Some(id) = module.enum_index.get(&t.path.join(".")) {
            return LirType::Enum(*id);
        }
        if short_name(first) == "Array" {
            return LirType::Array(Box::new(array_elem_ty(module, t)));
        }
    }
    map_ty(t)
}

fn genref_ty(t: &A::Type) -> LirType {
    match t.args.first() {
        Some(a) if a.fn_sig.is_none() && !a.path.is_empty() => {
            LirType::GenRef(Some(a.path.join(".")))
        }
        _ => LirType::GenRef(None),
    }
}

fn math_arity(n: &str) -> Option<usize> {
    match n {
        "__rnx_math_sqrt" | "__rnx_math_sin" | "__rnx_math_cos" | "__rnx_math_tan"
        | "__rnx_math_floor" | "__rnx_math_ceil" | "__rnx_math_round" | "__rnx_math_log" => Some(1),
        "__rnx_math_atan2" | "__rnx_math_pow" => Some(2),
        _ => None,
    }
}

fn intrin_sig(n: &str) -> Option<(usize, LirType, Simple)> {
    match n {
        "__rnx_crypto_random_u64" => Some((0, LirType::I64, Simple::Int)),
        "__rnx_prng_seed" => Some((2, LirType::Null, Simple::Other)),
        "__rnx_prng_next" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_file_open" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_file_close" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_file_read_text" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_file_read_text_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_file_write_text" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_file_flush" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_file_seek" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_file_tell" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_file_from_handle" => Some((3, LirType::I64, Simple::Int)),
        "__rnx_io_is_tty" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_io_pretty" => Some((2, LirType::Str, Simple::Other)),
        "__rnx_io_winsize" => Some((0, LirType::Array(Box::new(LirType::I64)), Simple::Other)),
        "__rnx_io_set_raw" => Some((2, LirType::Str, Simple::Other)),
        "__rnx_path_exists" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_path_remove" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_fs_exists" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_fs_is_file" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_fs_is_dir" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_fs_stat" => Some((1, LirType::Array(Box::new(LirType::I64)), Simple::Other)),
        "__rnx_fs_stat_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_read_dir" => Some((
            1,
            LirType::Array(Box::new(LirType::Str)),
            Simple::Other,
        )),
        "__rnx_fs_read_dir_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_glob" => Some((
            1,
            LirType::Array(Box::new(LirType::Str)),
            Simple::Other,
        )),
        "__rnx_fs_glob_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_read_link" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_read_link_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_remove" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_fs_remove_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_remove_all" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_fs_remove_all_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_mkdir_err" => Some((2, LirType::Str, Simple::Other)),
        "__rnx_fs_copy_err" => Some((3, LirType::Str, Simple::Other)),
        "__rnx_fs_move_err" => Some((2, LirType::Str, Simple::Other)),
        "__rnx_fs_rename_err" => Some((2, LirType::Str, Simple::Other)),
        "__rnx_fs_truncate_err" => Some((2, LirType::Str, Simple::Other)),
        "__rnx_fs_chmod_err" => Some((2, LirType::Str, Simple::Other)),
        "__rnx_fs_symlink_err" => Some((2, LirType::Str, Simple::Other)),
        "__rnx_fs_fsync_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_read_text" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_read_text_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_write_text" => Some((3, LirType::I64, Simple::Int)),
        "__rnx_fs_write_text_err" => Some((3, LirType::Str, Simple::Other)),
        "__rnx_fs_read_bytes" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_fs_read_bytes_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_write_bytes" => Some((3, LirType::I64, Simple::Int)),
        "__rnx_fs_write_bytes_err" => Some((3, LirType::Str, Simple::Other)),
        "__rnx_fs_mmap" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_fs_mmap_err" => Some((2, LirType::Str, Simple::Other)),
        "__rnx_fs_mmap_anon" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_fs_mmap_anon_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_fs_mmap_addr" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_fs_mmap_len" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_fs_mmap_flush" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_fs_mmap_close" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_fs_pool_depth" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_map_new" => Some((0, LirType::I64, Simple::Int)),
        "__rnx_map_set" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_map_get" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_map_has" => Some((2, LirType::Bool, Simple::Other)),
        "__rnx_map_delete" => Some((2, LirType::Bool, Simple::Other)),
        "__rnx_map_len" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_map_clear" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_map_keys" => Some((1, LirType::Array(Box::new(LirType::Str)), Simple::Other)),
        "__rnx_map_values" => Some((1, LirType::Array(Box::new(LirType::I64)), Simple::Other)),
        "__rnx_sync_atomic_get" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_sync_atomic_set" => Some((2, LirType::Null, Simple::Other)),
        "__rnx_sync_atomic_fetch_add" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_sync_atomic_cas" => Some((3, LirType::Bool, Simple::Other)),
        "__rnx_sync_channel_send" => Some((2, LirType::Null, Simple::Other)),
        "__rnx_sync_channel_send_str" => Some((2, LirType::Null, Simple::Other)),
        "__rnx_sync_channel_send_obj" => Some((2, LirType::Null, Simple::Other)),
        "__rnx_sync_channel_send_array" => Some((2, LirType::Null, Simple::Other)),
        "__rnx_sync_channel_recv" => Some((1, LirType::Any, Simple::Other)),
        "__rnx_sync_channel_try_recv" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_sync_channel_len" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_sync_channel_drop" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_mutex_lock" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_mutex_unlock" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_mutex_try_lock" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_rwlock_read_lock" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_rwlock_read_unlock" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_rwlock_write_lock" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_rwlock_write_unlock" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_rwlock_try_read_lock" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_rwlock_try_write_lock" => Some((1, LirType::Bool, Simple::Other)),
        "__rnx_condvar_wait" => Some((2, LirType::Null, Simple::Other)),        "__rnx_condvar_wait_timeout" => Some((3, LirType::Bool, Simple::Other)),
        "__rnx_condvar_notify_one" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_condvar_notify_all" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_barrier_wait" => Some((2, LirType::Bool, Simple::Other)),
        "__rnx_string_len" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_string_slice" => Some((3, LirType::Str, Simple::Other)),
        "__rnx_string_index_of" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_string_index_of_from" => Some((3, LirType::I64, Simple::Int)),
        "__rnx_string_trim" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_string_concat" => Some((2, LirType::Str, Simple::Other)),
        "__rnx_string_split" => Some((
            2,
            LirType::Array(Box::new(LirType::Str)),
            Simple::Other,
        )),
        "__rnx_string_char_code_at" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_string_from_char_code" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_int_to_str" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_float_to_str" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_bool_to_str" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_bytes_alloc" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_bytes_data" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_bytes_free" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_bytes_len" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_bytes_cap" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_bytes_copy_within" => Some((4, LirType::Null, Simple::Other)),
        "__rnx_bytes_read_u8" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_i8" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_u16le" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_u16be" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_i16le" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_i16be" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_u32le" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_u32be" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_i32le" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_i32be" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_i64le" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_read_i64be" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_bytes_write_u8" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_write_u16le" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_write_u16be" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_write_u32le" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_write_u32be" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_write_u64le" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_write_u64be" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_read_f32le" => Some((2, LirType::F64(FloatKind::Strict), Simple::Strict)),
        "__rnx_bytes_read_f32be" => Some((2, LirType::F64(FloatKind::Strict), Simple::Strict)),
        "__rnx_bytes_read_f64le" => Some((2, LirType::F64(FloatKind::Strict), Simple::Strict)),
        "__rnx_bytes_read_f64be" => Some((2, LirType::F64(FloatKind::Strict), Simple::Strict)),
        "__rnx_bytes_write_f32le" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_write_f32be" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_write_f64le" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_write_f64be" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_bytes_read_string" => Some((3, LirType::Str, Simple::Other)),
        "__rnx_bytes_write_string" => Some((3, LirType::I64, Simple::Int)),
        "__rnx_file_read_bytes" => Some((4, LirType::I64, Simple::Int)),
        "__rnx_file_write_bytes" => Some((4, LirType::I64, Simple::Int)),
        "__rnx_gmap_new" => Some((0, LirType::I64, Simple::Int)),
        "__rnx_gmap_free" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_gmap_set" => Some((3, LirType::Null, Simple::Other)),
        "__rnx_gmap_get" => Some((2, LirType::Any, Simple::Other)),
        "__rnx_gmap_has" => Some((2, LirType::Bool, Simple::Other)),
        "__rnx_gmap_delete" => Some((2, LirType::Bool, Simple::Other)),
        "__rnx_gmap_len" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_gmap_clear" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_gmap_keys" => Some((1, LirType::Array(Box::new(LirType::Any)), Simple::Other)),
        "__rnx_gmap_values" => Some((1, LirType::Array(Box::new(LirType::Any)), Simple::Other)),
        "__rnx_array_len" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_array_pop" => Some((2, LirType::Any, Simple::Other)),
        "__rnx_any_box" => Some((2, LirType::Any, Simple::Other)),
        "__rnx_any_unbox" => Some((1, LirType::Any, Simple::Other)),
        "__rnx_any_unbox_heap" => Some((1, LirType::Any, Simple::Other)),
        "__rnx_any_release_box" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_any_retain" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_json_parse" => Some((1, LirType::Any, Simple::Other)),
        "__rnx_json_parse_typed" => {
            Some((2, LirType::Array(Box::new(LirType::Any)), Simple::Other))
        }
        "__rnx_json_stringify" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_json_stringify_into" => Some((3, LirType::I64, Simple::Int)),
        "__rnx_json_unwrap" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_dns_lookup_start" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_dns_lookup_wait" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_dns_lookup_get" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_dns_lookup_error" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_thread_join_val" => Some((1, LirType::Any, Simple::Other)),
        "__rnx_thread_join_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_task_await_val" => Some((1, LirType::Any, Simple::Other)),
        "__rnx_task_await_err" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_pool_new" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_debug_live_count" => Some((0, LirType::I64, Simple::Int)),
        "__rnx_env_args_count" => Some((0, LirType::I64, Simple::Int)),
        "__rnx_env_args_get" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_env_get" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_env_set" => Some((2, LirType::Null, Simple::Other)),
        "__rnx_env_cwd" => Some((0, LirType::Str, Simple::Other)),
        "__rnx_host_version" => Some((0, LirType::Str, Simple::Other)),
        "__rnx_net_connect_start" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_net_take_error" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_net_connect_wait" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_net_recv_or_wait" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_net_send_or_wait" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_net_recv_get" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_net_error_text" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_net_close" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_net_listener_bind" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_net_listener_port" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_net_listener_accept_start" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_net_listener_accept_wait" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_net_listener_close" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_tls_connect_start" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_tls_handshake_start" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_tls_handshake_wait" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_tls_recv_or_wait" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_tls_send_or_wait" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_tls_recv_get" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_tls_error_text" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_tls_close" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_env_exit" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_process_pid" => Some((0, LirType::I64, Simple::Int)),
        "__rnx_process_remove_env" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_process_all_env_count" => Some((0, LirType::I64, Simple::Int)),
        "__rnx_process_all_env_get" => Some((1, LirType::Str, Simple::Other)),
        "__rnx_process_chdir" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_process_spawn" => Some((7, LirType::I64, Simple::Int)),
        "__rnx_process_run" => Some((7, LirType::I64, Simple::Int)),
        "__rnx_process_pid_of" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_process_write_stdin" => Some((4, LirType::I64, Simple::Int)),
        "__rnx_process_read_stdout" => Some((4, LirType::I64, Simple::Int)),
        "__rnx_process_read_stderr" => Some((4, LirType::I64, Simple::Int)),
        "__rnx_process_close_stdin" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_process_wait" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_process_try_wait" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_process_kill" => Some((2, LirType::I64, Simple::Int)),
        "__rnx_process_take_stdout" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_process_take_stderr" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_process_exit_code" => Some((1, LirType::I64, Simple::Int)),
        "__rnx_process_forget" => Some((1, LirType::Null, Simple::Other)),
        "__rnx_os_platform" => Some((0, LirType::Str, Simple::Other)),
        "__rnx_os_arch" => Some((0, LirType::Str, Simple::Other)),
        "__rnx_os_hostname" => Some((0, LirType::Str, Simple::Other)),
        "__rnx_os_tmpdir" => Some((0, LirType::Str, Simple::Other)),
        "__rnx_os_homedir" => Some((0, LirType::Str, Simple::Other)),
        "__rnx_os_cpu_count" => Some((0, LirType::I64, Simple::Int)),
        "__rnx_os_uptime" => Some((0, LirType::F64(FloatKind::Strict), Simple::Strict)),
        _ => None,
    }
}

fn simple_of(ty: &LirType) -> Simple {
    match ty {
        LirType::I64 | LirType::I8 => Simple::Int,
        LirType::F64(FloatKind::Strict) => Simple::Strict,
        LirType::F64(FloatKind::Fast) => Simple::Fast,
        _ => Simple::Other,
    }
}

#[derive(Clone)]
enum BodySrc {
    Fn(A::FnDecl),
    Init(Vec<A::Param>, A::Block),
    Bare(Vec<A::Param>, A::Block),
    Closure(PendingClosure),
}

#[derive(Clone)]
struct PendingClosure {
    name: String,
    params: Vec<A::Param>,
    param_hints: Vec<LirType>,
    ret: Option<A::Type>,
    throws: bool,
    body: A::FnBody,
    captures: Vec<Capture>,
    decay_this: bool,
    prefix: String,
    enclosing: Option<String>,
}

#[derive(Clone)]
struct Capture {
    name: Option<String>,
    ty: LirType,
    src: Local,
    is_this: bool,
    ret: Option<LirType>,
}

fn shell(
    module: &Module,
    name: &str,
    params: &[A::Param],
    ret: Option<&A::Type>,
    throws: bool,
    is_unsafe: bool,
    self_class: Option<String>,
    fn_tparams: &[String],
) -> Function {
    let mut locals = Vec::new();
    let mut lparams = Vec::new();
    let mut sig_params = Vec::new();
    let method_self = self_class.is_some();
    let self_name = self_class.clone();
    if let Some(cn) = self_class {
        let t = LirType::Obj(cn);
        locals.push(t.clone());
        lparams.push(t.clone());
        sig_params.push(t);
    }
    for p in params {
        let t = p.ty.as_ref().map(|t| meth_ty(module, self_name.as_deref(), fn_tparams, t)).unwrap_or(LirType::Any);
        sig_params.push(t.clone());
        for e in crate::instr::flat_sig(&t) {
            locals.push(e.clone());
            lparams.push(e);
        }
    }
    Function {
        name: name.to_string(),
        params: lparams,
        sig_params,
        ret: ret.map(|t| meth_ty(module, self_name.as_deref(), fn_tparams, t)).unwrap_or(LirType::Any),
        throws,
        is_unsafe,
        method_self,
        is_pub: false,
        is_closure: false,
        locals,
        blocks: vec![],
    }
}

struct LoopCtx {
    break_bb: BlockId,
    cont_bb: BlockId,
    defer_depth: usize,
}

struct CatchCtx {
    catch_bb: Option<BlockId>,
    err_local: Local,
    defer_depth: usize,
}

struct Builder<'a> {
    module: &'a Module,
    defaults: &'a BTreeMap<String, Vec<(String, A::Spanned<A::Expr>)>>,
    param_info: &'a BTreeMap<String, Vec<(String, Option<A::Spanned<A::Expr>>)>>,
    param_tys: &'a BTreeMap<String, Vec<(String, A::Type)>>,
    field_opt: &'a BTreeMap<String, LirType>,
    ret_tys: &'a BTreeMap<String, A::Type>,
    nub_fields: &'a BTreeSet<String>,
    nub: BTreeSet<Local>,
    precise: BTreeMap<Local, LirType>,
    var_targs: BTreeMap<String, Vec<LirType>>,
    targ_log: Vec<(String, Option<Vec<LirType>>)>,
    scope_marks: Vec<usize>,
    generic_fns: &'a BTreeMap<String, GenericSig>,
    extensions: &'a BTreeMap<(String, String), usize>,
    ext_self: &'a BTreeMap<String, A::Type>,
    mod_consts: &'a BTreeMap<String, (Lit, LirType, Simple)>,
    func: Function,
    blocks: Vec<Vec<Instr>>,
    terms: Vec<Option<Terminator>>,
    current: BlockId,
    terminated: bool,
    scopes: Vec<BTreeMap<String, (Local, Simple, LirType)>>,
    self_local: Option<Local>,
    this_alias: Option<Local>,
    throws_fn: bool,
    is_init: bool,
    implicit_super: Option<usize>,
    loop_stack: Vec<LoopCtx>,
    catch_stack: Vec<CatchCtx>,
    fallthrough: Option<(Vec<BlockId>, usize)>,
    defer_count: usize,
    unsafe_depth: usize,
    enclosing: Option<String>,
    fn_tparams: Vec<String>,
    moved: BTreeSet<Local>,
    fresh_arrays: BTreeSet<Local>,
    tuples: BTreeMap<Local, Vec<Local>>,
    records: BTreeMap<Local, Vec<(String, Local)>>,
    ranges: BTreeMap<Local, (Local, Local, Local, Local)>,
    closure_seq: usize,
    closure_prefix: String,
    closure_hints: Vec<Vec<LirType>>,
    closure_ret: BTreeMap<Local, LirType>,
    synth_seq: usize,
    pending: Vec<PendingClosure>,
    patches: Vec<(BlockId, usize, String)>,
    diags: Vec<Diagnostic>,
    failed: bool,
}

fn scope_ty_of(scopes: &[std::collections::BTreeMap<String, (Local, Simple, LirType)>], name: &str) -> Option<LirType> {
    for scope in scopes.iter().rev() {
        if let Some((_, _, t)) = scope.get(name) {
            return Some(t.clone());
        }
    }
    None
}


fn body_can_throw(body: &A::FnBody) -> bool {
    match body {
        A::FnBody::Block(b) => b.stmts.iter().any(|s| stmt_can_throw(&s.node)),
        A::FnBody::Expr(e) => expr_can_throw(&e.node),
    }
}

fn stmt_can_throw(s: &A::Stmt) -> bool {
    match s {
        A::Stmt::Throw(_) => true,
        A::Stmt::Var { value, .. } => expr_can_throw(&value.node),
        A::Stmt::DestructureTuple { value, .. }
        | A::Stmt::DestructureRecord { value, .. }
        | A::Stmt::DestructureArray { value, .. } => expr_can_throw(&value.node),
        A::Stmt::Assign { target, value, .. } => {
            expr_can_throw(&target.node) || expr_can_throw(&value.node)
        }
        A::Stmt::Expr(e) | A::Stmt::Assert(e) => expr_can_throw(&e.node),
        A::Stmt::Return(e) => e.as_ref().is_some_and(|x| expr_can_throw(&x.node)),
        A::Stmt::If { cond, then, .. } => {
            let c = match cond {
                A::IfCond::Expr(e) | A::IfCond::Let { value: e, .. } => expr_can_throw(&e.node),
            };
            c || then.stmts.iter().any(|x| stmt_can_throw(&x.node))
        }
        A::Stmt::While { cond, body } => {
            expr_can_throw(&cond.node) || body.stmts.iter().any(|x| stmt_can_throw(&x.node))
        }
        A::Stmt::DoWhile { body, cond } => {
            body.stmts.iter().any(|x| stmt_can_throw(&x.node)) || expr_can_throw(&cond.node)
        }
        A::Stmt::For { iter, body, .. } => {
            expr_can_throw(&iter.node) || body.stmts.iter().any(|x| stmt_can_throw(&x.node))
        }
        A::Stmt::Switch { scrutinee, cases, default } => {
            let mut out = expr_can_throw(&scrutinee.node);
            for c in cases {
                if let Some(g) = &c.guard {
                    out = out || expr_can_throw(&g.node);
                }
                out = out || c.body.iter().any(|s| stmt_can_throw(&s.node));
            }
            if let Some(d) = default {
                out = out || d.iter().any(|s| stmt_can_throw(&s.node));
            }
            out
        }
        A::Stmt::Defer(b)
        | A::Stmt::UnsafeBlock(b)
        | A::Stmt::Guard { otherwise: b, .. } => {
            b.stmts.iter().any(|x| stmt_can_throw(&x.node))
        }
        A::Stmt::Try { body, .. } => body.stmts.iter().any(|x| stmt_can_throw(&x.node)),
        _ => false,
    }
}

fn expr_can_throw(e: &A::Expr) -> bool {
    match e {
        A::Expr::Propagate(_) => true,
        A::Expr::Binary { lhs, rhs, .. } => {
            expr_can_throw(&lhs.node) || expr_can_throw(&rhs.node)
        }
        A::Expr::Unary { rhs, .. } => expr_can_throw(&rhs.node),
        A::Expr::Postfix { expr, .. } => expr_can_throw(&expr.node),
        A::Expr::Await(inner) => expr_can_throw(&inner.node),
        A::Expr::Tuple(items) => items.iter().any(|i| expr_can_throw(&i.node)),
        A::Expr::TupleGet { base, .. } => expr_can_throw(&base.node),
        A::Expr::Switch { scrutinee, cases, default } => {
            let mut out = expr_can_throw(&scrutinee.node);
            for c in cases {
                if let Some(g) = &c.guard {
                    out = out || expr_can_throw(&g.node);
                }
                out = out
                    || match &c.body {
                        A::SwitchExprBody::Expr(x) => expr_can_throw(&x.node),
                        A::SwitchExprBody::Block(b) => {
                            b.stmts.iter().any(|s| stmt_can_throw(&s.node))
                        }
                    };
            }
            if let Some(d) = default {
                out = out
                    || match d {
                        A::SwitchExprBody::Expr(x) => expr_can_throw(&x.node),
                        A::SwitchExprBody::Block(b) => {
                            b.stmts.iter().any(|s| stmt_can_throw(&s.node))
                        }
                    };
            }
            out
        }
        A::Expr::Ternary { cond, then, otherwise } => {
            expr_can_throw(&cond.node)
                || expr_can_throw(&then.node)
                || expr_can_throw(&otherwise.node)
        }
        A::Expr::Range { lo, hi, .. } => expr_can_throw(&lo.node) || expr_can_throw(&hi.node),
        A::Expr::Call { callee, args, .. } => {
            expr_can_throw(&callee.node)
                || args.iter().any(|a| expr_can_throw(&a.value.node))
        }
        A::Expr::Index { base, index } => {
            expr_can_throw(&base.node) || expr_can_throw(&index.node)
        }
        A::Expr::Member { base, .. } => expr_can_throw(&base.node),
        A::Expr::Cast { expr, .. } => expr_can_throw(&expr.node),
        A::Expr::Interp(parts) => parts.iter().any(|p| match p {
            A::InterpPart::Expr(x) => expr_can_throw(&x.node),
            _ => false,
        }),
        A::Expr::Array(items) => items.iter().any(|i| expr_can_throw(&i.expr.node)),
        A::Expr::Record(fields) => fields.iter().any(|e| expr_can_throw(&e.value().node)),
        A::Expr::MapLiteral(entries) => entries.iter().any(|e| expr_can_throw(&e.value().node)),
        A::Expr::Macro { args, .. } => args.iter().any(|a| expr_can_throw(&a.node)),
        A::Expr::Closure { body, .. } => body_can_throw(body),
        A::Expr::UnsafeBlock(b) => b.stmts.iter().any(|s| stmt_can_throw(&s.node)),
        _ => false,
    }
}

fn collect_drop_arrays(ty: &LirType, out: &mut Vec<LirType>) {
    if let LirType::Array(inner) = ty {
        if matches!(**inner, LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_)) {
            out.push((**inner).clone());
        }
    }
}

impl Lower {

    fn collect_foreign(&mut self, m: &A::Module) -> Result<(), Diagnostic> {
        for decl in &m.decls {
            match &decl.node {
                A::Decl::Import(d) => {
                    if let A::ImportSource::Native(lib) = &d.source {
                        match &d.clause {
                            A::ImportClause::Named(specs) | A::ImportClause::DefaultAndNamed(_, specs) => {
                                for spec in specs {
                                    if let Some(sig) = &spec.native_fn {
                                        self.register_foreign(lib, sig, decl.span)?;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                A::Decl::ExportFrom(d) => {
                    if let A::ImportSource::Native(lib) = &d.source
                        && let A::ExportClause::Native(sigs) = &d.clause
                    {
                        for sig in sigs {
                            self.register_foreign(lib, sig, decl.span)?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn register_foreign(&mut self, lib: &str, sig: &A::NativeFnSig, span: Span) -> Result<(), Diagnostic> {
        let lib = crate::instr::clean_native_lib(lib);
        if lib.is_empty() {
            return Err(Self::derr(Code::E108, "native library name must not be empty", span));
        }
        let local = sig.alias.clone().unwrap_or_else(|| sig.name.clone());
        if self.out.fn_index.contains_key(&local) || self.out.foreign_index.contains_key(&local) {
            return Err(Self::derr(Code::E108, format!("duplicate foreign function `{local}`"), span));
        }
        let mut params = Vec::with_capacity(sig.params.len());
        for p in &sig.params {
            let ty = match &p.ty {
                Some(t) => t,
                None => {
                    return Err(Self::derr(
                        Code::E108,
                        format!("foreign fn `{}` param `{}` needs an explicit C-ABI type", sig.name, p.name),
                        p.span,
                    ));
                }
            };
            let lt = map_ty(ty);
            if !is_c_abi(&lt) {
                return Err(Self::derr(
                    Code::E108,
                    format!("foreign fn `{}` param `{}` has non-C-ABI type; use Int, Float, Byte, Bool, or Pointer<T>", sig.name, p.name),
                    p.span,
                ));
            }
            params.push(lt);
        }
        let ret = match &sig.ret {
            Some(t) => {
                let lt = map_ty(t);
                if !is_c_abi(&lt) && !matches!(lt, LirType::Void) {
                    return Err(Self::derr(
                        Code::E108,
                        format!("foreign fn `{}` has non-C-ABI return type; use Int, Float, Byte, Bool, Pointer<T>, or Void", sig.name),
                        span,
                    ));
                }
                lt
            }
            None => LirType::Void,
        };
        let id = self.out.foreign.len();
        self.out.foreign.push(crate::instr::ForeignFn {
            local: local.clone(),
            lib,
            symbol: sig.name.clone(),
            params,
            ret,
        });
        self.out.foreign_index.insert(local, id);
        Ok(())
    }

    fn collect_consts(&mut self, m: &A::Module) -> Result<(), Diagnostic> {
        for decl in &m.decls {
            let (name, ty, value) = match &decl.node {
                A::Decl::Const { name, ty, value, .. } => (name.clone(), ty.clone(), value.clone()),
                _ => continue,
            };
            let lit = self.const_lit(&value)?;
            let lt = match &lit {
                Lit::Int(_) => LirType::I64,
                Lit::Float(_, _) => LirType::F64(FloatKind::Strict),
                Lit::Bool(_) => LirType::Bool,
                Lit::Str(_) => LirType::Str,
                Lit::Null => {
                    return Err(Lower::derr(
                        Code::E108,
                        format!("const `{name}` needs a constant Int, Float, Bool, or String value"),
                        value.span,
                    ));
                }
            };
            if let Some(t) = &ty {
                let want = t.path.last().map(|s| s.as_str()).unwrap_or("");
                let ok = matches!(
                    (want.rsplit('.').next().unwrap_or(want), &lit),
                    ("Int", Lit::Int(_))
                        | ("Float", Lit::Float(_, _))
                        | ("Bool", Lit::Bool(_))
                        | ("String", Lit::Str(_))
                );
                if !ok {
                    return Err(Lower::derr(
                        Code::E108,
                        format!("const `{name}` is declared `{want}` but its value is not constant of that type"),
                        value.span,
                    ));
                }
            }
            self.mod_consts.insert(
                name,
                (
                    lit,
                    lt.clone(),
                    match lt {
                        LirType::I64 => Simple::Int,
                        LirType::F64(_) => Simple::Strict,
                        _ => Simple::Other,
                    },
                ),
            );
        }
        Ok(())
    }

    fn const_lit(&self, e: &A::Spanned<A::Expr>) -> Result<Lit, Diagnostic> {
        match &e.node {
            A::Expr::Int(n) => Ok(Lit::Int(*n)),
            A::Expr::Float(x) => Ok(Lit::Float(*x, FloatKind::Strict)),
            A::Expr::Bool(b) => Ok(Lit::Bool(*b)),
            A::Expr::Interp(parts) => {
                let mut out = String::new();
                for p in parts {
                    match p {
                        A::InterpPart::Text(t) => out.push_str(t),
                        A::InterpPart::Expr(ex) => {
                            return Err(Lower::derr(
                                Code::E108,
                                "const string values cannot interpolate runtime expressions",
                                ex.span,
                            ));
                        }
                    }
                }
                Ok(Lit::Str(out))
            }
            A::Expr::Ident(n) => match self.mod_consts.get(n) {
                Some((lit, _, _)) => Ok(lit.clone()),
                None => Err(Lower::derr(
                    Code::E108,
                    format!("const value reads `{n}` before its declaration"),
                    e.span,
                )),
            },
            A::Expr::Unary { op, rhs } if op == &A::UnOp::Neg => match self.const_lit(rhs)? {
                Lit::Int(n) => Ok(Lit::Int(n.wrapping_neg())),
                Lit::Float(x, k) => Ok(Lit::Float(-x, k)),
                _ => Err(Lower::derr(
                    Code::E108,
                    "unary `-` in a const value needs an Int or Float",
                    e.span,
                )),
            },
            _ => Err(Lower::derr(
                Code::E108,
                "const values must be Int, Float, Bool, or String literals",
                e.span,
            )),
        }
    }

    fn collect_namespaces(&mut self) {
        let mut infos: Vec<(String, String, String)> = Vec::new();
        for name in self.out.class_index.keys() {
            if !name.starts_with("__ns_") {
                continue;
            }
            let rest = name.strip_prefix("__ns_").unwrap_or("");
            let Some((key, alias)) = rest.rsplit_once('.') else {
                continue;
            };
            if key.is_empty() || alias.is_empty() {
                continue;
            }
            infos.push((name.clone(), alias.to_string(), key.to_string()));
        }
        infos.sort();
        for (synth, alias, key) in infos {
            let prefix = format!("{key}.");
            let mut names: BTreeSet<String> = BTreeSet::new();
            for n in self.out.fn_index.keys() {
                if let Some(rest) = n.strip_prefix(&prefix) {
                    if let Some((first, _)) = rest.split_once('.') {
                        names.insert(first.to_string());
                    } else {
                        names.insert(rest.to_string());
                    }
                }
            }
            for n in self.out.class_index.keys() {
                if n.starts_with("__ns_") {
                    continue;
                }
                if let Some(rest) = n.strip_prefix(&prefix) {
                    if let Some((first, _)) = rest.split_once('.') {
                        names.insert(first.to_string());
                    } else {
                        names.insert(rest.to_string());
                    }
                }
            }
            for n in self.out.enum_index.keys() {
                if let Some(rest) = n.strip_prefix(&prefix) {
                    if let Some((first, _)) = rest.split_once('.') {
                        names.insert(first.to_string());
                    } else {
                        names.insert(rest.to_string());
                    }
                }
            }
            for n in self.mod_consts.keys() {
                if let Some(rest) = n.strip_prefix(&prefix) {
                    if !rest.contains('.') {
                        names.insert(rest.to_string());
                    }
                }
            }
            let mut exports = Vec::with_capacity(names.len());
            for plain in names {
                let mangled = format!("{key}.{plain}");
                let kind = if self.out.class_index.contains_key(&mangled) {
                    crate::instr::NsKind::Class
                } else if self.out.enum_index.contains_key(&mangled) {
                    crate::instr::NsKind::Enum
                } else if let Some((lit, _, _)) = self.mod_consts.get(&mangled).cloned() {
                    crate::instr::NsKind::Const(lit)
                } else if self.out.fn_index.contains_key(&mangled) {
                    crate::instr::NsKind::Function
                } else {
                    continue;
                };
                exports.push(crate::instr::NsExport { name: plain, kind });
            }
            self.out.namespaces.insert(
                synth,
                crate::instr::NamespaceDesc { alias, key, exports },
            );
        }
    }
}
impl Lower {
    fn classes(&mut self, m: &A::Module) -> Result<(), Diagnostic> {
        for decl in &m.decls {
            if let A::Decl::Enum { name, type_params, members, .. } = &decl.node {
                self.enum_desc(name, type_params, members, decl.span)?;
            }
        }
        for decl in &m.decls {
            match &decl.node {
                A::Decl::Enum { .. } => {}
                A::Decl::Class {
                    name,
                    type_params,
                    members,
                    ..
                } => self.class_desc(name, type_params, members, false)?,
                A::Decl::Struct { name, type_params, members, .. } => {
                    self.class_desc(name, type_params, members, true)?
                }
                A::Decl::Record { name, fields, .. } => self.record_desc(name, fields, decl.span)?,
                A::Decl::Interface { name, type_params, members, .. } => {
                    self.iface_desc(name, type_params, members, decl.span)?
                }
                _ => {}
            }
        }
        for decl in &m.decls {
            if let A::Decl::Trait { name, members, .. } = &decl.node {
                if let Err(d) = self.trait_iface_desc(name, members, decl.span) {
                    self.diags.push(d);
                }
            }
        }
        Ok(())
    }


    fn synthesize_dtors(&mut self) {
        let mut dtors: Vec<(usize, usize)> = Vec::new();
        for (ci, class) in self.out.classes.iter().enumerate() {
            if class.is_struct {
                continue;
            }
            let owned: Vec<usize> = class
                .fields
                .iter()
                .enumerate()
                .filter(|(_, f)| {
                    matches!(f.ty, LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Any)
                })
                .map(|(i, _)| i)
                .collect();
            let locals = vec![LirType::Obj(class.name.clone())];
            let mut instrs = Vec::new();
            for i in &owned {
                instrs.push(Instr::ReleaseField { span: crate::instr::UNKNOWN_SPAN, obj: 0, field: *i });
            }
            instrs.push(Instr::GenRefInvalidate { span: crate::instr::UNKNOWN_SPAN, obj: 0 });
            let id = self.out.functions.len();
            self.out.functions.push(Function {
                name: format!("__dtor_{}", class.name),
                params: vec![LirType::Obj(class.name.clone())],
                sig_params: vec![LirType::Obj(class.name.clone())],
                ret: LirType::Void,
                throws: false,
                is_unsafe: false,
                method_self: false,
                is_pub: false,
                is_closure: false,
                locals,
                blocks: vec![crate::instr::Block {
                    instrs,
                    term: Terminator::Ret(vec![]),
                }],
            });
            dtors.push((ci, id));
        }
        for (ci, id) in dtors {
            self.out.classes[ci].dtor = Some(id);
        }
        self.synthesize_array_dtors();
        self.synthesize_enum_dtors();
    }

    fn synthesize_enum_dtors(&mut self) {
        // Dtor payload loads are destructive takes: backends must not
        // retain there (see in_enum_dtor); the Release drops the enum's own ref.
        for ei in 0..self.out.enums.len() {
            let owned: Vec<(usize, Vec<(usize, LirType)>)> = self.out.enums[ei]
                .variants
                .iter()
                .enumerate()
                .map(|(vi, v)| {
                    let slots = v
                        .payload
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| {
                            matches!(t, LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Any)
                        })
                        .map(|(i, t)| (i, t.clone()))
                        .collect::<Vec<_>>();
                    (vi, slots)
                })
                .filter(|(_, slots)| !slots.is_empty())
                .collect();
            if owned.is_empty() {
                continue;
            }
            let ename = self.out.enums[ei].name.clone();
            let mut locals = vec![LirType::Enum(ei), LirType::I64];
            let exit_bb = 2 + owned.len();
            let default_bb = 1 + owned.len();
            let mut blocks = vec![crate::instr::Block {
                instrs: vec![Instr::EnumTag { span: crate::instr::UNKNOWN_SPAN, dst: 1, scrut: 0 }],
                term: Terminator::Br(default_bb),
            }];
            let mut cases: Vec<(SwitchPat, usize)> = Vec::new();
            for (n, (vi, slots)) in owned.iter().enumerate() {
                let mut instrs = Vec::new();
                for (slot, ty) in slots {
                    let dst = locals.len() as Local;
                    locals.push(ty.clone());
                    instrs.push(Instr::EnumPayload { span: crate::instr::UNKNOWN_SPAN, dst, scrut: 0, index: *slot });
                    instrs.push(Instr::Release { span: crate::instr::UNKNOWN_SPAN, obj: dst });
                }
                blocks.push(crate::instr::Block {
                    instrs,
                    term: Terminator::Br(exit_bb),
                });
                cases.push((SwitchPat::Int(*vi as i64), 1 + n));
            }
            blocks.push(crate::instr::Block {
                instrs: Vec::new(),
                term: Terminator::Br(exit_bb),
            });
            blocks.push(crate::instr::Block {
                instrs: Vec::new(),
                term: Terminator::Ret(vec![]),
            });
            blocks[0].term = Terminator::Switch {
                span: crate::instr::UNKNOWN_SPAN,
                scrut: 1,
                cases,
                default: default_bb,
            };
            let id = self.out.functions.len();
            self.out.functions.push(Function {
                name: format!("__enum_dtor_{ename}"),
                params: vec![LirType::Enum(ei)],
                sig_params: vec![LirType::Enum(ei)],
                ret: LirType::Void,
                throws: false,
                is_unsafe: false,
                method_self: false,
                is_pub: false,
                is_closure: false,
                locals,
                blocks,
            });
            self.out.enums[ei].dtor = Some(id);
        }
    }

    fn synthesize_array_dtors(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        let mut queue: Vec<LirType> = Vec::new();
        for f in &self.out.functions {
            for t in f.locals.iter().chain(f.params.iter()) {
                collect_drop_arrays(t, &mut queue);
            }
        }
        for c in &self.out.classes {
            for fld in &c.fields {
                collect_drop_arrays(&fld.ty, &mut queue);
            }
        }
        while let Some(elem) = queue.pop() {
            let key = type_key(&elem);
            if seen.contains(&key) || self.out.array_dtors.contains_key(&key) {
                continue;
            }
            seen.push(key.clone());
            if let LirType::Array(inner) = &elem {
                collect_drop_arrays(inner, &mut queue);
            }
            let id = self.out.functions.len();
            let name = format!("__arrdtor_{id}");
            let locals = vec![elem.clone()];
            let known_class = match &elem {
                LirType::Obj(n) => self.out.class_index.contains_key(n),
                _ => false,
            };
            let instrs = match &elem {
                LirType::Obj(_) if known_class => {
                    vec![Instr::Release { span: crate::instr::UNKNOWN_SPAN, obj: 0 }]
                }
                LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Any | LirType::Error => {
                    vec![Instr::Release { span: crate::instr::UNKNOWN_SPAN, obj: 0 }]
                }
                _ => Vec::new(),
            };
            let body = crate::instr::Block {
                instrs,
                term: Terminator::Ret(vec![]),
            };
            self.out.functions.push(Function {
                name: name.clone(),
                params: vec![elem.clone()],
                sig_params: vec![elem],
                ret: LirType::Void,
                throws: false,
                is_unsafe: false,
                method_self: false,
                is_pub: false,
                is_closure: false,
                locals,
                blocks: vec![body],
            });
            self.out.fn_index.insert(name, id);
            self.out.array_dtors.insert(key, id);
        }
    }

    fn bodies(&mut self, m: &A::Module) -> Result<(), Diagnostic> {
        let mut jobs: Vec<(usize, BodySrc)> = Vec::new();
        let mut implicit_super: BTreeMap<usize, usize> = BTreeMap::new();
        self.propagate_init_throws(m);
        for decl in &m.decls {
            match &decl.node {
                A::Decl::Fn(f) => {
                    let id = self.out.fn_index[&f.name];
                    jobs.push((id, BodySrc::Fn(f.clone())));
                }
                A::Decl::Class { name, members, .. }
                | A::Decl::Struct { name, members, .. } => {
                    for mem in members {
                        match &mem.node {
                            A::ClassMember::Method(f) => {
                                let q = format!("{}.{}", name, f.name);
                                jobs.push((self.out.fn_index[&q], BodySrc::Fn(f.clone())));
                            }
                            A::ClassMember::Init { params, body } => {
                                let q = format!("{name}.init");
                                let id = self.out.fn_index[&q];
                                self.plan_super(m, name, body, mem.span, id, &mut implicit_super);
                                jobs.push((
                                    id,
                                    BodySrc::Init(params.clone(), body.clone()),
                                ));
                            }
                            A::ClassMember::Deinit(body) => {
                                let q = format!("{name}.deinit");
                                jobs.push((self.out.fn_index[&q], BodySrc::Bare(vec![], body.clone())));
                            }
                            A::ClassMember::OnReload { params, body } => {
                                let q = format!("{name}.onReload");
                                jobs.push((
                                    self.out.fn_index[&q],
                                    BodySrc::Bare(params.clone(), body.clone()),
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                A::Decl::Extension { target, members, .. } => {
                    let tname = self
                        .ext_target_name(target, crate::instr::UNKNOWN_SPAN)
                        .unwrap_or_default();
                    for mem in members {
                        if let A::ClassMember::Method(f) = &mem.node {
                            let q = format!("__ext_{tname}__{}", f.name);
                            if let Some(id) = self.out.fn_index.get(&q).copied() {
                                jobs.push((id, BodySrc::Fn(f.clone())));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let mut queue: Vec<(usize, BodySrc)> = jobs;
        let mut head = 0;
        while head < queue.len() {
            let (id, src) = queue[head].clone();
            head += 1;
            let module = std::mem::replace(&mut self.out, Module::default());
            let prefix = module.functions.get(id).map(|f| format!("{}::", f.name)).unwrap_or_default();
            let mut b = Builder::new(&module, &self.defaults, &self.param_info, &self.param_tys, &self.field_opt, &self.ret_tys, &self.nub_fields, &self.generic_fns, &self.extensions, &self.ext_self, &self.mod_consts, id, prefix);
            b.implicit_super = implicit_super.get(&id).copied();
            let is_closure_body = matches!(src, BodySrc::Closure(_));
            if let BodySrc::Fn(f) = &src {
                b.fn_tparams = f.type_params.clone();
                if let Some(fname) = module.functions.get(id).map(|f| f.name.clone()) {
                    if let Some(target) = self.ext_self.get(&fname) {
                        for a in &target.args {
                            if a.fn_sig.is_none() && a.args.is_empty() && a.tuple.is_empty() && a.path.len() == 1
                                && !b.fn_tparams.iter().any(|p| p == &a.path[0])
                            {
                                b.fn_tparams.push(a.path[0].clone());
                            }
                        }
                    }
                }
            }
            let r = match src {
                BodySrc::Fn(f) => b.lower_fn_body(&f),
                BodySrc::Init(params, body) => b.lower_init_body(params, body),
                BodySrc::Bare(params, body) => b.lower_bare_body(params, body),
                BodySrc::Closure(p) => {
                    b.closure_prefix = p.prefix.clone();
                    b.enclosing = p.enclosing.clone();
                    b.lower_closure_body(p)
                }
            };
            let (mut func, patches, pending, mut diags) = b.finish();
            if is_closure_body {
                func.is_closure = true;
            }
            self.out = module;
            self.out.functions[id] = func;
            self.diags.append(&mut diags);
            r?;
            for p in pending {
                let cid = self.out.functions.len();
                let mut params: Vec<LirType> = p
                    .params
                    .iter()
                    .map(|x| x.ty.as_ref().map(|t| resolve_ty(&self.out, t)).unwrap_or(LirType::Any))
                    .collect();
                for c in &p.captures {
                    params.push(c.ty.clone());
                }
                self.out.functions.push(Function {
                    name: p.name.clone(),
                    params: params.clone(),
                    sig_params: params.clone(),
                    ret: p.ret.as_ref().map(|t| resolve_ty(&self.out, t)).unwrap_or(LirType::Any),
                    throws: p.throws,
                    is_unsafe: false,
                    method_self: false,
                    is_pub: false,
                    is_closure: false,
                    locals: params,
                    blocks: vec![],
                });
                self.out.fn_index.insert(p.name.clone(), cid);
                queue.push((cid, BodySrc::Closure(p)));
            }
            for (bb, ix, name) in patches {
                let cid = *self.out.fn_index.get(&name).ok_or_else(|| {
                    Diagnostic::new(Code::E108, "closure target lost")
                })?;
                match &mut self.out.functions[id].blocks[bb].instrs[ix] {
                    Instr::ClosureNew { func, .. } => *func = cid,
                    _ => {
                        return Err(Diagnostic::new(Code::E108, "closure patch lost"));
                    }
                }
            }
        }
        Ok(())
    }
}
impl<'a> Builder<'a> {


    fn variant_of(&self, name: &str) -> Option<(usize, usize)> {
        for (id, e) in self.module.enums.iter().enumerate() {
            if let Some(vi) = e.variant_index.get(name) {
                return Some((id, *vi));
            }
        }
        None
    }

    fn variant_of_path(&self, path: &[String]) -> Option<(usize, usize)> {
        let vname = path.last()?;
        for (id, e) in self.module.enums.iter().enumerate() {
            if let Some(vi) = e.variant_index.get(vname) {
                if path.len() > 1 && short_name(&e.name) != path[path.len() - 2] {
                    continue;
                }
                return Some((id, *vi));
            }
        }
        None
    }

    fn current_class(&self) -> Option<&str> {
        self.enclosing.as_deref()
    }

    fn check_field(&mut self, ci: usize, field: &str, span: Span) -> Result<usize, Diagnostic> {
        let desc = &self.module.classes[ci];
        let current = self.current_class().map(|c| c.to_string());
        let fi = *desc.field_index.get(field).ok_or_else(|| {
            self.err(Code::E108, format!("unknown field `{field}`"), span)
        })?;
        if desc.fields[fi].private && current.as_deref() != Some(desc.fields[fi].owner.as_str()) {
            return self.fail(self.err(
                Code::E203,
                format!("private field `{}.{field}` accessed outside its class", desc.fields[fi].owner),
                span,
            ));
        }
        Ok(fi)
    }

    fn check_method(&mut self, ci: usize, method: &str, span: Span) -> Result<usize, Diagnostic> {
        let desc = &self.module.classes[ci];
        let current = self.current_class().map(|c| c.to_string());
        let target = desc.methods.get(method).cloned().ok_or_else(|| {
            self.err(Code::E108, format!("unknown method `{method}`"), span)
        })?;
        if target.private && current.as_deref() != Some(target.owner.as_str()) {
            return self.fail(self.err(
                Code::E203,
                format!("private method `{}.{method}` accessed outside its class", target.owner),
                span,
            ));
        }
        Ok(target.id)
    }

    fn super_parent(&self, span: Span) -> Result<usize, Diagnostic> {
        let cname = self.current_class().ok_or_else(|| {
            self.err(Code::E108, "`super` used outside of a class method", span)
        })?;
        let ci = *self.module.class_index.get(cname).ok_or_else(|| {
            self.err(Code::E108, "`super` used outside of a class method", span)
        })?;
        self.module.classes[ci].parent.ok_or_else(|| {
            self.err(
                Code::E108,
                format!("`{cname}` has no parent class"),
                span,
            )
        })
    }

    fn lower_super_call(
        &mut self,
        field: &str,
        args: &[A::CallArg],
        targs: &[A::Type],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let pi = self.super_parent(span)?;
        let mid = self.check_method(pi, field, span)?;
        let mname = self.module.functions[mid].name.clone();
        let slf = self.self_local.ok_or_else(|| {
            self.err(Code::E108, "`super` used outside of a class method", span)
        })?;
        let needs_resolve = args.iter().any(|a| a.name.is_some())
            || self.param_info.get(&mname).map(|p| p.iter().any(|(_, d)| d.is_some())).unwrap_or(false);
        let mut full = vec![slf];
        let mut seed = BTreeMap::new();
        if needs_resolve {
            let (rv, rs) = self.resolve_call_args(mid, args, span)?;
            full.extend(rv);
            seed = rs;
        } else {
            let mname = self.module.functions[mid].name.clone();
            let ptys = self.param_tys.get(&mname).cloned().unwrap_or_default();
            for (i, a) in args.iter().enumerate() {
                match ptys.get(i) {
                    Some((_, d)) => full.push(self.lower_call_arg(Some(d), a)?.0),
                    None => full.push(self.lower_expr(&a.value.node, a.value.span)?.0),
                }
            }
        }
        let slf0 = full.first().copied();
        self.call_fn_ex(mid, targs, full, &seed, span, slf0)
    }

    fn lower_super_init(
        &mut self,
        args: &[A::CallArg],
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let pi = self.super_parent(span)?;
        let pname = self.module.classes[pi].name.clone();
        let q = format!("{pname}.init");
        let id = self.module.fn_index.get(&q).copied().ok_or_else(|| {
            self.err(
                Code::E108,
                format!("`{pname}` has no initializer to call with `super(...)`"),
                span,
            )
        })?;
        let slf = self.self_local.ok_or_else(|| {
            self.err(Code::E108, "`super` used outside of a class method", span)
        })?;
        let needs_resolve = args.iter().any(|a| a.name.is_some())
            || self.param_info.get(&q).map(|p| p.iter().any(|(_, d)| d.is_some())).unwrap_or(false);
        let mut argv = vec![slf];
        if needs_resolve {
            let (rv, _) = self.resolve_call_args(id, args, span)?;
            argv.extend(rv);
        } else {
            let ptys = self.param_tys.get(&q).cloned().unwrap_or_default();
            for (i, a) in args.iter().enumerate() {
                match ptys.get(i) {
                    Some((_, d)) => argv.push(self.lower_call_arg(Some(d), a)?.0),
                    None => argv.push(self.lower_expr(&a.value.node, a.value.span)?.0),
                }
            }
        }
        self.emit_checked_call(id, &[], &argv, span)?;
        let dst = self.local(LirType::Null);
        self.emit(Instr::Const { span, dst, lit: Lit::Null });
        Ok((dst, Simple::Other))
    }

    fn lookup(&self, name: &str) -> Option<(Local, Simple, LirType)> {
        for s in self.scopes.iter().rev() {
            if let Some(v) = s.get(name) {
                return Some(v.clone());
            }
        }
        None
    }

    fn mod_const_emit(&mut self, name: &str, span: Span) -> Option<(Local, Simple)> {
        let (lit, lt, s) = self.mod_consts.get(name).cloned()?;
        let dst = self.local(lt);
        self.emit(Instr::Const { span, dst, lit });
        Some((dst, s))
    }

    fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
        self.scope_marks.push(self.targ_log.len());
    }

    fn emit_scope_releases(&mut self) {
        if !self.terminated {
            let scope = self.scopes.last().cloned().unwrap_or_default();
            for (_, (l, _, t)) in scope.iter() {
                if self.moved.contains(l) {
                    continue;
                }
                if let LirType::Obj(name) = t {
                    if let Some(ci) = self.module.class_index.get(name) {
                        if !self.module.classes[*ci].is_struct {
                            self.emit(Instr::Release { span: crate::instr::UNKNOWN_SPAN, obj: *l });
                        }
                    } else if let Some(ii) = self.module.interface_index.get(name).copied() {
                        self.emit_iface_release(*l, ii, crate::instr::UNKNOWN_SPAN);
                    }
                } else if matches!(t, LirType::Str | LirType::Array(_) | LirType::Any | LirType::Error) {
                    self.emit(Instr::Release { span: crate::instr::UNKNOWN_SPAN, obj: *l });
                }
            }
        }
    }

    fn pop_scope(&mut self) {
        self.emit_scope_releases();
        self.scopes.pop();
        if let Some(mark) = self.scope_marks.pop() {
            while self.targ_log.len() > mark {
                if let Some((name, old)) = self.targ_log.pop() {
                    match old {
                        Some(t) => self.var_targs.insert(name, t),
                        None => self.var_targs.remove(&name),
                    };
                }
            }
        }
    }

    fn record_construct_targs(&mut self, name: &str, value: &A::Expr) {
        let targs = match value {
            A::Expr::Call { callee, type_args, .. } => match &callee.node {
                A::Expr::Ident(cn) => {
                    let is_generic = self
                        .module
                        .class_index
                        .get(cn)
                        .is_some_and(|ci| !self.module.classes[*ci].type_params.is_empty());
                    if is_generic && !type_args.is_empty() {
                        Some(type_args.iter().map(|t| self.resolve_here(t)).collect())
                    } else {
                        None
                    }
                }
                _ => None,
            },
            A::Expr::New { target, type_args, .. } => {
                let is_generic = self
                    .module
                    .class_index
                    .get(target)
                    .is_some_and(|ci| !self.module.classes[*ci].type_params.is_empty());
                if is_generic && !type_args.is_empty() {
                    Some(type_args.iter().map(|t| self.resolve_here(t)).collect())
                } else {
                    None
                }
            }
            _ => None,
        };
        self.record_targs(name, targs);
    }

    fn record_targs(&mut self, name: &str, targs: Option<Vec<LirType>>) {
        let old = self.var_targs.get(name).cloned();
        self.targ_log.push((name.to_string(), old));
        match targs {
            Some(t) => self.var_targs.insert(name.to_string(), t),
            None => self.var_targs.remove(name),
        };
    }

    fn resolve_targ(&self, t: &A::Type, tps: &[String], args: &[LirType]) -> LirType {
        if t.fn_sig.is_some() {
            return LirType::Closure;
        }
        if !t.tuple.is_empty() {
            return LirType::Tuple(t.tuple.iter().map(|x| self.resolve_targ(x, tps, args)).collect());
        }
        if t.path.len() == 1 {
            if let Some(i) = tps.iter().position(|p| p == &t.path[0]) {
                return args.get(i).cloned().unwrap_or(LirType::Any);
            }
        }
        if t.args.len() == 1 && t.args[0].fn_sig.is_none() && t.args[0].args.is_empty() && t.args[0].tuple.is_empty() {
            let full = t.path.join(".");
            let short = short_name(&full).to_string();
            if full == "std.prelude.Result" || short == "Result" {
                let inner = self.resolve_targ(&t.args[0], tps, args);
                if let Some(ei) = self.module.enum_index.get("std.prelude.Result").copied() {
                    let _ = inner;
                    return LirType::Enum(ei);
                }
            }
            if t.path.len() == 1 && t.path[0] == "Array" {
                return LirType::Array(Box::new(self.resolve_targ(&t.args[0], tps, args)));
            }
        }
        self.resolve_here(t)
    }

    fn recv_opt_payload(&self, recv: &str, fname: &str, class: &str) -> Option<LirType> {
        let targs = self.var_targs.get(recv)?.clone();
        let ci = *self.module.class_index.get(class)?;
        let tps = self.module.classes[ci].type_params.clone();
        if tps.len() != targs.len() || tps.is_empty() {
            return None;
        }
        let ret = self.ret_tys.get(fname)?.clone();
        let arg = option_arg(&ret)?;
        let inner = self.resolve_targ(arg, &tps, &targs);
        if matches!(inner, LirType::Any) {
            return None;
        }
        Some(inner)
    }

    fn expand_value(&mut self, v: Local, span: Span) -> Result<Vec<Local>, Diagnostic> {
        if let Some(elems) = self.tuples.get(&v).cloned() {
            let mut out = Vec::with_capacity(elems.len());
            for e in elems {
                out.extend(self.expand_value(e, span)?);
            }
            return Ok(out);
        }
        if let Some((lo, hi, incl, step)) = self.ranges.get(&v).cloned() {
            return Ok(vec![lo, hi, incl, step]);
        }
        Ok(vec![v])
    }

    fn lower_return_value(&mut self, v: Local, span: Span) -> Result<Vec<Local>, Diagnostic> {
        let rt = self.func.ret.clone();
        if self.tuples.contains_key(&v) || self.ranges.contains_key(&v) {
            let want_tuple = self.tuples.contains_key(&v);
            let want_range = self.ranges.contains_key(&v);
            if want_tuple && !matches!(rt, LirType::Tuple(_)) {
                return self.fail(self.err(
                    Code::E108,
                    "tuple value cannot be used in a return value",
                    span,
                ));
            }
            if want_range && !matches!(rt, LirType::Range) {
                return self.fail(self.err(
                    Code::E108,
                    "range value cannot be used in a return value",
                    span,
                ));
            }
            let slots = crate::instr::flat_sig(&rt);
            let elems = self.expand_value(v, span)?;
            if elems.len() != slots.len() {
                return self.fail(self.err(
                    Code::E108,
                    format!(
                        "multi-value return has {} values but the signature needs {}",
                        elems.len(),
                        slots.len()
                    ),
                    span,
                ));
            }
            return Ok(elems
                .into_iter()
                .zip(slots.iter())
                .map(|(el, st)| self.coerce_to_slot(el, st, span))
                .collect());
        }
        if matches!(rt, LirType::Tuple(_)) {
            return self.fail(self.err(Code::E108, "tuple return needs a tuple value", span));
        }
        if matches!(rt, LirType::Range) {
            return self.fail(self.err(Code::E108, "range return needs a range value", span));
        }
        let ret_nub = self
            .ret_tys
            .get(&self.func.name)
            .is_some_and(|t| nub_ty(t));
        if ret_nub {
            if self.nub_has(v) {
                return Ok(vec![v]);
            }
            let vt = self.func.locals.get(v as usize).cloned().unwrap_or(LirType::Any);
            match vt {
                LirType::I64 | LirType::Bool | LirType::F64(_) => {
                    let dst = self.local(vt);
                    self.nub_box(dst, v, span);
                    return Ok(vec![dst]);
                }
                LirType::Null => return Ok(vec![v]),
                _ => {}
            }
        }
        let v = self.nub_use(v, span);
        Ok(vec![self.coerce_to_slot(v, &rt, span)])
    }

    fn resolve_here(&self, t: &A::Type) -> LirType {
        meth_ty(self.module, self.enclosing.as_deref(), &self.fn_tparams, t)
    }

    fn eff_span(child: Span, parent: Span) -> Span {
        if child == crate::instr::UNKNOWN_SPAN {
            parent
        } else {
            child
        }
    }

    fn lower_vec_method(
        &mut self,
        obj: Local,
        ty: LirType,
        field: &str,
        argv: Vec<Local>,
        span: Span,
    ) -> Result<(Local, Simple), Diagnostic> {
        let is_f = ty == LirType::Vec4f;
        let lane_ty = if is_f { LirType::F64(FloatKind::Strict) } else { LirType::I64 };
        let lane_simple = if is_f { Simple::Strict } else { Simple::Int };
        match field {
            "x" | "y" | "z" | "w" => {
                if !argv.is_empty() {
                    return self.fail(self.err(Code::E108, format!("`{field}` takes no args"), span));
                }
                let lane = match field {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    _ => 3,
                };
                let li = self.local(LirType::I64);
                self.emit(Instr::Const { span,  dst: li, lit: Lit::Int(lane as i64) });
                let dst = self.local(lane_ty);
                self.emit(Instr::VecExtract { span,  dst, vec: obj, lane: li });
                Ok((dst, lane_simple))
            }
            "get" => {
                if argv.len() != 1 {
                    return self.fail(self.err(Code::E108, "`get` takes 1 arg", span));
                }
                if self.func.locals[argv[0] as usize] != LirType::I64 {
                    return self.fail(self.err(Code::E108, "`get` needs an Int lane", span));
                }
                let dst = self.local(lane_ty);
                self.emit(Instr::VecExtract { span,  dst, vec: obj, lane: argv[0] });
                Ok((dst, lane_simple))
            }
            "dot" => {
                if !is_f {
                    return self.fail(self.err(Code::E108, "`dot` needs Vec4f", span));
                }
                if argv.len() != 1 {
                    return self.fail(self.err(Code::E108, "`dot` takes 1 arg", span));
                }
                if self.func.locals[argv[0] as usize] != LirType::Vec4f {
                    return self.fail(self.err(Code::E108, "`dot` needs a Vec4f argument", span));
                }
                let dst = self.local(LirType::F64(FloatKind::Strict));
                self.emit(Instr::VecDot { span,  dst, lhs: obj, rhs: argv[0] });
                Ok((dst, Simple::Strict))
            }
            "min" | "max" => {
                if !is_f {
                    return self.fail(self.err(Code::E108, format!("`{field}` needs Vec4f"), span));
                }
                if argv.len() != 1 {
                    return self.fail(self.err(Code::E108, format!("`{field}` takes 1 arg"), span));
                }
                if self.func.locals[argv[0] as usize] != LirType::Vec4f {
                    return self.fail(self.err(Code::E108, format!("`{field}` needs a Vec4f argument"), span));
                }
                let op = if field == "min" { VecOp::Min } else { VecOp::Max };
                let dst = self.local(LirType::Vec4f);
                self.emit(Instr::VecArith { span,  dst, op, kind: VecKind::F, lhs: obj, rhs: argv[0] });
                Ok((dst, Simple::Other))
            }
            "sqrt" => {
                if !is_f {
                    return self.fail(self.err(Code::E108, "`sqrt` needs Vec4f", span));
                }
                if !argv.is_empty() {
                    return self.fail(self.err(Code::E108, "`sqrt` takes no args", span));
                }
                let dst = self.local(LirType::Vec4f);
                self.emit(Instr::VecUnary { span,  dst, op: VecUnaryOp::Sqrt, src: obj });
                Ok((dst, Simple::Other))
            }
            _ => self.fail(self.err(Code::E108, format!("unknown vector method `{field}`"), span)),
        }
    }

}
