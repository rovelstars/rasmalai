#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use diagnostics::Code;
use frontend::ast::{Decl, ImportSource};
use frontend::modules::ModuleGraph;
use frontend::parser::Parser;

fn parse(src: &str) -> frontend::ast::Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
}

fn check(src: &str) -> Vec<diagnostics::Diagnostic> {
    frontend::semantic::check(&parse(src))
}

fn tree(tag: &str, files: &[(&str, &str)]) -> crate::scratch::Scratch {
    let dir = crate::scratch::dir(&format!("rt-{tag}"));
    for (name, src) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, src).unwrap();
    }
    dir
}

#[test]
fn runtime_import_parses_with_fn_specs() {
    let m = parse("import { fn __rnx_clock_mono(): Int } from runtime \"time\"\n");
    let mut found = false;
    for decl in &m.decls {
        if let Decl::Import(d) = &decl.node {
            match &d.source {
                ImportSource::Runtime(module) => {
                    assert_eq!(module, "time");
                    found = true;
                }
                other => panic!("unexpected source: {other:?}"),
            }
        }
    }
    assert!(found, "runtime import missing");
}

#[test]
fn runtime_import_rejects_reexport() {
    let err = Parser::parse_module("export { fn f(): Int } from runtime \"time\"\n")
        .err()
        .expect("re-export must fail");
    assert_eq!(err.code, Code::E108);
    assert!(err.message.contains("cannot be re-exported"), "{}", err.message);
}

#[test]
fn runtime_import_rejects_export_modifier() {
    let err = Parser::parse_module(
        "import { export fn __rnx_clock_mono(): Int } from runtime \"time\"\n",
    )
    .err()
    .expect("export modifier must fail");
    assert_eq!(err.code, Code::E108);
    assert!(err.message.contains("cannot be re-exported"), "{}", err.message);
}

#[test]
fn runtime_import_rejects_default_and_named() {
    let err = Parser::parse_module(
        "import rt, { fn __rnx_clock_mono(): Int } from runtime \"time\"\n",
    )
    .err()
    .expect("default name must fail");
    assert_eq!(err.code, Code::E108);
}

#[test]
fn runtime_import_rejects_non_fn_specs() {
    let err = Parser::parse_module("import { __rnx_clock_mono } from runtime \"time\"\n")
        .err()
        .expect("plain specs must fail");
    assert_eq!(err.code, Code::E108);
}

#[test]
fn valid_runtime_import_passes_scope() {
    let diags = check("import { fn __rnx_clock_mono(): Int } from runtime \"time\"\nfn Main(): Int {\n    return __rnx_clock_mono();\n}\n");
    let errors: Vec<_> = diags.into_iter().filter(|d| !d.code.is_warning()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn unknown_runtime_module_rejected() {
    let diags = check("import { fn __rnx_clock_mono(): Int } from runtime \"nope\"\n");
    assert!(
        diags.iter().any(|d| d.code == Code::E108 && d.message.contains("unknown runtime module")),
        "{diags:?}"
    );
}

#[test]
fn unknown_runtime_symbol_rejected() {
    let diags = check("import { fn __rnx_nope(): Int } from runtime \"time\"\n");
    assert!(
        diags.iter().any(|d| d.code == Code::E108 && d.message.contains("unknown runtime symbol")),
        "{diags:?}"
    );
}

#[test]
fn wrong_arity_rejected() {
    let diags = check("import { fn __rnx_clock_mono(extra: Int): Int } from runtime \"time\"\n");
    assert!(
        diags.iter().any(|d| d.code == Code::E108 && d.message.contains("takes 0 args")),
        "{diags:?}"
    );
}

#[test]
fn wrong_param_type_rejected() {
    let diags = check("import { fn __rnx_file_open(path: Int, mode: Int): Int } from runtime \"fs\"\n");
    assert!(
        diags.iter().any(|d| d.code == Code::E108 && d.message.contains("must be `String`")),
        "{diags:?}"
    );
}

#[test]
fn wrong_ret_rejected() {
    let diags = check("import { fn __rnx_clock_mono(): String } from runtime \"time\"\n");
    assert!(
        diags.iter().any(|d| d.code == Code::E108 && d.message.contains("must return `Int`")),
        "{diags:?}"
    );
}

#[test]
fn wrong_module_for_symbol_rejected() {
    let diags = check("import { fn __rnx_clock_mono(): Int } from runtime \"fs\"\n");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Code::E108 && d.message.contains("lives in runtime")),
        "{diags:?}"
    );
}

#[test]
fn runtime_import_in_user_code_fails_closed() {
    let errs = frontend::check::check_source(
        "import { fn __rnx_clock_mono(): Int } from runtime \"time\"\nfn Main(): Int {\n    return __rnx_clock_mono();\n}\n",
    )
    .err()
    .expect("user runtime import must fail");
    assert!(
        errs.iter().any(|d| d.code == Code::E108
            && d.message.contains("__rnx_clock_mono")
            && d.message.contains("outside registry-verified")),
        "{errs:?}"
    );
    assert!(
        errs.iter().any(|d| d
            .hint
            .as_deref()
            .unwrap_or_default()
            .contains("@std/time")),
        "{errs:?}"
    );
}

