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
    /// prelude from the seeded global cache (downloading missing modules
    /// from the registry like any other dependency), and runs the same
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
            let src = load_std_module_source(&name)?;
            let module = crate::parser::Parser::parse_module(&src)?;
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
        let dep_gen = crate::depcache::begin_build();
        let mut edge_log: Vec<LoggedEdge> = Vec::new();
        if let Some(build_gen) = dep_gen {
            dep_restore_for_entry(entry_root.as_ref(), build_gen);
        }
        visit(
            &root,
            &base,
            &root,
            entry_root.as_ref(),
            &mut state,
            &mut order,
            &mut stack,
            &mut parse_errors,
            &mut edge_log,
        )
        .map_err(|e| vec![e])?;
        let want_prelude = if cfg!(target_arch = "wasm32") {
            crate::stdvfs::std_source("prelude").is_some()
        } else {
            true
        };
        if want_prelude {
            visit(
                &PathBuf::from("@std/prelude"),
                &base,
                &root,
                entry_root.as_ref(),
                &mut state,
                &mut order,
                &mut stack,
                &mut parse_errors,
                &mut edge_log,
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
                &mut edge_log,
            )
            .map_err(|e| vec![e])?;
        }
        if parse_errors.is_empty() {
            if dep_gen.is_some() {
                dep_populate_for_entry(entry_root.as_ref(), &edge_log);
            }
            Ok(ModuleGraph { root, files: order })
        } else {
            Err(parse_errors)
        }
    }

    pub fn build_collecting_parallel(root: &Path, jobs: usize) -> Result<ModuleGraph, Vec<Diagnostic>> {
        Self::build_collecting_extra_parallel(root, &[], jobs)
    }

    pub fn build_collecting_extra_parallel(
        root: &Path,
        extra: &[PathBuf],
        jobs: usize,
    ) -> Result<ModuleGraph, Vec<Diagnostic>> {
        if jobs <= 1 {
            return Self::build_collecting_extra(root, extra);
        }
        let pool = match rayon::ThreadPoolBuilder::new().num_threads(jobs).build() {
            Ok(pool) => Some(pool),
            Err(_) => None,
        };
        let root = canonical(root).map_err(|e| vec![e])?;
        let base = root.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let entry_root = package_ctx_of(&root)
            .map_err(|e| vec![e])?
            .map(|(r, _)| r);
        let dep_gen = crate::depcache::begin_build();
        if let Some(build_gen) = dep_gen {
            dep_restore_for_entry(entry_root.as_ref(), build_gen);
        }
        let overlay_base = snapshot_overlay();
        let extra_canon: Vec<(PathBuf, Result<PathBuf, Diagnostic>)> = extra
            .iter()
            .map(|e| {
                (
                    e.clone(),
                    canonical(e).map_err(|_| {
                        Diagnostic::new(Code::E108, format!("cannot read `{}`", e.display()))
                    }),
                )
            })
            .collect();
        let mut discover = Discover::new(pool.as_ref(), overlay_base);
        discover.run(vec![root.clone()]);
        let want_prelude = if cfg!(target_arch = "wasm32") {
            crate::stdvfs::std_source("prelude").is_some()
        } else {
            true
        };
        if !discover.clash
            && want_prelude
            && !discover.visited.contains(&PathBuf::from("@std/prelude"))
        {
            discover.run(vec![PathBuf::from("@std/prelude")]);
        }
        if !discover.clash {
            let mut seeds: Vec<PathBuf> = extra_canon
                .iter()
                .filter_map(|(_, r)| r.as_ref().ok().cloned())
                .collect();
            seeds.sort();
            seeds.dedup();
            seeds.retain(|p| !discover.visited.contains(p));
            if !seeds.is_empty() {
                discover.run(seeds);
            }
        }
        if discover.clash {
            restore_overlay(&discover.overlay_base);
            assemble_graph(&root, &base, entry_root, &extra_canon, want_prelude, &LiveBackend, dep_gen)
        } else {
            let backend = ReplayBackend { files: &discover.files };
            assemble_graph(&root, &base, entry_root, &extra_canon, want_prelude, &backend, dep_gen)
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
        if let Some(src) = crate::stdvfs::std_source("prelude") {
            if let Ok(module) = crate::parser::Parser::parse_module(&src) {
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

pub fn std_rest(source: &str) -> &str {
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

static PROJECT_MOUNT: std::sync::Mutex<BTreeMap<PathBuf, String>> =
    std::sync::Mutex::new(BTreeMap::new());

fn mount_lock() -> std::sync::MutexGuard<'static, BTreeMap<PathBuf, String>> {
    PROJECT_MOUNT.lock().unwrap_or_else(|e| e.into_inner())
}

fn normalize_mount(path: &Path) -> PathBuf {
    let mut parts: Vec<String> = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if parts.pop().is_none() {
                    parts.push("..".to_string());
                }
            }
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {}
        }
    }
    let mut out = PathBuf::new();
    out.extend(parts);
    out
}

pub fn project_mount(path: &Path, source: String) {
    mount_lock().insert(normalize_mount(path), source);
}

pub fn project_unmount_all() {
    mount_lock().clear();
}

fn project_get(path: &Path) -> Option<String> {
    mount_lock().get(&normalize_mount(path)).cloned()
}

fn project_has(path: &Path) -> bool {
    mount_lock().contains_key(&normalize_mount(path))
}

fn canonical(path: &Path) -> Result<PathBuf, Diagnostic> {
    if project_has(path) {
        return Ok(normalize_mount(path));
    }
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
        if let Some(cached) = crate::depcache::bytes_get(path) {
            return Ok(cached);
        }
        if let Some(src) = crate::stdvfs::std_source(&rest) {
            return Ok(src);
        }
        if cfg!(target_arch = "wasm32") {
            return Err(playground_std_error(&name));
        }
        if !crate::stdvfs::is_known_module(&rest) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("unknown standard library module `{name}`"),
            ));
        }
        return Err(std_cache_miss_error(&name, &std_cache_registry()));
    }
    if let Some(src) = project_get(path) {
        return Ok(src);
    }
    if let Some(src) = crate::depcache::bytes_get(path) {
        return Ok(src);
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
    resolve_import_meta(from, source).map(|(target, dep_root, _)| (target, dep_root))
}

fn resolve_import_meta(
    from: &Path,
    source: &str,
) -> Result<(PathBuf, Option<PathBuf>, Option<(String, String)>), Diagnostic> {
    if let Some(hit) = crate::depcache::memo_lookup(from, source) {
        return Ok(hit);
    }
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
        let empty: BTreeMap<String, RegistryConfig> = BTreeMap::new();
        return resolve_std_with_registries(from, source, rest, None, &empty);
    }
    if source.starts_with('.') {
        return resolve_relative(from, source).map(|(t, d)| (t, d, None));
    }
    resolve_package(from, source)
}

/// `@std/*` through the same-domain registry server under the hood: exact
/// pinned version from the global pin file, integrity-checked cache,
/// registry download on a miss. Explicit `dependencies` semver entries
/// override the pin like any other package. No URL imports exist in user
/// code; all fetching here is toolchain-initiated.
///
/// The resolved path is ALWAYS the virtual `@std/<rest>` path, so module
/// keys and merged output are identical online and offline. A registry hit
/// only swaps the *bytes* behind it via the overlay read by `read_source`.
/// A VFS hit (tests, playground preloads) short-circuits before any cache
/// or network access. Offline with an empty cache fails loudly as E108
/// naming the registry with an `rnx fetch-std` hint.
fn resolve_std_with_registries(
    from: &Path,
    source: &str,
    rest: &str,
    default: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
) -> Result<(PathBuf, Option<PathBuf>, Option<(String, String)>), Diagnostic> {
    let virtual_path = PathBuf::from(format!("{STD_SCOPE}{rest}"));
    let (pkg, sub) = match rest.split_once('/') {
        Some((top, s)) => (format!("@std/{top}"), Some(s)),
        None => (format!("@std/{rest}"), None),
    };
    let meta = Some((pkg.clone(), crate::depcache::KIND_STD.to_string()));
    if cfg!(target_arch = "wasm32") {
        if crate::stdvfs::std_source(rest).is_some() {
            return Ok((virtual_path, None, meta));
        }
        return Err(playground_std_error(source));
    }
    if !crate::stdvfs::is_known_module(rest) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("unknown standard library module `{source}`"),
        ));
    }
    if crate::stdvfs::get(rest).is_some() {
        clear_std_overlay(&virtual_path);
        return Ok((virtual_path, None, meta));
    }
    let (default_owned, owned_overrides, explicit, locked) = std_ctx_for(from, &pkg);
    let default_ref = default.or(default_owned.as_ref());
    let mut merged = owned_overrides;
    for (k, v) in overrides {
        merged.entry(k.clone()).or_insert_with(|| v.clone());
    }
    let base = crate::fetch::registry_base_for(&pkg, default_ref, &merged);
    let pin_ver = crate::stdlib_seed::read_std_pin()
        .and_then(|pin| pin.packages.get(&pkg).cloned());
    let cli = env!("CARGO_PKG_VERSION").to_string();
    let requirement = explicit.clone().or_else(|| pin_ver.clone()).unwrap_or(cli);
    let pinned = locked.or_else(|| match &explicit {
        None => pin_ver.or_else(|| exact_version(&requirement)),
        Some(range) => exact_version(range),
    });
    let have = crate::fetch::scan_cache_have();
    let mut detail: Option<Diagnostic> = None;
    match crate::fetch::resolve_registry_package(
        &pkg,
        &requirement,
        default_ref,
        &merged,
        pinned,
        &have,
    ) {
        Ok((dep_root, dep_cfg, _)) => {
            if let Ok(target) =
                resolve_subpath_target(&dep_root, sub.as_deref(), &dep_cfg, &pkg, source)
            {
                set_std_overlay(&virtual_path, &target);
                return Ok((virtual_path, None, meta));
            }
        }
        Err(e) => {
            detail = Some(e);
        }
    }
    clear_std_overlay(&virtual_path);
    if crate::stdvfs::std_source(rest).is_some() {
        return Ok((virtual_path, None, meta));
    }
    Err(match detail {
        Some(e) => std_fetch_error(source, &base, &e.message),
        None => std_cache_miss_error(source, &base),
    })
}

