use crate::ast::*;
use diagnostics::{decl_file, tag_new, Code, DeclFiles, Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};

mod lints;
mod walker;

use self::lints::{check_cyclic_defaults, check_w104, check_w108, check_w109};

pub fn check(module: &Module) -> Vec<Diagnostic> {
    check_with_files(module, &DeclFiles::new())
}

pub fn check_with_files(module: &Module, files: &DeclFiles) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for decl in &module.decls {
        if let Decl::Class { name, members, .. } = &decl.node {
            let from = diags.len();
            check_w104(name, members, &mut diags);
            check_w109(name, members, &mut diags);
            tag_new(&mut diags, from, decl_file(files, decl.span));
        }
    }
    check_w108(module, &mut diags, files);
    check_cyclic_defaults(module, &mut diags, files);
    check_scopes(module, &mut diags, files);
    diags.sort_by(|a, b| {
        a.code
            .as_str()
            .cmp(b.code.as_str())
            .then(a.message.cmp(&b.message))
    });
    diags
}
pub(crate) const SCOPE_BUILTINS: &[&str] = &[
    "print",
    "assert",
    "typeOf",
    "IO",
    "Thread",
    "ThreadPool",
    "Pointer",
    "Address",
    "__testCheck",
];

fn starts_uppercase(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn canon_ty(name: &str) -> String {
    match crate::prelude::short_name(name) {
        "Char" => "String".to_string(),
        short => short.to_string(),
    }
}

const REMOVED_TYPES: &[(&str, &str)] = &[("Str", "String"), ("Poll", "Promise")];

fn is_removed_poll_base(e: &Expr) -> bool {
    matches!(e, Expr::Ident(n) if n == "Poll")
}

fn builtin_method_sig(base: &str, method: &str) -> Option<(usize, Vec<&'static str>, Option<&'static str>)> {
    match (base, method) {
        ("String", "length") => Some((0, Vec::new(), Some("Int"))),
        ("String", "len") => Some((0, Vec::new(), Some("Int"))),
        ("String", "slice") => Some((2, vec!["Int", "Int"], Some("String"))),
        ("String", "indexOf") => Some((1, vec!["String"], Some("Int"))),
        ("String", "trim") => Some((0, Vec::new(), Some("String"))),
        ("String", "concat") => Some((1, vec!["String"], Some("String"))),
        ("String", "charCodeAt") => Some((1, vec!["Int"], Some("Int"))),
        ("Int", "toString") => Some((0, Vec::new(), Some("String"))),
        ("Float", "toString") => Some((0, Vec::new(), Some("String"))),
        ("Float", "toBits") => Some((0, Vec::new(), Some("Int"))),
        ("Float", "asFast") => Some((0, Vec::new(), Some("FastFloat"))),
        ("Float", "asStrict") => Some((0, Vec::new(), Some("Float"))),
        ("Bool", "toString") => Some((0, Vec::new(), Some("String"))),
        ("Array", "length") => Some((0, Vec::new(), Some("Int"))),
        ("Array", "len") => Some((0, Vec::new(), Some("Int"))),
        ("Array", "isEmpty") => Some((0, Vec::new(), Some("Bool"))),
        ("Array", "push") => Some((1, Vec::new(), None)),
        ("Array", "pop") => Some((0, Vec::new(), None)),
        ("Result", "isOk") => Some((0, Vec::new(), Some("Bool"))),
        ("Result", "isErr") => Some((0, Vec::new(), Some("Bool"))),
        ("Result", "unwrap") => Some((0, Vec::new(), None)),
        ("Result", "unwrapOr") => Some((1, Vec::new(), None)),
        _ => None,
    }
}

fn is_higher_order(base: &str, method: &str) -> bool {
    (base == "Array" || base == "Result") && method == "map"
        || (base == "Array" && method == "filter")
}

fn removed_replacement(name: &str) -> Option<&'static str> {
    REMOVED_TYPES.iter().find(|(old, _)| *old == name).map(|(_, new)| *new)
}

fn ty_name(t: &Type) -> Option<String> {
    if t.fn_sig.is_some() {
        return None;
    }
    if !t.tuple.is_empty() {
        return None;
    }
    let base = t.path.last().cloned()?;
    let short = crate::prelude::short_name(&base).to_string();
    if t.args.is_empty() {
        return Some(short);
    }
    let mut inner = Vec::with_capacity(t.args.len());
    for a in &t.args {
        inner.push(ty_name(a)?);
    }
    Some(format!("{short}<{}>", inner.join(", ")))
}

