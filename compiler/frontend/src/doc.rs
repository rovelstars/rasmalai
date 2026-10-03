use crate::ast as A;
use crate::project::Manifest;
use diagnostics::{Code, Diagnostic};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct DocInput {
    pub name: String,
    pub module: A::Module,
}

#[derive(Clone, Debug)]
pub struct DocFn {
    pub name: String,
    pub sig: String,
    pub docs: String,
}

#[derive(Clone, Debug)]
pub struct DocField {
    pub name: String,
    pub ty: String,
    pub docs: String,
}

#[derive(Clone, Debug)]
pub struct DocMethod {
    pub name: String,
    pub sig: String,
    pub docs: String,
}

#[derive(Clone, Debug)]
pub struct DocClass {
    pub name: String,
    pub docs: String,
    pub fields: Vec<DocField>,
    pub init: Option<String>,
    pub methods: Vec<DocMethod>,
}

#[derive(Clone, Debug)]
pub struct DocEnum {
    pub name: String,
    pub docs: String,
    pub variants: Vec<DocVariant>,
}

#[derive(Clone, Debug)]
pub struct DocVariant {
    pub name: String,
    pub payload: Vec<String>,
    pub docs: String,
}

#[derive(Clone, Debug)]
pub struct DocConst {
    pub name: String,
    pub ty: String,
    pub docs: String,
}

#[derive(Clone, Debug, Default)]
pub struct DocModuleDoc {
    pub name: String,
    pub path: String,
    pub docs: String,
    pub fns: Vec<DocFn>,
    pub classes: Vec<DocClass>,
    pub enums: Vec<DocEnum>,
    pub consts: Vec<DocConst>,
}

pub fn ty_name(ty: &A::Type) -> String {
    let mut out = ty.path.join(".");
    if !ty.args.is_empty() {
        let args: Vec<String> = ty.args.iter().map(ty_name).collect();
        out.push('<');
        out.push_str(&args.join(", "));
        out.push('>');
    }
    if let Some(sig) = &ty.fn_sig {
        let params: Vec<String> = sig.params.iter().map(ty_name).collect();
        out = format!("fn({})", params.join(", "));
        if let Some(ret) = &sig.ret {
            out.push_str(&format!(": {}", ty_name(ret)));
        }
    }
    if ty.nullable {
        out.push('?');
    }
    out
}

fn fn_sig(name: &str, f: &A::FnDecl) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| match &p.ty {
            Some(t) => format!("{}: {}", p.name, ty_name(t)),
            None => p.name.clone(),
        })
        .collect();
    let mut out = format!("fn {name}({})", params.join(", "));
    if let Some(ret) = &f.ret {
        out.push_str(&format!(": {}", ty_name(ret)));
    }
    if f.throws {
        out.push_str(" throws");
    }
    out
}

fn is_pub(access: &A::Access) -> bool {
    matches!(access, A::Access::Export)
}

