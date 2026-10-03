use crate::ast as A;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CABI {
    Int,
    Float,
    Bool,
    Void,
    Str,
}

impl CABI {
    pub fn ctype(&self) -> &'static str {
        match self {
            CABI::Int => "int64_t",
            CABI::Float => "double",
            CABI::Bool => "bool",
            CABI::Void => "void",
            CABI::Str => "const char*",
        }
    }

    pub fn of_ast(ty: &A::Type) -> Option<CABI> {
        if ty.fn_sig.is_some() {
            return None;
        }
        match ty.path.first().map(|s| s.as_str()).map(|h| h.rsplit('.').next().unwrap_or(h)) {
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
    let guard = guard_of(package_name);
    let mut out = String::new();
    out.push_str(&format!("#ifndef {guard}\n#define {guard}\n\n"));
    out.push_str("#include <stdint.h>\n#include <stdbool.h>\n\n");
    out.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");
    for f in exported_fns {
        let params = if f.params.is_empty() {
            "void".to_string()
        } else {
            f.params
                .iter()
                .map(|(n, t)| format!("{} {n}", t.ctype()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!("{} {}({});\n", f.ret.ctype(), f.name, params));
    }
    out.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n");
    out.push_str("#endif\n");
    out
}
