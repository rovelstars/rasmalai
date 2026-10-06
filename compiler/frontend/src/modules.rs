use crate::ast as A;
use crate::project::{self, ProjectConfig, RegistryConfig};
use diagnostics::{Code, Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub struct ModuleFile {
    pub path: PathBuf,
    pub key: String,
    pub kind: ModuleKind,
    pub module: A::Module,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleKind {
    Entry,
    Imported,
}

pub struct ModuleGraph {
    pub root: PathBuf,
    pub files: Vec<ModuleFile>,
}

pub struct FnLine {
    pub line: u32,
    pub col: u32,
}

/// Module-path sources referenced by `import ... from "..."` and
/// `export ... from "..."` decls. Native sources (`from native "lib"`)
/// declare foreign functions locally and reference no file.
pub fn module_sources(decls: &[A::Spanned<A::Decl>]) -> Vec<(&str, Span)> {
    let mut out = Vec::new();
    for decl in decls {
        match &decl.node {
            A::Decl::Import(A::ImportDecl { source: A::ImportSource::Module(s), source_span, .. }) => {
                out.push((s.as_str(), *source_span));
            }
            A::Decl::ExportFrom(A::ExportFromDecl {
                source: A::ImportSource::Module(s),
                source_span,
                ..
            }) => {
                out.push((s.as_str(), *source_span));
            }
            _ => {}
        }
    }
    out
}

/// Local names declared by the named/default+named clause of a native import.
pub fn native_import_locals(clause: &A::ImportClause) -> Vec<String> {
    match clause {
        A::ImportClause::Named(specs) | A::ImportClause::DefaultAndNamed(_, specs) => {
            specs.iter().map(import_local_name).collect()
        }
        _ => Vec::new(),
    }
}

fn mangle(key: &str, name: &str) -> String {
    if key.is_empty() {
        name.to_string()
    } else {
        format!("{key}.{name}")
    }
}
/// Local name bound by an import specifier for module-local use.
pub fn import_local_name(spec: &A::ImportSpecifier) -> String {
    if let Some(sig) = &spec.native_fn {
        return sig.alias.clone().unwrap_or_else(|| sig.name.clone());
    }
    spec.alias.clone().unwrap_or_else(|| spec.name.clone())
}

/// Local names declared by a native function signature list.
pub fn native_local_names(sigs: &[A::NativeFnSig]) -> Vec<String> {
    sigs.iter().map(|s| s.alias.clone().unwrap_or_else(|| s.name.clone())).collect()
}

/// True for decls erased during qualify-and-merge (module re-exports,
/// local export lists, default-export expressions). Native declarations
/// are retained so later passes see the foreign signatures.
pub fn is_erasable_decl(decl: &A::Decl) -> bool {
    match decl {
        A::Decl::Import(d) => matches!(d.source, A::ImportSource::Module(_)),
        A::Decl::ExportFrom(d) => matches!(d.source, A::ImportSource::Module(_)),
        A::Decl::ExportList(_) | A::Decl::ExportDefault(_) => true,
        _ => false,
    }
}

pub fn merged_decl_files(graph: &ModuleGraph) -> diagnostics::DeclFiles {
    let mut out = diagnostics::DeclFiles::new();
    for f in &graph.files {
        for decl in &f.module.decls {
            if is_erasable_decl(&decl.node) {
                continue;
            }
            let key = (decl.span.start, decl.span.end);
            if out.contains_key(&key) {
                out.remove(&key);
            } else {
                out.insert(key, f.path.clone());
            }
        }
    }
    out
}

fn line_col(src: &str, offset: u32) -> (u32, u32) {
    let off = (offset as usize).min(src.len());
    let head = &src[..off];
    let line = head.bytes().filter(|&b| b == b'\n').count() as u32 + 1;
    let col = off as u32 - head.rfind('\n').map(|i| i as u32 + 1).unwrap_or(0) + 1;
    (line, col)
}

pub fn fn_lines(graph: &ModuleGraph) -> BTreeMap<String, FnLine> {
    let mut out = BTreeMap::new();
    for f in &graph.files {
        let src = std::fs::read_to_string(&f.path).unwrap_or_default();
        let mangled = |name: &str| {
            if f.key.is_empty() {
                name.to_string()
            } else {
                format!("{}.{}", f.key, name)
            }
        };
        for decl in &f.module.decls {
            match &decl.node {
                A::Decl::Fn(d) => {
                    let (line, col) = line_col(&src, decl.span.start);
                    out.insert(mangled(&d.name), FnLine { line, col });
                }
                A::Decl::Class { name, members, .. }
                | A::Decl::Struct { name, members, .. } => {
                    let cname = mangled(name);
                    for mem in members {
                        let key = match &mem.node {
                            A::ClassMember::Method(m) => format!("{cname}.{}", m.name),
                            A::ClassMember::Init { .. } => {
                                format!("{cname}.init")
                            }
                            A::ClassMember::Deinit(_) => format!("{cname}.deinit"),
                            A::ClassMember::OnReload { .. } => format!("{cname}.onReload"),
                            A::ClassMember::Field(_) => continue,
                        };
                        let (line, col) = line_col(&src, mem.span.start);
                        out.insert(key, FnLine { line, col });
                    }
                }
                _ => {}
            }
        }
    }
    out
}

impl ModuleGraph {
    /// Merged module from an inline source string: parses the user code,
    /// pulls transitively imported `@std/*` modules plus the ambient
    /// prelude from the embedded stdlib, and runs the same
    /// qualify-and-merge resolution as file-based builds. String-based
    /// entry points (`check_source`, `run_source`, playground) must use
    /// this instead of bare `parse_module`, otherwise prelude-backed
    /// lowering (Option/Result methods, `task.await()`, `Promise`)
    /// fails with `missing std.prelude.*`.
    pub fn from_source(src: &str) -> Result<A::Module, Diagnostic> {
        let user = crate::parser::Parser::parse_module(src)?;
        let root = PathBuf::from("inline/main.rnx");
        let mut files = vec![ModuleFile { path: root.clone(), key: String::new(), kind: ModuleKind::Entry, module: user }];
        let mut queue: Vec<String> = Vec::new();
        for (spec, _) in module_sources(&files[0].module.decls) {
            if is_std_spec(spec) {
                let rest = std_rest(spec);
                if !rest.is_empty() && !queue.iter().any(|n| n == rest) {
                    queue.push(rest.to_string());
                }
            }
        }
        if !queue.iter().any(|n| n == "prelude") {
            queue.push("prelude".to_string());
        }
        while let Some(name) = queue.pop() {
            if files.iter().any(|f| f.path == PathBuf::from(format!("@std/{name}"))) {
                continue;
            }
            let src = stdlib::source(&name).ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("unknown standard library module `@std/{name}`"))
            })?;
            let module = crate::parser::Parser::parse_module(src)?;
            for (spec, _) in module_sources(&module.decls) {
                if is_std_spec(spec) {
                    let rest = std_rest(spec);
                    if !files.iter().any(|f| f.path == PathBuf::from(format!("@std/{rest}")))
                        && !queue.iter().any(|n| n == rest)
                        && !rest.is_empty()
                    {
                        queue.push(rest.to_string());
                    }
                }
            }
            files.push(ModuleFile {
                path: PathBuf::from(format!("@std/{name}")),
                key: format!("std.{}", name.replace('/', ".")),
                kind: ModuleKind::Imported,
                module,
            });
        }
        ModuleGraph { root, files }.resolve()
    }

    pub fn build(root: &Path) -> Result<ModuleGraph, Diagnostic> {        ModuleGraph::build_with_extra(root, &[])
    }

    pub fn build_with_extra(root: &Path, extra: &[PathBuf]) -> Result<ModuleGraph, Diagnostic> {
        Self::build_collecting_extra(root, extra).map_err(|mut v| v.remove(0))
    }

    pub fn build_collecting(root: &Path) -> Result<ModuleGraph, Vec<Diagnostic>> {
        Self::build_collecting_extra(root, &[])
    }

    pub fn build_collecting_extra(
        root: &Path,
        extra: &[PathBuf],
    ) -> Result<ModuleGraph, Vec<Diagnostic>> {
        let root = canonical(root).map_err(|e| vec![e])?;
        let base = root.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let entry_root = package_ctx_of(&root)
            .map_err(|e| vec![e])?
            .map(|(r, _)| r);
        let mut order: Vec<ModuleFile> = Vec::new();
        let mut state: BTreeMap<PathBuf, u8> = BTreeMap::new();
        let mut stack: Vec<PathBuf> = Vec::new();
        if let Some(r) = entry_root.clone() {
            stack.push(r);
        }
        let mut parse_errors: Vec<Diagnostic> = Vec::new();
        visit(
            &root,
            &base,
            &root,
            entry_root.as_ref(),
            &mut state,
            &mut order,
            &mut stack,
            &mut parse_errors,
        )
        .map_err(|e| vec![e])?;
        if stdlib::source("prelude").is_some() {
            visit(
                &PathBuf::from("@std/prelude"),
                &base,
                &root,
                entry_root.as_ref(),
                &mut state,
                &mut order,
                &mut stack,
                &mut parse_errors,
            )
            .map_err(|e| vec![e])?;
        }
        let mut extras: Vec<PathBuf> = Vec::new();
        for e in extra {
            match canonical(e) {
                Ok(c) => extras.push(c),
                Err(_) => {
                    parse_errors.push(Diagnostic::new(
                        Code::E108,
                        format!("cannot read `{}`", e.display()),
                    ));
                }
            }
        }
        extras.sort();
        extras.dedup();
        for e in extras {
            visit(
                &e,
                &base,
                &root,
                entry_root.as_ref(),
                &mut state,
                &mut order,
                &mut stack,
                &mut parse_errors,
            )
            .map_err(|e| vec![e])?;
        }
        if parse_errors.is_empty() {
            Ok(ModuleGraph { root, files: order })
        } else {
            Err(parse_errors)
        }
    }

    pub fn resolve(&self) -> Result<A::Module, Diagnostic> {
        for f in &self.files {
            if f.kind != ModuleKind::Imported || is_virtual(&f.path) {
                continue;
            }
            for decl in &f.module.decls {
                if let A::Decl::Stmt(_) = &decl.node {
                    return Err(Diagnostic::new(
                        Code::E112,
                        "top-level imperative statements and top-level 'await' are only permitted in the root entrypoint file",
                    )
                    .with_span(decl.span)
                    .with_file(f.path.clone())
                    .with_hint("move execution logic into a function or class method"));
                }
            }
        }
        let base = self.root.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let index: BTreeMap<&PathBuf, usize> =
            self.files.iter().enumerate().map(|(i, f)| (&f.path, i)).collect();
        let mut exports = self.export_tables(&base, &index)?;
        let mut merged: Vec<A::Spanned<A::Decl>> = Vec::new();
        let mut root_attrs = Vec::new();
        let mut all_ns: BTreeMap<String, (String, String)> = BTreeMap::new();
        for (i, f) in self.files.iter().enumerate() {
            if f.path == self.root {
                root_attrs = f.module.inner_attrs.clone();
            }
            let (own, aliases, namespaces) = self.file_scope(i, &mut exports, &index, &base)?;
            let mut rw = Rewriter {
                own: &own,
                aliases: &aliases,
                namespaces: &namespaces,
                locals: Vec::new(),
            };
            for (alias, key) in &namespaces {
                if key.is_empty() {
                    continue;
                }
                all_ns.entry(ns_class_name(alias, key)).or_insert((alias.clone(), key.clone()));
            }
            for decl in &f.module.decls {
                if is_erasable_decl(&decl.node) {
                    if let A::Decl::ExportDefault(d) = &decl.node {
                        let span = decl.span;
                        merged.push(A::sp(
                            A::Decl::Const {
                                access: A::Access::Internal,
                                name: "__default_export".to_string(),
                                ty: None,
                                value: d.expr.clone(),
                                docs: String::new(),
                            },
                            span,
                        ));
                    }
                    continue;
                }
                let mut decl = decl.clone();
                rename_decl(&mut decl.node, &own);
                rw.decl(&mut decl.node);
                merged.push(decl);
            }
        }
        for synth in all_ns.keys() {
            merged.push(A::sp(
                A::Decl::Class {
                    access: A::Access::Internal,
                    name: synth.clone(),
                    type_params: Vec::new(),
                    extends: None,
                    with: Vec::new(),
                    members: Vec::new(),
                    docs: String::new(),
                },
                diagnostics::Span { start: 0, end: 0 },
            ));
        }
        let mut docs = Vec::new();
        for f in &self.files {
            if is_virtual(&f.path) {
                continue;
            }
            if !f.module.docs.is_empty() {
                docs.push(f.module.docs.clone());
            }
        }
        docs.sort();
        docs.dedup();
        let mut warnings = Vec::new();
        for f in &self.files {
            if is_virtual(&f.path) {
                continue;
            }
            let mut w = f.module.warnings.clone();
            diagnostics::tag_new(&mut w, 0, Some(f.path.clone()));
            warnings.extend(w);
        }
        Ok(A::Module {
            inner_attrs: root_attrs,
            decls: merged,
            docs: docs.join("\n\n"),
            warnings,
        })
    }

    fn export_tables(
        &self,
        base: &Path,
        index: &BTreeMap<&PathBuf, usize>,
    ) -> Result<Vec<BTreeMap<String, (String, bool)>>, Diagnostic> {
        let mut exports: Vec<BTreeMap<String, (String, bool)>> = Vec::new();
        // bool marks explicitly `private` items, which never cross files (E203).
        for f in &self.files {
            let mut table = BTreeMap::new();
            for decl in &f.module.decls {
                match &decl.node {
                    A::Decl::Fn(d) => {
                        let mangled = mangle(&f.key, &d.name);
                        table.insert(d.name.clone(), (mangled, d.access == A::Access::Private));
                    }
                    A::Decl::Class { access, name, .. }
                    | A::Decl::Struct { access, name, .. }
                    | A::Decl::Enum { access, name, .. }
                    | A::Decl::Interface { access, name, .. }
                    | A::Decl::Record { access, name, .. } => {
                        let mangled = mangle(&f.key, name);
                        table.insert(name.clone(), (mangled, *access == A::Access::Private));
                    }
                    A::Decl::Const { access, name, .. } => {
                        let mangled = mangle(&f.key, name);
                        table.insert(name.clone(), (mangled, *access == A::Access::Private));
                    }
                    A::Decl::Import(d) => {
                        if let A::ImportSource::Native(_) = d.source {
                            for local in native_import_locals(&d.clause) {
                                table.insert(local.clone(), (local, false));
                            }
                        }
                    }
                    A::Decl::ExportFrom(d) => {
                        if let A::ImportSource::Native(_) = d.source
                            && let A::ExportClause::Native(sigs) = &d.clause
                        {
                            for local in native_local_names(sigs) {
                                table.insert(local.clone(), (local, false));
                            }
                        }
                    }
                    _ => {}
                }
            }
            exports.push(table);
        }
        for (i, f) in self.files.iter().enumerate() {
            let mut reexp: BTreeMap<String, (String, bool)> = BTreeMap::new();
            for decl in &f.module.decls {
                match &decl.node {
                    A::Decl::ExportFrom(d) => {
                        if let A::ImportSource::Module(source) = &d.source {
                            let target = resolve_source(&f.path, base, source)
                                .map_err(|mut e: Diagnostic| {
                                    e.span = Some(d.source_span);
                                    if e.file.is_none() {
                                        e.file = Some(f.path.clone());
                                    }
                                    e
                                })?;
                            let ti = *index.get(&target).ok_or_else(|| {
                                Diagnostic::new(Code::E108, format!("unknown module `{source}`"))
                                    .with_span(d.source_span)
                                    .with_file(f.path.clone())
                            })?;
                            match &d.clause {
                                A::ExportClause::Named(specs) => {
                                    for spec in specs {
                                        let (mangled, private) =
                                            exports[ti].get(&spec.name).ok_or_else(|| {
                                                Diagnostic::new(
                                                    Code::E108,
                                                    format!(
                                                        "`{}` is not exported by `{source}`",
                                                        spec.name
                                                    ),
                                                )
                                                .with_span(d.source_span)
                                                .with_file(f.path.clone())
                                            })?;
                                        if *private {
                                            return Err(Diagnostic::new(
                                                Code::E203,
                                                format!("`{}` is private in `{source}`", spec.name),
                                            )
                                            .with_span(d.source_span)
                                            .with_file(f.path.clone()));
                                        }
                                        reexp.insert(
                                            spec.alias.clone().unwrap_or_else(|| spec.name.clone()),
                                            (mangled.clone(), false),
                                        );
                                    }
                                }
                                A::ExportClause::All { alias } => {
                                    if let Some(ns) = alias {
                                        reexp.insert(ns.clone(), (self.files[ti].key.clone(), false));
                                    } else {
                                        for (plain, (mangled, private)) in &exports[ti] {
                                            if !private {
                                                reexp.insert(plain.clone(), (mangled.clone(), false));
                                            }
                                        }
                                    }
                                }
                                A::ExportClause::Native(_) => {}
                            }
                        }
                    }
                    A::Decl::ExportList(specs) => {
                        for spec in specs {
                            let entry = exports[i].get(&spec.name).cloned().or_else(|| {
                                reexp.get(&spec.name).cloned()
                            }).ok_or_else(|| {
                                Diagnostic::new(
                                    Code::E108,
                                    format!("`{}` is not defined in this module", spec.name),
                                )
                                .with_span(decl.span)
                                .with_file(f.path.clone())
                            })?;
                            reexp.insert(
                                spec.alias.clone().unwrap_or_else(|| spec.name.clone()),
                                (entry.0, false),
                            );
                        }
                    }
                    A::Decl::ExportDefault(_) => {
                        reexp.insert("default".to_string(), ("__default_export".to_string(), false));
                    }
                    _ => {}
                }
            }
            exports[i].extend(reexp);
        }
        Ok(exports)
    }

    fn file_scope(
        &self,
        idx: usize,
        exports: &mut Vec<BTreeMap<String, (String, bool)>>,
        index: &BTreeMap<&PathBuf, usize>,
        base: &Path,
    ) -> Result<
        (
            BTreeMap<String, String>,
            BTreeMap<String, String>,
            BTreeMap<String, String>,
        ),
        Diagnostic,
    > {
        let f = &self.files[idx];
        let mut own: BTreeMap<String, String> = BTreeMap::new();
        for (plain, (mangled, _)) in &exports[idx] {
            // Identity entries included: a file's own declarations
            // shadow prelude aliases of the same name in the Rewriter.
            own.insert(plain.clone(), mangled.clone());
        }
        let mut aliases: BTreeMap<String, String> = BTreeMap::new();
        let mut namespaces: BTreeMap<String, String> = BTreeMap::new();
        for decl in &f.module.decls {
            let import = match &decl.node {
                A::Decl::Import(d) => d,
                _ => continue,
            };
            let named: Vec<A::ImportSpecifier> = match &import.clause {
                A::ImportClause::Named(specs) => specs.clone(),
                A::ImportClause::DefaultAndNamed(_, specs) => specs.clone(),
                A::ImportClause::Default(d) => {
                    let nskey = match &import.source {
                        A::ImportSource::Module(source) => {
                            let target = resolve_source(&f.path, base, source)
                                .map_err(|mut e: Diagnostic| {
                                    e.span = Some(import.source_span);
                                    if e.file.is_none() {
                                        e.file = Some(f.path.clone());
                                    }
                                    e
                                })?;
                            index.get(&target).map(|ti| self.files[*ti].key.clone()).unwrap_or_default()
                        }
                        A::ImportSource::Native(_) => String::new(),
                    };
                    namespaces.entry(d.clone()).or_insert(nskey);
                    continue;
                }
                A::ImportClause::Namespace(ns) => {
                    if let A::ImportSource::Module(source) = &import.source {
                        let target = resolve_source(&f.path, base, source)
                            .map_err(|mut e: Diagnostic| {
                                e.span = Some(import.source_span);
                                if e.file.is_none() {
                                    e.file = Some(f.path.clone());
                                }
                                e
                            })?;
                        let ti = *index.get(&target).ok_or_else(|| {
                            Diagnostic::new(Code::E108, format!("unknown module `{source}`"))
                                .with_span(import.source_span)
                                .with_file(f.path.clone())
                        })?;
                        namespaces.entry(ns.clone()).or_insert_with(|| self.files[ti].key.clone());
                    }
                    continue;
                }
                A::ImportClause::Star => {
                    if let A::ImportSource::Module(source) = &import.source {
                        let target = resolve_source(&f.path, base, source)
                            .map_err(|mut e: Diagnostic| {
                                e.span = Some(import.source_span);
                                if e.file.is_none() {
                                    e.file = Some(f.path.clone());
                                }
                                e
                            })?;
                        let ti = *index.get(&target).ok_or_else(|| {
                            Diagnostic::new(Code::E108, format!("unknown module `{source}`"))
                                .with_span(import.source_span)
                                .with_file(f.path.clone())
                        })?;
                        for (plain, (mangled, private)) in &exports[ti] {
                            if !private {
                                aliases.entry(plain.clone()).or_insert_with(|| mangled.clone());
                            }
                        }
                    }
                    continue;
                }
                A::ImportClause::SideEffect => continue,
            };
            match &import.source {
                A::ImportSource::Native(_) => {
                    for spec in &named {
                        let local = import_local_name(spec);
                        aliases.entry(local.clone()).or_insert(local.clone());
                        if spec.is_export {
                            exports[idx].insert(local.clone(), (local.clone(), false));
                            own.insert(local.clone(), local.clone());
                        }
                    }
                    continue;
                }
                A::ImportSource::Module(source) => {
                    let target = resolve_source(&f.path, base, source)
                        .map_err(|mut e: Diagnostic| {
                            e.span = Some(import.source_span);
                            if e.file.is_none() {
                                e.file = Some(f.path.clone());
                            }
                            e
                        })?;
                    let ti = *index.get(&target).ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("unknown module `{source}`"))
                            .with_span(import.source_span)
                            .with_file(f.path.clone())
                    })?;
                    if let A::ImportClause::DefaultAndNamed(d, _) = &import.clause {
                        namespaces.entry(d.clone()).or_insert_with(|| self.files[ti].key.clone());
                    }
                    for spec in &named {
                        if spec.native_fn.is_some() {
                            return Err(Diagnostic::new(
                                Code::E108,
                                "type annotations are forbidden on module imports; use `from native` for foreign C ABI",
                            )
                            .with_span(import.source_span)
                            .with_file(f.path.clone()));
                        }
                        let (mangled, private) = exports[ti].get(&spec.name).cloned().ok_or_else(|| {
                            Diagnostic::new(Code::E108, format!("`{}` is not exported by `{source}`", spec.name))
                                .with_span(import.source_span)
                                .with_file(f.path.clone())
                        })?;
                        if private {
                            return Err(Diagnostic::new(
                                Code::E203,
                                format!("`{}` is private in `{source}`", spec.name),
                            )
                            .with_span(import.source_span)
                            .with_file(f.path.clone()));
                        }
                        let local = import_local_name(spec);
                        aliases.entry(local.clone()).or_insert_with(|| mangled.clone());
                        if spec.is_export {
                            exports[idx].insert(local.clone(), (mangled.clone(), false));
                            own.insert(local, mangled.clone());
                        }
                    }
                }
            }
        }
        if !is_virtual(&f.path) {
            if let Some(pi) = self.files.iter().position(|o| is_virtual(&o.path) && o.key == "std.prelude") {
                for (plain, (mangled, private)) in &exports[pi] {
                    if !private {
                        aliases.entry(plain.clone()).or_insert_with(|| mangled.clone());
                    }
                }
            }
        }
        Ok((own, aliases, namespaces))
    }

    /// Per-file import isolation: every user file must resolve each name it
    /// references through its own declarations, its own imports, or the
    /// prelude. The merged semantic check cannot see this because it runs
    /// once on the combined scope, where it silently accepts any
    /// UpperCamel reference and attributes lowercase misses to no file.
    pub fn isolation_errors(&self) -> Vec<Diagnostic> {
        let base = self.root.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let index: BTreeMap<&PathBuf, usize> =
            self.files.iter().enumerate().map(|(i, f)| (&f.path, i)).collect();
        let mut exports = match self.export_tables(&base, &index) {
            Ok(e) => e,
            Err(e) => return vec![e],
        };
        let mut out = Vec::new();
        let mut traits = BTreeSet::new();
        for f in &self.files {
            for decl in &f.module.decls {
                if let A::Decl::Trait { name, .. } = &decl.node {
                    traits.insert(name.clone());
                }
            }
        }
        for i in 0..self.files.len() {
            if is_virtual(&self.files[i].path) {
                continue;
            }
            let (own, aliases, namespaces) = match self.file_scope(i, &mut exports, &index, &base) {
                Ok(s) => s,
                Err(e) => {
                    out.push(e);
                    break;
                }
            };
            check_file_isolation(&self.files[i], &own, &aliases, &namespaces, &traits, &mut out);
        }
        out
    }
}