pub fn collect_module(name: &str, module: &A::Module, include_private: bool) -> DocModuleDoc {
    let mut doc = DocModuleDoc {
        name: name.to_string(),
        path: name.replace('.', "/"),
        docs: module.docs.clone(),
        ..Default::default()
    };
    let mut extensions: Vec<(A::Type, Vec<A::Spanned<A::ClassMember>>)> = Vec::new();
    for decl in &module.decls {
        match &decl.node {
            A::Decl::Fn(f) => {
                if f.is_test || (!include_private && !is_pub(&f.access)) {
                    continue;
                }
                doc.fns.push(DocFn { name: f.name.clone(), sig: fn_sig(&f.name, f), docs: f.docs.clone() });
            }
            A::Decl::Class { access, name, members, docs, .. } => {
                if !include_private && !is_pub(access) {
                    continue;
                }
                doc.classes.push(collect_class(name, members, docs, include_private));
            }
            A::Decl::Enum { access, name, members, docs, .. } => {
                if !include_private && !is_pub(access) {
                    continue;
                }
                let mut variants: Vec<DocVariant> = members
                    .iter()
                    .map(|m| DocVariant {
                        name: m.name.clone(),
                        payload: m.payload.iter().map(ty_name).collect(),
                        docs: m.docs.clone(),
                    })
                    .collect();
                variants.sort_by(|a, b| a.name.cmp(&b.name));
                doc.enums.push(DocEnum { name: name.clone(), docs: docs.clone(), variants });
            }
            A::Decl::Const { access, name, ty, docs, .. } => {
                if !include_private && !is_pub(access) {
                    continue;
                }
                let ty = ty.as_ref().map(ty_name).unwrap_or_else(|| "Any".to_string());
                doc.consts.push(DocConst { name: name.clone(), ty, docs: docs.clone() });
            }
            A::Decl::Extension { target, members, .. } => {
                extensions.push((target.clone(), members.clone()));
            }
            _ => {}
        }
    }
    for (target, members) in extensions {
        let Some(base) = target.path.last() else {
            continue;
        };
        let Some(cls) = doc.classes.iter_mut().find(|c| &c.name == base) else {
            continue;
        };
        for mem in &members {
            if let A::ClassMember::Method(f) = &mem.node {
                if !include_private && !is_pub(&f.access) {
                    continue;
                }
                if cls.methods.iter().any(|m| m.name == f.name) {
                    continue;
                }
                cls.methods.push(DocMethod {
                    name: f.name.clone(),
                    sig: fn_sig(&f.name, f),
                    docs: f.docs.clone(),
                });
            }
        }
        cls.methods.sort_by(|a, b| a.name.cmp(&b.name));
    }
    doc.fns.sort_by(|a, b| a.name.cmp(&b.name));
    doc.classes.sort_by(|a, b| a.name.cmp(&b.name));
    doc.enums.sort_by(|a, b| a.name.cmp(&b.name));
    doc.consts.sort_by(|a, b| a.name.cmp(&b.name));
    doc
}