#[test]
fn bare_rnx_call_is_unknown_name_error() {
    let errs = frontend::check::check_source("fn Main(): Int {\n    return __rnx_clock_mono();\n}\n")
        .err()
        .expect("bare use must fail");
    let hit = errs.iter().find(|d| d.code == Code::E303).expect("{errs:?}");
    assert!(hit.message.contains("__rnx_clock_mono"), "{}", hit.message);
    assert!(
        hit.hint
            .as_deref()
            .unwrap_or_default()
            .contains("from runtime"),
        "{hit:?}"
    );
}

#[test]
fn bare_rnx_let_binding_is_unknown_name_error() {
    let errs = frontend::check::check_source(
        "fn Main(): Int {\n    let f = __rnx_clock_mono;\n    return f();\n}\n",
    )
    .err()
    .expect("bare use must fail");
    assert!(errs.iter().any(|d| d.code == Code::E303), "{errs:?}");
}

#[test]
fn declared_native_alias_still_resolves() {
    let diags = check("import { fn clock(): Int as __rnx_clock_mono } from native \"rt\"\nfn Main(): Int {\n    unsafe {\n        return __rnx_clock_mono();\n    }\n}\n");
    let errors: Vec<_> = diags.into_iter().filter(|d| !d.code.is_warning()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn ffi_name_without_declaration_is_scope_error() {
    let diags = check("fn Main(): Int {\n    return __ffi_clock_mono();\n}\n");
    assert!(diags.iter().any(|d| d.code == Code::E303), "{diags:?}");
}

#[test]
fn runtime_import_alias_binds_local_name() {
    let diags = check("import { fn __rnx_clock_mono(): Int as tick } from runtime \"time\"\nfn Main(): Int {\n    return tick();\n}\n");
    let errors: Vec<_> = diags.into_iter().filter(|d| !d.code.is_warning()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

fn stdlib_files() -> Vec<std::path::PathBuf> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(top) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&top).unwrap().filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rnx") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn code_symbols(src: &str) -> Vec<String> {
    let mut code = String::new();
    let mut in_block = false;
    for line in src.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        let mut in_str = false;
        let mut esc = false;
        while i < chars.len() {
            let c = chars[i];
            let nxt = chars.get(i + 1).copied().unwrap_or('\0');
            if in_block {
                if c == '*' && nxt == '/' {
                    in_block = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if in_str {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                }
                i += 1;
                continue;
            }
            if c == '/' && nxt == '/' {
                break;
            }
            if c == '/' && nxt == '*' {
                in_block = true;
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = true;
                i += 1;
                continue;
            }
            code.push(c);
            i += 1;
        }
        code.push('\n');
    }
    let mut syms: Vec<String> = code
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| w.starts_with("__rnx_"))
        .map(|w| w.to_string())
        .collect();
    syms.sort();
    syms.dedup();
    syms
}

#[test]
fn stdlib_runtime_imports_cover_every_use() {
    let files = stdlib_files();
    assert!(!files.is_empty(), "stdlib sources must exist");
    for path in &files {
        let src = std::fs::read_to_string(path).unwrap();
        let module = Parser::parse_module(&src)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let declared: std::collections::BTreeSet<String> =
            frontend::security::collect_runtime_imports(&module)
                .into_iter()
                .map(|r| r.symbol)
                .collect();
        for sym in code_symbols(&src) {
            assert!(
                declared.contains(&sym),
                "{}: `{sym}` used without a `from runtime` declaration",
                path.display()
            );
        }
        for diag in frontend::semantic::check(&module) {
            if diag.code == Code::E303 && diag.message.contains("__rnx_") {
                panic!("{}: {diag:?}", path.display());
            }
            if diag.code == Code::E108 && diag.message.contains("runtime") {
                panic!("{}: {diag:?}", path.display());
            }
        }
    }
}

#[test]
fn third_party_runtime_import_shapes_rejected() {
    let dir = tree(
        "shapes",
        &[(
            "main.rnx",
            "import { fn __rnx_clock_mono(): Int } from runtime \"time\"\nfn Main(): Int {\n    return __rnx_clock_mono();\n}\n",
        )],
    );
    let graph = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let diags = frontend::security::verify_runtime_imports(&graph);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, Code::E108);
    assert!(diags[0].message.contains("__rnx_clock_mono"), "{}", diags[0].message);
    assert!(
        diags[0]
            .hint
            .as_deref()
            .unwrap_or_default()
            .contains("@std/time"),
        "{:?}",
        diags[0].hint
    );
    assert_eq!(diags[0].file.as_ref().unwrap(), &dir.join("main.rnx"));
}

#[test]
fn runtime_import_cannot_be_reexported() {
    let dir = tree(
        "reexp",
        &[(
            "main.rnx",
            "import { fn __rnx_clock_mono(): Int as tick } from runtime \"time\";\nexport { tick };\nfn Main(): Int {\n    return tick();\n}\n",
        )],
    );
    let graph = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let err = graph.resolve().err().expect("re-export must fail");
    assert_eq!(err.code, Code::E108);
    assert!(err.message.contains("not defined"), "{}", err.message);
}