fn starts_uppercase(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

const AMBIENT_TYS: &[&str] = &[
    "Int64", "Int32", "Int16", "Int8", "UInt", "UInt64", "UInt32", "UInt16", "UInt8", "Short",
    "Byte", "Float32", "FastFloat32", "Vec4f", "Vec4i", "Range",
];

fn prelude_variants() -> &'static BTreeSet<String> {
    static MEMBERS: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
    MEMBERS.get_or_init(|| {
        let mut out = BTreeSet::new();
        if let Some(src) = stdlib::source("prelude") {
            if let Ok(module) = crate::parser::Parser::parse_module(src) {
                for decl in &module.decls {
                    if let A::Decl::Enum { members, .. } = &decl.node {
                        for m in members {
                            out.insert(m.name.clone());
                        }
                    }
                }
            }
        }
        out
    })
}

fn check_file_isolation(
    file: &ModuleFile,
    own: &BTreeMap<String, String>,
    aliases: &BTreeMap<String, String>,
    namespaces: &BTreeMap<String, String>,
    traits: &BTreeSet<String>,
    diags: &mut Vec<Diagnostic>,
) {
    let mut variants = prelude_variants().clone();
    for decl in &file.module.decls {
        if let A::Decl::Enum { members, .. } = &decl.node {
            for m in members {
                variants.insert(m.name.clone());
            }
        }
    }
    let mut ck = IsoCheck {
        file: &file.path,
        own,
        aliases,
        namespaces,
        variants,
        traits,
        type_params: Vec::new(),
        locals: Vec::new(),
        diags: Vec::new(),
    };
    for decl in &file.module.decls {
        ck.decl(decl);
    }
    diags.extend(ck.diags);
}