fn collect_class(
    name: &str,
    members: &[A::Spanned<A::ClassMember>],
    docs: &str,
    include_private: bool,
) -> DocClass {
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut init = None;
    for mem in members {
        match &mem.node {
            A::ClassMember::Field(f) => {
                let ty = f.ty.as_ref().map(ty_name).unwrap_or_else(|| "Any".to_string());
                fields.push(DocField { name: f.name.clone(), ty, docs: f.docs.clone() });
            }
            A::ClassMember::Method(f) => {
                if !include_private && !is_pub(&f.access) {
                    continue;
                }
                methods.push(DocMethod { name: f.name.clone(), sig: fn_sig(&f.name, f), docs: f.docs.clone() });
            }
            A::ClassMember::Init { params, .. } => {
                let ps: Vec<String> = params
                    .iter()
                    .map(|p| match &p.ty {
                        Some(t) => format!("{}: {}", p.name, ty_name(t)),
                        None => p.name.clone(),
                    })
                    .collect();
                init = Some(format!("init({})", ps.join(", ")));
            }
            _ => {}
        }
    }
    fields.sort_by(|a, b| a.name.cmp(&b.name));
    methods.sort_by(|a, b| a.name.cmp(&b.name));
    DocClass { name: name.to_string(), docs: docs.to_string(), fields, init, methods }
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn docs_html(docs: &str) -> String {
    let parsed = parse_jsdoc(docs);
    let mut out = String::new();
    let desc = parsed.desc.trim();
    if !desc.is_empty() {
        out.push_str("<p class=\"docs\">");
        for (i, para) in desc.split("\n\n").enumerate() {
            if i > 0 {
                out.push_str("</p><p class=\"docs\">");
            }
            out.push_str(&inline_html(para));
        }
        out.push_str("</p>");
    }
    if !parsed.tags.is_empty() {
        out.push_str("<dl class=\"tags\">\n");
        for tag in &parsed.tags {
            out.push_str(&tag_html(tag));
        }
        out.push_str("</dl>\n");
    }
    out
}

fn inline_html(s: &str) -> String {
    esc(s).replace('\n', "<br>")
}

#[derive(Clone, Debug, PartialEq)]
struct JsDocTag {
    kind: String,
    name: String,
    text: String,
}

#[derive(Clone, Debug, Default)]
struct JsDoc {
    desc: String,
    tags: Vec<JsDocTag>,
}

fn parse_jsdoc(docs: &str) -> JsDoc {
    let mut out = JsDoc::default();
    let mut desc_lines: Vec<&str> = Vec::new();
    let mut cur: Option<JsDocTag> = None;
    for line in docs.split('\n') {
        let trimmed = line.trim();
        let mut tag_rest = None;
        if let Some(rest) = trimmed.strip_prefix('@')
            && rest.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        {
            tag_rest = Some(rest);
        }
        if let Some(rest) = tag_rest {
            if let Some(tag) = cur.take() {
                out.tags.push(tag);
            }
            let mut parts = rest.splitn(2, char::is_whitespace);
            let kind = parts.next().unwrap_or("").to_string();
            let tail = parts.next().unwrap_or("").trim().to_string();
            let (name, text) = if kind == "param" {
                let mut head = tail.splitn(2, char::is_whitespace);
                (
                    head.next().unwrap_or("").to_string(),
                    head.next().unwrap_or("").trim().to_string(),
                )
            } else {
                (String::new(), tail)
            };
            cur = Some(JsDocTag { kind, name, text });
            continue;
        }
        if let Some(tag) = cur.as_mut() {
            if !tag.text.is_empty() {
                tag.text.push('\n');
            }
            tag.text.push_str(trimmed);
        } else {
            desc_lines.push(line);
        }
    }
    if let Some(tag) = cur.take() {
        out.tags.push(tag);
    }
    while desc_lines.first().is_some_and(|l| l.trim().is_empty()) {
        desc_lines.remove(0);
    }
    while desc_lines.last().is_some_and(|l| l.trim().is_empty()) {
        desc_lines.pop();
    }
    out.desc = desc_lines.join("\n");
    out
}

fn tag_html(tag: &JsDocTag) -> String {
    match tag.kind.as_str() {
        "param" => format!(
            "<dt><code>{}</code></dt><dd>{}</dd>\n",
            esc(&tag.name),
            inline_html(&tag.text)
        ),
        "returns" | "return" => {
            format!("<dt>Returns</dt><dd>{}</dd>\n", inline_html(&tag.text))
        }
        "throws" | "error" => {
            format!("<dt>Throws</dt><dd>{}</dd>\n", inline_html(&tag.text))
        }
        "example" => {
            format!("<dt>Example</dt><dd><pre><code>{}</code></pre></dd>\n", esc(&tag.text))
        }
        _ => format!(
            "<dt>@{} {}</dt><dd>{}</dd>\n",
            esc(&tag.kind),
            esc(tag.name.trim()),
            inline_html(&tag.text)
        ),
    }
}

pub fn render_css() -> String {
    ":root{color-scheme:light dark}*{box-sizing:border-box}body{margin:0;font-family:system-ui,-apple-system,\"Segoe UI\",Roboto,sans-serif;line-height:1.6;color:#1a1a1a;background:#fff}nav{background:#f4f4f5;border-bottom:1px solid #e4e4e7;padding:.6rem 1.2rem}nav a{color:#2563eb;text-decoration:none;font-weight:600}main{max-width:56rem;margin:0 auto;padding:1.5rem 1.2rem 4rem}h1,h2,h3{line-height:1.25}h2{margin-top:2.5rem;border-bottom:1px solid #e4e4e7;padding-bottom:.4rem}code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.9em;background:#f4f4f5;padding:.1em .35em;border-radius:.25rem}pre{background:#f4f4f5;padding:1rem;border-radius:.5rem;overflow-x:auto}section.item{border:1px solid #e4e4e7;border-radius:.5rem;padding:.8rem 1rem;margin:.8rem 0}section.item h3{margin:.2rem 0 .5rem}p.docs{margin:.4rem 0}dl.tags{margin:.4rem 0}dl.tags dt{font-weight:600;margin-top:.35rem}dl.tags dd{margin:.1rem 0 .3rem 1rem}dl.tags pre{margin:.3rem 0}ul.fields{list-style:none;padding-left:0}ul.fields li{margin:.25rem 0}.sig{font-size:1.02em}footer{margin-top:3rem;color:#71717a;font-size:.85em}@media(prefers-color-scheme:dark){body{color:#e4e4e7;background:#18181b}nav{background:#27272a;border-color:#3f3f46}h2{border-color:#3f3f46}code,pre{background:#27272a}section.item{border-color:#3f3f46}nav a{color:#93c5fd}}\n".to_string()
}

fn page_shell(title: &str, pkg: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<link rel=\"stylesheet\" href=\"style.css\">\n</head>\n<body><nav><a href=\"index.html\">{}</a></nav>\n<main>\n{}\n</main></body>\n</html>\n",
        esc(title),
        esc(pkg),
        body
    )
}

