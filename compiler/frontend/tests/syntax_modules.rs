use frontend::ast::*;
use frontend::parser::Parser;

fn parse(src: &str) -> Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("parse failed: {e}\n---\n{src}"))
}

fn parse_err(src: &str) -> diagnostics::Diagnostic {
    Parser::parse_module(src).expect_err("expected parse error")
}

fn single(src: &str) -> Spanned<Decl> {
    let m = parse(src);
    assert_eq!(m.decls.len(), 1, "expected one decl in {src}");
    m.decls.into_iter().next().unwrap()
}

#[test]
fn parses_default_import() {
    match single("import Logger from \"./logger\"\n").node {
        Decl::Import(d) => {
            assert!(matches!(d.clause, ImportClause::Default(n) if n == "Logger"));
            assert!(matches!(d.source, ImportSource::Module(s) if s == "./logger"));
        }
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn parses_named_and_aliased_imports() {
    match single("import { readFile, writeFile as write } from \"./fs\"\n").node {
        Decl::Import(d) => match d.clause {
            ImportClause::Named(specs) => {
                assert_eq!(specs.len(), 2);
                assert_eq!(specs[0].name, "readFile");
                assert_eq!(specs[0].alias, None);
                assert!(!specs[0].is_export);
                assert_eq!(specs[1].name, "writeFile");
                assert_eq!(specs[1].alias.as_deref(), Some("write"));
            }
            other => panic!("wrong clause: {other:?}"),
        },
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn parses_combined_default_and_named_import() {
    match single("import React, { useState, useEffect } from \"./react\"\n").node {
        Decl::Import(d) => match d.clause {
            ImportClause::DefaultAndNamed(def, specs) => {
                assert_eq!(def, "React");
                assert_eq!(specs.len(), 2);
            }
            other => panic!("wrong clause: {other:?}"),
        },
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn parses_namespace_import() {
    match single("import * as MathUtils from \"./math\"\n").node {
        Decl::Import(d) => {
            assert!(matches!(d.clause, ImportClause::Namespace(n) if n == "MathUtils"));
        }
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn parses_side_effect_import() {
    match single("import \"./init\"\n").node {
        Decl::Import(d) => {
            assert!(matches!(d.clause, ImportClause::SideEffect));
        }
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn parses_inline_export_in_import() {
    match single("import { export readFile, writeFile } from \"./fs\"\n").node {
        Decl::Import(d) => match d.clause {
            ImportClause::Named(specs) => {
                assert!(specs[0].is_export);
                assert!(!specs[1].is_export);
            }
            other => panic!("wrong clause: {other:?}"),
        },
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn parses_export_fn_struct_const() {
    match single("export fn add(a: Int, b: Int): Int {\n return a;\n}\n").node {
        Decl::Fn(f) => assert_eq!(f.access, Visibility::Export),
        other => panic!("wrong decl: {other:?}"),
    }
    match single("export struct Point { let x: Float; let y: Float }\n").node {
        Decl::Struct { access, .. } => assert_eq!(access, Visibility::Export),
        other => panic!("wrong decl: {other:?}"),
    }
    match single("export const PI = 3.14\n").node {
        Decl::Const { access, name, .. } => {
            assert_eq!(access, Visibility::Export);
            assert_eq!(name, "PI");
        }
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn parses_export_default_forms() {
    match single("export default fn main() {\n}\n").node {
        Decl::Fn(f) => assert_eq!(f.access, Visibility::Export),
        other => panic!("wrong decl: {other:?}"),
    }
    match single("export default { name: \"x\" }\n").node {
        Decl::ExportDefault(d) => {
            assert!(matches!(d.expr.node, Expr::Record(_)));
        }
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn parses_reexport_named_and_star() {
    match single("export { readFile as read, writeFile } from \"./fs\"\n").node {
        Decl::ExportFrom(d) => match d.clause {
            ExportClause::Named(specs) => {
                assert_eq!(specs[0].name, "readFile");
                assert_eq!(specs[0].alias.as_deref(), Some("read"));
                assert_eq!(specs[1].name, "writeFile");
            }
            other => panic!("wrong clause: {other:?}"),
        },
        other => panic!("wrong decl: {other:?}"),
    }
    match single("export * from \"./types\"\n").node {
        Decl::ExportFrom(d) => {
            assert!(matches!(d.clause, ExportClause::All { alias: None }));
        }
        other => panic!("wrong decl: {other:?}"),
    }
    match single("export * as MathAPI from \"./math\"\n").node {
        Decl::ExportFrom(d) => match d.clause {
            ExportClause::All { alias } => assert_eq!(alias.as_deref(), Some("MathAPI")),
            other => panic!("wrong clause: {other:?}"),
        },
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn parses_native_import_with_and_without_alias() {
    match single(
        "import {\n fn puts(s: Pointer<Byte>): Int,\n fn malloc(size: Int): Pointer<Byte>,\n fn free(ptr: Pointer<Byte>)\n} from native \"libc\"\n",
    )
    .node
    {
        Decl::Import(d) => {
            assert!(matches!(d.source, ImportSource::Native(s) if s == "libc"));
            match d.clause {
                ImportClause::Named(specs) => {
                    assert_eq!(specs.len(), 3);
                    assert!(specs.iter().all(|s| s.native_fn.is_some()));
                    assert!(specs[2].native_fn.as_ref().unwrap().ret.is_none());
                }
                other => panic!("wrong clause: {other:?}"),
            }
        }
        other => panic!("wrong decl: {other:?}"),
    }
    match single(
        "import {\n fn c_get_time(): Int as getTime,\n fn sqlite3_open(name: Pointer<Byte>, db: Pointer<Pointer<Byte>>): Int as openDb\n} from native \"sqlite3\"\n",
    )
    .node
    {
        Decl::Import(d) => match d.clause {
            ImportClause::Named(specs) => {
                let first = specs[0].native_fn.as_ref().unwrap();
                assert_eq!(first.name, "c_get_time");
                assert_eq!(first.alias.as_deref(), Some("getTime"));
                assert_eq!(frontend::modules::import_local_name(&specs[1]), "openDb");
            }
            other => panic!("wrong clause: {other:?}"),
        },
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn parses_native_reexport_and_mixed_native_import() {
    match single(
        "export {\n fn compressBound(sourceLen: Int): Int,\n fn compress(dest: Pointer<Byte>, destLen: Pointer<Int>, src: Pointer<Byte>, srcLen: Int): Int\n} from native \"z\"\n",
    )
    .node
    {
        Decl::ExportFrom(d) => {
            assert!(matches!(d.source, ImportSource::Native(s) if s == "z"));
            match d.clause {
                ExportClause::Native(sigs) => assert_eq!(sigs.len(), 2),
                other => panic!("wrong clause: {other:?}"),
            }
        }
        other => panic!("wrong decl: {other:?}"),
    }
    match single(
        "import {\n export fn compress(dest: Pointer<Byte>, destLen: Pointer<Int>, src: Pointer<Byte>, srcLen: Int): Int,\n fn deflateInit_(strm: Pointer<Byte>, level: Int, version: Pointer<Byte>, stream_size: Int): Int\n} from native \"z\"\n",
    )
    .node
    {
        Decl::Import(d) => match d.clause {
            ImportClause::Named(specs) => {
                assert!(specs[0].is_export);
                assert!(!specs[1].is_export);
                assert!(specs.iter().all(|s| s.native_fn.is_some()));
            }
            other => panic!("wrong clause: {other:?}"),
        },
        other => panic!("wrong decl: {other:?}"),
    }
}

#[test]
fn pub_parses_as_export_with_w204() {
    let m = parse("pub fn helper(): Void {\n}\n");
    match &m.decls[0].node {
        Decl::Fn(f) => assert_eq!(f.access, Visibility::Export),
        other => panic!("wrong decl: {other:?}"),
    }
    assert!(
        m.warnings.iter().any(|d| d.code == diagnostics::Code::W204),
        "expected W204, got {:?}",
        m.warnings
    );
    let m = parse("public const K = 1\n");
    assert!(m.warnings.iter().any(|d| d.code == diagnostics::Code::W204));
    assert!(matches!(m.decls[0].node, Decl::Const { access: Visibility::Export, .. }));
}

#[test]
fn rejects_dynamic_import() {
    let e = parse_err("fn Main(): Int {\n let m = import(\"./dynamic\");\n return 0;\n}\n");
    assert_eq!(e.code, diagnostics::Code::E108);
    assert!(e.message.contains("not supported"), "{e:?}");
}

#[test]
fn rejects_fn_sig_on_module_import() {
    let e = parse_err("import { fn calculate(x: Int): String } from \"./math\"\n");
    assert_eq!(e.code, diagnostics::Code::E108);
    assert!(e.message.contains("forbidden on module imports"), "{e:?}");
}

#[test]
fn rejects_bare_native_import() {
    let e = parse_err("import from native \"libc\"\n");
    assert_eq!(e.code, diagnostics::Code::E108);
    assert!(e.message.contains("expected `{`"), "{e:?}");
}

#[test]
fn rejects_bare_native_block() {
    let e = parse_err("native \"c\" {\n fn puts(s: String): Int\n}\n");
    assert_eq!(e.code, diagnostics::Code::E108);
    assert!(e.message.contains("from native"), "{e:?}");
}

#[test]
fn formatter_round_trips_new_syntax() {
    let srcs = [
        "export fn add(a: Int, b: Int): Int {\n return a;\n}\n",
        "import { readFile, writeFile as write } from \"./fs\"\n",
        "import { fn puts(s: Pointer<Byte>): Int } from native \"libc\"\n",
        "export { readFile as read } from \"./fs\"\n",
        "export * as MathAPI from \"./math\"\n",
    ];
    for src in srcs {
        let once = frontend::fmt::format_source(src).expect("formats");
        let m1 = parse(src);
        let m2 = parse(&once);
        assert_eq!(m1.decls.len(), m2.decls.len(), "decl count drift for {src}");
        let twice = frontend::fmt::format_source(&once).expect("reformats");
        assert_eq!(once, twice, "formatter not idempotent for {src}");
    }
}