struct IsoCheck<'a> {
    file: &'a PathBuf,
    own: &'a BTreeMap<String, String>,
    aliases: &'a BTreeMap<String, String>,
    namespaces: &'a BTreeMap<String, String>,
    variants: BTreeSet<String>,
    traits: &'a BTreeSet<String>,
    type_params: Vec<BTreeSet<String>>,
    locals: Vec<BTreeSet<String>>,
    diags: Vec<Diagnostic>,
}

impl IsoCheck<'_> {
    fn err(&mut self, name: &str, span: Span) {
        self.diags.push(
            Diagnostic::new(Code::E303, format!("unresolved identifier `{name}`"))
                .with_span(span)
                .with_file(self.file.clone())
                .with_hint(format!("add an `import` for `{name}` to this file")),
        );
    }

    fn visible_global(&self, name: &str) -> bool {
        if self.own.contains_key(name)
            || self.aliases.contains_key(name)
            || self.namespaces.contains_key(name)
        {
            return true;
        }
        if self.variants.contains(name) || self.traits.contains(name) {
            return true;
        }
        if crate::prelude::provides(name) {
            return true;
        }
        if crate::semantic::SCOPE_BUILTINS.contains(&name) {
            return true;
        }
        if AMBIENT_TYS.contains(&name) {
            return true;
        }
        name == "rnx" || name.starts_with("__rnx_")
    }

    fn visible_value(&self, name: &str) -> bool {
        if self.locals.iter().any(|s| s.contains(name)) {
            return true;
        }
        self.visible_global(name)
    }

    fn visible_ty_head(&self, name: &str) -> bool {
        if self.type_params.iter().any(|s| s.contains(name)) {
            return true;
        }
        self.visible_global(name)
    }

    fn scope<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.locals.push(BTreeSet::new());
        let out = f(self);
        self.locals.pop();
        out
    }

    fn with_type_params<T>(&mut self, params: &[String], f: impl FnOnce(&mut Self) -> T) -> T {
        self.type_params.push(params.iter().cloned().collect());
        let out = f(self);
        self.type_params.pop();
        out
    }

    fn declare(&mut self, name: &str) {
        if let Some(top) = self.locals.last_mut() {
            top.insert(name.to_string());
        }
    }

    fn block(&mut self, b: &A::Block) {
        self.scope(|ck| {
            for s in &b.stmts {
                ck.stmt(s);
            }
        });
    }

    fn decl(&mut self, decl: &A::Spanned<A::Decl>) {
        match &decl.node {
            A::Decl::Fn(d) => self.func(d),
            A::Decl::Class { type_params, extends, with, members, .. } => {
                self.with_type_params(type_params, |ck| {
                    if let Some(t) = extends {
                        ck.ty(t, decl.span);
                    }
                    for t in with {
                        ck.ty(t, decl.span);
                    }
                    for m in members {
                        ck.member(m);
                    }
                });
            }
            A::Decl::Struct { type_params, members, .. } => {
                self.with_type_params(type_params, |ck| {
                    for m in members {
                        ck.member(m);
                    }
                });
            }
            A::Decl::Trait { members, .. } => {
                for m in members {
                    self.member(m);
                }
            }
            A::Decl::Interface { type_params, members, .. } => {
                self.with_type_params(type_params, |ck| {
                    for m in members {
                        ck.member(m);
                    }
                });
            }
            A::Decl::Extension { target, members, .. } => {
                self.ty(target, decl.span);
                for m in members {
                    self.member(m);
                }
            }
            A::Decl::Record { type_params, fields, .. } => {
                self.with_type_params(type_params, |ck| {
                    for f in fields {
                        ck.ty(&f.ty, decl.span);
                    }
                });
            }
            A::Decl::Enum { type_params, members, .. } => {
                self.with_type_params(type_params, |ck| {
                    for m in members {
                        for p in &m.payload {
                            ck.ty(p, decl.span);
                        }
                    }
                });
            }
            A::Decl::Const { ty, value, .. } => {
                if let Some(t) = ty {
                    self.ty(t, decl.span);
                }
                self.expr(value);
            }
            A::Decl::Stmt(s) => {
                self.scope(|ck| ck.stmt(s));
            }
            A::Decl::ExportDefault(d) => {
                self.expr(&d.expr);
            }
            A::Decl::Import(_) | A::Decl::ExportFrom(_) | A::Decl::ExportList(_) => {}
        }
    }

    fn member(&mut self, m: &A::Spanned<A::ClassMember>) {
        match &m.node {
            A::ClassMember::Field(f) => {
                if let Some(t) = &f.ty {
                    self.ty(t, m.span);
                }
                if let Some(v) = &f.value {
                    self.expr(v);
                }
            }
            A::ClassMember::Method(d) => self.func(d),
            A::ClassMember::Init { params, body } => {
                self.scope(|ck| {
                    for p in params {
                        ck.param(p);
                    }
                    ck.block(body);
                });
            }
            A::ClassMember::Deinit(b) => self.scope(|ck| ck.block(b)),
            A::ClassMember::OnReload { params, body } => {
                self.scope(|ck| {
                    for p in params {
                        ck.param(p);
                    }
                    ck.block(body);
                });
            }
        }
    }

    fn func(&mut self, d: &A::FnDecl) {
        self.with_type_params(&d.type_params, |ck| {
            for p in &d.params {
                if let Some(t) = &p.ty {
                    ck.ty(t, p.span);
                }
            }
            if let Some(t) = &d.ret {
                ck.ty(t, Span { start: 0, end: 0 });
            }
            ck.scope(|ck| {
                for p in &d.params {
                    ck.declare(&p.name);
                }
                for p in &d.params {
                    if let Some(v) = &p.default {
                        ck.expr(v);
                    }
                }
                match &d.body {
                    A::FnBody::Block(b) => ck.block(b),
                    A::FnBody::Expr(e) => ck.expr(e),
                }
            });
        });
    }

    fn param(&mut self, p: &A::Param) {
        if let Some(t) = &p.ty {
            self.ty(t, p.span);
        }
        if let Some(v) = &p.default {
            self.expr(v);
        }
        self.declare(&p.name);
    }

    fn ty(&mut self, t: &A::Type, span: Span) {
        if !t.tuple.is_empty() {
            for x in &t.tuple {
                self.ty(x, span);
            }
            return;
        }
        if let Some(sig) = t.fn_sig.as_ref() {
            for p in &sig.params {
                self.ty(p, span);
            }
            if let Some(r) = sig.ret.as_ref() {
                self.ty(r, span);
            }
            return;
        }
        if let Some(first) = t.path.first() {
            if !self.resolved_ty_head(first) {
                self.err(first, span);
            }
        }
        for a in &t.args {
            self.ty(a, span);
        }
    }

    fn resolved_ty_head(&self, first: &str) -> bool {
        if first == "fn" {
            return true;
        }
        if matches!(first, "Option" | "Str" | "Poll") {
            return true;
        }
        self.visible_ty_head(first)
    }

    fn stmt(&mut self, s: &A::Spanned<A::Stmt>) {
        match &s.node {
            A::Stmt::Var { ty, value, name, .. } => {
                if let Some(t) = ty {
                    self.ty(t, s.span);
                }
                self.expr(value);
                self.declare(name);
            }
            A::Stmt::DestructureTuple { names, ty, value } => {
                if let Some(t) = ty {
                    self.ty(t, s.span);
                }
                self.expr(value);
                for n in names {
                    self.declare(n);
                }
            }
            A::Stmt::DestructureRecord { fields, rest, value } => {
                self.expr(value);
                for (_, local) in fields {
                    self.declare(local);
                }
                if let Some(r) = rest {
                    self.declare(r);
                }
            }
            A::Stmt::DestructureArray { names, rest, value } => {
                self.expr(value);
                for n in names.iter().chain(rest.iter()) {
                    self.declare(n);
                }
            }
            A::Stmt::Assign { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
            A::Stmt::Expr(e) => self.expr(e),
            A::Stmt::If { cond, then, otherwise } => {
                match cond {
                    A::IfCond::Expr(e) => {
                        self.expr(e);
                        if let A::Expr::Is { base, .. } = &e.node {
                            if let A::Expr::Ident(n) = &base.node {
                                let name = n.clone();
                                self.scope(|ck| {
                                    ck.declare(&name);
                                    ck.block(then);
                                });
                                if let Some(el) = otherwise {
                                    self.else_(el);
                                }
                                return;
                            }
                        }
                        self.block(then);
                        if let Some(el) = otherwise {
                            self.else_(el);
                        }
                    }
                    A::IfCond::Let { name, value } => {
                        self.expr(value);
                        let name = name.clone();
                        self.scope(|ck| {
                            ck.declare(&name);
                            ck.block(then);
                        });
                        if let Some(el) = otherwise {
                            self.else_(el);
                        }
                    }
                }
            }
            A::Stmt::For { binding, iter, body } => {
                self.expr(iter);
                let names = match binding {
                    A::ForBinding::One(n) => vec![n.clone()],
                    A::ForBinding::Many(ns) => ns.clone(),
                };
                self.scope(|ck| {
                    for n in &names {
                        ck.declare(n);
                    }
                    ck.block(body);
                });
            }
            A::Stmt::While { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            A::Stmt::DoWhile { body, cond } => {
                self.block(body);
                self.expr(cond);
            }
            A::Stmt::Switch { scrutinee, cases, default } => {
                self.expr(scrutinee);
                let narrow = match &scrutinee.node {
                    A::Expr::Ident(n) => Some(n.clone()),
                    _ => None,
                };
                for c in cases {
                    let mut binds = Vec::new();
                    Self::pattern_binds(&c.pattern, &mut binds);
                    let is_pat = matches!(&c.pattern, A::Pattern::Is(_));
                    if is_pat {
                        if let A::Pattern::Is(t) = &c.pattern {
                            self.ty(t, s.span);
                        }
                        if let Some(n) = &narrow {
                            binds.push(n.clone());
                        }
                    }
                    self.scope(|ck| {
                        for b in &binds {
                            ck.declare(b);
                        }
                        if !is_pat {
                            ck.pattern_uses(&c.pattern, &binds);
                        }
                        if let Some(g) = &c.guard {
                            ck.expr(g);
                        }
                        for st in &c.body {
                            ck.stmt(st);
                        }
                    });
                }
                if let Some(stmts) = default {
                    self.scope(|ck| {
                        for st in stmts {
                            ck.stmt(st);
                        }
                    });
                }
            }
            A::Stmt::Return(e) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            A::Stmt::Assert(e) => self.expr(e),
            A::Stmt::Defer(b) => self.block(b),
            A::Stmt::Guard { name, value, otherwise } => {
                self.expr(value);
                self.declare(name);
                self.block(otherwise);
            }
            A::Stmt::Try { body, catch, finally } => {
                self.block(body);
                if let Some((name, block)) = catch {
                    let name = name.clone();
                    self.scope(|ck| {
                        ck.declare(&name);
                        ck.block(block);
                    });
                }
                if let Some(b) = finally {
                    self.block(b);
                }
            }
            A::Stmt::Throw(e) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            A::Stmt::UnsafeBlock(b) => self.block(b),
            A::Stmt::Break
            | A::Stmt::Continue
            | A::Stmt::Fallthrough
            | A::Stmt::Pass
            | A::Stmt::Empty => {}
        }
    }

    fn else_(&mut self, el: &A::Else) {
        match el {
            A::Else::Block(b) => self.block(b),
            A::Else::If(s) => self.stmt(s),
        }
    }

    fn pattern_binds(p: &A::Pattern, out: &mut Vec<String>) {
        match p {
            A::Pattern::Enum { args, .. } => {
                for a in args {
                    match a {
                        A::Pattern::Literal(e) => {
                            if let A::Expr::Ident(n) = &e.node {
                                out.push(n.clone());
                            }
                        }
                        nested => Self::pattern_binds(nested, out),
                    }
                }
            }
            _ => {}
        }
    }

    fn pattern_uses(&mut self, p: &A::Pattern, bound: &[String]) {
        match p {
            A::Pattern::Literal(e) => {
                if let A::Expr::Ident(n) = &e.node {
                    if bound.contains(n) {
                        return;
                    }
                }
                self.expr(e);
            }
            A::Pattern::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            A::Pattern::Enum { path, args } => {
                if let Some(first) = path.first() {
                    if !matches!(first.as_str(), "Some" | "None" | "Option")
                        && !self.visible_global(first)
                    {
                        self.err(first, Span { start: 0, end: 0 });
                    }
                }
                for a in args {
                    self.pattern_uses(a, bound);
                }
            }
            A::Pattern::Is(t) => self.ty(t, Span { start: 0, end: 0 }),
            A::Pattern::Wildcard => {}
        }
    }

    fn expr(&mut self, e: &A::Spanned<A::Expr>) {
        match &e.node {
            A::Expr::Ident(n) => {
                if n == "this" || n == "super" {
                    return;
                }
                if matches!(n.as_str(), "Some" | "None" | "Option") {
                    return;
                }
                if !starts_uppercase(n) {
                    return;
                }
                if !self.visible_value(n) {
                    self.err(n, e.span);
                }
            }
            A::Expr::This
            | A::Expr::Super
            | A::Expr::Bool(_)
            | A::Expr::Null
            | A::Expr::Int(_)
            | A::Expr::Float(_) => {}
            A::Expr::Interp(parts) => {
                for part in parts {
                    if let A::InterpPart::Expr(x) = part {
                        self.expr(x);
                    }
                }
            }
            A::Expr::Array(items) => {
                for i in items {
                    self.expr(&i.expr);
                }
            }
            A::Expr::Record(fields) => {
                for x in fields {
                    self.expr(x.value());
                }
            }
            A::Expr::MapLiteral(entries) => {
                for x in entries {
                    self.expr(x.value());
                }
            }
            A::Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            A::Expr::Unary { rhs, .. } => self.expr(rhs),
            A::Expr::Postfix { expr, .. } => self.expr(expr),
            A::Expr::Ternary { cond, then, otherwise } => {
                self.expr(cond);
                self.expr(then);
                self.expr(otherwise);
            }
            A::Expr::Coalesce { lhs, rhs } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            A::Expr::OptChain { base, .. } => self.expr(base),
            A::Expr::OptCall { base, args, .. } => {
                self.expr(base);
                for a in args {
                    self.expr(&a.value);
                }
            }
            A::Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            A::Expr::Call { callee, type_args, args, trailing } => {
                if !matches!(&callee.node, A::Expr::ImplicitMember(_)) {
                    self.expr(callee);
                }
                for t in type_args {
                    self.ty(t, e.span);
                }
                for a in args {
                    self.expr(&a.value);
                }
                if let Some(b) = trailing {
                    self.block(b);
                }
            }
            A::Expr::New { target, type_args, args } => {
                let first = target.split('.').next().unwrap_or(target);
                if !matches!(first, "Some" | "None" | "Option") && !self.visible_value(first) {
                    self.err(first, e.span);
                }
                for t in type_args {
                    self.ty(t, e.span);
                }
                for a in args {
                    self.expr(&a.value);
                }
            }
            A::Expr::Index { base, index } => {
                self.expr(base);
                self.expr(index);
            }
            A::Expr::Member { base, .. } => self.expr(base),
            A::Expr::Cast { expr, ty } => {
                self.ty(ty, e.span);
                self.expr(expr);
            }
            A::Expr::Is { base, target } => {
                self.ty(target, e.span);
                self.expr(base);
            }
            A::Expr::ImplicitMember(_) => {}
            A::Expr::Macro { name, args, .. } => {
                if !matches!(name.as_str(), "Assert" | "PanicIf" | "Some" | "None" | "Option")
                    && !self.visible_value(name)
                {
                    self.err(name, e.span);
                }
                for a in args {
                    self.expr(a);
                }
            }
            A::Expr::Closure { params, ret, body, .. } => {
                for p in params {
                    if let Some(t) = &p.ty {
                        self.ty(t, e.span);
                    }
                }
                if let Some(t) = ret {
                    self.ty(t, e.span);
                }
                self.scope(|ck| {
                    for p in params {
                        ck.declare(&p.name);
                    }
                    for p in params {
                        if let Some(v) = &p.default {
                            ck.expr(v);
                        }
                    }
                    match body {
                        A::FnBody::Block(b) => ck.block(b),
                        A::FnBody::Expr(x) => ck.expr(x),
                    }
                });
            }
            A::Expr::UnsafeBlock(b) => self.block(b),
            A::Expr::Await(inner) | A::Expr::Propagate(inner) => self.expr(inner),
            A::Expr::Tuple(items) => {
                for i in items {
                    self.expr(i);
                }
            }
            A::Expr::TupleGet { base, .. } => self.expr(base),
            A::Expr::Switch { scrutinee, cases, default } => {
                self.expr(scrutinee);
                let narrow = match &scrutinee.node {
                    A::Expr::Ident(n) => Some(n.clone()),
                    _ => None,
                };
                for c in cases {
                    let mut binds = Vec::new();
                    Self::pattern_binds(&c.pattern, &mut binds);
                    let is_pat = matches!(&c.pattern, A::Pattern::Is(_));
                    if is_pat {
                        if let A::Pattern::Is(t) = &c.pattern {
                            self.ty(t, e.span);
                        }
                        if let Some(n) = &narrow {
                            binds.push(n.clone());
                        }
                    }
                    self.scope(|ck| {
                        for b in &binds {
                            ck.declare(b);
                        }
                        if !is_pat {
                            ck.pattern_uses(&c.pattern, &binds);
                        }
                        if let Some(g) = &c.guard {
                            ck.expr(g);
                        }
                        match &c.body {
                            A::SwitchExprBody::Expr(x) => ck.expr(x),
                            A::SwitchExprBody::Block(b) => ck.block(b),
                        }
                    });
                }
                if let Some(d) = default {
                    self.scope(|ck| match d {
                        A::SwitchExprBody::Expr(x) => ck.expr(x),
                        A::SwitchExprBody::Block(b) => ck.block(b),
                    });
                }
            }
        }
    }
}

pub fn ns_class_name(alias: &str, key: &str) -> String {    format!("__ns_{key}.{alias}")
}

pub fn parse_ns_class(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix("__ns_")?;
    let (key, alias) = rest.rsplit_once('.')?;
    if key.is_empty() || alias.is_empty() {
        return None;
    }
    Some((alias.to_string(), key.to_string()))
}

pub const STD_SCOPE: &str = "@std/";
pub const STD_BARE: &str = "std/";

pub fn is_std_spec(source: &str) -> bool {
    source == "@std"
        || source.starts_with(STD_SCOPE)
        || source == "std"
        || source.starts_with(STD_BARE)
}

fn std_rest(source: &str) -> &str {
    source
        .strip_prefix(STD_SCOPE)
        .or_else(|| source.strip_prefix(STD_BARE))
        .unwrap_or("")
}
pub const PRELUDE_SPEC: &str = "@std/prelude";

pub fn prelude_spec() -> &'static str {
    PRELUDE_SPEC
}