pub fn render_module(pkg: &str, m: &DocModuleDoc) -> String {
    let mut body = format!("<h1>module <code>{}</code></h1>\n", esc(&m.name));
    body.push_str(&docs_html(&m.docs));
    if !m.fns.is_empty() {
        body.push_str("<h2>Functions</h2>\n");
        for f in &m.fns {
            body.push_str(&format!(
                "<section class=\"item\"><h3 class=\"sig\"><code>{}</code></h3>\n{}</section>\n",
                esc(&f.sig),
                docs_html(&f.docs)
            ));
        }
    }
    if !m.classes.is_empty() {
        body.push_str("<h2>Classes</h2>\n");
        for c in &m.classes {
            body.push_str(&format!(
                "<section class=\"item\"><h3><code>class {}</code></h3>\n{}",
                esc(&c.name),
                docs_html(&c.docs)
            ));
            if !c.fields.is_empty() {
                body.push_str("<h4>Fields</h4>\n<ul class=\"fields\">\n");
                for f in &c.fields {
                    body.push_str(&format!(
                        "<li><code>{}: {}</code>{}</li>\n",
                        esc(&f.name),
                        esc(&f.ty),
                        docs_html(&f.docs)
                    ));
                }
                body.push_str("</ul>\n");
            }
            if let Some(init) = &c.init {
                body.push_str(&format!("<h4>Constructor</h4>\n<p><code>{}</code></p>\n", esc(init)));
            }
            if !c.methods.is_empty() {
                body.push_str("<h4>Methods</h4>\n");
                for mt in &c.methods {
                    body.push_str(&format!(
                        "<p><code>{}</code></p>\n{}",
                        esc(&mt.sig),
                        docs_html(&mt.docs)
                    ));
                }
            }
            body.push_str("</section>\n");
        }
    }
    if !m.enums.is_empty() {
        body.push_str("<h2>Enums</h2>\n");
        for e in &m.enums {
            body.push_str(&format!(
                "<section class=\"item\"><h3><code>enum {}</code></h3>\n{}",
                esc(&e.name),
                docs_html(&e.docs)
            ));
            if !e.variants.is_empty() {
                body.push_str("<ul class=\"fields\">\n");
                for v in &e.variants {
                    let pay = if v.payload.is_empty() {
                        String::new()
                    } else {
                        format!("({})", v.payload.join(", "))
                    };
                    body.push_str(&format!(
                        "<li><code>{}{}</code>{}</li>\n",
                        esc(&v.name),
                        esc(&pay),
                        docs_html(&v.docs)
                    ));
                }
                body.push_str("</ul>\n");
            }
            body.push_str("</section>\n");
        }
    }
    if !m.consts.is_empty() {
        body.push_str("<h2>Constants</h2>\n<ul class=\"fields\">\n");
        for c in &m.consts {
            body.push_str(&format!(
                "<li><code>{}: {}</code>{}</li>\n",
                esc(&c.name),
                esc(&c.ty),
                docs_html(&c.docs)
            ));
        }
        body.push_str("</ul>\n");
    }
    page_shell(&format!("{pkg} :: {}", m.name), pkg, &body)
}

pub fn render_index(
    pkg: &str,
    version: &str,
    description: &str,
    modules: &[DocModuleDoc],
) -> String {
    let mut body = format!("<h1><code>{}</code> {}</h1>\n", esc(pkg), esc(version));
    if !description.is_empty() {
        body.push_str(&docs_html(description));
    }
    body.push_str("<h2>Modules</h2>\n<ul class=\"fields\">\n");
    for m in modules {
        body.push_str(&format!(
            "<li><a href=\"{}.html\"><code>{}</code></a></li>\n",
            esc(&page_file(&m.name)),
            esc(&m.name)
        ));
    }
    body.push_str("</ul>\n");
    page_shell(pkg, pkg, &body)
}

