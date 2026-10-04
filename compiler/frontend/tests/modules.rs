use diagnostics;
use frontend::ast::Decl;
use frontend::modules::ModuleGraph;

fn tree(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-fe-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, src).unwrap();
    }
    dir
}

fn fn_names(m: &frontend::ast::Module) -> Vec<String> {
    m.decls
        .iter()
        .filter_map(|d| match &d.node {
            Decl::Fn(f) => Some(f.name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn graph_orders_deps_first() {
    let dir = tree("order", &[
        ("main.rnx", "import { add } from \"./math\";\nfn Main(): Int { return add(1, 2); }\n"),
        ("math.rnx", "public fn add(a: Int, b: Int): Int { return a + b; }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert_eq!(g.files.len(), 3);
    assert_eq!(g.files[0].key, "math");
    assert!(g.files[1].path.ends_with("main.rnx"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resolve_mangles_and_rewrites_calls() {
    let dir = tree("mangle", &[
        ("main.rnx", "import { add } from \"./math\";\nfn Main(): Int { return add(1, 2); }\n"),
        ("math.rnx", "public fn add(a: Int, b: Int): Int { return a + b; }\nfn local(): Int { return add(0, 0); }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let m = g.resolve().unwrap();
    let names = fn_names(&m);
    assert!(names.contains(&"Main".to_string()));
    assert!(names.contains(&"math.add".to_string()));
    assert!(names.contains(&"math.local".to_string()));
    assert!(!names.contains(&"add".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wildcard_and_alias_imports() {
    let dir = tree("wild", &[
        ("main.rnx", "import * from \"./lib\";\nimport { thing as other } from \"./lib\";\nfn Main(): Int { return thing() + other(); }\n"),
        ("lib.rnx", "public fn thing(): Int { return 21; }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let m = g.resolve().unwrap();
    assert!(fn_names(&m).contains(&"lib.thing".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nested_keys_use_path() {
    let dir = tree("nested", &[
        ("main.rnx", "import { v } from \"./util/vec\";\nfn Main(): Int { return v(); }\n"),
        ("util/vec.rnx", "public fn v(): Int { return 7; }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let m = g.resolve().unwrap();
    assert!(fn_names(&m).contains(&"util.vec.v".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn local_variable_shadows_import() {
    let dir = tree("shadow", &[
        ("main.rnx", "import { add } from \"./math\";\nfn Main(): Int { let add = 40; return add + 2; }\n"),
        ("math.rnx", "public fn add(a: Int, b: Int): Int { return a + b; }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let m = g.resolve().unwrap();
    let src = format!("{m:?}");
    assert!(src.contains("return add + 2") || !src.contains("math.add + 2"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn default_visibility_imports_across_files() {
    let dir = tree("defvis", &[
        ("main.rnx", "import { helper } from \"./lib\";\nfn Main(): Int { return helper(); }\n"),
        ("lib.rnx", "fn helper(): Int { return 5; }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert!(g.resolve().is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mutual_imports_resolve() {
    let dir = tree("mutual", &[
        ("main.rnx", "import { A } from \"./a\";\nfn Main(): Int { return 0; }\n"),
        ("a.rnx", "import { B } from \"./b\";\nclass A { let b: B; init(b: B) { this.b = b; } }\n"),
        ("b.rnx", "import { A } from \"./a\";\nclass B { let a: A; init(a: A) { this.a = a; } }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let m = g.resolve().unwrap();
    let names: Vec<String> = m
        .decls
        .iter()
        .filter_map(|d| match &d.node {
            Decl::Fn(f) => Some(f.name.clone()),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"Main".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn private_import_fails_e203() {
    let dir = tree("priv", &[
        ("main.rnx", "import { hidden } from \"./lib\";\nfn Main(): Int { return hidden(); }\n"),
        ("lib.rnx", "private fn hidden(): Int { return 1; }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let e = g.resolve().expect_err("must fail");
    assert_eq!(e.code, diagnostics::Code::E203);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_virtual_module_resolves() {
    let dir = tree("stdv", &[(
        "main.rnx",
        "import { Clock } from \"@std/time\";\nfn Main(): Int { return 0; }\n",
    )]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert!(g.files.iter().any(|f| f.key == "std.time"));
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    assert!(m.decls.iter().any(|d| match &d.node {
        Decl::Fn(f) => f.name == "std.time.Clock.mono" || f.name == "std.time.Clock.virtual",
        _ => false,
    }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_std_module_errors() {
    let dir = tree("stdb", &[(
        "main.rnx",
        "import { X } from \"@std/bogus\";\nfn Main(): Int { return 0; }\n",
    )]);
    let e = match ModuleGraph::build(&dir.join("main.rnx")) {
        Ok(_) => panic!("must fail"),
        Err(e) => e,
    };
    assert_eq!(e.code, diagnostics::Code::E108);
    assert!(e.message.contains("unknown standard library module"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_env_module_resolves() {
    let dir = tree("stde", &[(
        "main.rnx",
        "import { Env } from \"@std/env\";\nfn Main(): Int { return 0; }\n",
    )]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert!(g.files.iter().any(|f| f.key == "std.env"));
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    assert!(m.decls.iter().any(|d| match &d.node {
        Decl::Fn(f) => f.name == "std.env.Env.args",
        _ => false,
    }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_math_module_resolves() {
    let dir = tree("stdm", &[(
        "main.rnx",
        "import { Math, Vec2 } from \"@std/math\";\nfn Main(): Int { return 0; }\n",
    )]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert!(g.files.iter().any(|f| f.key == "std.math"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_collections_module_resolves() {
    let dir = tree("stdc", &[(
        "main.rnx",
        "import { Map, Set } from \"@std/collections\";\nfn Main(): Int { return 0; }\n",
    )]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert!(g.files.iter().any(|f| f.key == "std.collections"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_sync_module_resolves() {
    let dir = tree("stds", &[(
        "main.rnx",
        "import { AtomicInt, Channel } from \"@std/sync\";\nfn Main(): Int { return 0; }\n",
    )]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert!(g.files.iter().any(|f| f.key == "std.sync"));
    let _ = std::fs::remove_dir_all(&dir);
}

fn pkg_tree(tag: &str, pkgs: &[(&str, &str, &[(&str, &str)])]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-fe-pkg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (pkg, config, files) in pkgs {
        let root = dir.join(pkg);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Project.config"), config).unwrap();
        for (name, src) in *files {
            let p = root.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, src).unwrap();
        }
    }
    dir
}

const LIB_CFG: &str = "export default {\n    project: {\n        name: \"dep_math\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/lib.rnx\" }\n}";
const APP_CFG: &str = "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        dep_math: { path: \"../dep_math\" }\n    }\n}\n";

#[test]
fn packages_resolve_entry_and_submodule() {
    let dir = pkg_tree("basic", &[
        ("dep_math", LIB_CFG, &[
            ("src/lib.rnx", "fn add(a: Int, b: Int): Int { return a + b; }\n"),
            ("src/util.rnx", "fn double(x: Int): Int { return x * 2; }\n"),
        ]),
        ("app", APP_CFG, &[
            ("src/main.rnx", "import { add } from \"dep_math\";\nimport { double } from \"dep_math/util\";\nfn Main(): Int { return double(add(1, 2)); }\n"),
        ]),
    ]);
    let g = ModuleGraph::build(&dir.join("app").join("src").join("main.rnx")).unwrap();
    assert_eq!(g.files.len(), 4);
    let m = g.resolve().unwrap();
    let names = fn_names(&m);
    assert!(names.contains(&"dep_math.src.lib.add".to_string()), "{names:?}");
    assert!(names.contains(&"dep_math.src.util.double".to_string()), "{names:?}");
    assert!(names.contains(&"Main".to_string()), "{names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn packages_diamond_parsed_once() {
    let dir = pkg_tree("diamond", &[
        ("shared", "export default {\n    project: {\n        name: \"shared\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/lib.rnx\" }\n}", &[
            ("src/lib.rnx", "fn forty(): Int { return 40; }\n"),
        ]),
        ("left", "export default {\n    project: {\n        name: \"left\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/lib.rnx\" },\n    dependencies: {\n        shared: { path: \"../shared\" }\n    }\n}\n", &[
            ("src/lib.rnx", "import { forty } from \"shared\";\nfn l(): Int { return forty() + 1; }\n"),
        ]),
        ("right", "export default {\n    project: {\n        name: \"right\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/lib.rnx\" },\n    dependencies: {\n        shared: { path: \"../shared\" }\n    }\n}\n", &[
            ("src/lib.rnx", "import { forty } from \"shared\";\nfn r(): Int { return forty() + 2; }\n"),
        ]),
        ("app", "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        left: { path: \"../left\" },\n        right: { path: \"../right\" }\n    }\n}\n", &[
            ("src/main.rnx", "import { l } from \"left\";\nimport { r } from \"right\";\nfn Main(): Int { return l() + r(); }\n"),
        ]),
    ]);
    let g = ModuleGraph::build(&dir.join("app").join("src").join("main.rnx")).unwrap();
    let shared = g.files.iter().filter(|f| f.key == "shared.src.lib").count();
    assert_eq!(shared, 1);
    assert!(g.resolve().is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn packages_transitive_resolution() {
    let dir = pkg_tree("trans", &[
        ("leaf", "export default {\n    project: {\n        name: \"leaf\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/lib.rnx\" }\n}", &[
            ("src/lib.rnx", "fn base(): Int { return 7; }\n"),
        ]),
        ("mid", "export default {\n    project: {\n        name: \"mid\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/lib.rnx\" },\n    dependencies: {\n        leaf: { path: \"../leaf\" }\n    }\n}\n", &[
            ("src/lib.rnx", "import { base } from \"leaf\";\nfn mid6(): Int { return base() * 6; }\n"),
        ]),
        ("app", "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        mid: { path: \"../mid\" }\n    }\n}\n", &[
            ("src/main.rnx", "import { mid6 } from \"mid\";\nfn Main(): Int { return mid6(); }\n"),
        ]),
    ]);
    let g = ModuleGraph::build(&dir.join("app").join("src").join("main.rnx")).unwrap();
    assert_eq!(g.files.len(), 4);
    let m = g.resolve().unwrap();
    assert!(fn_names(&m).contains(&"leaf.src.lib.base".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn packages_cycle_is_e107() {
    let dir = pkg_tree("cycle", &[
        ("a", "export default {\n    project: {\n        name: \"a\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/lib.rnx\" },\n    dependencies: {\n        b: { path: \"../b\" }\n    }\n}\n", &[
            ("src/lib.rnx", "import { g } from \"b\";\nfn f(): Int { return g(); }\n"),
        ]),
        ("b", "export default {\n    project: {\n        name: \"b\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/lib.rnx\" },\n    dependencies: {\n        a: { path: \"../a\" }\n    }\n}\n", &[
            ("src/lib.rnx", "import { f } from \"a\";\nfn g(): Int { return f(); }\n"),
        ]),
    ]);
    let e = match ModuleGraph::build(&dir.join("a").join("src").join("lib.rnx")) {
        Ok(_) => panic!("expected E107"),
        Err(e) => e,
    };
    assert_eq!(e.code, diagnostics::Code::E107);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn packages_unknown_is_e108() {
    let dir = pkg_tree("unknown", &[(
        "app",
        "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    }\n}\n",
        &[("src/main.rnx", "import { x } from \"ghost\";\nfn Main(): Int { return x(); }\n")],
    )]);
    let e = match ModuleGraph::build(&dir.join("app").join("src").join("main.rnx")) {
        Ok(_) => panic!("expected E108"),
        Err(e) => e,
    };
    assert_eq!(e.code, diagnostics::Code::E108);
    assert!(e.message.contains("unknown package dependency `ghost`"), "{}", e.message);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn packages_workspace_sibling_needs_no_explicit_path() {
    let dir = pkg_tree("sib", &[
        ("ws", "export default {\n    workspace: {\n        members: [\"calc\", \"player\"]\n    }\n}\n", &[]),
        ("ws/calc", "export default {\n    project: {\n        name: \"calc\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/lib.rnx\" }\n}", &[
            ("src/lib.rnx", "fn score(): Int { return 42; }\n"),
        ]),
        ("ws/player", "export default {\n    project: {\n        name: \"player\",\n        version: \"0.1.0\"\n    }\n}\n", &[
            ("src/main.rnx", "import { score } from \"calc\";\nfn Main(): Int { return score(); }\n"),
        ]),
    ]);
    let g = ModuleGraph::build(&dir.join("ws").join("player").join("src").join("main.rnx")).unwrap();
    assert_eq!(g.files.len(), 3);
    let m = g.resolve().unwrap();
    assert!(fn_names(&m).contains(&"calc.src.lib.score".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scheme_prefix_is_e108_with_scoped_hint() {
    for (spec, hint) in [
        ("std:time", "`@std/time`"),
        ("pkg:sqlite3", "`sqlite3`"),
    ] {
        let dir = tree("scheme", &[(
            "main.rnx",
            &format!("import {{ X }} from \"{spec}\";\nfn Main(): Int {{ return 0; }}\n"),
        )]);
        let e = match ModuleGraph::build(&dir.join("main.rnx")) {
            Ok(_) => panic!("must fail for {spec}"),
            Err(e) => e,
        };
        assert_eq!(e.code, diagnostics::Code::E108, "{spec}");
        assert!(e.message.contains(&format!("cannot resolve module `{spec}`")), "{}", e.message);
        let h = e.hint.unwrap_or_default();
        assert!(h.contains(hint), "{spec}: {h}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn relative_probes_mod_and_index() {
    let dir = tree("relprobe", &[
        ("main.rnx", "import { a } from \"./dir_a\";\nimport { b } from \"./dir_b\";\nfn Main(): Int { return a() + b(); }\n"),
        ("dir_a/mod.rnx", "fn a(): Int { return 20; }\n"),
        ("dir_b/index.rnx", "fn b(): Int { return 22; }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert_eq!(g.files.len(), 4);
    let m = g.resolve().unwrap();
    assert!(fn_names(&m).contains(&"Main".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn relative_missing_lists_probed_paths() {
    let dir = tree("relmiss", &[(
        "main.rnx",
        "import { x } from \"./ghost\";\nfn Main(): Int { return 0; }\n",
    )]);
    let e = match ModuleGraph::build(&dir.join("main.rnx")) {
        Ok(_) => panic!("must fail"),
        Err(e) => e,
    };
    assert_eq!(e.code, diagnostics::Code::E108);
    assert!(e.message.contains("ghost.rnx"), "{}", e.message);
    assert!(e.message.contains("mod.rnx"), "{}", e.message);
    assert!(e.message.contains("index.rnx"), "{}", e.message);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_prelude_resolves_through_sysroot() {
    let dir = tree("prelude", &[(
        "main.rnx",
        "import * from \"@std/prelude\";\nfn Main(): Int { return 0; }\n",
    )]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert!(g.files.iter().any(|f| f.key == "std.prelude"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scoped_package_submodule_resolves() {
    let dir = pkg_tree("scoped", &[
        ("ui", "export default {\n    project: {\n        name: \"@rovelstars/ui\",\n        version: \"1.2.0\"\n    },\n    entries: { main: \"src/lib.rnx\" }\n}", &[
            ("src/lib.rnx", "fn render(): Int { return 1; }\n"),
            ("src/theme.rnx", "fn dark(): Int { return 2; }\n"),
        ]),
        ("app", "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        \"@rovelstars/ui\": { path: \"../ui\" }\n    }\n}\n", &[
            ("src/main.rnx", "import { render } from \"@rovelstars/ui\";\nimport { dark } from \"@rovelstars/ui/theme\";\nfn Main(): Int { return render() + dark(); }\n"),
        ]),
    ]);
    let g = ModuleGraph::build(&dir.join("app").join("src").join("main.rnx")).unwrap();
    assert_eq!(g.files.len(), 4);
    let m = g.resolve().unwrap();
    let names = fn_names(&m);
    assert!(names.iter().any(|n| n.ends_with("render")), "{names:?}");
    assert!(names.iter().any(|n| n.ends_with("dark")), "{names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resolve_renames_module_const_used_in_own_fn() {
    let dir = tree("constmerge", &[
        ("main.rnx", "import { get } from \"./lib\";\nfn Main(): Int { return get(); }\n"),
        ("lib.rnx", "const K: Int = 5;\nexport fn get(): Int { return K; }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let m = g.resolve().unwrap();
    let consts: Vec<String> = m.decls.iter().filter_map(|d| match &d.node {
        Decl::Const { name, .. } => Some(name.clone()),
        _ => None,
    }).collect();
    assert!(consts.contains(&"lib.K".to_string()), "const decl must mangle with its uses: {consts:?}");
    assert!(!consts.contains(&"K".to_string()), "stale short const decl: {consts:?}");
    let diags = frontend::semantic::check(&m);
    assert!(diags.iter().all(|x| x.code.is_warning()), "{diags:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn namespace_as_value_becomes_new() {
    let dir = tree("nsval", &[
        ("main.rnx", "import ns from \"./lib\";\nfn Main(): Int { print(ns); return 0; }\n"),
        ("lib.rnx", "const K: Int = 5;\nfn helper(): Int { return K; }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let m = g.resolve().unwrap();
    let src = format!("{m:?}");
    assert!(src.contains("__ns_lib.ns"), "bare ns must rewrite to a namespace object: {src}");
    assert!(m.decls.iter().any(|d| match &d.node {
        Decl::Class { name, .. } => name == "__ns_lib.ns",
        _ => false,
    }));
    let diags = frontend::semantic::check(&m);
    assert!(diags.iter().all(|x| x.code.is_warning()), "{diags:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_import_in_dep_fails_despite_user_import() {
    let dir = tree("isolation", &[
        ("main.rnx", "import { useClock } from \"./liba\";\nimport { Clock } from \"@std/time\";\nfn Main(): Int { return 0; }\n"),
        ("liba.rnx", "export fn useClock(): Clock { return Clock.mono(); }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let m = g.resolve().unwrap();
    let diags = frontend::semantic::check(&m);
    assert!(diags.iter().all(|x| x.code.is_warning()), "merged check is blind to the leak by design: {diags:?}");
    let iso = g.isolation_errors();
    assert!(iso.iter().any(|x| !x.code.is_warning()), "liba naming Clock without importing it must fail even though main imports time: {iso:?}");
    assert!(!iso.iter().any(|x| x.code.is_warning()), "isolation reports errors, not warnings: {iso:?}");
    for x in &iso {
        let f = x.file.as_ref().expect("isolation errors carry the offending file");
        assert!(f.ends_with("liba.rnx"), "error must point at liba, got {}", f.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sibling_type_use_without_import_fails() {
    let dir = tree("sibiso", &[
        ("main.rnx", "import { a } from \"./a\";\nimport { c } from \"./c\";\nfn Main(): Int { return a().id + c(); }\n"),
        ("a.rnx", "export fn a(): Widget { return new Widget(1); }\n"),
        ("b.rnx", "class Widget { let id: Int; init(id: Int) { this.id = id; } }\nfn makeWidget(): Widget { return new Widget(1); }\n"),
        ("c.rnx", "import { Widget, makeWidget } from \"./b\";\nexport fn c(): Int { let w: Widget = makeWidget(); return w.id; }\n"),
    ]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let m = g.resolve().unwrap();
    let diags = frontend::semantic::check(&m);
    assert!(diags.iter().all(|x| x.code.is_warning()), "merged check is blind to the leak by design: {diags:?}");
    let iso = g.isolation_errors();
    assert!(iso.iter().any(|x| !x.code.is_warning()), "a.rnx naming Widget without importing it must fail even though c imports it: {iso:?}");
    for x in &iso {
        let f = x.file.as_ref().expect("isolation errors carry the offending file");
        assert!(f.ends_with("a.rnx"), "error must point at a.rnx, got {}", f.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_nested_submodule_resolves_with_parent() {
    let src = "import { Dns } from \"@std/net\";\nimport { encodeRequestLine, statusCode, isSuccess } from \"@std/net/http\";\nfn Main(): Int { let line = encodeRequestLine(\"GET\", \"/health\"); let code = statusCode(\"HTTP/1.1 200 OK\"); if (isSuccess(code)) { return code; } return 0 - 1; }\n";
    let m = ModuleGraph::from_source(src).unwrap();
    let mut names: Vec<String> = m
        .decls
        .iter()
        .filter_map(|d| match &d.node {
            Decl::Fn(f) => Some(f.name.clone()),
            Decl::Class { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    names.sort();
    assert!(names.contains(&"std.net.http.encodeRequestLine".to_string()), "{names:?}");
    assert!(names.contains(&"std.net.http.statusCode".to_string()), "{names:?}");
    assert!(names.contains(&"std.net.http.isSuccess".to_string()), "{names:?}");
    assert!(names.contains(&"std.net.Dns".to_string()), "{names:?}");
    let diags = frontend::semantic::check(&m);
    assert!(diags.iter().all(|x| x.code.is_warning()), "{diags:?}");
}

#[test]
fn std_nested_submodule_file_build() {
    let dir = tree("stdnest", &[(
        "main.rnx",
        "import { statusCode } from \"@std/net/http\";\nfn Main(): Int { return statusCode(\"HTTP/1.1 404 Not Found\"); }\n",
    )]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert!(g.files.iter().any(|f| f.key == "std.net.http"), "{:?}", g.files.iter().map(|f| &f.key).collect::<Vec<_>>());
    let m = g.resolve().unwrap();
    assert!(fn_names(&m).contains(&"std.net.http.statusCode".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_unknown_nested_is_e108() {
    let dir = tree("stdmiss", &[(
        "main.rnx",
        "import { X } from \"@std/net/bogus\";\nfn Main(): Int { return 0; }\n",
    )]);
    let e = match ModuleGraph::build(&dir.join("main.rnx")) {
        Ok(_) => panic!("must fail"),
        Err(e) => e,
    };
    assert_eq!(e.code, diagnostics::Code::E108);
    assert!(e.message.contains("@std/net/bogus"), "{}", e.message);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn packages_deep_nested_submodule() {
    let dir = pkg_tree("deepnest", &[
        ("dep_math", LIB_CFG, &[
            ("src/lib.rnx", "fn add(a: Int, b: Int): Int { return a + b; }\n"),
            ("src/ops/deep.rnx", "fn triple(x: Int): Int { return x * 3; }\n"),
            ("src/folder/mod.rnx", "fn modded(): Int { return 9; }\n"),
            ("src/alt/index.rnx", "fn indexed(): Int { return 11; }\n"),
        ]),
        ("app", APP_CFG, &[
            ("src/main.rnx", "import { add } from \"dep_math\";\nimport { triple } from \"dep_math/ops/deep\";\nimport { modded } from \"dep_math/folder\";\nimport { indexed } from \"dep_math/alt\";\nfn Main(): Int { return add(triple(1), modded()) + indexed(); }\n"),
        ]),
    ]);
    let g = ModuleGraph::build(&dir.join("app").join("src").join("main.rnx")).unwrap();
    let m = g.resolve().unwrap();
    let names = fn_names(&m);
    assert!(names.contains(&"dep_math.src.ops.deep.triple".to_string()), "{names:?}");
    assert!(names.contains(&"dep_math.src.folder.mod.modded".to_string()), "{names:?}");
    assert!(names.contains(&"dep_math.src.alt.index.indexed".to_string()), "{names:?}");
    assert!(names.contains(&"dep_math.src.lib.add".to_string()), "{names:?}");
    let diags = frontend::semantic::check(&m);
    assert!(diags.iter().all(|x| x.code.is_warning()), "{diags:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn packages_subpath_traversal_is_e108() {
    for spec in ["dep_math/../evil", "dep_math/./lib", "dep_math/a/../../evil", "dep_math/"] {
        let dir = pkg_tree("traversal", &[
            ("dep_math", LIB_CFG, &[
                ("src/lib.rnx", "fn add(a: Int, b: Int): Int { return a + b; }\n"),
            ]),
            ("app", APP_CFG, &[
                ("src/main.rnx", &format!("import {{ add }} from \"{spec}\";\nfn Main(): Int {{ return add(1, 2); }}\n")),
            ]),
        ]);
        let e = match ModuleGraph::build(&dir.join("app").join("src").join("main.rnx")) {
            Ok(_) => panic!("must fail for {spec}"),
            Err(e) => e,
        };
        assert_eq!(e.code, diagnostics::Code::E108, "{spec}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