fn std_ctx_for(
    from: &Path,
    pkg: &str,
) -> (
    Option<RegistryConfig>,
    BTreeMap<String, RegistryConfig>,
    Option<String>,
    Option<String>,
) {
    let (env_default, env_overrides) = crate::stdlib_seed::std_registry_from_env();
    let ctx = package_ctx_of(from).ok().flatten();
    let Some((proj_root, cfg)) = ctx else {
        return (env_default, env_overrides, None, None);
    };
    let default = cfg.registry.clone().or(env_default);
    let mut merged = env_overrides;
    for (k, v) in &cfg.registries {
        merged.entry(k.clone()).or_insert_with(|| v.clone());
    }
    let explicit = match cfg.dependencies.get(pkg) {
        Some(crate::project::DependencySpec::Semver { version }) => Some(version.clone()),
        _ => None,
    };
    let scope_root = project::find_workspace_root(&proj_root).unwrap_or(proj_root);
    let locked = locked_registry_version(&scope_root, pkg);
    (default, merged, explicit, locked)
}

pub(crate) fn exact_version(range: &str) -> Option<String> {
    let r = range.trim();
    if r.is_empty() || r == "*" || r == "latest" {
        return None;
    }
    if r.starts_with(['^', '~', '>', '<', '=']) {
        return None;
    }
    let (core, _) = match r.split_once('-') {
        Some((c, _)) => (c, true),
        None => (r, false),
    };
    let mut parts = core.split('.');
    let ok = parts.clone().count() == 3
        && parts.all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if ok {
        Some(r.to_string())
    } else {
        None
    }
}

pub fn load_std_module_source(rest: &str) -> Result<String, Diagnostic> {
    let rest = crate::stdvfs::normalize_rest(rest);
    if rest.is_empty() {
        return Err(
            Diagnostic::new(Code::E108, "cannot resolve module `@std`")
                .with_hint("use `@std/<module>`, e.g. `@std/time`"),
        );
    }
    if let Some(src) = crate::stdvfs::std_source(&rest) {
        return Ok(src);
    }
    if cfg!(target_arch = "wasm32") {
        return Err(playground_std_error(&format!("@std/{rest}")));
    }
    if !crate::stdvfs::is_known_module(&rest) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("unknown standard library module `@std/{rest}`"),
        ));
    }
    let (default, overrides) = crate::stdlib_seed::std_registry_from_env();
    let top = rest.split('/').next().unwrap_or(rest.as_str());
    let pkg = format!("@std/{top}");
    let base = crate::fetch::registry_base_for(&pkg, default.as_ref(), &overrides);
    let version = crate::stdlib_seed::read_std_pin()
        .and_then(|pin| pin.packages.get(&pkg).cloned())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let have = crate::fetch::scan_cache_have();
    match crate::fetch::resolve_registry_package(
        &pkg,
        &version,
        default.as_ref(),
        &overrides,
        Some(version.clone()),
        &have,
    ) {
        Ok(_) => crate::stdvfs::std_source(&rest)
            .ok_or_else(|| std_cache_miss_error(&format!("@std/{rest}"), &base)),
        Err(e) => Err(std_fetch_error(&format!("@std/{rest}"), &base, &e.message)),
    }
}

fn playground_std_error(source: &str) -> Diagnostic {
    Diagnostic::new(
        Code::E108,
        format!("standard library module `{source}` is not preloaded in the playground engine"),
    )
    .with_hint("the playground preloads `@std/prelude` at engine download and fetches other `@std/*` modules from the same-domain registry before checking")
}

fn std_fetch_error(source: &str, base: &str, detail: &str) -> Diagnostic {
    Diagnostic::new(
        Code::E108,
        format!("cannot resolve `{source}` from registry `{base}`: {detail}"),
    )
    .with_hint("run `rnx fetch-std` to seed the global cache (or `rnx doctor --repair-std` to repair it)")
}

fn std_cache_miss_error(source: &str, base: &str) -> Diagnostic {
    Diagnostic::new(
        Code::E108,
        format!("cannot resolve `{source}`: stdlib cache is empty and registry `{base}` is unreachable"),
    )
    .with_hint("run `rnx fetch-std` to seed the global cache (or `rnx doctor --repair-std` to repair it)")
}

fn std_cache_registry() -> String {
    crate::stdlib_seed::read_std_pin()
        .map(|pin| pin.registry)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| crate::fetch::expand_registry_base(""))
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
        if project_has(candidate) {
            return Ok((normalize_mount(candidate), None));
        }
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

fn resolve_package(
    from: &Path,
    source: &str,
) -> Result<(PathBuf, Option<PathBuf>, Option<(String, String)>), Diagnostic> {
    let (pkg, sub) = split_package_spec(source);
    if pkg.is_empty() || pkg == "@" {
        return Err(Diagnostic::new(Code::E108, format!("cannot resolve module `{source}`")));
    }
    let unknown = || Diagnostic::new(Code::E108, format!("unknown package dependency `{pkg}`"));
    let (proj_root, cfg) = package_ctx_of(from)?.ok_or_else(unknown)?;
    let (dep_root, dep_cfg, kind) = match cfg.dependencies.get(pkg.as_str()) {
        Some(crate::project::DependencySpec::Path { path }) => {
            let dep_root = std::fs::canonicalize(proj_root.join(path)).map_err(|_| {
                Diagnostic::new(Code::E108, format!("cannot resolve package `{pkg}`"))
            })?;
            let dep_cfg = ProjectConfig::load_from_dir(&dep_root)?.ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("package `{pkg}` has no Project.config"))
            })?;
            (dep_root, dep_cfg, crate::depcache::KIND_PATH.to_string())
        }
        Some(crate::project::DependencySpec::Git { git, rev }) => {
            let anchor = crate::fetch::cache_anchor(&proj_root);
            let dep_root = crate::fetch::resolve_git_dep(&anchor, &pkg, git, rev)?;
            let dep_cfg = ProjectConfig::load_from_dir(&dep_root)?.ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("package `{pkg}` has no Project.config"))
            })?;
            (dep_root, dep_cfg, crate::depcache::KIND_GIT.to_string())
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
            (dep_root, dep_cfg, crate::depcache::KIND_SEMVER.to_string())
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
        None => workspace_sibling(&proj_root, &pkg)?
            .ok_or_else(unknown)
            .map(|(r, c)| (r, c, crate::depcache::KIND_WORKSPACE.to_string()))?,
    };
    let target = resolve_subpath_target(&dep_root, sub.as_deref(), &dep_cfg, &pkg, source)?;
    Ok((target, Some(dep_root), Some((pkg, kind))))
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

fn std_overlay_lookup(path: &Path) -> Option<PathBuf> {
    STD_OVERLAY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .cloned()
}

fn dep_spec_string(spec: &crate::project::DependencySpec) -> String {
    match spec {
        crate::project::DependencySpec::Semver { version } => format!("semver:{version}"),
        crate::project::DependencySpec::Path { path } => format!("path:{}", path.display()),
        crate::project::DependencySpec::Git { git, rev } => format!("git:{git}@{rev}"),
        crate::project::DependencySpec::Url { version, url, checksum } => {
            format!("url:{url}@{version}:{}", checksum.as_deref().unwrap_or(""))
        }
        crate::project::DependencySpec::Native { lib, system, path } => format!(
            "native:{lib}:{system}:{}",
            path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
        ),
    }
}

fn dep_scope_root(entry_root: &Path) -> PathBuf {
    project::find_workspace_root(entry_root).unwrap_or_else(|| entry_root.to_path_buf())
}

fn dep_entry_snapshot(
    entry_root: &Path,
) -> Option<(ProjectConfig, Vec<u8>, Vec<u8>, Vec<u8>)> {
    let entry_cfg = ProjectConfig::load_from_dir(entry_root).ok()??;
    let entry_raw =
        std::fs::read(entry_root.join(project::MANIFEST_FILE)).unwrap_or_default();
    let scope_root = dep_scope_root(entry_root);
    let scope_raw = if scope_root == *entry_root {
        entry_raw.clone()
    } else {
        std::fs::read(scope_root.join(project::MANIFEST_FILE)).unwrap_or_default()
    };
    let lock_raw = std::fs::read(scope_root.join(crate::deplock::LOCK_FILE)).unwrap_or_default();
    Some((entry_cfg, entry_raw, scope_raw, lock_raw))
}

fn dep_set_hex(
    entry_raw: &[u8],
    scope_raw: &[u8],
    lock_raw: &[u8],
    dep_cfg_raw: &[u8],
    dep_subs: &[String],
) -> String {
    let mut parts = vec![
        format!("entry={}", crate::depcache::hex_bytes(entry_raw)),
        format!("scope={}", crate::depcache::hex_bytes(scope_raw)),
        format!("lock={}", crate::depcache::hex_bytes(lock_raw)),
        format!("dep={}", crate::depcache::hex_bytes(dep_cfg_raw)),
    ];
    for s in dep_subs {
        parts.push(format!("sub={s}"));
    }
    crate::depcache::dep_set_digest_hex(&parts)
}

fn dep_sub_specs(cfg: &ProjectConfig) -> Vec<String> {
    cfg.dependencies
        .iter()
        .map(|(n, s)| format!("{n}={}", dep_spec_string(s)))
        .collect()
}

fn dep_version_key(
    kind: &str,
    pkg: &str,
    entry_root: &Path,
    entry_cfg: &ProjectConfig,
    dep_cfg: Option<&ProjectConfig>,
) -> Option<String> {
    use crate::depcache::*;
    if kind == KIND_PATH || kind == KIND_WORKSPACE {
        return dep_cfg.map(|c| c.version.clone());
    }
    if kind == KIND_GIT {
        return match entry_cfg.dependencies.get(pkg) {
            Some(crate::project::DependencySpec::Git { rev, .. }) => Some(rev.clone()),
            _ => None,
        };
    }
    if kind == KIND_SEMVER {
        let requirement = match entry_cfg.dependencies.get(pkg) {
            Some(crate::project::DependencySpec::Semver { version }) => version.clone(),
            _ => return None,
        };
        let scope_root = dep_scope_root(entry_root);
        if let Some(locked) = locked_registry_version(&scope_root, pkg) {
            return Some(locked);
        }
        return exact_version(&requirement);
    }
    if kind == KIND_STD {
        let scope_root = dep_scope_root(entry_root);
        if let Some(locked) = locked_registry_version(&scope_root, pkg) {
            return Some(locked);
        }
        let explicit = match entry_cfg.dependencies.get(pkg) {
            Some(crate::project::DependencySpec::Semver { version }) => Some(version.clone()),
            _ => None,
        };
        let pin_ver = crate::stdlib_seed::read_std_pin()
            .and_then(|pin| pin.packages.get(pkg).cloned());
        let requirement = explicit
            .or(pin_ver)
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
        return exact_version(&requirement)
            .or_else(|| Some(requirement));
    }
    None
}

