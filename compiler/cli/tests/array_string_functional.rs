use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-arrstr-{tag}-{}", std::process::id()));
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
    let numbers = [10, 20, 30, 40, 50];\n\
    let found = numbers.find((n) => n > 25);\n\
    assert(found != null && found == 30, \"array find match\");\n\
    let not_found = numbers.find((n) => n > 100);\n\
    assert(not_found == null, \"array find none\");\n\
    let idx = numbers.findIndex((n) => n == 40);\n\
    assert(idx == 3, \"array findIndex match\");\n\
    assert(numbers.findIndex((n) => n == 999) == -1, \"array findIndex missing\");\n\
    assert(numbers.some((n) => n == 30), \"array some true\");\n\
    assert(!numbers.some((n) => n == 99), \"array some false\");\n\
    assert(numbers.every((n) => n > 0), \"array every true\");\n\
    assert(!numbers.every((n) => n > 25), \"array every false\");\n\
    let sum = numbers.reduce(0, (acc, n) => acc + n);\n\
    assert(sum == 150, \"array reduce sum\");\n\
    let joined_str = numbers.reduce(\"\", (acc, n) => acc + (acc == \"\" ? \"\" : \",\") + n.toString());\n\
    assert(joined_str == \"10,20,30,40,50\", \"array reduce to string\");\n\
    let words = [\"hello\", \"world\", \"rasmalai\"];\n\
    assert(words.join(\" \") == \"hello world rasmalai\", \"array join with space\");\n\
    assert(words.join() == \"helloworldrasmalai\", \"array join default empty\");\n\
    let rev = numbers.reversed();\n\
    assert(rev[0] == 50 && rev[4] == 10 && rev.length() == 5, \"array reversed\");\n\
    let greeting = \"hello world from rasmalai\";\n\
    assert(greeting.contains(\"world\"), \"string contains substring\");\n\
    assert(!greeting.contains(\"rust\"), \"string does not contain\");\n\
    assert(greeting.startsWith(\"hello\"), \"string startsWith true\");\n\
    assert(!greeting.startsWith(\"world\"), \"string startsWith false\");\n\
    assert(greeting.endsWith(\"rasmalai\"), \"string endsWith true\");\n\
    assert(!greeting.endsWith(\"from\"), \"string endsWith false\");\n\
    assert(greeting.indexOf(\"world\") == 6, \"string indexOf match\");\n\
    assert(greeting.indexOf(\"world\", 10) == -1, \"string indexOf from_index miss\");\n\
    assert(greeting.indexOf(\"missing\") == -1, \"string indexOf missing\");\n\
    let parts = greeting.split(\" \");\n\
    assert(parts.length() == 4, \"string split count\");\n\
    assert(parts[0] == \"hello\", \"string split part 0\");\n\
    assert(parts[3] == \"rasmalai\", \"string split part 3\");\n\
    let csv = \"a,b,c\";\n\
    let csv_parts = csv.split(\",\");\n\
    assert(csv_parts.length() == 3 && csv_parts[1] == \"b\", \"string split comma\");\n\
    let rep = \"foo bar foo\".replace(\"foo\", \"baz\");\n\
    assert(rep == \"baz bar foo\", \"string replace first\");\n\
    let rep_all = \"foo bar foo\".replaceAll(\"foo\", \"baz\");\n\
    assert(rep_all == \"baz bar baz\", \"string replace all\");\n\
    assert(\"na \".repeat(3) == \"na na na \", \"string repeat\");\n\
    assert(\"hello\".toUpperCase() == \"HELLO\", \"string toUpperCase\");\n\
    assert(\"HeLLo\".toLowerCase() == \"hello\", \"string toLowerCase\");\n\
    return 0;\n\
}\n";

const EDGES: &str = "fn Main(): Int {\n\
    let empty: Array<Int> = [];\n\
    assert(empty.find((n) => n > 0) == null, \"empty find\");\n\
    assert(empty.findIndex((n) => n > 0) == -1, \"empty findIndex\");\n\
    assert(!empty.some((n) => true), \"empty some\");\n\
    assert(empty.every((n) => false), \"empty every\");\n\
    assert(empty.reduce(7, (acc, n) => acc + n) == 7, \"empty reduce\");\n\
    assert(empty.join(\",\") == \"\", \"empty join\");\n\
    assert(empty.reversed().length() == 0, \"empty reversed\");\n\
    assert(\"\".contains(\"\"), \"empty contains empty\");\n\
    assert(\"abc\".contains(\"\"), \"contains empty\");\n\
    assert(\"\".startsWith(\"\"), \"empty startsWith empty\");\n\
    assert(\"abc\".endsWith(\"\"), \"endsWith empty\");\n\
    assert(\"abc\".indexOf(\"\") == 0, \"indexOf empty\");\n\
    assert(\"abc\".indexOf(\"\", 2) == 2, \"indexOf empty from\");\n\
    assert(\"abc\".indexOf(\"c\", 99) == -1, \"indexOf past end\");\n\
    let chars = \"ab\".split(\"\");\n\
    assert(chars.length() == 2 && chars[0] == \"a\" && chars[1] == \"b\", \"split chars\");\n\
    let solo = \"abc\".split(\",\");\n\
    assert(solo.length() == 1 && solo[0] == \"abc\", \"split missing\");\n\
    let trail = \"a,\".split(\",\");\n\
    assert(trail.length() == 2 && trail[1] == \"\", \"split trailing\");\n\
    assert(\"aaa\".replaceAll(\"a\", \"b\") == \"bbb\", \"replaceAll overlap walk\");\n\
    assert(\"abc\".replaceAll(\"\", \"x\") == \"abc\", \"replaceAll empty target\");\n\
    assert(\"abc\".replace(\"z\", \"x\") == \"abc\", \"replace missing\");\n\
    assert(\"ab\".repeat(0) == \"\", \"repeat zero\");\n\
    assert(\"ab\".repeat(-2) == \"\", \"repeat negative\");\n\
    assert(\"\".repeat(3) == \"\", \"repeat empty\");\n\
    assert(\"\".toUpperCase() == \"\", \"toUpperCase empty\");\n\
    assert(\"\".toLowerCase() == \"\", \"toLowerCase empty\");\n\
    assert(\"HELLO 123!\".toUpperCase() == \"HELLO 123!\", \"toUpperCase already upper\");\n\
    assert(\"hello 123!\".toLowerCase() == \"hello 123!\", \"toLowerCase already lower\");\n\
    assert(\"héllo\".toUpperCase() == \"HéLLO\", \"toUpperCase non ascii passthrough\");\n\
    assert(\"HÉLLO\".toLowerCase() == \"hÉllo\", \"toLowerCase non ascii passthrough\");\n\
    assert(\"日本語\".toUpperCase() == \"日本語\", \"toUpperCase cjk untouched\");\n\
    assert(\"a\".toUpperCase().charCodeAt(0) == 65, \"toUpperCase folds a\");\n\
    assert(\"z\".toUpperCase() == \"Z\", \"toUpperCase folds z\");\n\
    assert(\"A\".toLowerCase() == \"a\", \"toLowerCase folds A\");\n\
    assert(\"Z\".toLowerCase() == \"z\", \"toLowerCase folds Z\");\n\
    assert(\"Straße 42\".toUpperCase() == \"STRAßE 42\", \"toUpperCase boundary\");\n\
    return 0;\n\
}\n";

#[test]
fn array_string_functional_all_backends() {
    check_all_backends(SCRIPT, 0, &[], "script");
}

#[test]
fn array_string_edge_cases_all_backends() {
    check_all_backends(EDGES, 0, &[], "edges");
}