fn scheme_hint(source: &str) -> String {
    if let Some(rest) = source.strip_prefix("std:") {
        return format!("scheme prefixes are removed; use `@std/{rest}` instead");
    }
    if let Some(rest) = source.strip_prefix("pkg:") {
        if rest.contains(':') {
            return "scheme prefixes are removed; use `<pkg>` or `<pkg>/<sub>` instead".to_string();
        }
        let (pkg, sub) = match rest.split_once('/') {
            Some((p, s)) => (p, format!("/{s}")),
            None => (rest, String::new()),
        };
        return format!("scheme prefixes are removed; use `{pkg}{sub}` instead");
    }
    "scheme prefixes are removed; use `./path`, `@std/<mod>`, `@<scope>/<pkg>`, or `<pkg>` instead".to_string()
}

fn scheme_error(source: &str) -> Diagnostic {
    Diagnostic::new(Code::E108, format!("cannot resolve module `{source}`")).with_hint(scheme_hint(source))
}

fn canonical(path: &Path) -> Result<PathBuf, Diagnostic> {
    std::fs::canonicalize(path)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", path.display())))
}

fn is_virtual(path: &Path) -> bool {
    path.to_string_lossy().starts_with(STD_SCOPE)
}

fn std_key(path: &Path) -> String {
    let s = path.to_string_lossy();
    let rest = s.strip_prefix(STD_SCOPE).unwrap_or(&s);
    let mut out = String::from("std");
    for part in rest.split('/') {
        out.push_str(".");
        out.push_str(part);
    }
    out
}