fn dep_expected_spec(
    kind: &str,
    pkg: &str,
    entry_cfg: &ProjectConfig,
) -> Option<String> {
    use crate::depcache::*;
    if kind == KIND_STD {
        let top = pkg.strip_prefix("@std/")?;
        if top.is_empty() || top.contains('/') || !crate::stdvfs::is_known_module(top) {
            return None;
        }
        return Some("std".to_string());
    }
    if kind == KIND_WORKSPACE {
        return Some("workspace".to_string());
    }
    entry_cfg.dependencies.get(pkg).map(dep_spec_string)
}

fn dep_expected_root(
    kind: &str,
    pkg: &str,
    entry_root: &Path,
    entry_cfg: &ProjectConfig,
    version_key: &str,
) -> Option<PathBuf> {
    use crate::depcache::*;
    if kind == KIND_PATH {
        let crate::project::DependencySpec::Path { path } = entry_cfg.dependencies.get(pkg)?
        else {
            return None;
        };
        return std::fs::canonicalize(entry_root.join(path)).ok();
    }
    if kind == KIND_GIT {
        let crate::project::DependencySpec::Git { rev, .. } = entry_cfg.dependencies.get(pkg)?
        else {
            return None;
        };
        let anchor = crate::fetch::cache_anchor(entry_root);
        let vendor = anchor.join("vendor").join(pkg);
        if vendor.join(crate::project::MANIFEST_FILE).is_file() {
            return std::fs::canonicalize(&vendor).ok();
        }
        let dir = anchor
            .join(".rnx-cache")
            .join("cache")
            .join("git")
            .join(format!("{pkg}-{}", crate::fetch::short_rev(rev)));
        return std::fs::canonicalize(&dir).ok();
    }
    if kind == KIND_SEMVER {
        let base = crate::fetch::registry_base_for(
            pkg,
            entry_cfg.registry.as_ref(),
            &entry_cfg.registries,
        );
        let dir = crate::fetch::cached_package_dir(&base, pkg, version_key);
        return std::fs::canonicalize(&dir).ok();
    }
    if kind == KIND_WORKSPACE {
        let ws_root = project::find_workspace_root_strict(entry_root)?;
        let manifest = project::load_manifest(&ws_root).ok()??;
        let members = project::resolve_workspace_members(&ws_root, &manifest.workspace?).ok()?;
        let (member_root, _) = members.get(pkg)?;
        return std::fs::canonicalize(member_root).ok();
    }
    None
}

fn dep_expected_base(
    kind: &str,
    pkg: &str,
    entry_root: &Path,
    entry_cfg: &ProjectConfig,
) -> String {
    use crate::depcache::*;
    if kind == KIND_SEMVER {
        return crate::fetch::registry_base_for(
            pkg,
            entry_cfg.registry.as_ref(),
            &entry_cfg.registries,
        );
    }
    if kind == KIND_STD {
        let from = entry_cfg.main_path(entry_root);
        let (default_owned, owned_overrides, _, _) = std_ctx_for(&from, pkg);
        return crate::fetch::registry_base_for(
            pkg,
            default_owned.as_ref(),
            &owned_overrides,
        );
    }
    String::new()
}

fn dep_git_commit(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(".rnx-fetch")).ok()?;
    text.lines().nth(2).map(|s| s.trim().to_string())
}

fn collect_dep_products(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 4 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.file_type().is_dir() {
            collect_dep_products(&path, out, depth + 1);
        } else if meta.file_type().is_file()
            && path.file_name().is_some_and(|n| n == crate::depcache::PRODUCT_FILE)
        {
            out.push(path);
        }
    }
}

fn dep_restore_for_entry(entry_root: Option<&PathBuf>, build_gen: u64) {
    let Some(entry_root) = entry_root else { return };
    let Some((entry_cfg, entry_raw, scope_raw, lock_raw)) = dep_entry_snapshot(entry_root) else {
        return;
    };
    let toolchain = crate::depcache::dep_toolchain_hash(None);
    let mut products = Vec::new();
    collect_dep_products(&crate::cache::project_deps_dir(entry_root), &mut products, 0);
    products.sort();
    let mut restored: BTreeSet<(String, String)> = BTreeSet::new();
    for path in products {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let Some(product) = crate::depcache::decode_product(&bytes) else {
            continue;
        };
        if validate_dep_product(
            &product,
            entry_root,
            &entry_cfg,
            &entry_raw,
            &scope_raw,
            &lock_raw,
            &toolchain,
            build_gen,
        ) {
            restored.insert((product.pkg.clone(), product.kind.clone()));
        }
    }
    dep_restore_global(
        entry_root,
        &entry_cfg,
        &entry_raw,
        &scope_raw,
        &lock_raw,
        &toolchain,
        build_gen,
        &mut restored,
    );
}

fn dep_restore_global(
    entry_root: &Path,
    entry_cfg: &ProjectConfig,
    entry_raw: &[u8],
    scope_raw: &[u8],
    lock_raw: &[u8],
    toolchain: &str,
    build_gen: u64,
    restored: &mut BTreeSet<(String, String)>,
) {
    use crate::depcache::*;
    let mut candidates: Vec<(String, &str)> = Vec::new();
    for (pkg, spec) in &entry_cfg.dependencies {
        match spec {
            crate::project::DependencySpec::Git { .. } => {
                candidates.push((pkg.clone(), KIND_GIT));
            }
            crate::project::DependencySpec::Semver { .. } if pkg.starts_with("@std/") => {
                candidates.push((pkg.clone(), KIND_STD));
            }
            crate::project::DependencySpec::Semver { .. } => {
                candidates.push((pkg.clone(), KIND_SEMVER));
            }
            _ => {}
        }
    }
    candidates.sort();
    candidates.dedup();
    if !candidates.iter().any(|(pkg, _)| pkg == "@std/prelude") {
        candidates.push(("@std/prelude".to_string(), KIND_STD));
    }
    for (pkg, kind) in candidates {
        if restored.contains(&(pkg.clone(), kind.to_string())) {
            continue;
        }
        let Some(version_key) = dep_version_key(kind, &pkg, entry_root, entry_cfg, None) else {
            continue;
        };
        if !crate::products::exact_pin(kind, &version_key) {
            continue;
        }
        let base = dep_expected_base(kind, &pkg, entry_root, entry_cfg);
        let Some(dir) = crate::products::product_dir(kind, &base, &pkg, &version_key, toolchain)
        else {
            continue;
        };
        for path in crate::products::scan_product_files(&dir) {
            let Some(product) = crate::products::read_product_file(&path) else {
                continue;
            };
            if validate_dep_product(
                &product,
                entry_root,
                entry_cfg,
                entry_raw,
                scope_raw,
                lock_raw,
                toolchain,
                build_gen,
            ) {
                restored.insert((product.pkg.clone(), product.kind.clone()));
                break;
            }
        }
    }
}

pub fn product_statuses(entry_root: &Path) -> Vec<crate::products::ProductStatus> {
    use crate::depcache::*;
    let mut out = Vec::new();
    let entry_cfg = match ProjectConfig::load_from_dir(entry_root) {
        Ok(Some(cfg)) => cfg,
        _ => return out,
    };
    let toolchain = crate::depcache::dep_toolchain_hash(None);
    let mut pkgs: Vec<(String, &str)> = Vec::new();
    for (pkg, spec) in &entry_cfg.dependencies {
        match spec {
            crate::project::DependencySpec::Git { .. } => pkgs.push((pkg.clone(), KIND_GIT)),
            crate::project::DependencySpec::Semver { .. } if pkg.starts_with("@std/") => {
                pkgs.push((pkg.clone(), KIND_STD));
            }
            crate::project::DependencySpec::Semver { .. } => {
                pkgs.push((pkg.clone(), KIND_SEMVER));
            }
            _ => {}
        }
    }
    pkgs.sort();
    pkgs.dedup();
    for (pkg, kind) in pkgs {
        let version = dep_version_key(kind, &pkg, entry_root, &entry_cfg, None);
        let present = match &version {
            Some(version_key) if crate::products::exact_pin(kind, version_key) => {
                let base = dep_expected_base(kind, &pkg, entry_root, &entry_cfg);
                crate::products::product_dir(kind, &base, &pkg, version_key, &toolchain)
                    .is_some_and(|dir| !crate::products::scan_product_files(&dir).is_empty())
            }
            _ => false,
        };
        out.push(crate::products::ProductStatus {
            pkg,
            kind: kind.to_string(),
            version,
            present,
        });
    }
    out
}

fn read_product_current(files: &[crate::depcache::DepFile]) -> Option<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let path = PathBuf::from(&f.path);
        let bytes = if is_virtual(&path) {
            // Bypass the resolve overlay: restore runs before any resolve
            // in this build, so the overlay can only hold another build's
            // stale mapping. The pinned cache is the validation source.
            let rest = std_submodule(&path);
            crate::stdvfs::std_source(&rest)?.into_bytes()
        } else {
            std::fs::read(&path).ok()?
        };
        out.push((f.path.clone(), bytes));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Some(out)
}

fn validate_dep_product(
    product: &crate::depcache::DepProduct,
    entry_root: &Path,
    entry_cfg: &ProjectConfig,
    entry_raw: &[u8],
    scope_raw: &[u8],
    lock_raw: &[u8],
    toolchain: &str,
    build_gen: u64,
) -> bool {
    use crate::depcache::*;
    if product.toolchain != toolchain {
        return false;
    }
    let Some(expected_spec) = dep_expected_spec(&product.kind, &product.pkg, entry_cfg) else {
        return false;
    };
    if product.spec != expected_spec {
        return false;
    }
    let root_path = PathBuf::from(&product.root);
    let dep_cfg_raw = match std::fs::read(root_path.join(crate::project::MANIFEST_FILE)) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let dep_cfg = match ProjectConfig::load_from_dir(&root_path) {
        Ok(Some(c)) => c,
        _ => return false,
    };
    let Some(version_key) =
        dep_version_key(&product.kind, &product.pkg, entry_root, entry_cfg, Some(&dep_cfg))
    else {
        return false;
    };
    if product.version_key != version_key {
        return false;
    }
    if product.kind == KIND_STD {
        if dep_cfg.name != product.pkg || dep_cfg.version != version_key {
            return false;
        }
    } else {
        let Some(expected) =
            dep_expected_root(&product.kind, &product.pkg, entry_root, entry_cfg, &version_key)
        else {
            return false;
        };
        if expected != root_path {
            return false;
        }
    }
    if product.kind == KIND_SEMVER || product.kind == KIND_STD {
        if dep_expected_base(&product.kind, &product.pkg, entry_root, entry_cfg) != product.base {
            return false;
        }
    }
    if product.kind == KIND_GIT {
        if product.commit.is_empty() {
            let anchor = crate::fetch::cache_anchor(entry_root);
            let vendor = anchor.join("vendor").join(&product.pkg);
            if !vendor.join(crate::project::MANIFEST_FILE).is_file() {
                return false;
            }
        } else if dep_git_commit(&root_path).as_deref() != Some(product.commit.as_str()) {
            return false;
        }
    }
    // KIND_STD uses a project-independent dep set: std resolution inputs
    // are fully covered by version_key, base, and the content digest, so
    // entry/scope/lock bytes are pinned empty. Entry-manifest edits
    // elsewhere then keep hitting, while dep-manifest edits still
    // invalidate. This lets install/fetch-std-time std products restore
    // into any project on the same pin instead of sharding per entry.
    let expected_dep_set = if product.kind == KIND_STD {
        dep_set_hex(b"", b"", b"", &dep_cfg_raw, &dep_sub_specs(&dep_cfg))
    } else {
        dep_set_hex(entry_raw, scope_raw, lock_raw, &dep_cfg_raw, &dep_sub_specs(&dep_cfg))
    };
    if expected_dep_set != product.dep_set {
        return false;
    }
    let Some(current) = read_product_current(&product.files) else {
        return false;
    };
    if crate::depcache::content_digest_hex(&current) != product.content {
        return false;
    }
    let mut edges = Vec::with_capacity(product.edges.len());
    for e in &product.edges {
        edges.push((
            PathBuf::from(&e.from),
            e.source.clone(),
            PathBuf::from(&e.target),
            e.dep_root.as_ref().map(PathBuf::from),
            e.meta.clone(),
        ));
    }
    let mut blobs = Vec::with_capacity(product.files.len());
    for f in &product.files {
        let bytes = current
            .iter()
            .find(|(p, _)| p == &f.path)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        blobs.push((PathBuf::from(&f.path), text));
    }
    crate::depcache::memo_install(build_gen, &edges);
    crate::depcache::bytes_install(build_gen, &blobs);
    crate::depcache::note_hit();
    true
}

