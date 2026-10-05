use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-strarr-{tag}-{}", std::process::id()));
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
    let text = \"  hello world  \";\n\
    assert(text.trim() == \"hello world\", \"trim\");\n\
    assert(text.trim().slice(0, 5) == \"hello\", \"slice\");\n\
    assert(text.trim().indexOf(\"world\") == 6, \"index\");\n\
    assert(\"abc\".concat(\"def\") == \"abcdef\", \"concat\");\n\
    assert(\"abc\".charCodeAt(1) == 98, \"char\");\n\
    assert(\"hello\".length() == 5, \"len call\");\n\
    assert(\"hello\".length == 5, \"len prop\");\n\
    let arr = [1, 2, 3];\n\
    arr.push(4);\n\
    assert(arr.length() == 4, \"push len\");\n\
    assert(arr.length == 4, \"len prop\");\n\
    assert(arr.pop() == 4, \"pop\");\n\
    assert(arr.length() == 3, \"pop len\");\n\
    return 0;\n\
}\n";

#[test]
fn string_array_methods_all_backends() {
    check_all_backends(SCRIPT, 0, &[], "script");
}

#[test]
fn string_methods_print_output() {
    check_all_backends(
        "fn Main(): Int {\nprint(\"  hi  \".trim());\nprint(\"abcdef\".slice(2, 5));\nprint([].isEmpty());\nprint([1].len());\nreturn 0;\n}\n",
        0,
        &["hi".to_string(), "cde".to_string(), "true".to_string(), "1".to_string()],
        "print",
    );
}


const SPLIT_LINEAR_SRC: &str = "import { Clock } from \"@std/time\";\nfn timeSplit(s: String, delim: String): Int {\n    let t0 = Clock.mono().toMillis();\n    let parts = s.split(delim);\n    let ms = Clock.mono().toMillis() - t0;\n    return parts.length() * 1000000 + ms;\n}\nfn Main(): Int {\n    let line = \"abcdefghij\\n\";\n    let small = line.repeat(8000);\n    let big = line.repeat(32000);\n    let r0 = timeSplit(small, \"\\n\");\n    assert(r0 / 1000000 == 8001, \"small split count\");\n    let r1 = timeSplit(big, \"\\n\");\n    assert(r1 / 1000000 == 32001, \"big split count\");\n    let ms_small = r0 % 1000000;\n    let ms_big = r1 % 1000000;\n    assert(ms_big <= ms_small * 10 + 3000, \"8x input must not take 10x the small time\");\n    let uni = \"h\u{e9}llo\u{25c6}world\".split(\"\u{25c6}\");\n    assert(uni.length() == 2 && uni[0] == \"h\u{e9}llo\", \"multibyte split\");\n    let chars = \"hey\".split(\"\");\n    assert(chars.length() == 3 && chars[1] == \"e\", \"empty delimiter splits chars\");\n    print(\"split linear ok\");\n    return 0;\n}\n";

#[test]
fn string_split_scales_linearly() {
    check_all_backends(SPLIT_LINEAR_SRC, 0, &["split linear ok".to_string()], "splitlinear");
}