fn array_elem(t: &Type) -> Option<String> {
    let last = t.path.last()?;
    if crate::prelude::short_name(last) != "Array" {
        return None;
    }
    t.args.first().and_then(ty_name)
}

fn promise_inner(ty: &str) -> Option<String> {
    if crate::prelude::short_name(ty) != "Promise" {
        return None;
    }
    let start = ty.find('<')?;
    let end = ty.rfind('>')?;
    if end <= start + 1 {
        return None;
    }
    let inner = ty[start + 1..end].trim();
    if inner.is_empty() {
        return None;
    }
    Some(inner.to_string())
}

struct VarInfo {
    ty: Option<String>,
    elem: Option<String>,
    mutable: bool,
}

struct ScopeCheck<'a> {
    module: &'a Module,
    globals: BTreeSet<String>,
    consts: BTreeSet<String>,
    let_names: BTreeSet<String>,
    classes: BTreeSet<String>,
    methods: BTreeMap<(String, String), (usize, usize)>,
    statics: BTreeSet<(String, String)>,
    fields: BTreeSet<(String, String)>,
    fn_rets: BTreeMap<String, Option<String>>,
    fn_arity: BTreeMap<String, (usize, usize)>,
    ctors: BTreeMap<String, (usize, usize)>,
    fn_params: BTreeMap<String, Vec<(String, Option<String>)>>,
    method_params: BTreeMap<(String, String), Vec<(String, Option<String>)>>,
    ctor_params: BTreeMap<String, Vec<(String, Option<String>)>>,
    ext_params: BTreeMap<(String, String), Vec<(String, Option<String>)>>,
    extensions: BTreeMap<(String, String), (usize, usize)>,
    conforms: BTreeMap<String, Vec<String>>,
    variants: BTreeMap<String, String>,
    async_fns: BTreeSet<String>,
    async_methods: BTreeSet<(String, String)>,
    scopes: Vec<BTreeMap<String, VarInfo>>,
    used_prelude: BTreeSet<String>,
    allow_this: bool,
    expected: Option<String>,
    enum_hint: Option<String>,
    stmt_span: Span,
    diags: Vec<Diagnostic>,
}
fn check_returns_for_fn(
    checker: &mut ScopeCheck,
    params: &[Param],
    ret: &Option<Type>,
    body: &FnBody,
    allow_this: bool,
    sig_span: Span,
) {
    checker.walk_fn_body(params, ret, body, allow_this, sig_span);
}