pub fn page_file(name: &str) -> String {
    let safe: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if safe.is_empty() {
        "module".to_string()
    } else {
        safe
    }
}

pub fn render_workspace_index(
    members: &[(String, String, String)],
    css_rel: &str,
) -> String {
    let mut body = String::from("<h1>Workspace</h1>\n<h2>Members</h2>\n<ul class=\"fields\">\n");
    for (name, version, _) in members {
        body.push_str(&format!(
            "<li><a href=\"{0}/index.html\"><code>{0}</code></a> {1}</li>\n",
            esc(name),
            esc(version)
        ));
    }
    body.push_str("</ul>\n");
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Workspace</title>\n<link rel=\"stylesheet\" href=\"{}\">\n</head>\n<body><main>\n{}\n</main></body>\n</html>\n",
        css_rel, body
    )
}

pub fn generate_package_docs(
    manifest: &Manifest,
    modules: &[DocInput],
    out_dir: &Path,
    include_private: bool,
) -> Result<(), Diagnostic> {
    let project = manifest.project.as_ref().ok_or_else(|| {
        Diagnostic::new(Code::E108, "rnx doc needs a [project] manifest".to_string())
    })?;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot write {}: {e}", out_dir.display())))?;
    let mut docs: Vec<DocModuleDoc> = modules
        .iter()
        .map(|m| collect_module(&m.name, &m.module, include_private))
        .collect();
    docs.sort_by(|a, b| a.name.cmp(&b.name));
    for m in &docs {
        let page = render_module(&project.name, m);
        let path = out_dir.join(format!("{}.html", page_file(&m.name)));
        std::fs::write(&path, page)
            .map_err(|e| Diagnostic::new(Code::E108, format!("cannot write {}: {e}", path.display())))?;
    }
    let index = render_index(&project.name, &project.version, &project.description, &docs);
    let index_path = out_dir.join("index.html");
    std::fs::write(&index_path, index)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot write {}: {e}", index_path.display())))?;
    let css_path = out_dir.join("style.css");
    std::fs::write(&css_path, render_css())
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot write {}: {e}", css_path.display())))?;
    Ok(())
}

pub fn render_hover_markdown(signature: &str, docs: &str) -> String {
    let mut out = format!("```rasmalai\n{signature}\n```");
    let parsed = parse_jsdoc(docs);
    let desc = parsed.desc.trim();
    if !desc.is_empty() {
        out.push_str("\n\n");
        out.push_str(&md_prose(desc));
    }
    let mut params = Vec::new();
    let mut returns = Vec::new();
    let mut throws = Vec::new();
    let mut examples = Vec::new();
    let mut other = Vec::new();
    for tag in &parsed.tags {
        match tag.kind.as_str() {
            "param" => params.push(tag),
            "returns" | "return" => returns.push(tag),
            "throws" | "error" => throws.push(tag),
            "example" => examples.push(tag),
            _ => other.push(tag),
        }
    }
    if !params.is_empty() {
        out.push_str("\n\n**Parameters**\n\n");
        for tag in params {
            out.push_str(&format!("- `{}` — {}\n", tag.name, md_prose(&tag.text)));
        }
    }
    if !returns.is_empty() {
        out.push_str("\n**Returns**\n\n");
        for tag in returns {
            out.push_str(&md_prose(&tag.text));
            out.push('\n');
        }
    }
    if !throws.is_empty() {
        out.push_str("\n**Throws**\n\n");
        for tag in throws {
            out.push_str(&md_prose(&tag.text));
            out.push('\n');
        }
    }
    for tag in examples {
        out.push_str("\n**Example**\n\n```rasmalai\n");
        out.push_str(tag.text.trim());
        out.push_str("\n```\n");
    }
    for tag in other {
        out.push_str(&format!("\n*@{} {}* — {}\n", tag.kind, tag.name.trim(), md_prose(&tag.text)));
    }
    out
}