fn dep_populate_for_entry(entry_root: Option<&PathBuf>, log: &[LoggedEdge]) {
    let Some(entry_root) = entry_root else { return };
    let Some((entry_cfg, entry_raw, scope_raw, lock_raw)) = dep_entry_snapshot(entry_root) else {
        return;
    };
    let toolchain = crate::depcache::dep_toolchain_hash(None);
    let mut groups: BTreeMap<(String, String), Vec<&LoggedEdge>> = BTreeMap::new();
    for e in log {
        let Some((pkg, kind)) = e.meta.clone() else {
            continue;
        };
        groups.entry((pkg, kind)).or_default().push(e);
    }
    for ((pkg, kind), edges) in &groups {
        build_dep_product(
            entry_root,
            &entry_cfg,
            &entry_raw,
            &scope_raw,
            &lock_raw,
            &toolchain,
            pkg,
            kind,
            edges,
            log,
        );
    }
}

fn build_dep_product(
    entry_root: &Path,
    entry_cfg: &ProjectConfig,
    entry_raw: &[u8],
    scope_raw: &[u8],
    lock_raw: &[u8],
    toolchain: &str,
    pkg: &str,
    kind: &str,
    edges: &[&LoggedEdge],
    log: &[LoggedEdge],
) {
    use crate::depcache::*;
    let mut dep_root: Option<PathBuf> = None;
    for e in edges {
        match (&dep_root, &e.dep_root) {
            (None, r) => dep_root = r.clone(),
            (Some(a), Some(b)) if a == b => {}
            _ => return,
        }
    }
    if kind != KIND_STD && dep_root.is_none() {
        return;
    }
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    for e in edges {
        files.insert(e.target.clone());
    }
    loop {
        let mut grown = false;
        for e in log {
            if e.meta.is_some() {
                continue;
            }
            if files.contains(&e.from) && files.insert(e.target.clone()) {
                grown = true;
            }
        }
        if !grown {
            break;
        }
    }
    let mut real_paths: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for target in &files {
        if is_virtual(target) {
            let Some(real) = std_overlay_lookup(target) else {
                return;
            };
            real_paths.insert(target.clone(), real);
        }
    }
    let package_root = match dep_root.clone() {
        Some(r) => r,
        None => {
            let Some(first_virtual) = files.iter().find(|p| is_virtual(p)) else {
                return;
            };
            let first_real = &real_paths[first_virtual];
            let Some(root) = find_manifest_root(first_real) else {
                return;
            };
            for target in &files {
                if is_virtual(target) && !real_paths[target].starts_with(&root) {
                    return;
                }
            }
            root
        }
    };
    if kind != KIND_STD && package_root == *entry_root {
        return;
    }
    let mut blobs: Vec<(String, Vec<u8>)> = Vec::with_capacity(files.len());
    for target in &files {
        let disk = real_paths.get(target).unwrap_or(target);
        let bytes = match std::fs::read(disk) {
            Ok(b) => b,
            Err(_) => return,
        };
        if std::str::from_utf8(&bytes).is_err() {
            return;
        }
        blobs.push((target.to_string_lossy().replace('\\', "/"), bytes));
    }
    let dep_cfg_raw = match std::fs::read(package_root.join(crate::project::MANIFEST_FILE)) {
        Ok(b) => b,
        Err(_) => return,
    };
    let dep_cfg = match ProjectConfig::load_from_dir(&package_root) {
        Ok(Some(c)) => c,
        _ => return,
    };
    let Some(version_key) = dep_version_key(kind, pkg, entry_root, entry_cfg, Some(&dep_cfg))
    else {
        return;
    };
    let Some(spec) = dep_expected_spec(kind, pkg, entry_cfg) else {
        return;
    };
    let base = dep_expected_base(kind, pkg, entry_root, entry_cfg);
    let commit = if kind == KIND_GIT {
        let anchor = crate::fetch::cache_anchor(entry_root);
        let vendor = anchor.join("vendor").join(pkg);
        if vendor.join(crate::project::MANIFEST_FILE).is_file() {
            String::new()
        } else {
            match dep_git_commit(&package_root) {
                Some(c) => c,
                None => return,
            }
        }
    } else {
        String::new()
    };
    let mut stored_edges: BTreeSet<(String, String, String, Option<String>)> = BTreeSet::new();
    for e in edges {
        stored_edges.insert((
            e.from.to_string_lossy().replace('\\', "/"),
            e.source.clone(),
            e.target.to_string_lossy().replace('\\', "/"),
            e.dep_root.as_ref().map(|p| p.to_string_lossy().replace('\\', "/")),
        ));
    }
    for e in log {
        if e.meta.is_some() {
            continue;
        }
        if files.contains(&e.from) {
            stored_edges.insert((
                e.from.to_string_lossy().replace('\\', "/"),
                e.source.clone(),
                e.target.to_string_lossy().replace('\\', "/"),
                e.dep_root.as_ref().map(|p| p.to_string_lossy().replace('\\', "/")),
            ));
        }
    }
    let dep_set = if kind == KIND_STD {
        dep_set_hex(b"", b"", b"", &dep_cfg_raw, &dep_sub_specs(&dep_cfg))
    } else {
        dep_set_hex(entry_raw, scope_raw, lock_raw, &dep_cfg_raw, &dep_sub_specs(&dep_cfg))
    };
    let product = DepProduct {
        pkg: pkg.to_string(),
        kind: kind.to_string(),
        spec,
        version_key: version_key.clone(),
        toolchain: toolchain.to_string(),
        dep_set,
        content: content_digest_hex(&blobs),
        root: package_root.to_string_lossy().replace('\\', "/"),
        base,
        commit,
        files: blobs
            .iter()
            .map(|(p, b)| DepFile { path: p.clone(), bytes: b.clone() })
            .collect(),
        edges: stored_edges
            .iter()
            .map(|(from, source, target, dep_root)| DepEdge {
                from: from.clone(),
                source: source.clone(),
                target: target.clone(),
                dep_root: dep_root.clone(),
                meta: Some((pkg.to_string(), kind.to_string())),
            })
            .collect(),
    };
    let path = product_path(entry_root, pkg, &version_key, toolchain);
    if path.parent().is_some_and(|p| std::fs::create_dir_all(p).is_err()) {
        return;
    }
    if std::fs::write(&path, encode_product(&product)).is_ok() {
        note_populated();
        crate::products::mirror_product(&product);
    }
}

fn find_manifest_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start.parent()?.to_path_buf();
    for _ in 0..8 {
        if dir.join(crate::project::MANIFEST_FILE).is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
    None
}

pub struct LoggedEdge {
    pub from: PathBuf,
    pub source: String,
    pub target: PathBuf,
    pub dep_root: Option<PathBuf>,
    pub meta: Option<(String, String)>,
}

enum LoadResult {
    Ready(A::Module),
    ParseErrs(Vec<Diagnostic>),
    Fatal(Diagnostic),
}

trait VisitBackend {
    fn load(&self, path: &Path) -> LoadResult;
    fn resolve(
        &self,
        path: &Path,
        source: &str,
    ) -> Result<(PathBuf, Option<PathBuf>, Option<(String, String)>), Diagnostic>;
}

struct LiveBackend;

impl VisitBackend for LiveBackend {
    fn load(&self, path: &Path) -> LoadResult {
        match read_source(path) {
            Err(e) => LoadResult::Fatal(e),
            Ok(src) => match crate::parser::Parser::parse_module_all(&src) {
                Ok(module) => LoadResult::Ready(module),
                Err(errs) => LoadResult::ParseErrs(tag_file(errs, path)),
            },
        }
    }

    fn resolve(
        &self,
        path: &Path,
        source: &str,
    ) -> Result<(PathBuf, Option<PathBuf>, Option<(String, String)>), Diagnostic> {
        resolve_import_meta(path, source)
    }
}

fn tag_file(errs: Vec<Diagnostic>, path: &Path) -> Vec<Diagnostic> {
    errs.into_iter()
        .map(|e| match e.file {
            Some(_) => e,
            None => e.with_file(path.to_path_buf()),
        })
        .collect()
}

struct CachedFile {
    load: LoadResult,
}

struct ReplayBackend<'a> {
    files: &'a BTreeMap<PathBuf, CachedFile>,
}

impl VisitBackend for ReplayBackend<'_> {
    fn load(&self, path: &Path) -> LoadResult {
        match self.files.get(path) {
            Some(f) => match &f.load {
                LoadResult::Ready(m) => LoadResult::Ready(m.clone()),
                LoadResult::ParseErrs(e) => LoadResult::ParseErrs(e.clone()),
                LoadResult::Fatal(e) => LoadResult::Fatal(e.clone()),
            },
            None => LiveBackend.load(path),
        }
    }

    fn resolve(
        &self,
        path: &Path,
        source: &str,
    ) -> Result<(PathBuf, Option<PathBuf>, Option<(String, String)>), Diagnostic> {
        LiveBackend.resolve(path, source)
    }
}

