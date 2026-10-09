use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-join-{tag}-{}", std::process::id()));
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

const SCRIPT: &str = "fn Main(): Int {\n\
    let empty: Array<String> = [];\n\
    assert(empty.join(\",\") == \"\", \"empty explicit sep\");\n\
    assert(empty.join() == \"\", \"empty default sep\");\n\
    assert([\"only\"].join(\"SEP\") == \"only\", \"single\");\n\
    assert([\"\"].join(\",\") == \"\", \"single empty\");\n\
    assert([\"a\", \"b\", \"c\"].join(\",\") == \"a,b,c\", \"basic\");\n\
    assert([\"a\", \"b\"].join() == \"ab\", \"default sep\");\n\
    assert([\"\", \"\", \"\"].join(\"-\") == \"--\", \"all empty\");\n\
    assert([\"\", \"x\", \"\"].join(\", \") == \", x, \", \"empty edges\");\n\
    assert([\"\", \"\"].join(\"\") == \"\", \"empty pieces empty sep\");\n\
    assert([\"a:b\", \"c\"].join(\":\") == \"a:b:c\", \"sep inside element\");\n\
    assert([\"ab\", \"cd\"].join(\"LONGSEP\") == \"abLONGSEPcd\", \"sep longer than pieces\");\n\
    assert([\"hi\", \"yo\"].join(\"\\u{1F600}\") == \"hi\\u{1F600}yo\", \"astral separator\");\n\
    assert([\"h\\u{E9}llo\", \"w\\u{F6}rld\"].join(\"|\") == \"h\\u{E9}llo|w\\u{F6}rld\", \"latin multibyte\");\n\
    assert([\"\\u{65E5}\\u{672C}\\u{8A9E}\", \"\\u{30C6}\\u{30AD}\\u{30B9}\\u{30C8}\"].join(\"\\u{2192}\\u{2190}\") == \"\\u{65E5}\\u{672C}\\u{8A9E}\\u{2192}\\u{2190}\\u{30C6}\\u{30AD}\\u{30B9}\\u{30C8}\", \"cjk content arrows sep\");\n\
    assert([\"a\\u{1F600}b\", \"\", \"c\\u{1F601}d\"].join(\"-\") == \"a\\u{1F600}b--c\\u{1F601}d\", \"astral mixed empties\");\n\
    assert([[\"a\", \"b\"], [\"c\"]].join(\",\") == \"[a, b],[c]\", \"nested arrays\");\n\
    let long = \"0123456789abcdef\";\n\
    let arr: Array<String> = [];\n\
    let i = 0;\n\
    while i < 50 {\n\
        arr.push(long);\n\
        i = i + 1;\n\
    }\n\
    let j = arr.join(\"SEP\");\n\
    assert(j.length() == 50 * 16 + 49 * 3, \"long join length\");\n\
    assert(j.slice(0, 16) == long, \"long join head\");\n\
    assert(j.slice(j.length() - 16, j.length()) == long, \"long join tail\");\n\
    assert(j.slice(16, 19) == \"SEP\", \"long join first sep\");\n\
    let uni: Array<String> = [];\n\
    i = 0;\n\
    while i < 30 {\n\
        uni.push(\"\\u{65E5}\\u{672C}\\u{8A9E}\\u{1F600}\");\n\
        i = i + 1;\n\
    }\n\
    let u = uni.join(\"\\u{2192}\");\n\
    assert(u.length() == 30 * 4 + 29, \"unicode join chars\");\n\
    assert(u.slice(0, 4) == \"\\u{65E5}\\u{672C}\\u{8A9E}\\u{1F600}\", \"unicode join head\");\n\
    assert(u.slice(4, 5) == \"\\u{2192}\", \"unicode join sep\");\n\
    let many: Array<String> = [];\n\
    i = 0;\n\
    while i < 200 {\n\
        many.push(\"e\" + (\"\" + i));\n\
        i = i + 1;\n\
    }\n\
    let m = many.join(\",\");\n\
    assert(m.slice(0, 4) == \"e0,e\", \"many head\");\n\
    assert(m.slice(m.length() - 6, m.length()) == \"8,e199\", \"many tail\");\n\
    print(\"join torture OK\");\n\
    return 0;\n\
}\n";

#[test]
fn join_torture_byte_exact() {
    check_all_backends(SCRIPT, 0, &["join torture OK".to_string()], "torture");
}