fn std_submodule(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.strip_prefix(STD_SCOPE).unwrap_or(&s).to_string()
}

fn read_source(path: &Path) -> Result<String, Diagnostic> {
    if is_virtual(path) {
        if let Some(real) = STD_OVERLAY
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(path)
            .cloned()
        {
            return std::fs::read_to_string(&real).map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", real.display()))
            });
        }
        let rest = std_submodule(path);
        let name = format!("@std/{rest}");
        return stdlib::source(&rest)
            .map(|s| s.to_string())
            .ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("unknown standard library module `{name}`"))
            });
    }
    std::fs::read_to_string(path)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", path.display())))
}

fn module_key(root_base: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root_base).unwrap_or(path);
    let mut parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if let Some(last) = parts.last_mut() {
        if let Some(stem) = last.strip_suffix(".rnx") {
            *last = stem.to_string();
        }
    }
    parts.join(".")
}

fn resolve_source(from: &Path, _base: &Path, source: &str) -> Result<PathBuf, Diagnostic> {
    resolve_import(from, source).map(|(target, _)| target)
}

fn package_ctx_of(file: &Path) -> Result<Option<(PathBuf, ProjectConfig)>, Diagnostic> {
    if is_virtual(file) {
        return Ok(None);
    }
    let dir = file.parent().unwrap_or(Path::new("."));
    match project::find_project_root(dir) {
        None => Ok(None),
        Some(root) => match ProjectConfig::load_from_dir(&root)? {
            Some(cfg) => Ok(Some((root, cfg))),
            None => Ok(None),
        },
    }
}

fn resolve_import(from: &Path, source: &str) -> Result<(PathBuf, Option<PathBuf>), Diagnostic> {
    if source.contains(':') {
        return Err(scheme_error(source));
    }
    if is_std_spec(source) {
        let rest = std_rest(source);
        if rest.is_empty() {
            return Err(Diagnostic::new(
                Code::E108,
                format!("cannot resolve module `{source}`"),
            )
            .with_hint("use `@std/<module>`, e.g. `@std/time`"));
        }
        let overrides: BTreeMap<String, RegistryConfig> = BTreeMap::new();
        return resolve_std_with_registries(source, rest, None, &overrides);
    }
    if source.starts_with('.') {
        return resolve_relative(from, source);
    }
    resolve_package(from, source)
}

/// `@std/*` through the same-domain registry server under the hood: exact
/// CLI-pinned version, integrity-checked cache, embedded sysroot as offline
/// fallback. No URL imports exist in user code; all fetching here is
/// toolchain-initiated.
///
/// The resolved path is ALWAYS the virtual `@std/<rest>` path, so module
/// keys and merged output are identical online and offline. A registry hit
/// only swaps the *bytes* behind it via the overlay read by `read_source`.
fn resolve_std_with_registries(
    source: &str,
    rest: &str,
    default: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
) -> Result<(PathBuf, Option<PathBuf>), Diagnostic> {
    let virtual_path = PathBuf::from(format!("{STD_SCOPE}{rest}"));
    let (pkg, sub) = match rest.split_once('/') {
        Some((top, s)) => (format!("@std/{top}"), Some(s)),
        None => (format!("@std/{rest}"), None),
    };
    let pin = env!("CARGO_PKG_VERSION").to_string();
    let base = crate::fetch::registry_base_for(&pkg, default, overrides);
    // WebAssembly has no filesystem, threads, or sockets: the registry
    // path cannot run there. Embedded sysroot only (see 11_TOOLING).
    let attempt = if cfg!(target_arch = "wasm32") {
        None
    } else {
        cached_std_package(&base, &pkg, &pin).or_else(|| {
            let have = crate::fetch::scan_cache_have();
            crate::fetch::resolve_registry_package(&pkg, &pin, default, overrides, Some(pin.clone()), &have)
                .map(|(dir, cfg, _)| (dir, cfg))
                .ok()
        })
    };
    if let Some((dep_root, dep_cfg)) = attempt {
        if let Ok(target) = resolve_subpath_target(&dep_root, sub.as_deref(), &dep_cfg, &pkg, source) {
            set_std_overlay(&virtual_path, &target);
            return Ok((virtual_path, None));
        }
    }
    clear_std_overlay(&virtual_path);
    if stdlib::source(rest).is_none() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("unknown standard library module `{source}`"),
        ));
    }
    Ok((virtual_path, None))
}

static STD_OVERLAY: std::sync::Mutex<BTreeMap<PathBuf, PathBuf>> =
    std::sync::Mutex::new(BTreeMap::new());

fn set_std_overlay(virtual_path: &Path, real_path: &Path) {
    let mut overlay = STD_OVERLAY.lock().unwrap_or_else(|e| e.into_inner());
    overlay.insert(virtual_path.to_path_buf(), real_path.to_path_buf());
}

fn clear_std_overlay(virtual_path: &Path) {
    let mut overlay = STD_OVERLAY.lock().unwrap_or_else(|e| e.into_inner());
    overlay.remove(virtual_path);
}

#[cfg(test)]
fn std_overlay_get(virtual_path: &Path) -> Option<PathBuf> {
    STD_OVERLAY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(virtual_path)
        .cloned()
}

fn cached_std_package(base: &str, pkg: &str, pin: &str) -> Option<(PathBuf, ProjectConfig)> {
    let dir = crate::fetch::cached_package_dir(base, pkg, pin);
    if !dir.join(crate::project::MANIFEST_FILE).is_file() {
        return None;
    }
    if crate::fetch::cached_integrity(&dir).is_none() {
        return None;
    }
    let dir = std::fs::canonicalize(&dir).ok()?;
    let cfg = ProjectConfig::load_from_dir(&dir).ok()??;
    Some((dir, cfg))
}

