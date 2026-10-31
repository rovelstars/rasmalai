#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, crate::scratch::Scratch) {
    let dir = crate::scratch::dir(&format!("mapsnap-{tag}"));
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check_with_files(&m, &frontend::modules::merged_decl_files(&g));
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
}

const MAP: &str = "import { Map } from \"@std/collections\";\n";
const HEADERS: &str = "import { Headers } from \"@std/web\";\n";

#[test]
fn string_keys_survive_map_deinit() {
    check_all_backends(
        &format!("{MAP}fn take(): Array<String> {{\n\
        let m = new Map<String, String>();\n\
        m.set(\"alpha\", \"one\");\n\
        m.set(\"beta\", \"two\");\n\
        return m.keys();\n\
        }}\n\
        fn Main(): Int {{\n\
        let ks = take();\n\
        assert(ks.length() == 2, \"len\");\n\
        assert(ks[0].len() == 5, \"element len\");\n\
        assert(ks[0] == \"alpha\" && ks[1] == \"beta\", \"element eq\");\n\
        print(ks.join(\",\"));\n\
        return 0;\n\
        }}\n"),
        0,
        &["alpha,beta".to_string()],
        "strkeys",
    );
}

#[test]
fn int_keys_are_real_ints_after_map_deinit() {
    check_all_backends(
        &format!("{MAP}fn take(): Array<Int> {{\n\
        let m = new Map<Int, String>();\n\
        m.set(7, \"seven\");\n\
        m.set(9, \"nine\");\n\
        return m.keys();\n\
        }}\n\
        fn Main(): Int {{\n\
        let ks = take();\n\
        assert(ks.length() == 2, \"len\");\n\
        assert(ks[0] == 7 && ks[1] == 9, \"element eq\");\n\
        assert(ks[0] + 1 == 8, \"element arithmetic\");\n\
        print(ks[0].toString() + \":\" + ks[1].toString());\n\
        return 0;\n\
        }}\n"),
        0,
        &["7:9".to_string()],
        "intkeys",
    );
}

#[test]
fn string_values_survive_map_deinit() {
    check_all_backends(
        &format!("{MAP}fn take(): Array<String> {{\n\
        let m = new Map<String, String>();\n\
        m.set(\"alpha\", \"one\");\n\
        m.set(\"beta\", \"two\");\n\
        return m.values();\n\
        }}\n\
        fn Main(): Int {{\n\
        let vs = take();\n\
        assert(vs.length() == 2, \"len\");\n\
        assert(vs[0] == \"one\" && vs[1] == \"two\", \"element eq\");\n\
        assert(vs[1].len() == 3, \"element len\");\n\
        print(vs.join(\",\"));\n\
        return 0;\n\
        }}\n"),
        0,
        &["one,two".to_string()],
        "strvals",
    );
}

#[test]
fn headers_snapshots_survive_headers_deinit() {
    check_all_backends(
        &format!("{HEADERS}fn names(): Array<String> {{\n\
        let h = Headers.fromPairs([[\"X-Alpha\", \"1\"], [\"X-Beta\", \"2\"]]);\n\
        return h.keys();\n\
        }}\n\
        fn values(): Array<String> {{\n\
        let h = Headers.fromPairs([[\"X-Alpha\", \"1\"], [\"X-Beta\", \"2\"]]);\n\
        return h.values();\n\
        }}\n\
        fn Main(): Int {{\n\
        let ks = names();\n\
        assert(ks.length() == 2, \"key len\");\n\
        assert(ks[0] == \"x-alpha\" && ks[1] == \"x-beta\", \"key eq\");\n\
        print(ks[0], ks[1]);\n\
        let vs = values();\n\
        assert(vs.length() == 2, \"value len\");\n\
        assert(vs[0] == \"1\" && vs[1] == \"2\", \"value eq\");\n\
        print(vs[0], vs[1]);\n\
        return 0;\n\
        }}\n"),
        0,
        &["x-alpha x-beta".to_string(), "1 2".to_string()],
        "headersnap",
    );
}

#[test]
fn boxed_values_stay_boxed_for_any_maps() {
    check_all_backends(
        &format!("{MAP}fn take(): Array<Any> {{\n\
        let m = new Map<String, Any>();\n\
        m.set(\"n\", 41);\n\
        m.set(\"s\", \"text\");\n\
        return m.values();\n\
        }}\n\
        fn Main(): Int {{\n\
        let vs = take();\n\
        assert(vs.length() == 2, \"len\");\n\
        assert((vs[0] as Int) + 1 == 42, \"boxed int\");\n\
        assert((vs[1] as String) == \"text\", \"boxed string\");\n\
        print(\"any-ok\");\n\
        return 0;\n\
        }}\n"),
        0,
        &["any-ok".to_string()],
        "anyvals",
    );
}