fn md_prose(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn js_tag_json(tag: &JsDocTag) -> String {
    format!(
        "{{\"kind\":\"{}\",\"name\":\"{}\",\"text\":\"{}\"}}",
        diagnostics::escape_json(&tag.kind),
        diagnostics::escape_json(&tag.name),
        diagnostics::escape_json(&tag.text),
    )
}

fn jsdoc_json(docs: &str) -> String {
    let parsed = parse_jsdoc(docs);
    let mut out = String::from("{\"description\":\"");
    out.push_str(&diagnostics::escape_json(parsed.desc.trim()));
    out.push_str("\",\"tags\":[");
    for (i, tag) in parsed.tags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&js_tag_json(tag));
    }
    out.push_str("]}");
    out
}

impl DocFn {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"name\":\"{}\",\"sig\":\"{}\",\"docs\":{}}}",
            diagnostics::escape_json(&self.name),
            diagnostics::escape_json(&self.sig),
            jsdoc_json(&self.docs),
        )
    }
}

impl DocField {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"name\":\"{}\",\"ty\":\"{}\",\"docs\":{}}}",
            diagnostics::escape_json(&self.name),
            diagnostics::escape_json(&self.ty),
            jsdoc_json(&self.docs),
        )
    }
}

impl DocMethod {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"name\":\"{}\",\"sig\":\"{}\",\"docs\":{}}}",
            diagnostics::escape_json(&self.name),
            diagnostics::escape_json(&self.sig),
            jsdoc_json(&self.docs),
        )
    }
}

impl DocClass {
    pub fn to_json(&self) -> String {
        let mut out = format!(
            "{{\"name\":\"{}\",\"docs\":{},\"init\":",
            diagnostics::escape_json(&self.name),
            jsdoc_json(&self.docs),
        );
        match &self.init {
            Some(init) => {
                out.push('"');
                out.push_str(&diagnostics::escape_json(init));
                out.push('"');
            }
            None => out.push_str("null"),
        }
        out.push_str(",\"fields\":[");
        for (i, f) in self.fields.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&f.to_json());
        }
        out.push_str("],\"methods\":[");
        for (i, m) in self.methods.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&m.to_json());
        }
        out.push_str("]}");
        out
    }
}

impl DocVariant {
    pub fn to_json(&self) -> String {
        let mut out = format!(
            "{{\"name\":\"{}\",\"payload\":[",
            diagnostics::escape_json(&self.name),
        );
        for (i, p) in self.payload.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(&diagnostics::escape_json(p));
            out.push('"');
        }
        out.push_str("],\"docs\":");
        out.push_str(&jsdoc_json(&self.docs));
        out.push('}');
        out
    }
}

impl DocEnum {
    pub fn to_json(&self) -> String {
        let mut out = format!(
            "{{\"name\":\"{}\",\"docs\":{},\"variants\":[",
            diagnostics::escape_json(&self.name),
            jsdoc_json(&self.docs),
        );
        for (i, v) in self.variants.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&v.to_json());
        }
        out.push_str("]}");
        out
    }
}

impl DocConst {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"name\":\"{}\",\"ty\":\"{}\",\"docs\":{}}}",
            diagnostics::escape_json(&self.name),
            diagnostics::escape_json(&self.ty),
            jsdoc_json(&self.docs),
        )
    }
}

impl DocModuleDoc {
    pub fn to_json(&self) -> String {
        let mut out = format!(
            "{{\"name\":\"{}\",\"path\":\"{}\",\"docs\":{},\"functions\":[",
            diagnostics::escape_json(&self.name),
            diagnostics::escape_json(&self.path),
            jsdoc_json(&self.docs),
        );
        for (i, f) in self.fns.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&f.to_json());
        }
        out.push_str("],\"classes\":[");
        for (i, c) in self.classes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&c.to_json());
        }
        out.push_str("],\"enums\":[");
        for (i, e) in self.enums.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&e.to_json());
        }
        out.push_str("],\"constants\":[");
        for (i, c) in self.consts.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&c.to_json());
        }
        out.push_str("]}");
        out
    }
}

