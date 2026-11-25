use crate::rustdep::docs::{RustItem, RustItemKind, docs_to_rnx, rnx_function_name, rnx_type_name};
use crate::rustdep::graph::BRIDGE_CRATE_NAME;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiType {
    Int,
    Float,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CExport {
    pub symbol: String,
    pub rust_name: String,
    pub rust_call: String,
    pub alias: String,
    pub rnx_name: String,
    pub params: Vec<(String, AbiType, String)>,
    pub result: Option<(AbiType, String)>,
    pub doc: String,
}

fn classify(text: &str) -> Option<AbiType> {
    match text.trim() {
        "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize"
        | "bool" => Some(AbiType::Int),
        "f32" | "f64" => Some(AbiType::Float),
        _ => None,
    }
}

fn abi_type(ty: AbiType) -> &'static str {
    match ty {
        AbiType::Int => "i64",
        AbiType::Float => "f64",
    }
}

fn rnx_type(ty: AbiType) -> &'static str {
    match ty {
        AbiType::Int => "Int",
        AbiType::Float => "Float",
    }
}

pub fn type_suffix(rust_type: &str) -> String {
    let cleaned = rust_type
        .rsplit("::")
        .next()
        .unwrap_or(rust_type)
        .replace(['<', '>', ',', ' '], "");
    let known = match cleaned.as_str() {
        "i32" => "I32",
        "u32" => "U32",
        "i64" => "I64",
        "u64" => "U64",
        "f32" => "F32",
        "f64" => "F64",
        "bool" => "Bool",
        "String" => "String",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    rnx_type_name(&cleaned)
}

fn monomorph_suffix(types: &[String]) -> String {
    types.iter().map(|ty| type_suffix(ty)).collect()
}

fn substitute_signature(signature: &str, params: &[String], types: &[String]) -> String {
    if params.is_empty() {
        return signature.to_string();
    }
    let mut out = String::with_capacity(signature.len());
    let mut rest = signature;
    while !rest.is_empty() {
        let index = rest
            .char_indices()
            .find(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
            .map(|(i, _)| i);
        let Some(index) = index else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        let len = rest
            .char_indices()
            .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
            .map(|(i, c)| i + c.len_utf8())
            .last()
            .unwrap_or(0);
        let word = &rest[..len];
        let replacement = params
            .iter()
            .zip(types.iter())
            .find(|(param, _)| word == param.as_str())
            .map(|(_, ty)| ty.as_str());
        out.push_str(replacement.unwrap_or(word));
        rest = &rest[len..];
    }
    out
}

fn parse_signature(signature: &str) -> Option<(Vec<(String, AbiType, String)>, Option<(AbiType, String)>)> {
    let body = signature.strip_prefix("fn ")?;
    let open = body.find('(')?;
    let close = body.find(')')?;
    let params_text = &body[open + 1..close];
    let result_text = body[close + 1..]
        .trim_start()
        .strip_prefix("->")
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut params = Vec::new();
    if !params_text.trim().is_empty() {
        for part in params_text.split(',') {
            let (name, ty) = part.split_once(':')?;
            let ty = ty.trim();
            params.push((name.trim().to_string(), classify(ty)?, ty.to_string()));
        }
    }
    let result = match result_text {
        Some(text) => Some((classify(text)?, text.to_string())),
        None => None,
    };
    Some((params, result))
}

pub fn exports_from_items(
    module: &str,
    items: &[RustItem],
    monomorphs: &[Vec<String>],
) -> Vec<CExport> {
    let mut out = Vec::new();
    for item in items {
        if item.kind != RustItemKind::Function && item.kind != RustItemKind::Method {
            continue;
        }
        let Some(signature) = &item.signature else {
            continue;
        };
        let variants: Vec<(String, String, String, Vec<Vec<String>>)> = if item.is_generic {
            monomorphs
                .iter()
                .filter(|types| types.len() == item.type_params.len())
                .map(|types| {
                    let suffix = monomorph_suffix(types);
                    (
                        format!("{}_{}{}", module, item.name, suffix),
                        format!("{}::<{}>", item.name, types.join(", ")),
                        format!("{}{}", rnx_function_name(&item.name), suffix),
                        vec![types.clone()],
                    )
                })
                .collect()
        } else {
            vec![(
                format!("{}_{}", module, item.name),
                item.name.clone(),
                rnx_function_name(&item.name),
                vec![Vec::new()],
            )]
        };
        for (symbol, rust_call, rnx_name, substitutions) in variants {
            let concrete = substitute_signature(signature, &item.type_params, &substitutions[0]);
            let Some((params, result)) = parse_signature(&concrete) else {
                continue;
            };
            out.push(CExport {
                symbol: format!("rnx_{symbol}"),
                rust_name: item.name.clone(),
                rust_call,
                alias: format!("rust{}", capitalize(&rnx_name)),
                rnx_name,
                params,
                result,
                doc: item.doc.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    out
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

pub fn bridge_c_exports(module: &str, exports: &[CExport]) -> String {
    if exports.is_empty() {
        return String::new();
    }
    let mut out = String::from("// Generated C ABI exports for rnx wrappers.\n");
    let _ = module;
    for export in exports {
        let args: Vec<String> = export
            .params
            .iter()
            .map(|(name, ty, _)| format!("{name}: {}", abi_type(*ty)))
            .collect();
        let result = export
            .result
            .as_ref()
            .map(|(ty, _)| format!(" -> {}", abi_type(*ty)))
            .unwrap_or_default();
        out.push_str(&format!(
            "#[no_mangle]\npub unsafe extern \"C\" fn {}({}){result} {{\n",
            export.symbol,
            args.join(", ")
        ));
        let call_args: Vec<String> = export
            .params
            .iter()
            .map(|(name, _, rust_type)| format!("{name} as {rust_type}"))
            .collect();
        let call = format!("crate::{module}::{}({})", export.rust_call, call_args.join(", "));
        let body = match &export.result {
            None => String::new(),
            Some((ty, _)) => format!("    ({call}) as {}\n", abi_type(*ty)),
        };
        out.push_str(&body);
        out.push_str("}\n\n");
    }
    out
}

pub fn wrapper_rnx_source(exports: &[CExport]) -> String {
    let mut out = String::from("// Generated by rnx from rustdoc JSON. Do not edit.\n");
    let imports: Vec<String> = exports
        .iter()
        .map(|e| {
            let params: Vec<String> = e
                .params
                .iter()
                .map(|(name, ty, _)| format!("{name}: {}", rnx_type(*ty)))
                .collect();
            let result = e
                .result
                .as_ref()
                .map(|(ty, _)| format!(": {}", rnx_type(*ty)))
                .unwrap_or_default();
            format!("    fn {}({}){result} as {}", e.symbol, params.join(", "), e.alias)
        })
        .collect();
    out.push_str(&format!(
        "import {{\n{}\n}} from native \"{}\"\n\n",
        imports.join(",\n"),
        BRIDGE_CRATE_NAME
    ));
    for export in exports {
        if !export.doc.is_empty() {
            out.push_str(&docs_to_rnx(&export.doc));
        }
        let params: Vec<String> = export
            .params
            .iter()
            .map(|(name, ty, _)| format!("{name}: {}", rnx_type(*ty)))
            .collect();
        let result = export
            .result
            .as_ref()
            .map(|(ty, _)| format!(": {}", rnx_type(*ty)))
            .unwrap_or_default();
        out.push_str(&format!(
            "export fn {}({}){result} {{\n",
            export.rnx_name,
            params.join(", ")
        ));
        let args: Vec<String> = export.params.iter().map(|(name, _, _)| name.clone()).collect();
        let call = format!("{}({})", export.alias, args.join(", "));
        if export.result.is_none() {
            out.push_str(&format!("    unsafe {{ {call} }}\n"));
        } else {
            let zero = match export.result.as_ref().map(|(ty, _)| *ty) {
                Some(AbiType::Float) => "0.0",
                _ => "0",
            };
            out.push_str(&format!("    let result = {zero}\n"));
            out.push_str(&format!("    unsafe {{ result = {call} }}\n"));
            out.push_str("    return result\n");
        }
        out.push_str("}\n\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, signature: &str, doc: &str) -> RustItem {
        RustItem {
            name: name.to_string(),
            kind: RustItemKind::Function,
            doc: doc.to_string(),
            signature: Some(signature.to_string()),
            is_generic: false,
            type_params: Vec::new(),
        }
    }

    #[test]
    fn lowers_a_documented_function_to_c_and_rnx() {
        let items = vec![item(
            "add_numbers",
            "fn add_numbers(a: i32, b: i32) -> i32",
            "Adds two numbers.",
        )];
        let exports = exports_from_items("probe", &items, &[]);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].symbol, "rnx_probe_add_numbers");
        assert_eq!(exports[0].rnx_name, "addNumbers");
        let rust = bridge_c_exports("probe", &exports);
        assert!(rust.contains("pub unsafe extern \"C\" fn rnx_probe_add_numbers(a: i64, b: i64) -> i64"));
        assert!(rust.contains("crate::probe::add_numbers(a as i32, b as i32)"));
        let rnx = wrapper_rnx_source(&exports);
        assert!(rnx.contains("fn rnx_probe_add_numbers(a: Int, b: Int): Int as rustAddNumbers"));
        assert!(rnx.contains("/**\n * Adds two numbers.\n */"));
        assert!(rnx.contains("export fn addNumbers(a: Int, b: Int): Int {"));
        assert!(rnx.contains("result = rustAddNumbers(a, b)"));
        assert!(crate::parser::Parser::parse_module(&rnx).is_ok());
    }

    #[test]
    fn monomorphizes_native_generics_with_stable_names() {
        let mut generic = item("scale", "fn scale(value: T, by: i32) -> T", "Scales a value.");
        generic.is_generic = true;
        generic.type_params = vec!["T".to_string()];
        let monomorphs = vec![vec!["i32".to_string()], vec!["f64".to_string()]];
        let exports = exports_from_items("probe", &[generic], &monomorphs);
        let names: Vec<&str> = exports.iter().map(|e| e.rnx_name.as_str()).collect();
        assert_eq!(names, vec!["scaleF64", "scaleI32"]);
        let rust = bridge_c_exports("probe", &exports);
        assert!(rust.contains("crate::probe::scale::<i32>(value as i32, by as i32)"));
        assert!(rust.contains("crate::probe::scale::<f64>(value as f64, by as i32)"));
        let rnx = wrapper_rnx_source(&exports);
        assert!(rnx.contains("export fn scaleI32(value: Int, by: Int): Int {"));
        assert!(rnx.contains("export fn scaleF64(value: Float, by: Int): Float {"));
        assert_eq!(exports_from_items("probe", &[], &monomorphs).len(), 0);
    }

    #[test]
    fn skips_unsupported_signatures() {
        let items = vec![
            item("take_text", "fn take_text(value: String) -> i32", ""),
            item("no_sig", "not-a-signature", ""),
        ];
        assert!(exports_from_items("probe", &items, &[]).is_empty());
    }

    #[test]
    fn type_suffix_is_deterministic() {
        assert_eq!(type_suffix("i32"), "I32");
        assert_eq!(type_suffix("f64"), "F64");
        assert_eq!(type_suffix("std::string::String"), "String");
        assert_eq!(type_suffix("Vec<T>"), "VecT");
    }
}
