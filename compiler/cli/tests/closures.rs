use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-clo-{tag}-{}", std::process::id()));
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

#[test]
fn closure_pure_call() {
    check_all_backends(
        "fn Main(): Int {\nlet f = (): Int => 42;\nassert(f() == 42, \"pure closure\");\nreturn 0;\n}\n",
        0,
        &[],
        "pure",
    );
}

#[test]
fn closure_captures_env() {
    check_all_backends(
        "fn Main(): Int {\nlet factor = 10;\nlet mult = (x: Int): Int => x * factor;\nassert(mult(5) == 50, \"captured env\");\nreturn 0;\n}\n",
        0,
        &[],
        "capture",
    );
}

#[test]
fn closure_captures_string() {
    check_all_backends(
        "fn Main(): Int {\nlet prefix = \"ab\";\nlet wrap = (x: Int): Int => x + prefix.length();\nassert(wrap(1) == 3, \"string capture\");\nreturn 0;\n}\n",
        0,
        &[],
        "strcap",
    );
}

#[test]
fn array_map_end_to_end() {
    check_all_backends(
        "fn Main(): Int {\n\
        let nums = [1, 2, 3, 4, 5];\n\
        let doubled = nums.map((x: Int) => x * 2);\n\
        assert(doubled.length() == 5, \"map length\");\n\
        assert(doubled[0] == 2 && doubled[4] == 10, \"map elements\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "map",
    );
}

#[test]
fn array_filter_end_to_end() {
    check_all_backends(
        "fn Main(): Int {\n\
        let nums = [1, 2, 3, 4, 5];\n\
        let evens = nums.filter((x: Int) => x % 2 == 0);\n\
        assert(evens.length() == 2, \"filter length\");\n\
        assert(evens[0] == 2 && evens[1] == 4, \"filter elements\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "filter",
    );
}

#[test]
fn array_map_stored_closure() {
    check_all_backends(
        "fn Main(): Int {\n\
        let nums = [1, 2, 3];\n\
        let triple = (x: Int) => x * 3;\n\
        let out = nums.map(triple);\n\
        assert(out.length() == 3, \"stored map\");\n\
        assert(out[2] == 9, \"stored map elem\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "stored",
    );
}
