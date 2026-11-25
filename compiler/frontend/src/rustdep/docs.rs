use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustItemKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Constant,
    TypeAlias,
    Module,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustItem {
    pub name: String,
    pub kind: RustItemKind,
    pub doc: String,
    pub signature: Option<String>,
    pub is_generic: bool,
    pub type_params: Vec<String>,
}

fn kind_of(value: &Value) -> RustItemKind {
    let key = value
        .get("inner")
        .and_then(Value::as_object)
        .and_then(|inner| inner.keys().next().map(String::as_str))
        .or_else(|| {
            value
                .get("kind")
                .and_then(Value::as_object)
                .and_then(|kind| kind.keys().next().map(String::as_str))
        })
        .unwrap_or("");
    match key {
        "function" => {
            let is_method = value
                .get("inner")
                .and_then(|i| i.get("function"))
                .and_then(|f| f.get("header"))
                .and_then(|h| h.get("is_method").or_else(|| h.get("method")))
                .is_some();
            if is_method { RustItemKind::Method } else { RustItemKind::Function }
        }
        "struct" => RustItemKind::Struct,
        "enum" => RustItemKind::Enum,
        "trait" => RustItemKind::Trait,
        "const" => RustItemKind::Constant,
        "type_alias" => RustItemKind::TypeAlias,
        "module" => RustItemKind::Module,
        _ => RustItemKind::Other,
    }
}

fn is_visible(value: &Value) -> bool {
    if value.get("visibility").and_then(Value::as_str) != Some("public") {
        return false;
    }
    let hidden = value
        .get("attrs")
        .and_then(Value::as_array)
        .is_some_and(|attrs| {
            attrs.iter().any(|attr| {
                attr.as_str().is_some_and(|s| s.contains("doc(hidden)"))
                    || attr.get("other").and_then(Value::as_str).is_some_and(|s| s.contains("doc(hidden)"))
            })
        });
    !hidden
}

fn render_type(value: &Value) -> String {
    if let Some(primitive) = value.get("primitive").and_then(Value::as_str) {
        return primitive.to_string();
    }
    if let Some(generic) = value.get("generic").and_then(Value::as_str) {
        return generic.to_string();
    }
    if let Some(path) = value.get("resolved_path") {
        let name = path.get("name").and_then(Value::as_str).unwrap_or("_").to_string();
        let args = path
            .get("args")
            .and_then(|a| a.get("angle_bracketed"))
            .and_then(|a| a.get("args"))
            .and_then(Value::as_array);
        if let Some(args) = args {
            let rendered: Vec<String> = args
                .iter()
                .filter_map(|arg| arg.get("type").and_then(render_type_checked))
                .collect();
            if !rendered.is_empty() {
                return format!("{name}<{}>", rendered.join(", "));
            }
        }
        return name;
    }
    if let Some(tuple) = value.get("tuple").and_then(Value::as_array) {
        let parts: Vec<String> = tuple.iter().map(render_type).collect();
        return format!("({})", parts.join(", "));
    }
    if let Some(slice) = value.get("slice") {
        return format!("[{}]", render_type(slice));
    }
    if let Some(array) = value.get("array") {
        let inner = array.get("type").map(render_type).unwrap_or_default();
        let len = array.get("len").and_then(Value::as_str).unwrap_or("_");
        return format!("[{inner}; {len}]");
    }
    if let Some(reference) = value.get("ref").or_else(|| value.get("borrowed_ref")) {
        let inner = reference.get("type").map(render_type).unwrap_or_default();
        let mutable = reference.get("mutability").and_then(Value::as_str) == Some("mut");
        let lifetime = reference
            .get("lifetime")
            .and_then(Value::as_str)
            .map(|l| format!("{l} "))
            .unwrap_or_default();
        return format!("&{lifetime}{}{inner}", if mutable { "mut " } else { "" });
    }
    if let Some(pointer) = value.get("raw_pointer") {
        let inner = pointer.get("type").map(render_type).unwrap_or_default();
        let mutable = pointer.get("mutability").and_then(Value::as_str) == Some("mut");
        return format!("*{} {inner}", if mutable { "mut" } else { "const" });
    }
    "_".to_string()
}

fn render_type_checked(value: &Value) -> Option<String> {
    if value.is_null() {
        None
    } else {
        Some(render_type(value))
    }
}

fn signature_of(value: &Value, name: &str, kind: RustItemKind) -> Option<String> {
    if !matches!(kind, RustItemKind::Function | RustItemKind::Method) {
        return None;
    }
    let function = value.get("inner")?.get("function")?;
    let decl = function.get("sig").or_else(|| function.get("decl"))?;
    let inputs = decl.get("inputs").and_then(Value::as_array)?;
    let params: Vec<String> = inputs
        .iter()
        .filter_map(|input| {
            let pair = input.as_array()?;
            let param = pair.first()?.as_str()?;
            let ty = pair.get(1).map(render_type).unwrap_or_default();
            Some(format!("{param}: {ty}"))
        })
        .collect();
    let output = match decl.get("output") {
        Some(Value::Null) | None => String::new(),
        Some(ty) => format!(" -> {}", render_type(ty)),
    };
    Some(format!("fn {name}({}){output}", params.join(", ")))
}

