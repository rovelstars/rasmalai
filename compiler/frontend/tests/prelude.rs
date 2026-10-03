use frontend::modules::ModuleGraph;
use frontend::parser::Parser;
use frontend::semantic;

fn check_src(src: &str) -> Vec<diagnostics::Diagnostic> {
    let m = Parser::parse_module(src).expect("parses");
    semantic::check(&m)
}

fn errors(src: &str) -> Vec<diagnostics::Diagnostic> {
    check_src(src).into_iter().filter(|d| !d.code.is_warning()).collect()
}

fn tree(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-fe-prelude-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, src).unwrap();
    }
    dir
}

#[test]
fn catalog_covers_foundation_types() {
    for name in [
        "Int", "Float", "FastFloat", "Bool", "String", "Char", "Array", "Map", "Set",
        "Date", "Error", "Result", "Any", "Void", "GenRef",
    ] {
        assert!(frontend::prelude::provides(name), "{name} missing from prelude");
    }
    assert!(!frontend::prelude::provides("Option"), "Option must stay removed");
    assert!(!frontend::prelude::provides("Ghost"));
    assert!(!frontend::prelude::provides("Str"));
    assert!(frontend::prelude::symbol_count() >= 15);
}

#[test]
fn bare_prelude_types_check_clean_with_zero_imports() {
    let errs = errors("fn Main(): Int {\nlet count: Int = 0;\nlet items: Array = [];\nlet greeting: String = \"hi\";\nreturn count;\n}\n");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn removed_str_spelling_fails_e303() {
    for src in [
        "fn Main(): Int {\nlet legacy: Str = \"hi\";\nreturn 0;\n}\n",
        "fn take(s: Str): Int { return 0; }\nfn Main(): Int { return 0; }\n",
        "fn give(): Str { return \"hi\"; }\nfn Main(): Int { return 0; }\n",
    ] {
        let errs = errors(src);
        assert!(
            errs.iter().any(|d| d.code == diagnostics::Code::E303 && d.message.contains("`Str`")),
            "{src}: {errs:?}"
        );
        assert!(
            errs.iter().any(|d| d.hint.as_deref().unwrap_or_default().contains("String")),
            "{src}: {errs:?}"
        );
    }
}

#[test]
fn prelude_use_tracking_is_demand_driven() {
    let m = Parser::parse_module("fn Main(): Int {\nlet count: Int = 0;\nlet items: Array = [];\nlet greeting: String = \"hi\";\nreturn count;\n}\n")
        .expect("parses");
    let used = semantic::prelude_uses(&m);
    assert!(used.contains("Int"), "{used:?}");
    assert!(used.contains("Array"), "{used:?}");
    assert!(used.contains("String"), "{used:?}");
    assert!(!used.contains("Date"), "{used:?}");
    assert!(!used.contains("Map"), "{used:?}");
}

#[test]
fn user_struct_shadows_prelude_symbol() {
    let src = "struct Option { let value: Int }\nfn pick(o: Option): Int { return o.value; }\nfn Main(): Int { return 0; }\n";
    let errs = errors(src);
    assert!(errs.is_empty(), "{errs:?}");
    let m = Parser::parse_module(src).expect("parses");
    let used = semantic::prelude_uses(&m);
    assert!(!used.contains("Option"), "{used:?}");
    assert!(used.contains("Int"), "{used:?}");
}

#[test]
fn mismatch_messages_spell_canonical_string() {
    let errs = errors("fn f(): String { return 42; }\nfn Main(): Int { return 0; }\n");
    assert!(errs.iter().any(|d| d.code == diagnostics::Code::E304 && d.message.contains("`String`")), "{errs:?}");
    assert!(errs.iter().all(|d| !d.message.contains("`Str`")), "{errs:?}");
}

#[test]
fn unknown_names_still_error() {
    let errs = errors("fn Main(): Int {\nlet ghost = frobnicate;\nreturn ghost;\n}\n");
    assert!(errs.iter().any(|d| d.code == diagnostics::Code::E303), "{errs:?}");
}

#[test]
fn merged_graph_carries_zero_prelude_bloat() {
    let dir = tree("bloat", &[(
        "main.rnx",
        "fn Main(): Int {\nlet count: Int = 0;\nlet items: Array = [];\nreturn count;\n}\n",
    )]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert!(g.files.iter().any(|f| f.key == "std.prelude"));
    let m = g.resolve().unwrap();
    let mut fn_names = Vec::new();
    let mut ty_names = Vec::new();
    for d in &m.decls {
        match &d.node {
            frontend::ast::Decl::Fn(f) => fn_names.push(f.name.clone()),
            frontend::ast::Decl::Class { name, .. }
            | frontend::ast::Decl::Struct { name, .. }
            | frontend::ast::Decl::Enum { name, .. }
            | frontend::ast::Decl::Record { name, .. } => ty_names.push(name.clone()),
            _ => {}
        }
    }
    assert!(fn_names.iter().all(|n| !n.contains("prelude")), "{fn_names:?}");
    assert!(fn_names.contains(&"Main".to_string()), "{fn_names:?}");
    assert!(ty_names.iter().any(|n| n == "std.prelude.Result"), "{ty_names:?}");
    assert!(!ty_names.iter().any(|n| n == "std.prelude.Option"), "{ty_names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
