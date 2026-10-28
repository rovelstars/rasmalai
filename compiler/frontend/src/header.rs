use crate::ast as A;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CABI {
    Int,
    Float,
    Bool,
    Void,
    Str,
    PtrByte,
}

pub const RNX_ABI_VERSION: u32 = 1;

fn is_byte_ty(ty: &A::Type) -> bool {
    ty.fn_sig.is_none()
        && ty.args.is_empty()
        && ty.path.first().is_some_and(|s| s.rsplit('.').next().unwrap_or(s) == "Byte")
}

impl CABI {
    pub fn ctype(&self) -> &'static str {
        match self {
            CABI::Int => "int64_t",
            CABI::Float => "double",
            CABI::Bool => "bool",
            CABI::Void => "void",
            CABI::Str => "const char*",
            CABI::PtrByte => "const uint8_t*",
        }
    }

    pub fn of_ast(ty: &A::Type) -> Option<CABI> {
        if ty.fn_sig.is_some() {
            return None;
        }
        match ty.path.first().map(|s| s.as_str()).map(|h| h.rsplit('.').next().unwrap_or(h)) {
            Some("Pointer") | Some("Address") => match ty.args.first() {
                Some(inner) if is_byte_ty(inner) => Some(CABI::PtrByte),
                _ => None,
            },
            Some("Int") | Some("Int64") | Some("Int32") | Some("Int16") | Some("Int8")
            | Some("UInt") | Some("UInt64") | Some("UInt32") | Some("UInt16") | Some("UInt8")
            | Some("Byte") | Some("Short") => Some(CABI::Int),
            Some("Float") | Some("Float32") | Some("FastFloat") | Some("FastFloat32") => {
                Some(CABI::Float)
            }
            Some("Bool") => Some(CABI::Bool),
            Some("String") | Some("Char") => Some(CABI::Str),
            Some("Void") => Some(CABI::Void),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExportedFn {
    pub name: String,
    pub params: Vec<(String, CABI)>,
    pub ret: CABI,
    pub docs: String,
}

pub fn c_symbol(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}_{name}")
    }
}

pub fn is_valid_prefix(prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let mut chars = prefix.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn sanitized_doc_line(line: &str) -> String {
    line.replace("*/", "*_/")
}

pub fn guard_of(package_name: &str) -> String {
    let mut out = String::from("RNX_");
    for c in package_name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push('_');
        }
    }
    out.push_str("_H");
    out
}

pub fn generate_c_header(package_name: &str, exported_fns: &[ExportedFn]) -> String {
    generate_c_header_prefixed(package_name, exported_fns, "")
}

pub fn generate_c_header_prefixed(
    package_name: &str,
    exported_fns: &[ExportedFn],
    prefix: &str,
) -> String {
    let guard = guard_of(package_name);
    let mut out = String::new();
    out.push_str(&format!("#ifndef {guard}\n#define {guard}\n\n"));
    out.push_str(&format!("#define RNX_ABI_VERSION {}\n\n", RNX_ABI_VERSION));
    out.push_str("#include <stdint.h>\n#include <stdbool.h>\n\n");
    out.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");
    out.push_str("void rnx_init(int argc, const char *argv[]);\n");
    out.push_str("const char *rnx_last_error(void);\n\n");
    for f in exported_fns {
        for line in f.docs.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            out.push_str(&format!("// {}\n", sanitized_doc_line(line)));
        }
        let params = if f.params.is_empty() {
            "void".to_string()
        } else {
            f.params
                .iter()
                .map(|(n, t)| format!("{} {n}", t.ctype()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "{} {}({});\n",
            f.ret.ctype(),
            c_symbol(prefix, &f.name),
            params
        ));
    }
    out.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n");
    out.push_str("#endif\n");
    out
}