fn resolve_relative(from: &Path, source: &str) -> Result<(PathBuf, Option<PathBuf>), Diagnostic> {
    let base = from.parent().unwrap_or(Path::new(".")).join(source);
    let mut probed: Vec<PathBuf> = Vec::new();
    if base.extension().is_some() {
        probed.push(base.clone());
    } else {
        probed.push(base.with_extension("rnx"));
        probed.push(base.join("mod.rnx"));
        probed.push(base.join("index.rnx"));
    }
    for candidate in &probed {
        if let Ok(target) = std::fs::canonicalize(candidate) {
            return Ok((target, None));
        }
    }
    let listed = probed
        .iter()
        .map(|p| format!("`{}`", p.display()))
        .collect::<Vec<_>>()
        .join(", ");
    Err(Diagnostic::new(
        Code::E108,
        format!("cannot resolve module `{source}`: tried {listed}"),
    ))
}

fn split_package_spec(source: &str) -> (String, Option<String>) {
    let mut parts = source.split('/');
    if source.starts_with('@') {
        let scope = parts.next().unwrap_or("");
        let name = parts.next().unwrap_or("");
        if name.is_empty() {
            return (source.to_string(), None);
        }
        let pkg = format!("{scope}/{name}");
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() {
            (pkg, None)
        } else {
            (pkg, Some(rest.join("/")))
        }
    } else {
        let name = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() {
            (name.to_string(), None)
        } else {
            (name.to_string(), Some(rest.join("/")))
        }
    }
}

fn valid_subpath(s: &str) -> bool {
    !s.is_empty()
        && s.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

fn resolve_package(from: &Path, source: &str) -> Result<(PathBuf, Option<PathBuf>), Diagnostic> {
    let (pkg, sub) = split_package_spec(source);
    if pkg.is_empty() || pkg == "@" {
        return Err(Diagnostic::new(Code::E108, format!("cannot resolve module `{source}`")));
    }
    let unknown = || Diagnostic::new(Code::E108, format!("unknown package dependency `{pkg}`"));
    let (proj_root, cfg) = package_ctx_of(from)?.ok_or_else(unknown)?;
    let (dep_root, dep_cfg) = match cfg.dependencies.get(pkg.as_str()) {
        Some(crate::project::DependencySpec::Path { path }) => {
            let dep_root = std::fs::canonicalize(proj_root.join(path)).map_err(|_| {
                Diagnostic::new(Code::E108, format!("cannot resolve package `{pkg}`"))
            })?;
            let dep_cfg = ProjectConfig::load_from_dir(&dep_root)?.ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("package `{pkg}` has no Project.config"))
            })?;
            (dep_root, dep_cfg)
        }
        Some(crate::project::DependencySpec::Git { git, rev }) => {
            let anchor = crate::fetch::cache_anchor(&proj_root);
            let dep_root = crate::fetch::resolve_git_dep(&anchor, &pkg, git, rev)?;
            let dep_cfg = ProjectConfig::load_from_dir(&dep_root)?.ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("package `{pkg}` has no Project.config"))
            })?;
            (dep_root, dep_cfg)
        }
        Some(crate::project::DependencySpec::Semver { version }) => {
            let scope_root =
                project::find_workspace_root(&proj_root).unwrap_or_else(|| proj_root.clone());
            let pinned = locked_registry_version(&scope_root, &pkg);
            let have = crate::fetch::scan_cache_have();
            let (dep_root, dep_cfg, _) = crate::fetch::resolve_registry_package(
                &pkg,
                version,
                cfg.registry.as_ref(),
                &cfg.registries,
                pinned,
                &have,
            )?;
            (dep_root, dep_cfg)
        }
        Some(crate::project::DependencySpec::Url { url, .. }) => {
            return Err(Diagnostic::new(
                Code::E108,
                format!("cannot resolve package `{pkg}`: tarball `{url}` fetch is not implemented yet"),
            ));
        }
        Some(crate::project::DependencySpec::Native { lib, .. }) => {
            return Err(Diagnostic::new(
                Code::E108,
                format!("cannot import native system dependency `{pkg}` (`{lib}`) as a module; call it with `from native \"{lib}\"`"),
            ));
        }
        None => workspace_sibling(&proj_root, &pkg)?.ok_or_else(unknown)?,
    };
    let target = resolve_subpath_target(&dep_root, sub.as_deref(), &dep_cfg, &pkg, source)?;
    Ok((target, Some(dep_root)))
}

fn resolve_subpath_target(
    dep_root: &Path,
    sub: Option<&str>,
    dep_cfg: &ProjectConfig,
    pkg: &str,
    source: &str,
) -> Result<PathBuf, Diagnostic> {
    let target = match sub {
        None => {
            let main = dep_cfg.main_path(&dep_root);
            if main.is_file() {
                main
            } else if let Some(lib) = dep_cfg.lib_path(&dep_root) {
                if lib.is_file() {
                    lib
                } else {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!(
                            "package `{pkg}` entries.main `{}` not found and lib fallback `{}` not found",
                            dep_cfg.entries.main,
                            dep_cfg.entries.lib.as_deref().unwrap_or(""),
                        ),
                    ));
                }
            } else {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!(
                        "package `{pkg}` entries.main `{}` not found with no lib fallback",
                        dep_cfg.entries.main,
                    ),
                ));
            }
        }
        Some(s) => {
            if !valid_subpath(&s) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("cannot resolve module `{source}`"),
                ));
            }
            let base = dep_root.join("src");
            let mut probed: Vec<PathBuf> = Vec::new();
            if s.ends_with(".rnx") {
                probed.push(base.join(&s));
            } else {
                probed.push(base.join(format!("{s}.rnx")));
                probed.push(base.join(&s).join("mod.rnx"));
                probed.push(base.join(&s).join("index.rnx"));
            }
            let mut resolved = None;
            for candidate in &probed {
                if let Ok(target) = std::fs::canonicalize(candidate)
                    && target.starts_with(&dep_root)
                {
                    resolved = Some(target);
                    break;
                }
            }
            resolved.ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("cannot resolve module `{source}`"))
            })?
        }
    };
    let target = std::fs::canonicalize(&target)
        .map_err(|_| Diagnostic::new(Code::E108, format!("cannot resolve module `{source}`")))?;
    if !target.starts_with(dep_root) {
        return Err(Diagnostic::new(Code::E108, format!("cannot resolve module `{source}`")));
    }
    Ok(target)
}

fn locked_registry_version(scope_root: &Path, pkg: &str) -> Option<String> {
    let lock = crate::deplock::ProjectDepLock::load(scope_root).ok()??;
    let entry = lock.packages.iter().find(|p| p.name == pkg)?;
    let rest = entry.source.strip_prefix("registry:")?;
    let (full, version) = rest.rsplit_once('@')?;
    if full == pkg && !version.is_empty() {
        Some(version.to_string())
    } else {
        None
    }
}

fn workspace_sibling(
    proj_root: &Path,
    pkg: &str,
) -> Result<Option<(PathBuf, ProjectConfig)>, Diagnostic> {
    let ws_root = match project::find_workspace_root_strict(proj_root) {
        Some(r) => r,
        None => return Ok(None),
    };
    let manifest = project::load_manifest(&ws_root)?.ok_or_else(|| {
        Diagnostic::new(Code::E108, format!("cannot read workspace `{}`", ws_root.display()))
    })?;
    let ws = match manifest.workspace {
        Some(w) => w,
        None => return Ok(None),
    };
    let members = project::resolve_workspace_members(&ws_root, &ws)?;
    Ok(members.get(pkg).cloned())
}

fn visit(
    path: &Path,
    base: &Path,
    root: &Path,
    entry_root: Option<&PathBuf>,
    state: &mut BTreeMap<PathBuf, u8>,
    order: &mut Vec<ModuleFile>,
    stack: &mut Vec<PathBuf>,
    parse_errors: &mut Vec<Diagnostic>,
) -> Result<(), Diagnostic> {
    match state.get(path) {
        Some(_) => return Ok(()),
        None => {}
    }
    state.insert(path.to_path_buf(), 1);
    let src = read_source(path)?;
    let module = match crate::parser::Parser::parse_module_all(&src) {
        Ok(module) => module,
        Err(errs) => {
            parse_errors.extend(errs.into_iter().map(|e| match e.file {
                Some(_) => e,
                None => e.with_file(path.to_path_buf()),
            }));
            return Ok(());
        }
    };
    let mut deps: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    for (source, source_span) in module_sources(&module.decls) {
        {
            let child = resolve_import(path, source)
                .map_err(|mut e: Diagnostic| {
                    e.span = Some(source_span);
                    if e.file.is_none() {
                        e.file = Some(path.to_path_buf());
                    }
                    e
                })?;
            if let Some(dep_root) = &child.1
                && stack.contains(dep_root)
            {
                let mut e = Diagnostic::new(
                    Code::E107,
                    format!("circular package dependency on `{source}`"),
                );
                e.span = Some(source_span);
                e.file = Some(path.to_path_buf());
                return Err(e);
            }
            deps.push(child);
        }
    }
    deps.sort();
    deps.dedup();
    for (child, dep_root) in deps {
        if let Some(dep) = dep_root {
            stack.push(dep);
            let r = visit(&child, base, root, entry_root, state, order, stack, parse_errors);
            stack.pop();
            r?;
        } else {
            visit(&child, base, root, entry_root, state, order, stack, parse_errors)?;
        }
    }
    state.insert(path.to_path_buf(), 2);
    let key = if path == root {
        String::new()
    } else if is_virtual(path) {
        std_key(path)
    } else {
        package_key(path, base, entry_root)?
    };
    order.push(ModuleFile {
        path: path.to_path_buf(),
        key,
        kind: if path == root { ModuleKind::Entry } else { ModuleKind::Imported },
        module,
    });
    Ok(())
}

fn package_key(path: &Path, base: &Path, entry_root: Option<&PathBuf>) -> Result<String, Diagnostic> {
    match package_ctx_of(path)? {
        Some((pkg_root, cfg)) if Some(&pkg_root) != entry_root => {
            let rel = path.strip_prefix(&pkg_root).unwrap_or(path);
            let mut parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            if let Some(last) = parts.last_mut() {
                *last = last.strip_suffix(".rnx").unwrap_or(last).to_string();
            }
            Ok(format!("{}.{}", cfg.name, parts.join(".")))
        }
        _ => {
            if path.strip_prefix(base).is_err()
                && let Some(root) = entry_root
                && let Ok(rel) = path.strip_prefix(root)
            {
                let mut parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                if let Some(last) = parts.last_mut() {
                    *last = last.strip_suffix(".rnx").unwrap_or(last).to_string();
                }
                return Ok(parts.join("."));
            }
            Ok(module_key(base, path))
        }
    }
}