pub fn parse_rustdoc_json(text: &str) -> Result<Vec<RustItem>, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("invalid rustdoc JSON: {e}"))?;
    let Some(index) = root.get("index").and_then(Value::as_object) else {
        return Err("rustdoc JSON has no `index` object".to_string());
    };
    let mut items = Vec::new();
    for value in index.values() {
        if !is_visible(value) {
            continue;
        }
        let Some(name) = value.get("name").and_then(Value::as_str) else {
            continue;
        };
        let kind = kind_of(value);
        let doc = value.get("docs").and_then(Value::as_str).unwrap_or("").to_string();
        let signature = signature_of(value, name, kind);
        let params = value
            .get("inner")
            .and_then(|i| i.get("function"))
            .and_then(|f| f.get("generics"))
            .and_then(|g| g.get("params"))
            .and_then(Value::as_array);
        let type_params: Vec<String> = params
            .into_iter()
            .flatten()
            .filter_map(|param| {
                param.as_str().map(str::to_string).or_else(|| {
                    param.get("name").and_then(Value::as_str).map(str::to_string)
                })
            })
            .collect();
        let is_generic = !type_params.is_empty();
        items.push(RustItem { name: name.to_string(), kind, doc, signature, is_generic, type_params });
    }
    items.sort_by(|a, b| a.kind_rank().cmp(&b.kind_rank()).then(a.name.cmp(&b.name)));
    Ok(items)
}

impl RustItemKind {
    fn rank(self) -> u8 {
        match self {
            RustItemKind::Struct | RustItemKind::Enum | RustItemKind::Trait | RustItemKind::TypeAlias => 0,
            RustItemKind::Function | RustItemKind::Method => 1,
            RustItemKind::Constant => 2,
            RustItemKind::Module => 3,
            RustItemKind::Other => 4,
        }
    }
}

impl RustItem {
    fn kind_rank(&self) -> u8 {
        self.kind.rank()
    }
}

pub fn docs_to_rnx(doc: &str) -> String {
    let cleaned = doc.replace("*/", "* /");
    let mut out = String::from("/**\n");
    for line in cleaned.lines() {
        out.push_str(" * ");
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.push_str(" */\n");
    out
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

pub fn rnx_function_name(rust: &str) -> String {
    let mut parts = rust.split('_').filter(|p| !p.is_empty());
    let Some(first) = parts.next() else {
        return rust.to_string();
    };
    let mut out = first.to_string();
    for part in parts {
        out.push_str(&capitalize(part));
    }
    out
}

pub fn rnx_type_name(rust: &str) -> String {
    rust.split('_').filter(|p| !p.is_empty()).map(capitalize).collect()
}

pub fn monomorph_suffix(base: &str, type_names: &[&str]) -> String {
    let mut out = base.to_string();
    for ty in type_names {
        out.push_str(&rnx_type_name(ty));
    }
    out
}

pub fn dedupe_names(names: Vec<String>) -> Vec<String> {
    let mut seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    names
        .into_iter()
        .map(|name| {
            let count = seen.entry(name.clone()).or_insert(0);
            let suffix = if *count == 0 {
                String::new()
            } else {
                count.to_string()
            };
            *count += 1;
            format!("{name}{suffix}")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r##"{
      "format_version": 1,
      "index": {
        "0:1": {
          "id": "0:1", "name": "request_device", "visibility": "public",
          "kind": {"function": null}, "attrs": [], "docs": "Requests a `Device`.\n\nReturns a future.",
          "inner": {"function": {"decl": {
            "inputs": [["adapter", {"primitive": "u32"}], ["label", {"resolved_path": {"name": "String", "id": "0:2"}}]],
            "output": {"resolved_path": {"name": "Device", "id": "0:3"}}
          }, "generics": {}, "header": {}}}
        },
        "0:2": {
          "id": "0:2", "name": "Device", "visibility": "public",
          "kind": {"struct": null}, "attrs": [], "docs": "A GPU device."
        },
        "0:3": {
          "id": "0:3", "name": "internal_secret", "visibility": "public",
          "kind": {"function": null}, "attrs": ["#[doc(hidden)]"], "docs": "Hidden."
        },
        "0:4": {
          "id": "0:4", "name": "private_fn", "visibility": "crate",
          "kind": {"function": null}, "attrs": [], "docs": "Not public."
        },
        "0:5": {
          "id": "0:5", "name": "send", "visibility": "public",
          "kind": {"function": null}, "attrs": [], "docs": "Sends a value.",
          "inner": {"function": {
            "decl": {"inputs": [["value", {"generic": "T"}]], "output": null},
            "generics": {}, "header": {"method": "send"}
          }}
        }
      }
    }"##;

    #[test]
    fn filters_public_items_and_renders_signatures() {
        let items = parse_rustdoc_json(JSON).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"request_device"));
        assert!(names.contains(&"Device"));
        assert!(names.contains(&"send"));
        assert!(!names.contains(&"internal_secret"));
        assert!(!names.contains(&"private_fn"));
        let function = items.iter().find(|i| i.name == "request_device").unwrap();
        assert_eq!(function.signature.as_deref(), Some("fn request_device(adapter: u32, label: String) -> Device"));
        let method = items.iter().find(|i| i.name == "send").unwrap();
        assert_eq!(method.kind, RustItemKind::Method);
    }

    #[test]
    fn converts_inline_docs() {
        let items = parse_rustdoc_json(JSON).unwrap();
        let device = items.iter().find(|i| i.name == "Device").unwrap();
        assert_eq!(docs_to_rnx(&device.doc), "/**\n * A GPU device.\n */\n");
        assert_eq!(docs_to_rnx("ends with */ marker"), "/**\n * ends with * / marker\n */\n");
    }

    #[test]
    fn naming_follows_language_rules() {
        assert_eq!(rnx_function_name("request_device"), "requestDevice");
        assert_eq!(rnx_function_name("render"), "render");
        assert_eq!(rnx_type_name("texture_format"), "TextureFormat");
        assert_eq!(monomorph_suffix("requestDevice", &["i32"]), "requestDeviceI32");
        assert_eq!(dedupe_names(vec!["open".into(), "open".into(), "open".into()]), vec!["open", "open1", "open2"]);
    }
}