pub fn modules_to_json(modules: &[DocModuleDoc]) -> String {
    let mut out = String::from("{\"modules\":[");
    for (i, m) in modules.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&m.to_json());
    }
    out.push_str("]}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_docs_render_without_tag_list() {
        let html = docs_html("Adds two numbers together.");
        assert!(html.contains("Adds two numbers together."));
        assert!(!html.contains("<dl"));
    }

    #[test]
    fn param_and_returns_tags_render() {
        let html = docs_html("Quote for ore.\n@param base price per unit.\n@param demand buyer pressure.\n@returns scaled quote.");
        assert!(html.contains("<p class=\"docs\">Quote for ore.</p>"));
        assert!(html.contains("<dt><code>base</code></dt><dd>price per unit.</dd>"));
        assert!(html.contains("<dt><code>demand</code></dt><dd>buyer pressure.</dd>"));
        assert!(html.contains("<dt>Returns</dt><dd>scaled quote.</dd>"));
    }

    #[test]
    fn example_tag_renders_verbatim_and_escapes_html() {
        let html = docs_html("Usage.\n@example\nquote(1.0, 2, <3)");
        assert!(html.contains("<pre><code>quote(1.0, 2, &lt;3)</code></pre>"));
    }

    #[test]
    fn unknown_tags_render_generically_and_nothing_drops() {
        let html = docs_html("Old helper.\n@since 1.0.0\n@deprecated use quote instead.");
        assert!(html.contains("@since"));
        assert!(html.contains("1.0.0"));
        assert!(html.contains("@deprecated"));
        assert!(html.contains("use quote instead."));
    }

    #[test]
    fn at_sign_mid_line_stays_description() {
        let html = docs_html("Ping me at home for help.");
        assert!(!html.contains("<dl"));
        assert!(html.contains("Ping me at home for help."));
    }

    #[test]
    fn empty_docs_render_empty() {
        assert_eq!(docs_html(""), String::new());
    }

    #[test]
    fn hover_renders_ts_style_sections() {
        let md = render_hover_markdown(
            "fn quote(base: Float, demand: Int): Float",
            "Quote for one unit of ore.\n@param base price per unit.\n@returns scaled quote.\n@example\nquote(2.0, 3)",
        );
        assert!(md.contains("```rasmalai\nfn quote(base: Float, demand: Int): Float\n```"));
        assert!(md.contains("Quote for one unit of ore."));
        assert!(md.contains("**Parameters**"));
        assert!(md.contains("- `base` — price per unit."));
        assert!(md.contains("**Returns**"));
        assert!(md.contains("scaled quote."));
        assert!(md.contains("**Example**\n\n```rasmalai\nquote(2.0, 3)\n```"));
    }

    #[test]
    fn hover_without_docs_is_signature_only() {
        let md = render_hover_markdown("class Packet", "");
        assert_eq!(md, "```rasmalai\nclass Packet\n```");
    }

    #[test]
    fn nested_module_path_uses_slashes() {
        let module = crate::parser::Parser::parse_module("fn f(): Int { return 1; }").unwrap();
        let m = collect_module("std.net.http", &module, true);
        assert_eq!(m.name, "std.net.http");
        assert_eq!(m.path, "std/net/http");
        let json = m.to_json();
        assert!(json.contains("\"path\":\"std/net/http\""));
        let flat = collect_module("ore", &module, true);
        assert_eq!(flat.path, "ore");
    }

    #[test]
    fn module_json_exports_items_and_jsdoc_tags() {
        let m = DocModuleDoc {
            name: "ore".to_string(),
            path: "ore".to_string(),
            docs: "Mine docs.".to_string(),
            fns: vec![DocFn {
                name: "quote".to_string(),
                sig: "fn quote(base: Float): Float".to_string(),
                docs: "Quote ore.\n@param base price per unit.\n@returns scaled \"quote\".".to_string(),
            }],
            classes: Vec::new(),
            enums: Vec::new(),
            consts: vec![DocConst {
                name: "RATE".to_string(),
                ty: "Float".to_string(),
                docs: String::new(),
            }],
        };
        let json = m.to_json();
        assert!(json.contains("\"name\":\"ore\""));
        assert!(json.contains("\"path\":\"ore\""));
        assert!(json.contains("\"sig\":\"fn quote(base: Float): Float\""));
        assert!(json.contains("\"kind\":\"param\""));
        assert!(json.contains("\"name\":\"base\""));
        assert!(json.contains("\"kind\":\"returns\""));
        assert!(json.contains("scaled \\\"quote\\\""));
        assert!(json.contains("\"constants\":[{\"name\":\"RATE\""));
    }
}