fn rename_decl(decl: &mut A::Decl, own: &BTreeMap<String, String>) {
    let slot = match decl {
        A::Decl::Fn(d) => Some(&mut d.name),
        A::Decl::Class { name, .. }
        | A::Decl::Struct { name, .. }
        | A::Decl::Enum { name, .. }
        | A::Decl::Interface { name, .. }
        | A::Decl::Record { name, .. } => Some(name),
        A::Decl::Const { name, .. } => Some(name),
        _ => None,
    };
    if let Some(name) = slot {
        if let Some(mangled) = own.get(name) {
            *name = mangled.clone();
        }
    }
}

struct Rewriter<'a> {
    own: &'a BTreeMap<String, String>,
    aliases: &'a BTreeMap<String, String>,
    namespaces: &'a BTreeMap<String, String>,
    locals: Vec<BTreeSet<String>>,
}

impl Rewriter<'_> {
    fn mapped(&self, name: &str) -> Option<String> {
        if self.locals.iter().any(|s| s.contains(name)) {
            return None;
        }
        if let Some(m) = self.own.get(name) {
            return Some(m.clone());
        }
        self.aliases.get(name).cloned()
    }

    fn ident(&self, name: &mut String) {
        if let Some(m) = self.mapped(name) {
            *name = m;
        }
    }

    fn ty(&self, t: &mut A::Type) {
        if let Some(first) = t.path.first() {
            let key = first.clone();
            if self.locals.iter().all(|s| !s.contains(&key)) {
                if let Some(m) = self.own.get(&key).or_else(|| self.aliases.get(&key)) {
                    t.path[0] = m.clone();
                } else if let Some(nskey) = self.namespaces.get(&key) {
                    if t.path.len() > 1 {
                        t.path[0] = format!("{}.{}", nskey, t.path[1]);
                        t.path.remove(1);
                    }
                }
            }
        }
        for a in &mut t.args {
            self.ty(a);
        }
        for x in &mut t.tuple {
            self.ty(x);
        }
        if let Some(sig) = t.fn_sig.as_mut() {
            for p in &mut sig.params {
                self.ty(p);
            }
            if let Some(r) = sig.ret.as_mut() {
                self.ty(r);
            }
        }
    }

    fn scope<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.locals.push(BTreeSet::new());
        let out = f(self);
        self.locals.pop();
        out
    }

    fn decl(&mut self, decl: &mut A::Decl) {
        match decl {
            A::Decl::Fn(d) => self.func(d),
            A::Decl::Class { extends, with, members, .. } => {
                if let Some(t) = extends {
                    self.ty(t);
                }
                for t in with.iter_mut() {
                    self.ty(t);
                }
                for m in members.iter_mut() {
                    self.member(&mut m.node);
                }
            }
            A::Decl::Struct { members, .. } => {
                for m in members.iter_mut() {
                    self.member(&mut m.node);
                }
            }
            A::Decl::Interface { members, .. } => {
                for m in members.iter_mut() {
                    self.member(&mut m.node);
                }
            }
            A::Decl::Extension { target, members, .. } => {
                self.ty(target);
                for m in members.iter_mut() {
                    self.member(&mut m.node);
                }
            }
            A::Decl::Record { fields, .. } => {
                for f in fields.iter_mut() {
                    self.ty(&mut f.ty);
                }
            }
            A::Decl::Enum { members, .. } => {
                for m in members.iter_mut() {
                    for p in m.payload.iter_mut() {
                        self.ty(p);
                    }
                }
            }
            A::Decl::Const { ty, value, .. } => {
                if let Some(t) = ty {
                    self.ty(t);
                }
                self.expr(&mut value.node);
            }
            A::Decl::Stmt(s) => {
                self.scope(|rw| rw.stmt(&mut s.node));
            }
            _ => {}
        }
    }

    fn member(&mut self, m: &mut A::ClassMember) {
        match m {
            A::ClassMember::Field(f) => {
                if let Some(t) = f.ty.as_mut() {
                    self.ty(t);
                }
                if let Some(v) = f.value.as_mut() {
                    self.expr(&mut v.node);
                }
            }
            A::ClassMember::Method(d) => self.func(d),
            A::ClassMember::Init { params, body } => {
                self.scope(|rw| {
                    for p in params.iter_mut() {
                        rw.param(p);
                    }
                    rw.block(body);
                });
            }
            A::ClassMember::Deinit(b) => self.scope(|rw| rw.block(b)),
            A::ClassMember::OnReload { params, body } => {
                self.scope(|rw| {
                    for p in params.iter_mut() {
                        rw.param(p);
                    }
                    rw.block(body);
                });
            }
        }
    }

    fn func(&mut self, d: &mut A::FnDecl) {
        for p in d.params.iter_mut() {
            if let Some(t) = p.ty.as_mut() {
                self.ty(t);
            }
        }
        if let Some(t) = d.ret.as_mut() {
            self.ty(t);
        }
        self.scope(|rw| {
            for p in d.params.iter() {
                rw.declare(&p.name);
            }
            for p in d.params.iter_mut() {
                if let Some(v) = p.default.as_mut() {
                    rw.expr(&mut v.node);
                }
            }
            match &mut d.body {
                A::FnBody::Block(b) => rw.block(b),
                A::FnBody::Expr(e) => rw.expr(&mut e.node),
            }
        });
    }

    fn param(&mut self, p: &mut A::Param) {
        if let Some(t) = p.ty.as_mut() {
            self.ty(t);
        }
        if let Some(v) = p.default.as_mut() {
            self.expr(&mut v.node);
        }
        self.declare(&p.name);
    }

    fn declare(&mut self, name: &str) {
        if let Some(top) = self.locals.last_mut() {
            top.insert(name.to_string());
        }
    }

    fn block(&mut self, b: &mut A::Block) {
        self.scope(|rw| {
            for s in b.stmts.iter_mut() {
                rw.stmt(&mut s.node);
            }
        });
    }

    fn stmt(&mut self, s: &mut A::Stmt) {
        match s {
            A::Stmt::Var { ty, value, name, .. } => {
                if let Some(t) = ty {
                    self.ty(t);
                }
                self.expr(&mut value.node);
                self.declare(name);
            }
            A::Stmt::DestructureTuple { names, ty, value } => {
                if let Some(t) = ty {
                    self.ty(t);
                }
                self.expr(&mut value.node);
                for n in names {
                    self.declare(n);
                }
            }
            A::Stmt::DestructureRecord { fields, rest, value } => {
                self.expr(&mut value.node);
                for (_, local) in fields {
                    self.declare(local);
                }
                if let Some(r) = rest {
                    self.declare(r);
                }
            }
            A::Stmt::DestructureArray { names, rest, value } => {
                self.expr(&mut value.node);
                for n in names.iter().chain(rest.iter()) {
                    self.declare(n);
                }
            }
            A::Stmt::Assign { target, value, .. } => {
                self.expr(&mut target.node);
                self.expr(&mut value.node);
            }
            A::Stmt::Expr(e) => self.expr(&mut e.node),
            A::Stmt::If { cond, then, otherwise } => {
                match cond {
                    A::IfCond::Expr(e) => self.expr(&mut e.node),
                    A::IfCond::Let { name, value } => {
                        self.expr(&mut value.node);
                        self.declare(name);
                    }
                }
                self.block(then);
                if let Some(e) = otherwise {
                    match e {
                        A::Else::Block(b) => self.block(b),
                        A::Else::If(s) => self.stmt(&mut s.node),
                    }
                }
            }
            A::Stmt::For { binding, iter, body } => {
                self.expr(&mut iter.node);
                let names = match binding {
                    A::ForBinding::One(n) => vec![n.clone()],
                    A::ForBinding::Many(ns) => ns.clone(),
                };
                self.scope(|rw| {
                    for n in &names {
                        rw.declare(n);
                    }
                    rw.block(body);
                });
            }
            A::Stmt::While { cond, body } => {
                self.expr(&mut cond.node);
                self.block(body);
            }
            A::Stmt::DoWhile { body, cond } => {
                self.block(body);
                self.expr(&mut cond.node);
            }
            A::Stmt::Switch { scrutinee, cases, default } => {
                self.expr(&mut scrutinee.node);
                for c in cases.iter_mut() {
                    self.pattern(&mut c.pattern);
                    if let Some(g) = c.guard.as_mut() {
                        self.expr(&mut g.node);
                    }
                    self.scope(|rw| {
                        for s in c.body.iter_mut() {
                            rw.stmt(&mut s.node);
                        }
                    });
                }
                if let Some(stmts) = default {
                    self.scope(|rw| {
                        for s in stmts.iter_mut() {
                            rw.stmt(&mut s.node);
                        }
                    });
                }
            }
            A::Stmt::Return(e) => {
                if let Some(e) = e {
                    self.expr(&mut e.node);
                }
            }
            A::Stmt::Assert(e) => self.expr(&mut e.node),
            A::Stmt::Defer(b) => self.block(b),
            A::Stmt::Guard { name, value, otherwise } => {
                self.expr(&mut value.node);
                self.declare(name);
                self.block(otherwise);
            }
            A::Stmt::Try { body, catch, finally } => {
                self.block(body);
                if let Some((name, block)) = catch {
                    self.scope(|rw| {
                        rw.declare(name);
                        rw.block(block);
                    });
                }
                if let Some(b) = finally {
                    self.block(b);
                }
            }
            A::Stmt::Throw(e) => {
                if let Some(e) = e {
                    self.expr(&mut e.node);
                }
            }
            A::Stmt::UnsafeBlock(b) => self.block(b),
            A::Stmt::Break | A::Stmt::Continue | A::Stmt::Fallthrough | A::Stmt::Pass | A::Stmt::Empty => {}
        }
    }

    fn pattern(&mut self, p: &mut A::Pattern) {
        match p {
            A::Pattern::Literal(e) => self.expr(&mut e.node),
            A::Pattern::Range { lo, hi, .. } => {
                self.expr(&mut lo.node);
                self.expr(&mut hi.node);
            }
            A::Pattern::Is(t) => self.ty(t),
            A::Pattern::Enum { path, args } => {
                if let Some(first) = path.first() {
                    let key = first.clone();
                    if let Some(m) = self.own.get(&key).or_else(|| self.aliases.get(&key)) {
                        path[0] = m.clone();
                    }
                }
                for a in args.iter_mut() {
                    self.pattern(a);
                }
            }
            A::Pattern::Wildcard => {}
        }
    }

    fn expr(&mut self, e: &mut A::Expr) {
        match e {
            A::Expr::Ident(n) => {
                if n == "rnx"
                    && !self.locals.iter().any(|s| s.contains(n.as_str()))
                    && !self.own.contains_key(n)
                    && !self.aliases.contains_key(n)
                    && !self.namespaces.contains_key(n)
                {
                    *e = A::Expr::New {
                        target: "std.prelude.RnxHost".to_string(),
                        type_args: Vec::new(),
                        args: Vec::new(),
                    };
                    return;
                }
                if !self.locals.iter().any(|s| s.contains(n.as_str()))
                    && !self.own.contains_key(n)
                    && !self.aliases.contains_key(n)
                {
                    if let Some(nskey) = self.namespaces.get(n).cloned() {
                        if !nskey.is_empty() {
                            *e = A::Expr::New {
                                target: ns_class_name(n, &nskey),
                                type_args: Vec::new(),
                                args: Vec::new(),
                            };
                            return;
                        }
                    }
                }
                self.ident(n)
            }
            A::Expr::Array(items) => {
                for i in items.iter_mut() {
                    self.expr(&mut i.expr.node);
                }
            }
            A::Expr::Record(fields) => {
                for e in fields.iter_mut() {
                    self.expr(&mut e.value_mut().node);
                }
            }
            A::Expr::MapLiteral(entries) => {
                for e in entries.iter_mut() {
                    self.expr(&mut e.value_mut().node);
                }
            }
            A::Expr::Binary { lhs, rhs, .. } => {
                self.expr(&mut lhs.node);
                self.expr(&mut rhs.node);
            }
            A::Expr::Is { base, target } => {
                self.expr(&mut base.node);
                self.ty(target);
            }
            A::Expr::Unary { rhs, .. } => self.expr(&mut rhs.node),
            A::Expr::Postfix { expr, .. } => self.expr(&mut expr.node),
            A::Expr::Await(e) => self.expr(&mut e.node),
            A::Expr::Tuple(items) => {
                for i in items.iter_mut() {
                    self.expr(&mut i.node);
                }
            }
            A::Expr::TupleGet { base, .. } => self.expr(&mut base.node),
            A::Expr::Ternary { cond, then, otherwise } => {
                self.expr(&mut cond.node);
                self.expr(&mut then.node);
                self.expr(&mut otherwise.node);
            }
            A::Expr::Coalesce { lhs, rhs } => {
                self.expr(&mut lhs.node);
                self.expr(&mut rhs.node);
            }
            A::Expr::OptChain { base, .. } => self.expr(&mut base.node),
            A::Expr::OptCall { base, args, .. } => {
                self.expr(&mut base.node);
                for a in args.iter_mut() {
                    self.expr(&mut a.value.node);
                }
            }
            A::Expr::Range { lo, hi, .. } => {
                self.expr(&mut lo.node);
                self.expr(&mut hi.node);
            }
            A::Expr::Call { callee, type_args, args, trailing } => {
                self.expr(&mut callee.node);
                for t in type_args.iter_mut() {
                    self.ty(t);
                }
                for a in args.iter_mut() {
                    self.expr(&mut a.value.node);
                }
                if let Some(b) = trailing {
                    self.block(b);
                }
            }
            A::Expr::New { target, type_args, args } => {
                self.ident(target);
                for t in type_args.iter_mut() {
                    self.ty(t);
                }
                for a in args.iter_mut() {
                    self.expr(&mut a.value.node);
                }
            }
            A::Expr::Index { base, index } => {
                self.expr(&mut base.node);
                self.expr(&mut index.node);
            }
            A::Expr::Member { base, field } => {
                if let A::Expr::Ident(ns) = &base.node {
                    if !self.locals.iter().any(|s| s.contains(ns))
                        && !self.own.contains_key(ns)
                        && !self.aliases.contains_key(ns)
                    {
                        if let Some(nskey) = self.namespaces.get(ns).cloned() {
                            *e = A::Expr::Ident(format!("{nskey}.{field}"));
                            return;
                        }
                    }
                }
                self.expr(&mut base.node)
            }
            A::Expr::Macro { name, args, .. } => {
                self.ident(name);
                for a in args.iter_mut() {
                    self.expr(&mut a.node);
                }
            }
            A::Expr::Closure { params, ret, body, .. } => {
                for p in params.iter_mut() {
                    if let Some(t) = p.ty.as_mut() {
                        self.ty(t);
                    }
                }
                if let Some(t) = ret {
                    self.ty(t);
                }
                self.scope(|rw| {
                    for p in params.iter() {
                        rw.declare(&p.name);
                    }
                    match body {
                        A::FnBody::Block(b) => rw.block(b),
                        A::FnBody::Expr(x) => rw.expr(&mut x.node),
                    }
                });
            }
            A::Expr::UnsafeBlock(b) => self.block(b),
            A::Expr::Cast { expr, ty } => {
                self.expr(&mut expr.node);
                self.ty(ty);
            }
            A::Expr::Propagate(inner) => self.expr(&mut inner.node),
            A::Expr::Switch { scrutinee, cases, default } => {
                self.expr(&mut scrutinee.node);
                for c in cases.iter_mut() {
                    self.pattern(&mut c.pattern);
                    if let Some(g) = c.guard.as_mut() {
                        self.expr(&mut g.node);
                    }
                    self.scope(|rw| match &mut c.body {
                        A::SwitchExprBody::Expr(x) => rw.expr(&mut x.node),
                        A::SwitchExprBody::Block(b) => rw.block(b),
                    });
                }
                if let Some(d) = default {
                    self.scope(|rw| match d {
                        A::SwitchExprBody::Expr(x) => rw.expr(&mut x.node),
                        A::SwitchExprBody::Block(b) => rw.block(b),
                    });
                }
            }
            A::Expr::Interp(parts) => {
                for part in parts.iter_mut() {
                    if let A::InterpPart::Expr(x) = part {
                        self.expr(&mut x.node);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::testkit;

    fn registry_app(tag: &str, registry_url: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-modreg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("app");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n        \"@acme/widget\": {{ version: \"^1.0.0\" }}\n    }},\n    registry: {{ url: \"{registry_url}\" }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("src").join("main.rnx"),
            "import { hello } from \"@acme/widget\";\nfn Main(): Int { return hello(); }\n",
        )
        .unwrap();
        root
    }

    fn start_widget_server() -> testkit::MockRegistry {
        let (gz, sha) = testkit::fixture_tarball("@acme/widget", "1.2.0");
        let node = testkit::node_json("@acme/widget", "1.2.0", &sha, false);
        testkit::MockRegistry::start(testkit::MockConfig {
            version_spec: 1,
            resolve_status: 200,
            resolve_body: testkit::resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: gz,
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        })
    }

    #[test]
    fn semver_import_resolves_through_registry() {
        let (_guard, _cache) = testkit::isolate_cache("modreg");
        let server = start_widget_server();
        let root = registry_app("live", &server.base);
        let entry = root.join("src").join("main.rnx");
        let graph = ModuleGraph::build(&entry).unwrap();
        assert!(graph.files.iter().any(|f| f.path.ends_with("src/main.rnx")));
        let merged = graph.resolve().unwrap();
        assert!(!merged.decls.is_empty());
        let bodies = server.resolve_bodies();
        assert!(!bodies.is_empty());
        assert_eq!(testkit::requirement(&bodies[0], "@acme/widget"), "^1.0.0");
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn locked_registry_version_pins_the_request() {
        let (_guard, _cache) = testkit::isolate_cache("modpin");
        let server = start_widget_server();
        let root = registry_app("pinned", &server.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let mut lock = crate::deplock::ProjectDepLock::resolve(&root, &cfg).unwrap();
        lock.write(&root).unwrap();
        let entry = root.join("src").join("main.rnx");
        ModuleGraph::build(&entry).unwrap();
        let bodies = server.resolve_bodies();
        assert!(bodies.len() >= 2);
        let last = bodies.last().unwrap();
        assert_eq!(testkit::requirement(last, "@acme/widget"), "1.2.0");
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    fn std_overrides(url: &str) -> BTreeMap<String, crate::project::RegistryConfig> {
        let mut m = BTreeMap::new();
        m.insert("@std".to_string(), testkit::registry_cfg(url));
        m
    }

    fn dead_overrides() -> BTreeMap<String, crate::project::RegistryConfig> {
        std_overrides("http://127.0.0.1:1")
    }

    fn start_std_server() -> testkit::MockRegistry {
        let pin = env!("CARGO_PKG_VERSION");
        let (gz, sha) = testkit::fixture_tarball("@std/fs", pin);
        let node = testkit::node_json("@std/fs", pin, &sha, false);
        testkit::MockRegistry::start(testkit::MockConfig {
            version_spec: 1,
            resolve_status: 200,
            resolve_body: testkit::resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: gz,
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        })
    }

    #[test]
    fn std_resolves_through_registry_under_the_hood() {
        let (_guard, _cache) = testkit::isolate_cache("stdreg");
        let server = start_std_server();
        let (target, dep) =
            resolve_std_with_registries("@std/fs", "fs", None, &std_overrides(&server.base)).unwrap();
        assert_eq!(target, PathBuf::from("@std/fs"));
        assert!(dep.is_none());
        let overlaid = std_overlay_get(&PathBuf::from("@std/fs")).unwrap();
        assert!(overlaid.ends_with("src/main.rnx"));
        let bodies = server.resolve_bodies();
        assert!(!bodies.is_empty());
        assert_eq!(
            testkit::requirement(bodies.last().unwrap(), "@std/fs"),
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn std_cache_hit_needs_no_network() {
        let (_guard, _cache) = testkit::isolate_cache("stdcache");
        let server = start_std_server();
        let first =
            resolve_std_with_registries("@std/fs", "fs", None, &std_overrides(&server.base)).unwrap();
        let served = server.requests().len();
        assert!(served > 0);
        let second =
            resolve_std_with_registries("@std/fs", "fs", None, &std_overrides(&server.base)).unwrap();
        assert_eq!(first, second);
        assert_eq!(server.requests().len(), served);
    }

    #[test]
    fn std_falls_back_to_embedded_offline() {
        let (_guard, _cache) = testkit::isolate_cache("stdfb");
        let (target, dep) =
            resolve_std_with_registries("@std/fs", "fs", None, &dead_overrides()).unwrap();
        assert_eq!(target, PathBuf::from("@std/fs"));
        assert!(dep.is_none());
        assert_eq!(std_overlay_get(&PathBuf::from("@std/fs")), None);
    }

    #[test]
    fn std_unknown_module_errors() {
        let (_guard, _cache) = testkit::isolate_cache("stdbad");
        let err = resolve_std_with_registries("@std/nope", "nope", None, &dead_overrides()).unwrap_err();
        assert!(err.message.contains("unknown standard library module"));
        assert_eq!(std_overlay_get(&PathBuf::from("@std/nope")), None);
    }

    #[test]
    fn std_overlay_serves_registry_bytes() {
        let (_guard, _cache) = testkit::isolate_cache("stdread");
        let server = start_std_server();
        resolve_std_with_registries("@std/fs", "fs", None, &std_overrides(&server.base)).unwrap();
        let src = read_source(&PathBuf::from("@std/fs")).unwrap();
        assert!(src.contains("export fn hello"));
    }
}