fn collect_let_names(module: &Module) -> BTreeSet<String> {
    fn block(b: &Block, out: &mut BTreeSet<String>) {
        for s in &b.stmts {
            stmt(&s.node, out);
        }
    }
    fn body(b: &FnBody, out: &mut BTreeSet<String>) {
        match b {
            FnBody::Block(b) => block(b, out),
            FnBody::Expr(_) => {}
        }
    }
    fn stmt(s: &Stmt, out: &mut BTreeSet<String>) {
        match s {
            Stmt::Var { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::DestructureTuple { names, .. } => {
                for n in names {
                    out.insert(n.clone());
                }
            }
            Stmt::DestructureArray { names, rest, .. } => {
                for n in names {
                    out.insert(n.clone());
                }
                if let Some(r) = rest {
                    out.insert(r.clone());
                }
            }
            Stmt::DestructureRecord { fields, rest, .. } => {
                for (_, local) in fields {
                    out.insert(local.clone());
                }
                if let Some(r) = rest {
                    out.insert(r.clone());
                }
            }
            Stmt::If { then, otherwise, .. } => {
                block(then, out);
                if let Some(e) = otherwise {
                    match e {
                        Else::Block(b) => block(b, out),
                        Else::If(s) => stmt(&s.node, out),
                    }
                }
            }
            Stmt::For { binding, body: b, .. } => {
                match binding {
                    ForBinding::One(n) => {
                        out.insert(n.clone());
                    }
                    ForBinding::Many(ns) => {
                        for n in ns {
                            out.insert(n.clone());
                        }
                    }
                }
                block(b, out);
            }
            Stmt::While { body: b, .. } | Stmt::DoWhile { body: b, .. } => block(b, out),
            Stmt::Switch { cases, default, .. } => {
                for c in cases {
                    for s in &c.body {
                        stmt(&s.node, out);
                    }
                }
                if let Some(d) = default {
                    for s in d {
                        stmt(&s.node, out);
                    }
                }
            }
            Stmt::Defer(b) | Stmt::UnsafeBlock(b) => block(b, out),
            Stmt::Guard { name, otherwise, .. } => {
                out.insert(name.clone());
                block(otherwise, out);
            }
            Stmt::Try { body: b, catch, finally, .. } => {
                block(b, out);
                if let Some((n, cb)) = catch {
                    out.insert(n.clone());
                    block(cb, out);
                }
                if let Some(f) = finally {
                    block(f, out);
                }
            }
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    for decl in &module.decls {
        match &decl.node {
            Decl::Fn(f) => body(&f.body, &mut out),
            Decl::Class { members, .. } | Decl::Struct { members, .. } => {
                for m in members {
                    match &m.node {
                        ClassMember::Method(f) => body(&f.body, &mut out),
                        ClassMember::Init { body: b, .. } => block(b, &mut out),
                        ClassMember::Deinit(b) => block(b, &mut out),
                        ClassMember::OnReload { body: b, .. } => block(b, &mut out),
                        _ => {}
                    }
                }
            }
            Decl::Stmt(s) => stmt(&s.node, &mut out),
            _ => {}
        }
    }
    out
}

fn run_scope_check<'m>(module: &'m Module, files: &DeclFiles) -> ScopeCheck<'m> {
    let mut globals = BTreeSet::new();
    let mut consts = BTreeSet::new();
    let mut classes = BTreeSet::new();
    let mut fn_rets: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut async_fns: BTreeSet<String> = BTreeSet::new();
    let mut async_methods: BTreeSet<(String, String)> = BTreeSet::new();
    let mut conforms: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for decl in &module.decls {
        match &decl.node {
            Decl::Fn(f) => {
                globals.insert(f.name.clone());
                fn_rets.insert(f.name.clone(), f.ret.as_ref().and_then(ty_name));
                if f.is_async {
                    async_fns.insert(f.name.clone());
                }
            }
            Decl::Class { name, with, .. } => {
                globals.insert(name.clone());
                classes.insert(name.clone());
                let cshort = crate::prelude::short_name(name).to_string();
                for t in with {
                    if let Some(last) = t.path.last() {
                        let ishort =
                            last.rsplit('.').next().unwrap_or(last).to_string();
                        conforms.entry(cshort.clone()).or_default().push(ishort);
                    }
                }
            }
            Decl::Struct { name, .. } => {
                globals.insert(name.clone());
            }
            Decl::Enum { name, .. } | Decl::Record { name, .. } => {
                globals.insert(name.clone());
            }
            Decl::Const { name, .. } => {
                globals.insert(name.clone());
                consts.insert(name.clone());
            }
            Decl::Import(d) => {
                match &d.clause {
                    ImportClause::Named(specs) | ImportClause::DefaultAndNamed(_, specs) => {
                        for spec in specs {
                            globals.insert(crate::modules::import_local_name(spec));
                            if let Some(sig) = &spec.native_fn {
                                let local = sig.alias.clone().unwrap_or_else(|| sig.name.clone());
                                fn_rets.insert(local, sig.ret.as_ref().and_then(ty_name));
                            }
                        }
                    }
                    _ => {}
                }
                match &d.clause {
                    ImportClause::Namespace(ns) | ImportClause::Default(ns) => {
                        globals.insert(ns.clone());
                    }
                    ImportClause::DefaultAndNamed(d, _) => {
                        globals.insert(d.clone());
                    }
                    _ => {}
                }
                if let ImportSource::Native(_) = &d.source {
                    for local in crate::modules::native_import_locals(&d.clause) {
                        globals.insert(local);
                    }
                }
            }
            Decl::ExportFrom(d) => {
                if let ImportSource::Native(_) = &d.source
                    && let ExportClause::Native(sigs) = &d.clause
                {
                    for sig in sigs {
                        let local = sig.alias.clone().unwrap_or_else(|| sig.name.clone());
                        globals.insert(local.clone());
                        fn_rets.insert(local, sig.ret.as_ref().and_then(ty_name));
                    }
                }
            }
            Decl::ExportList(specs) => {
                for spec in specs {
                    globals.insert(spec.alias.clone().unwrap_or_else(|| spec.name.clone()));
                }
            }
            Decl::ExportDefault(_) => {
                globals.insert("__default_export".to_string());
                consts.insert("__default_export".to_string());
            }
            _ => {}
        }
    }
    for decl in &module.decls {
        if let Decl::Enum { members, .. } = &decl.node {
            for m in members {
                globals.insert(m.name.clone());
            }
        }
    }
    let mut variants: BTreeMap<String, String> = BTreeMap::new();
    for decl in &module.decls {
        if let Decl::Enum { name, members, .. } = &decl.node {
            let eshort = crate::prelude::short_name(name).to_string();
            for m in members {
                variants.insert(
                    crate::prelude::short_name(&m.name).to_string(),
                    eshort.clone(),
                );
            }
        }
    }
    let mut methods: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    let mut statics: BTreeSet<(String, String)> = BTreeSet::new();
    let mut method_params: BTreeMap<(String, String), Vec<(String, Option<String>)>> =
        BTreeMap::new();
    let mut fields: BTreeSet<(String, String)> = BTreeSet::new();
    let mut fn_arity: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut ctors: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut fn_params: BTreeMap<String, Vec<(String, Option<String>)>> = BTreeMap::new();
    let mut ctor_params: BTreeMap<String, Vec<(String, Option<String>)>> = BTreeMap::new();
    for decl in &module.decls {
        if let Decl::Fn(f) = &decl.node {
            let total = f.params.len();
            let mandatory = f.params.iter().filter(|p| p.default.is_none()).count();
            fn_arity.insert(f.name.clone(), (mandatory, total));
            fn_params.insert(
                f.name.clone(),
                f.params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.as_ref().and_then(ty_name)))
                    .collect(),
            );
        }
    }
    for decl in &module.decls {
        let (name, members) = match &decl.node {
            Decl::Class { name, members, .. }
            | Decl::Struct { name, members, .. }
            | Decl::Trait { name, members, .. }
            | Decl::Interface { name, members, .. } => (name, members),
            _ => continue,
        };
        let cshort = crate::prelude::short_name(name).to_string();
        for m in members {
            match &m.node {
                ClassMember::Method(f) => {
                    let total = f.params.len();
                    let mandatory = f.params.iter().filter(|p| p.default.is_none()).count();
                    if f.is_static {
                        statics.insert((cshort.clone(), f.name.clone()));
                    }
                    methods.insert((cshort.clone(), f.name.clone()), (mandatory, total));
                    method_params.insert(
                        (cshort.clone(), f.name.clone()),
                        f.params
                            .iter()
                            .map(|p| (p.name.clone(), p.ty.as_ref().and_then(ty_name)))
                            .collect(),
                    );
                    if f.is_async {
                        async_methods.insert((cshort.clone(), f.name.clone()));
                    }
                }
                ClassMember::Field(f) => {
                    fields.insert((cshort.clone(), f.name.clone()));
                }
                ClassMember::Init { params, .. } => {
                    let total = params.len();
                    let mandatory = params.iter().filter(|p| p.default.is_none()).count();
                    ctors.insert(cshort.clone(), (mandatory, total));
                    ctor_params.insert(
                        cshort.clone(),
                        params
                            .iter()
                            .map(|p| (p.name.clone(), p.ty.as_ref().and_then(ty_name)))
                            .collect(),
                    );
                }
                _ => {}
            }
        }
    }
    let mut links: Vec<(String, String)> = Vec::new();
    for decl in &module.decls {
        if let Decl::Class { name, extends, with, .. } = &decl.node {
            let cshort = crate::prelude::short_name(name).to_string();
            if let Some(p) = extends {
                if let Some(last) = p.path.last() {
                    links.push((cshort.clone(), crate::prelude::short_name(last).to_string()));
                }
            }
            for t in with {
                if let Some(last) = t.path.last() {
                    links.push((cshort.clone(), crate::prelude::short_name(last).to_string()));
                }
            }
        }
    }
    links.sort();
    links.dedup();
    loop {
        let mut changed = false;
        for (child, parent) in &links {
            if child == parent {
                continue;
            }
            for ((c, m), sig) in methods.clone() {
                if &c == parent && !methods.contains_key(&(child.clone(), m.clone())) {
                    methods.insert((child.clone(), m), sig);
                    changed = true;
                }
            }
            for (c, f) in fields.clone() {
                if &c == parent {
                    changed = fields.insert((child.clone(), f)) || changed;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut checker = ScopeCheck {
        module,
        globals,
        consts,
        let_names: collect_let_names(module),
        classes,
        methods,
        statics,
        fields,
        fn_rets,
        fn_arity,
        ctors,
        fn_params,
        method_params,
        ctor_params,
        ext_params: BTreeMap::new(),
        extensions: BTreeMap::new(),
        conforms,
        variants,
        async_fns,
        async_methods,
        scopes: Vec::new(),
        used_prelude: BTreeSet::new(),
        allow_this: false,
        expected: None,
        enum_hint: None,
        stmt_span: Span { start: 0, end: 0 },
        diags: Vec::new(),
    };
    for decl in &module.decls {
        if let Decl::Extension { target, members, .. } = &decl.node {
            let tname = target
                .path
                .last()
                .map(|s| s.rsplit('.').next().unwrap_or(s).to_string())
                .unwrap_or_default();
            for m in members {
                if let ClassMember::Method(f) = &m.node {
                    let total = f.params.len();
                    let mandatory = f.params.iter().filter(|p| p.default.is_none()).count();
                    checker.extensions.insert((tname.clone(), f.name.clone()), (mandatory, total));
                    checker.ext_params.insert(
                        (tname.clone(), f.name.clone()),
                        f.params
                            .iter()
                            .map(|p| (p.name.clone(), p.ty.as_ref().and_then(ty_name)))
                            .collect(),
                    );
                }
            }
        }
    }
    for decl in &module.decls {
        let from = checker.diags.len();
        let file = decl_file(files, decl.span);
        match &decl.node {
            Decl::Fn(f) => {
                check_returns_for_fn(&mut checker, &f.params, &f.ret, &f.body, false, decl.span);
            }
            Decl::Class { members, .. } | Decl::Struct { members, .. } => {
                let ctors = members
                    .iter()
                    .filter(|m| {
                        matches!(
                            &m.node,
                            ClassMember::Init { .. }
                        )
                    })
                    .count();
                if ctors > 1 {
                    checker.diags.push(
                        Diagnostic::new(Code::E108, "a class declares at most one constructor")
                            .with_span(checker.span(decl.span))
                            .with_hint("keep only one `init`, not both"),
                    );
                }
                for m in members {
                    match &m.node {
                        ClassMember::Method(f) => {
                            check_returns_for_fn(&mut checker, &f.params, &f.ret, &f.body, true, m.span);
                        }
                        ClassMember::Field(fd) => {
                            if let Some(t) = &fd.ty {
                                checker.deny_removed_ty(t, m.span);
                            }
                            if let Some(v) = &fd.value {
                                checker.walk_expr(v);
                            }
                        }
                        ClassMember::Init { params, body } => {
                            checker.walk_fn_body(params, &None, &FnBody::Block(body.clone()), true, m.span);
                        }
                        ClassMember::Deinit(b) => {
                            checker.scopes.push(BTreeMap::new());
                            let outer = checker.allow_this;
                            checker.allow_this = true;
                            checker.walk_block(b);
                            checker.allow_this = outer;
                            checker.scopes.pop();
                        }
                        ClassMember::OnReload { params, body } => {
                            checker.walk_fn_body(params, &None, &FnBody::Block(body.clone()), true, m.span);
                        }
                    }
                }
            }
            Decl::Const { ty, value, .. } => {
                if let Some(t) = ty {
                    checker.deny_removed_ty(t, decl.span);
                }
                checker.walk_expr(value);
            }
            Decl::Record { fields, .. } => {
                for f in fields {
                    checker.deny_removed_ty(&f.ty, decl.span);
                }
            }
            Decl::Enum { members, .. } => {
                for m in members {
                    for p in &m.payload {
                        checker.deny_removed_ty(p, decl.span);
                    }
                }
            }
            _ => {}
        }
        tag_new(&mut checker.diags, from, file);
    }
    checker
}

pub fn prelude_uses(module: &Module) -> BTreeSet<String> {
    run_scope_check(module, &DeclFiles::new()).used_prelude
}

fn check_scopes(module: &Module, diags: &mut Vec<Diagnostic>, files: &DeclFiles) {
    let checker = run_scope_check(module, files);
    diags.extend(checker.diags);
    diags.sort_by(|a, b| {
        a.code
            .as_str()
            .cmp(b.code.as_str())
            .then(a.message.cmp(&b.message))
    });
}