fn snapshot_overlay() -> BTreeMap<PathBuf, PathBuf> {
    STD_OVERLAY.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn restore_overlay(snap: &BTreeMap<PathBuf, PathBuf>) {
    *STD_OVERLAY.lock().unwrap_or_else(|e| e.into_inner()) = snap.clone();
}

fn overlay_value(path: &Path) -> Option<PathBuf> {
    STD_OVERLAY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .cloned()
}

struct Discover<'a> {
    pool: Option<&'a rayon::ThreadPool>,
    files: BTreeMap<PathBuf, CachedFile>,
    visited: BTreeSet<PathBuf>,
    overlay_base: BTreeMap<PathBuf, PathBuf>,
    observed: BTreeMap<PathBuf, (Option<PathBuf>, BTreeSet<Option<PathBuf>>)>,
    clash: bool,
}

impl<'a> Discover<'a> {
    fn new(pool: Option<&'a rayon::ThreadPool>, overlay_base: BTreeMap<PathBuf, PathBuf>) -> Self {
        Discover {
            pool,
            files: BTreeMap::new(),
            visited: BTreeSet::new(),
            overlay_base,
            observed: BTreeMap::new(),
            clash: false,
        }
    }

    fn observe_std(&mut self, source: &str) {
        let key = PathBuf::from(format!("{}{}", STD_SCOPE, std_rest(source)));
        let current = overlay_value(&key);
        let entry = self
            .observed
            .entry(key.clone())
            .or_insert_with(|| (self.overlay_base.get(&key).cloned(), BTreeSet::new()));
        entry.1.insert(current);
        let (base, seen) = entry;
        if seen.len() > 1
            || (seen.len() == 1 && seen.iter().next().and_then(|o| o.as_ref()) != base.as_ref())
        {
            self.clash = true;
        }
    }

    fn run(&mut self, seeds: Vec<PathBuf>) {
        let mut frontier = seeds;
        loop {
            if self.clash {
                return;
            }
            frontier.sort();
            frontier.dedup();
            let batch: Vec<PathBuf> = frontier
                .drain(..)
                .filter(|p| !self.visited.contains(p))
                .collect();
            for p in &batch {
                self.visited.insert(p.clone());
            }
            if batch.is_empty() {
                break;
            }
            let loaded: Vec<(PathBuf, LoadResult)> = match self.pool {
                Some(pool) => pool.install(|| {
                    use rayon::prelude::*;
                    batch.par_iter().map(|p| (p.clone(), LiveBackend.load(p))).collect()
                }),
                None => batch.into_iter().map(|p| (p.clone(), LiveBackend.load(&p))).collect(),
            };
            let mut next: Vec<PathBuf> = Vec::new();
            for (path, load) in loaded {
                if self.clash {
                    return;
                }
                let sources: Vec<String> = match &load {
                    LoadResult::Ready(m) => {
                        module_sources(&m.decls).into_iter().map(|(s, _)| s.to_string()).collect()
                    }
                    _ => Vec::new(),
                };
                for source in &sources {
                    let target = match resolve_import_meta(&path, source) {
                        Ok((target, _, _)) => Some(target),
                        Err(_) => None,
                    };
                    if is_std_spec(source) {
                        self.observe_std(source);
                        if self.clash {
                            return;
                        }
                    }
                    if let Some(target) = target {
                        next.push(target);
                    }
                }
                self.files.insert(path, CachedFile { load });
            }
            frontier = next;
        }
    }
}

fn assemble_graph<B: VisitBackend>(
    root: &Path,
    base: &Path,
    entry_root: Option<PathBuf>,
    extra_canon: &[(PathBuf, Result<PathBuf, Diagnostic>)],
    want_prelude: bool,
    backend: &B,
    dep_gen: Option<u64>,
) -> Result<ModuleGraph, Vec<Diagnostic>> {
    let mut order: Vec<ModuleFile> = Vec::new();
    let mut state: BTreeMap<PathBuf, u8> = BTreeMap::new();
    let mut stack: Vec<PathBuf> = Vec::new();
    if let Some(r) = entry_root.clone() {
        stack.push(r);
    }
    let mut parse_errors: Vec<Diagnostic> = Vec::new();
    let mut edge_log: Vec<LoggedEdge> = Vec::new();
    visit_impl(
        root,
        base,
        root,
        entry_root.as_ref(),
        &mut state,
        &mut order,
        &mut stack,
        &mut parse_errors,
        &mut edge_log,
        backend,
    )
    .map_err(|e| vec![e])?;
    if want_prelude {
        visit_impl(
            &PathBuf::from("@std/prelude"),
            base,
            root,
            entry_root.as_ref(),
            &mut state,
            &mut order,
            &mut stack,
            &mut parse_errors,
            &mut edge_log,
            backend,
        )
        .map_err(|e| vec![e])?;
    }
    let mut extras: Vec<PathBuf> = Vec::new();
    for (orig, result) in extra_canon {
        match result {
            Ok(c) => extras.push(c.clone()),
            Err(_) => {
                parse_errors.push(Diagnostic::new(
                    Code::E108,
                    format!("cannot read `{}`", orig.display()),
                ));
            }
        }
    }
    extras.sort();
    extras.dedup();
    for e in extras {
        visit_impl(
            &e,
            base,
            root,
            entry_root.as_ref(),
            &mut state,
            &mut order,
            &mut stack,
            &mut parse_errors,
            &mut edge_log,
            backend,
        )
        .map_err(|e| vec![e])?;
    }
    if parse_errors.is_empty() {
        if dep_gen.is_some() {
            dep_populate_for_entry(entry_root.as_ref(), &edge_log);
        }
        Ok(ModuleGraph { root: root.to_path_buf(), files: order })
    } else {
        Err(parse_errors)
    }
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
    edge_log: &mut Vec<LoggedEdge>,
) -> Result<(), Diagnostic> {
    visit_impl(
        path, base, root, entry_root, state, order, stack, parse_errors, edge_log, &LiveBackend,
    )
}

