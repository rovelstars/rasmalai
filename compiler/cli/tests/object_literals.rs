use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-objlit-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check(&m);
    assert!(c.iter().all(|x| x.code.is_warning()), "{c:?}");
    let mut out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    lir::opt::optimize_lir(&mut out, 1, "Main");
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    (out, dir)
}

fn check_all_backends(src: &str, want: i64, want_out: &[String], tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(v) if v == want => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    let got = machine.output.clone();
    let want_out: Vec<String> = want_out.to_vec();
    assert_eq!(got, want_out, "interpreter {tag} stdout");

    let mut jit = cranelift::jit::Jit::compile(leaked).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), want, "cranelift {tag}");
    assert_eq!(llvm::codegen::execute(leaked, "Main").unwrap(), want, "llvm {tag}");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), want as i32, "aot {tag} exit");
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const PRELUDE: &str = "import { Map } from \"@std/collections\";\n";

#[test]
fn empty_object_literal_is_map() {
    let src = format!(
        "{PRELUDE}\nfn Main(): Int {{\n    let a = {{}};\n    assert(typeOf(a) == \"Map\", \"empty is Map\");\n    assert(a.len() == 0, \"empty len\");\n    assert(a.get(\"missing\") == null, \"missing is null\");\n    print(\"empty-ok\");\n    return 0;\n}}\n"
    );
    check_all_backends(&src, 0, &["empty-ok".to_string()], "empty");
}

#[test]
fn populated_object_literal_gets() {
    let src = format!(
        "{PRELUDE}\nfn Main(): Int {{\n    let config = {{ \"host\": \"127.0.0.1\", \"port\": 8080 }};\n    assert(config.get(\"host\") == \"127.0.0.1\", \"host\");\n    assert((config.get(\"port\") ?? -1) == 8080, \"port\");\n    config.set(\"port\", 9090);\n    assert((config.get(\"port\") ?? -1) == 9090, \"overwrite\");\n    print(\"populated-ok\");\n    return 0;\n}}\n"
    );
    check_all_backends(&src, 0, &["populated-ok".to_string()], "populated");
}

#[test]
fn nested_object_literals() {
    let src = format!(
        "{PRELUDE}\nfn Main(): Int {{\n    let nested = {{ \"user\": {{ \"name\": \"Alice\" }}, \"active\": true, }};\n    let user: Map<String, Any> = nested.get(\"user\");\n    assert(user.get(\"name\") == \"Alice\", \"nested name\");\n    assert((nested.get(\"active\") ?? false) == true, \"nested bool\");\n    print(\"nested-ok\");\n    return 0;\n}}\n"
    );
    check_all_backends(&src, 0, &["nested-ok".to_string()], "nested");
}

#[test]
fn ident_keys_and_dynamic_strings() {
    let src = format!(
        "{PRELUDE}\nfn Main(): Int {{\n    let headers = {{ \"content-type\": \"application/json\", count: 42 }};\n    assert(headers.get(\"content-type\") == \"application/json\", \"dashed key\");\n    assert((headers.get(\"count\") ?? -1) == 42, \"ident key\");\n    assert(headers.has(\"count\"), \"has\");\n    headers.delete(\"count\");\n    assert(!headers.has(\"count\"), \"deleted\");\n    print(\"keys-ok\");\n    return 0;\n}}\n"
    );
    check_all_backends(&src, 0, &["keys-ok".to_string()], "keys");
}

#[test]
fn ident_only_braces_stay_records() {
    let src = format!(
        "{PRELUDE}\nfn Main(): Int {{\n    let user = {{ id: 1, name: \"Alice\" }};\n    assert(user.id == 1, \"record field\");\n    let {{ id, ...meta }} = user;\n    assert(id == 1, \"destructure\");\n    assert(meta.name == \"Alice\", \"rest\");\n    print(\"record-ok\");\n    return 0;\n}}\n"
    );
    check_all_backends(&src, 0, &["record-ok".to_string()], "record");
}

#[test]
fn map_literal_without_import_fails_with_hint() {
    let src = "fn Main(): Int {\n    let m = { \"a\": 1 };\n    return 0;\n}\n";
    let dir = std::env::temp_dir().join(format!("rnx-objlit-noimp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check(&m);
    assert!(c.iter().all(|x| x.code.is_warning()), "{c:?}");
    let err = lir::lower::lower(&m).expect_err("expected missing-import error");
    assert!(err.message.contains("need `import { Map }"), "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