fn visit_impl<B: VisitBackend>(
    path: &Path,
    base: &Path,
    root: &Path,
    entry_root: Option<&PathBuf>,
    state: &mut BTreeMap<PathBuf, u8>,
    order: &mut Vec<ModuleFile>,
    stack: &mut Vec<PathBuf>,
    parse_errors: &mut Vec<Diagnostic>,
    edge_log: &mut Vec<LoggedEdge>,
    backend: &B,
) -> Result<(), Diagnostic> {
    match state.get(path) {
        Some(_) => return Ok(()),
        None => {}
    }
    state.insert(path.to_path_buf(), 1);
    let module = match backend.load(path) {
        LoadResult::Ready(module) => module,
        LoadResult::ParseErrs(errs) => {
            parse_errors.extend(errs);
            return Ok(());
        }
        LoadResult::Fatal(e) => return Err(e),
    };
    let mut deps: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    for (source, source_span) in module_sources(&module.decls) {
        {
            let (target, dep_root, meta) = backend.resolve(path, source)
                .map_err(|mut e: Diagnostic| {
                    e.span = Some(source_span);
                    if e.file.is_none() {
                        e.file = Some(path.to_path_buf());
                    }
                    e
                })?;
            let child = (target, dep_root);
            edge_log.push(LoggedEdge {
                from: path.to_path_buf(),
                source: source.to_string(),
                target: child.0.clone(),
                dep_root: child.1.clone(),
                meta: meta.clone(),
            });
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
            let r = visit_impl(
                &child, base, root, entry_root, state, order, stack, parse_errors, edge_log,
                backend,
            );
            stack.pop();
            r?;
        } else {
            visit_impl(
                &child, base, root, entry_root, state, order, stack, parse_errors, edge_log,
                backend,
            )?;
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
        plant_prelude_cache(&server.base);
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
        plant_prelude_cache(&server.base);
        let root = registry_app("pinned", &server.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let mut lock = crate::deplock::ProjectDepLock::resolve(&root, &cfg).unwrap();
        lock.write(&root).unwrap();
        let entry = root.join("src").join("main.rnx");
        let graph = ModuleGraph::build(&entry).unwrap();
        assert!(!graph.resolve().unwrap().decls.is_empty());
        let bodies = server.resolve_bodies();
        assert_eq!(bodies.len(), 1);
        assert_eq!(testkit::requirement(&bodies[0], "@acme/widget"), "^1.0.0");
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

    fn dead_from() -> PathBuf {
        std::env::temp_dir().join(format!("rnx-nostd-{}", std::process::id()))
    }

    fn plant_prelude_cache(base: &str) {
        let pin = env!("CARGO_PKG_VERSION");
        let dir = crate::fetch::cached_package_dir(base, "@std/prelude", pin);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join(crate::project::MANIFEST_FILE),
            format!(
                "export default {{\n    project: {{\n        name: \"@std/prelude\",\n        version: \"{pin}\"\n    }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("src").join("main.rnx"),
            "export fn hello(): Int { return 1; }\n",
        )
        .unwrap();
        std::fs::write(dir.join(".rnx-integrity"), format!("@std/prelude\n{pin}\ntest\n"))
            .unwrap();
    }

    #[test]
    fn std_resolves_through_registry_under_the_hood() {
        let (_guard, _cache) = testkit::isolate_cache("stdreg");
        let server = start_std_server();
        let (target, dep, _) = resolve_std_with_registries(
            &dead_from(),
            "@std/fs",
            "fs",
            None,
            &std_overrides(&server.base),
        )
        .unwrap();
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
        let first = resolve_std_with_registries(
            &dead_from(),
            "@std/fs",
            "fs",
            None,
            &std_overrides(&server.base),
        )
        .unwrap();
        let served = server.requests().len();
        assert!(served > 0);
        let second = resolve_std_with_registries(
            &dead_from(),
            "@std/fs",
            "fs",
            None,
            &std_overrides(&server.base),
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(server.requests().len(), served);
    }

    #[test]
    fn std_missing_cache_fails_loudly_offline() {
        let (_guard, _cache) = testkit::isolate_cache("stdfb");
        let err = resolve_std_with_registries(&dead_from(), "@std/fs", "fs", None, &dead_overrides())
            .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("127.0.0.1:1"), "{}", err.message);
        assert!(
            err.hint.as_deref().unwrap_or_default().contains("rnx fetch-std"),
            "{:?}",
            err.hint
        );
        assert_eq!(std_overlay_get(&PathBuf::from("@std/fs")), None);
    }

    #[test]
    fn std_unknown_module_errors() {
        let (_guard, _cache) = testkit::isolate_cache("stdbad");
        let err =
            resolve_std_with_registries(&dead_from(), "@std/nope", "nope", None, &dead_overrides())
                .unwrap_err();
        assert!(err.message.contains("unknown standard library module"));
        assert_eq!(std_overlay_get(&PathBuf::from("@std/nope")), None);
    }

    #[test]
    fn std_overlay_serves_registry_bytes() {
        let (_guard, _cache) = testkit::isolate_cache("stdread");
        let server = start_std_server();
        resolve_std_with_registries(
            &dead_from(),
            "@std/fs",
            "fs",
            None,
            &std_overrides(&server.base),
        )
        .unwrap();
        let src = read_source(&PathBuf::from("@std/fs")).unwrap();
        assert!(src.contains("export fn hello"));
    }

    #[test]
    fn from_source_offline_no_cache_fails_loudly() {
        let (_guard, _cache) = testkit::isolate_cache("srcfb");
        let err = ModuleGraph::from_source("fn Main(): Int { return 1; }\n").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("registry"), "{}", err.message);
        assert!(
            err.hint.as_deref().unwrap_or_default().contains("rnx fetch-std"),
            "{:?}",
            err.hint
        );
    }

    static DEP_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvUnsetOnDrop;
    impl EnvUnsetOnDrop {
        fn set(key: &str, val: &str) -> Self {
            unsafe {
                std::env::set_var(key, val);
            }
            EnvUnsetOnDrop
        }
    }
    impl Drop for EnvUnsetOnDrop {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("RNX_DEP_CACHE");
            }
        }
    }

    fn depcache_fixture(tag: &str, dep_version: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("rnx-depcache-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app = base.join("app");
        let dep = base.join("libs").join("utils");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir_all(dep.join("src")).unwrap();
        std::fs::write(
            dep.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"utils\",\n        version: \"{dep_version}\"\n    }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            dep.join("src").join("main.rnx"),
            "import { bonus } from \"./extra\";\nexport fn helper(): Int { return bonus(); }\n",
        )
        .unwrap();
        std::fs::write(
            dep.join("src").join("extra.rnx"),
            "export fn bonus(): Int { return 1; }\n",
        )
        .unwrap();
        std::fs::write(
            app.join("Project.config"),
            "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        \"utils\": \"../libs/utils\"\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(
            app.join("src").join("main.rnx"),
            "import { helper } from \"utils\";\nfn Main(): Int { return helper(); }\n",
        )
        .unwrap();
        (app.join("src").join("main.rnx"), app, dep)
    }

    fn plant_default_prelude() {
        plant_prelude_cache(&crate::fetch::expand_registry_base(""));
    }

    fn merged_debug(entry: &Path) -> String {
        let graph = ModuleGraph::build(entry).unwrap();
        format!("{:?}", graph.resolve().unwrap())
    }

    fn dep_product_file(app: &Path, version: &str) -> PathBuf {
        crate::depcache::product_path(
            app,
            "utils",
            version,
            &crate::depcache::dep_toolchain_hash(None),
        )
    }

    #[test]
    fn depcache_second_build_hits_with_identical_merge() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("dephit");
        crate::depcache::reset_stats();
        plant_default_prelude();
        let (entry, app, _dep) = depcache_fixture("hit", "1.0.0");
        let first = merged_debug(&entry);
        assert!(first.contains("Int(1)"), "{first}");
        let (_, _, populated) = crate::depcache::stats();
        assert!(populated > 0, "miss path must populate the dep product");
        assert!(dep_product_file(&app, "1.0.0").is_file());
        let (_, memo_before, _) = crate::depcache::stats();
        let (hits_before, _, _) = crate::depcache::stats();
        let second = merged_debug(&entry);
        assert_eq!(first, second);
        let (hits_after, memo_after, _) = crate::depcache::stats();
        assert!(hits_after > hits_before, "second build must restore a dep product");
        assert!(memo_after > memo_before, "second build must skip resolution via memo");
        let _no_cache = EnvUnsetOnDrop::set("RNX_DEP_CACHE", "0");
        let third = merged_debug(&entry);
        drop(_no_cache);
        assert_eq!(first, third);
        let _ = std::fs::remove_dir_all(app.parent().unwrap());
    }

    #[test]
    fn depcache_dep_edit_invalidates() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("depedit");
        crate::depcache::reset_stats();
        plant_default_prelude();
        let (entry, app, dep) = depcache_fixture("edit", "1.0.0");
        let first = merged_debug(&entry);
        assert!(first.contains("Int(1)"), "{first}");
        assert_eq!(merged_debug(&entry), first);
        std::fs::write(dep.join("src").join("extra.rnx"), "export fn bonus(): Int { return 2; }\n")
            .unwrap();
        let (hits_before, _, _) = crate::depcache::stats();
        let third = merged_debug(&entry);
        assert!(third.contains("Int(2)"), "{third}");
        assert_ne!(first, third);
        let (hits_after, _, _) = crate::depcache::stats();
        assert_eq!(hits_before, hits_after, "edited dep must miss the cache");
        let fourth = merged_debug(&entry);
        assert_eq!(third, fourth);
        let (hits_final, _, _) = crate::depcache::stats();
        assert!(hits_final > hits_after, "repopulation must hit again");
        let _ = std::fs::remove_dir_all(app.parent().unwrap());
    }

    #[test]
    fn depcache_version_bump_and_toolchain_change_invalidate() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("depver");
        crate::depcache::reset_stats();
        plant_default_prelude();
        let (entry, app, dep) = depcache_fixture("ver", "1.0.0");
        let first = merged_debug(&entry);
        assert!(dep_product_file(&app, "1.0.0").is_file());
        let prof = crate::depcache::DepProfile {
            release: true,
            opt_level: 2,
            target: None,
            host: "x86_64-unknown-linux-gnu".to_string(),
            debug: false,
            runtime_hash: "rt".to_string(),
            llvm_version: "llvm22".to_string(),
        };
        let other_toolchain = crate::depcache::dep_toolchain_hash(Some(&prof));
        assert_ne!(other_toolchain, crate::depcache::dep_toolchain_hash(None));
        let other_path =
            crate::depcache::product_path(&app, "utils", "1.0.0", &other_toolchain);
        assert_ne!(other_path, dep_product_file(&app, "1.0.0"));
        std::fs::write(
            dep.join("Project.config"),
            "export default {\n    project: {\n        name: \"utils\",\n        version: \"2.0.0\"\n    }\n}\n",
        )
        .unwrap();
        let (hits_before, _, _) = crate::depcache::stats();
        let second = merged_debug(&entry);
        assert_eq!(first, second, "version bump must not change merged output");
        let (hits_after, _, _) = crate::depcache::stats();
        assert_eq!(hits_before, hits_after, "version bump must miss the cache");
        assert!(dep_product_file(&app, "2.0.0").is_file());
        let third = merged_debug(&entry);
        assert_eq!(second, third);
        let (hits_final, _, _) = crate::depcache::stats();
        assert!(hits_final > hits_after, "bumped version must hit after repopulation");
        let _ = std::fs::remove_dir_all(app.parent().unwrap());
    }

    #[test]
    fn depcache_corrupt_product_falls_back() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("depcorrupt");
        crate::depcache::reset_stats();
        plant_default_prelude();
        let (entry, app, _dep) = depcache_fixture("corrupt", "1.0.0");
        let first = merged_debug(&entry);
        let path = dep_product_file(&app, "1.0.0");
        assert!(path.is_file());
        std::fs::write(&path, b"not a dep product").unwrap();
        let second = merged_debug(&entry);
        assert_eq!(first, second, "corrupt cache must fall back to a full build");
        let valid = std::fs::read(&path).unwrap();
        std::fs::write(&path, &valid[..valid.len() / 2]).unwrap();
        let third = merged_debug(&entry);
        assert_eq!(first, third, "truncated cache must fall back to a full build");
        let _ = std::fs::remove_dir_all(app.parent().unwrap());
    }

    #[test]
    fn depcache_registry_dep_hits_without_network() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("depreg");
        crate::depcache::reset_stats();
        let server = start_widget_server();
        plant_prelude_cache(&server.base);
        let root = registry_app("dep", &server.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock = crate::deplock::ProjectDepLock::resolve(&root, &cfg).unwrap();
        lock.write(&root).unwrap();
        let entry = root.join("src").join("main.rnx");
        let first = merged_debug(&entry);
        assert!(!first.is_empty());
        let served = server.requests().len();
        assert!(served > 0);
        let product = crate::depcache::product_path(
            &root,
            "@acme/widget",
            "1.2.0",
            &crate::depcache::dep_toolchain_hash(None),
        );
        assert!(product.is_file(), "{}", product.display());
        let (hits_before, _, _) = crate::depcache::stats();
        let second = merged_debug(&entry);
        assert_eq!(first, second);
        assert_eq!(
            server.requests().len(),
            served,
            "hit path must not touch the registry"
        );
        let (hits_after, _, _) = crate::depcache::stats();
        assert!(hits_after > hits_before, "registry dep must restore from cache");
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn global_products_survive_project_cache_wipe() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("globwipe");
        crate::depcache::reset_stats();
        crate::products::reset_stats();
        let server = start_widget_server();
        plant_prelude_cache(&server.base);
        let root = registry_app("glob", &server.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock = crate::deplock::ProjectDepLock::resolve(&root, &cfg).unwrap();
        lock.write(&root).unwrap();
        let entry = root.join("src").join("main.rnx");
        let first = merged_debug(&entry);
        assert!(!first.is_empty());
        let served = server.requests().len();
        assert!(served > 0);
        let (files, _) = crate::products::products_usage();
        assert!(files > 0, "build must mirror dep products into the global store");
        std::fs::remove_dir_all(crate::cache::project_cache_dir(&root)).unwrap();
        assert!(!crate::cache::project_cache_dir(&root).exists());
        let (hits_before, _, _) = crate::depcache::stats();
        let second = merged_debug(&entry);
        assert_eq!(first, second);
        let (hits_after, _, _) = crate::depcache::stats();
        assert!(hits_after > hits_before, "wiped project cache must restore from global products");
        assert_eq!(
            server.requests().len(),
            served,
            "global hit path must not touch the registry"
        );
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn global_products_corrupt_falls_back_and_heals() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("globcorrupt");
        crate::depcache::reset_stats();
        crate::products::reset_stats();
        let server = start_widget_server();
        plant_prelude_cache(&server.base);
        let root = registry_app("globbad", &server.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock = crate::deplock::ProjectDepLock::resolve(&root, &cfg).unwrap();
        lock.write(&root).unwrap();
        let entry = root.join("src").join("main.rnx");
        let first = merged_debug(&entry);
        assert!(!first.is_empty());
        for path in crate::products::product_files() {
            std::fs::write(&path, b"corrupt").unwrap();
        }
        std::fs::remove_dir_all(crate::cache::project_cache_dir(&root)).unwrap();
        let second = merged_debug(&entry);
        assert_eq!(first, second, "corrupt global products must fall back to a full build");
        for path in crate::products::product_files() {
            assert!(
                crate::products::read_product_file(&path).is_some(),
                "repopulation must heal corrupt entries"
            );
        }
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn global_products_entry_change_invalidates() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("globentry");
        crate::depcache::reset_stats();
        crate::products::reset_stats();
        let server = start_widget_server();
        plant_prelude_cache(&server.base);
        let root = registry_app("globinv", &server.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock = crate::deplock::ProjectDepLock::resolve(&root, &cfg).unwrap();
        lock.write(&root).unwrap();
        let entry = root.join("src").join("main.rnx");
        let first = merged_debug(&entry);
        assert!(!first.is_empty());
        let manifest = root.join("Project.config");
        let mut text = std::fs::read_to_string(&manifest).unwrap();
        text.push('\n');
        std::fs::write(&manifest, &text).unwrap();
        std::fs::remove_dir_all(crate::cache::project_cache_dir(&root)).unwrap();
        let (hits_before, _, _) = crate::depcache::stats();
        let second = merged_debug(&entry);
        assert_eq!(first, second);
        let (hits_after, _, _) = crate::depcache::stats();
        assert_eq!(hits_before, hits_after, "changed entry manifest must miss global products");
        let third = merged_debug(&entry);
        assert_eq!(second, third);
        let (hits_final, _, _) = crate::depcache::stats();
        assert!(hits_final > hits_after, "repopulation must hit again");
        let widget_files: Vec<_> = crate::products::product_files()
            .into_iter()
            .filter(|path| path.to_string_lossy().contains("@acme/widget"))
            .collect();
        assert_eq!(widget_files.len(), 2, "distinct dep sets shard into distinct files");
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn floating_range_never_populates_global() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("globfloat");
        crate::depcache::reset_stats();
        crate::products::reset_stats();
        let server = start_widget_server();
        plant_prelude_cache(&server.base);
        let root = registry_app("globfl", &server.base);
        let entry = root.join("src").join("main.rnx");
        let first = merged_debug(&entry);
        assert!(!first.is_empty());
        let statuses = product_statuses(&root);
        let widget = statuses.iter().find(|status| status.pkg == "@acme/widget").expect("widget");
        assert_eq!(widget.version, None);
        assert!(!widget.present);
        let widget_files: Vec<_> = crate::products::product_files()
            .into_iter()
            .filter(|path| path.to_string_lossy().contains("@acme/widget"))
            .collect();
        assert!(widget_files.is_empty(), "floating ranges must never populate");
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn fetch_precompile_populates_global_without_build() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("globpre");
        crate::depcache::reset_stats();
        crate::products::reset_stats();
        let server = start_widget_server();
        plant_prelude_cache(&server.base);
        let root = registry_app("globpre", &server.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock = crate::deplock::ProjectDepLock::resolve(&root, &cfg).unwrap();
        lock.write(&root).unwrap();
        let report = crate::products::precompile_scope(&root);
        assert_eq!(report.projects, 1);
        assert_eq!(report.entries, 1);
        assert!(report.populated > 0, "precompile must populate global products");
        let served = server.requests().len();
        assert!(served > 0);
        crate::depcache::reset_stats();
        let entry = root.join("src").join("main.rnx");
        let merged = merged_debug(&entry);
        assert!(!merged.is_empty());
        let (hits, _, _) = crate::depcache::stats();
        assert!(hits > 0, "first build after precompile must restore from global products");
        assert_eq!(
            server.requests().len(),
            served,
            "precompiled build must not touch the registry"
        );
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    struct EnvVarGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let prev = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, val);
            }
            EnvVarGuard { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    fn std_install_tarball(full: &str, version: &str, main_rnx: &str) -> (Vec<u8>, String) {
        let manifest = format!(
            "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"{version}\"\n    }}\n}}\n"
        );
        let mut buf = Vec::new();
        {
            let mut tar = crate::tar::TarWriter::new(&mut buf);
            tar.add_file("Project.config", manifest.as_bytes()).unwrap();
            tar.add_file("src/main.rnx", main_rnx.as_bytes()).unwrap();
            tar.finish().unwrap();
        }
        let gz = crate::gzip::compress_gzip(&buf);
        let sha = crate::checksum::Sha256::hexdigest(&gz);
        (gz, sha)
    }

    struct StdInstallServer {
        base: String,
        log: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    }

    impl StdInstallServer {
        fn requests(&self) -> Vec<(String, String)> {
            self.log.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    fn start_std_install_server() -> StdInstallServer {
        use std::io::{Read, Write};
        let pin = env!("CARGO_PKG_VERSION");
        let mut balls: BTreeMap<(String, String), (Vec<u8>, String)> = BTreeMap::new();
        for full in crate::stdlib_seed::std_package_names() {
            let top = full.strip_prefix("@std/").unwrap_or(&full);
            let main = if top == "time" {
                "export fn tick(): Int { return 1; }\nexport fn hello(): Int { return 1; }\n"
            } else {
                "export fn hello(): Int { return 1; }\n"
            };
            let (gz, sha) = std_install_tarball(&full, pin, main);
            balls.insert((full, pin.to_string()), (gz, sha));
        }
        let nodes: Vec<String> = balls
            .iter()
            .map(|((full, version), (_, sha))| testkit::node_json(full, version, sha, false))
            .collect();
        let resolve_body = testkit::resolve_json_static(&nodes);
        let balls = std::sync::Arc::new(balls);
        let log: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                loop {
                    match stream.read(&mut byte) {
                        Ok(0) => break,
                        Ok(_) => head.push(byte[0]),
                        Err(_) => break,
                    }
                    if head.ends_with(b"\r\n\r\n") || head.len() > 65536 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&head).into_owned();
                let mut lines = text.lines();
                let request = lines.next().unwrap_or_default().to_string();
                let mut parts = request.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                seen.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((method.clone(), path.clone()));
                let route = path.split('?').next().unwrap_or_default().to_string();
                let mut respond = |status: u16, body: &[u8], json: bool| {
                    let text = format!(
                        "HTTP/1.1 {status} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        if status == 200 { "OK" } else { "Error" },
                        if json { "application/json" } else { "application/octet-stream" },
                        body.len()
                    );
                    let _ = stream.write_all(text.as_bytes());
                    let _ = stream.write_all(body);
                    let _ = stream.flush();
                };
                if method == "GET" && route == "/api/version" {
                    respond(200, b"{\"spec\": 1}", true);
                } else if method == "POST" && route == "/api/resolve" {
                    respond(200, resolve_body.as_bytes(), true);
                } else if method == "GET" && route.ends_with("/chunks") {
                    let hit = route.find("@std/").and_then(|i| {
                        route[i..].strip_suffix("/chunks").and_then(|key| {
                            key.rsplit_once('@').and_then(|(full, version)| {
                                balls.get(&(full.to_string(), version.to_string())).cloned()
                            })
                        })
                    });
                    match hit {
                        Some((gz, _)) => respond(200, testkit::fallback_manifest(&gz).as_bytes(), true),
                        None => respond(404, b"{}", true),
                    }
                } else if method == "GET" && route.contains("/chunk/") {
                    let hash = route.rsplit('/').next().unwrap_or_default();
                    let hit = balls.values().find(|(_, sha)| sha == hash).map(|(gz, _)| gz.clone());
                    match hit {
                        Some(gz) => respond(200, &gz, false),
                        None => respond(404, b"{}", true),
                    }
                } else {
                    respond(404, b"{}", true);
                }
            }
        });
        StdInstallServer { base, log }
    }

    fn std_install_app(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-stdapp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("app");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let pin = env!("CARGO_PKG_VERSION");
        std::fs::write(
            root.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n        \"@std/time\": {{ version: \"{pin}\" }}\n    }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("src").join("main.rnx"),
            "import { tick } from \"@std/time\";\nfn Main(): Int { return tick(); }\n",
        )
        .unwrap();
        root
    }

    fn try_seed_from_env() -> Result<crate::stdlib_seed::StdSeedReport, diagnostics::Diagnostic> {
        let (default, overrides) = crate::stdlib_seed::std_registry_from_env();
        crate::stdlib_seed::seed_stdlib_cache(default.as_ref(), &overrides)
    }

    fn seed_from_env() -> crate::stdlib_seed::StdSeedReport {
        try_seed_from_env().unwrap()
    }

    #[test]
    fn install_cold_build_populates_but_never_hits() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _dir) = testkit::isolate_cache("stdcold");
        crate::depcache::reset_stats();
        crate::products::reset_stats();
        crate::stdvfs::clear();
        let server = start_std_install_server();
        let _reg = EnvVarGuard::set("RNX_REGISTRY", &server.base);
        let seed = match try_seed_from_env() {
            Ok(report) => report,
            Err(e) => {
                eprintln!("cold requests: {:#?}", server.requests());
                panic!("seed failed: {e}");
            }
        };
        assert_eq!(seed.packages.len(), crate::stdlib_seed::std_package_names().len());
        let root = std_install_app("cold");
        let entry = root.join("src").join("main.rnx");
        let first = merged_debug(&entry);
        assert!(first.contains("Int(1)"), "{first}");
        let (hits, _, _) = crate::depcache::stats();
        assert_eq!(hits, 0, "cold build with no precompile must not restore anything");
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn install_flow_warms_std_products_and_later_build_hits() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _dir) = testkit::isolate_cache("stdinstall");
        crate::depcache::reset_stats();
        crate::products::reset_stats();
        crate::stdvfs::clear();
        let server = start_std_install_server();
        let _reg = EnvVarGuard::set("RNX_REGISTRY", &server.base);
        let seed = seed_from_env();
        assert_eq!(seed.packages.len(), crate::stdlib_seed::std_package_names().len());
        assert!(crate::products::product_files().is_empty());
        let start = std::time::Instant::now();
        let pre = crate::products::precompile_std().expect("standalone std precompile");
        let cost = start.elapsed();
        eprintln!(
            "std precompile: populated={} entries={} projects={} cost={}ms",
            pre.populated,
            pre.entries,
            pre.projects,
            cost.as_millis()
        );
        assert_eq!(pre.projects, 1);
        assert_eq!(pre.entries, 1);
        assert!(pre.populated > 0, "precompile must populate global std products");
        assert!(
            !crate::products::std_scaffold_dir().exists(),
            "scaffold project must be removed after precompile"
        );
        let (files, _) = crate::products::products_usage();
        assert!(files > 0, "precompile must leave product files behind");
        for path in crate::products::product_files() {
            assert!(
                crate::products::read_product_file(&path).is_some(),
                "warmed product must decode: {}",
                path.display()
            );
            let dir = path.parent().expect("product dir");
            let lines = crate::products::manifest_lines(dir).expect("product manifest");
            assert_eq!(lines.len(), 9);
            assert!(lines[0].starts_with("@std/"));
            assert!(crate::products::exact_pin(&lines[1], &lines[3]));
        }
        let (sum_files, sum_pkgs) =
            crate::products::std_products_summary().expect("std products summary");
        assert_eq!(sum_files, files);
        assert!(sum_pkgs > 0);
        let served = server.requests().len();
        assert!(served > 0);
        let root = std_install_app("warmed");
        let entry = root.join("src").join("main.rnx");
        crate::depcache::reset_stats();
        let first = merged_debug(&entry);
        assert!(first.contains("Int(1)"), "{first}");
        let (hits, _, _) = crate::depcache::stats();
        assert!(hits > 0, "later build must restore install-warmed std products");
        assert_eq!(
            server.requests().len(),
            served,
            "warmed build must not touch the registry"
        );
        crate::depcache::reset_stats();
        let second = merged_debug(&entry);
        assert_eq!(first, second);
        let (_, memo_hits, _) = crate::depcache::stats();
        assert!(memo_hits > 0, "repeat build must resolve through the restored memo");
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn precompile_std_without_seed_is_fail_open() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _dir) = testkit::isolate_cache("stdoffpre");
        crate::products::reset_stats();
        crate::stdvfs::clear();
        let err = crate::products::precompile_std().unwrap_err();
        assert!(err.contains("pin"), "{err}");
        assert!(crate::products::product_files().is_empty());
        assert!(!crate::products::std_scaffold_dir().exists());
        assert_eq!(crate::products::std_products_summary(), None);
    }

    #[test]
    fn precompile_std_dead_registry_is_fail_open() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _dir) = testkit::isolate_cache("stddeadpre");
        crate::depcache::reset_stats();
        crate::products::reset_stats();
        crate::stdvfs::clear();
        let server = start_std_install_server();
        let _reg = EnvVarGuard::set("RNX_REGISTRY", &server.base);
        let seed = seed_from_env();
        assert!(!seed.packages.is_empty());
        drop(_reg);
        let _dead = EnvVarGuard::set("RNX_REGISTRY", "http://127.0.0.1:9");
        let pre = crate::products::precompile_std().expect("must not fail hard offline");
        assert_eq!(
            pre.populated, 0,
            "unreachable registry must not mint products from thin air"
        );
        assert!(crate::products::product_files().is_empty());
        assert!(!crate::products::std_scaffold_dir().exists());
    }

    #[test]
    fn depcache_hit_vs_miss_timing() {
        let _serial = DEP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _cache) = testkit::isolate_cache("deptime");
        crate::depcache::reset_stats();
        plant_default_prelude();
        let base = std::env::temp_dir().join(format!("rnx-deptime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app = base.join("app");
        let dep = base.join("libs").join("wide");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir_all(dep.join("src")).unwrap();
        std::fs::write(
            dep.join("Project.config"),
            "export default {\n    project: {\n        name: \"wide\",\n        version: \"1.0.0\"\n    }\n}\n",
        )
        .unwrap();
        let mut imports = String::new();
        let mut calls = String::new();
        for i in 0..24 {
            imports.push_str(&format!("import {{ f{i} }} from \"wide/mod{i}\";\n"));
            calls.push_str(&format!("    acc = acc + f{i}();\n"));
            std::fs::write(
                dep.join("src").join(format!("mod{i}.rnx")),
                format!("export fn f{i}(): Int {{ return {i}; }}\n"),
            )
            .unwrap();
        }
        std::fs::write(
            dep.join("src").join("main.rnx"),
            "export fn wide_entry(): Int { return 0; }\n",
        )
        .unwrap();
        std::fs::write(
            app.join("Project.config"),
            "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        \"wide\": \"../libs/wide\"\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(
            app.join("src").join("main.rnx"),
            format!("{imports}fn Main(): Int {{\n    let acc: Int = 0;\n{calls}    return acc;\n}}\n"),
        )
        .unwrap();
        let entry = app.join("src").join("main.rnx");
        let start = std::time::Instant::now();
        let first = merged_debug(&entry);
        let miss = start.elapsed();
        let start = std::time::Instant::now();
        let second = merged_debug(&entry);
        let hit = start.elapsed();
        assert_eq!(first, second);
        let (hits, memo_hits, _) = crate::depcache::stats();
        eprintln!(
            "depcache timing: miss={}ms hit={}ms hits={hits} memo_hits={memo_hits}",
            miss.as_millis(),
            hit.as_millis()
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    fn par_fixture(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let base = std::env::temp_dir().join(format!("rnx-parmod-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for (name, src) in files {
            let p = base.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, src).unwrap();
        }
        base.join("main.rnx")
    }

    fn par_cleanup(tag: &str) {
        let base = std::env::temp_dir().join(format!("rnx-parmod-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
    }

    fn collected(entry: &Path, jobs: Option<usize>) -> Result<String, String> {
        let graph = match jobs {
            Some(j) => ModuleGraph::build_collecting_extra_parallel(entry, &[], j),
            None => ModuleGraph::build_collecting(entry),
        };
        match graph {
            Ok(g) => {
                let mut files = String::new();
                for f in &g.files {
                    files.push_str(&format!("{}|{}|{:?}\n", f.path.display(), f.key, f.kind));
                }
                Ok(format!("{files}{:?}", g.resolve()))
            }
            Err(e) => Err(format!("{e:?}")),
        }
    }

    fn assert_parallel_matches_sequential(tag: &str, files: &[(&str, &str)]) {
        let (_guard, _cache) = testkit::isolate_cache(&format!("parmod-{tag}"));
        plant_default_prelude();
        let entry = par_fixture(tag, files);
        let expect = collected(&entry, None);
        for jobs in [0, 1, 2, 8] {
            assert_eq!(collected(&entry, Some(jobs)), expect, "tag={tag} jobs={jobs}");
        }
        match tag {
            "dia" | "cyc" => assert!(expect.is_ok(), "tag={tag}: {expect:?}"),
            _ => assert!(expect.is_err(), "tag={tag}: {expect:?}"),
        }
        par_cleanup(tag);
    }

    #[test]
    fn parallel_matches_sequential_diamond() {
        assert_parallel_matches_sequential(
            "dia",
            &[
                (
                    "main.rnx",
                    "import { a } from \"./a\";\nimport { b } from \"./b\";\nfn Main(): Int { return a() + b(); }\n",
                ),
                (
                    "a.rnx",
                    "import { c } from \"./c\";\nexport fn a(): Int { return c() + 1; }\n",
                ),
                (
                    "b.rnx",
                    "import { c } from \"./c\";\nexport fn b(): Int { return c() + 2; }\n",
                ),
                ("c.rnx", "export fn c(): Int { return 10; }\n"),
            ],
        );
    }

    #[test]
    fn parallel_matches_sequential_cycle() {
        assert_parallel_matches_sequential(
            "cyc",
            &[
                ("main.rnx", "import { a } from \"./a\";\nfn Main(): Int { return a(); }\n"),
                (
                    "a.rnx",
                    "import { b } from \"./b\";\nexport fn a(): Int { return b() + 1; }\n",
                ),
                (
                    "b.rnx",
                    "import { a } from \"./a\";\nexport fn b(): Int { return a() + 1; }\n",
                ),
            ],
        );
    }

    #[test]
    fn parallel_matches_sequential_errors() {
        assert_parallel_matches_sequential(
            "errmix",
            &[
                (
                    "main.rnx",
                    "import { x } from \"./bad\";\nimport { y } from \"./fatal\";\nfn Main(): Int { return x() + y(); }\n",
                ),
                ("bad.rnx", "export fn x(: Int { return ;;; broken (((\n"),
                (
                    "fatal.rnx",
                    "import { z } from \"./nonexistent\";\nexport fn y(): Int { return z(); }\n",
                ),
            ],
        );
        let (_guard, _cache) = testkit::isolate_cache("parmod-errtwo");
        plant_default_prelude();
        let entry = par_fixture(
            "errtwo",
            &[
                (
                    "main.rnx",
                    "import { x } from \"./fa\";\nimport { y } from \"./fb\";\nfn Main(): Int { return x() + y(); }\n",
                ),
                (
                    "fa.rnx",
                    "import { z } from \"./missing_a\";\nexport fn x(): Int { return z(); }\n",
                ),
                (
                    "fb.rnx",
                    "import { z } from \"./missing_b\";\nexport fn y(): Int { return z(); }\n",
                ),
            ],
        );
        let expect = collected(&entry, None);
        assert!(matches!(&expect, Err(e) if e.contains("missing_a")), "{expect:?}");
        for jobs in [0, 1, 2, 8] {
            assert_eq!(collected(&entry, Some(jobs)), expect, "jobs={jobs}");
        }
        par_cleanup("errtwo");
    }

    #[test]
    fn parallel_parse_timing() {
        let (_guard, _cache) = testkit::isolate_cache("parmod-time");
        plant_default_prelude();
        let tag = "time";
        let base = std::env::temp_dir().join(format!("rnx-parmod-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let n = 48;
        let per = 20;
        let mut imports = String::new();
        let mut calls = String::new();
        for i in 0..n {
            imports.push_str(&format!("import {{ g{i}_0 }} from \"./m{i}\";\n"));
            calls.push_str(&format!("    acc = acc + g{i}_0(acc);\n"));
            let mut body = String::new();
            for k in 0..per {
                body.push_str(&format!(
                    "export fn g{i}_{k}(x: Int): Int {{\n    let y: Int = x + {k};\n    return y * {i} + {k};\n}}\n"
                ));
            }
            std::fs::write(base.join(format!("m{i}.rnx")), body).unwrap();
        }
        std::fs::write(
            base.join("main.rnx"),
            format!("{imports}fn Main(): Int {{\n    let acc: Int = 0;\n{calls}    return acc;\n}}\n"),
        )
        .unwrap();
        let entry = base.join("main.rnx");
        let start = std::time::Instant::now();
        let seq = collected(&entry, None);
        let seq_ms = start.elapsed();
        let start = std::time::Instant::now();
        let par2 = collected(&entry, Some(2));
        let par2_ms = start.elapsed();
        let start = std::time::Instant::now();
        let par = collected(&entry, Some(8));
        let par_ms = start.elapsed();
        assert_eq!(par2, seq);
        assert_eq!(par, seq);
        assert!(matches!(&seq, Ok(_)), "{seq:?}");
        eprintln!(
            "parallel lex_parse: seq={}ms par2={}ms par8={}ms",
            seq_ms.as_millis(),
            par2_ms.as_millis(),
            par_ms.as_millis()
        );
        par_cleanup(tag);
    }
}
