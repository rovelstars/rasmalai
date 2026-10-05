use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-switchtry-{tag}-{}", std::process::id()));
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

fn run_backend(rnx: &str, dir: &PathBuf, backend: &str) -> String {
    let out = std::process::Command::new(rnx)
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{backend}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn check_stdout_everywhere(src: &str, want: &str, tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(0) => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    assert_eq!(machine.output.join("\n") + "\n", want, "interpreter {tag} stdout");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    for backend in ["cranelift", "llvm"] {
        let got = run_backend(rnx, &dir, backend);
        assert_eq!(got, want, "{backend} {tag} stdout");
    }

    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0, "aot {tag} exit");
    assert_eq!(String::from_utf8(run.stdout).unwrap(), want, "aot {tag} stdout");
    let _ = std::fs::remove_dir_all(&dir);
}

fn check_fails_with(src: &str, want: &[&str], tag: &str) {
    let dir = std::env::temp_dir().join(format!("rnx-switchtry-err-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Project.config"), "export default {\n    project: {\n        name: \"m\",\n        version: \"0.1.0\"\n    }\n}\n").unwrap();
    std::fs::write(dir.join("src/main.rnx"), src).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("build")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .arg("src/main.rnx")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "expected failure");
    let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    for w in want {
        assert!(text.contains(w), "missing `{w}` in: {text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn switch_guards_see_bindings() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let o: Result = Result.Ok(42);\n\
        switch o {\n\
            case .Ok(v) if v > 100:\n\
                print(1);\n\
                pass;\n\
            case .Ok(v):\n\
                print(2);\n\
                pass;\n\
            case .Err(e):\n\
                print(0);\n\
                pass;\n\
        }\n\
        return 0;\n\
        }\n",
        "2\n",
        "guards",
    );
}

#[test]
fn switch_custom_enum_payloads() {
    check_stdout_everywhere(
        "enum Shape {\n\
        Circle(Float),\n\
        Rect(Float, Float),\n\
        Point,\n\
        }\n\
        fn area(s: Shape): Float {\n\
            switch s {\n\
                case .Circle(r) if r > 100.0:\n\
                    return 0.0;\n\
                case .Circle(r):\n\
                    return 3.0 * r * r;\n\
                case .Rect(w, h):\n\
                    return w * h;\n\
                case .Point:\n\
                    return 0.0;\n\
            }\n\
        }\n\
        fn Main(): Int {\n\
        print(area(Shape.Circle(1.0)));\n\
        print(area(Shape.Rect(3.0, 4.0)));\n\
        print(area(Shape.Circle(200.0)));\n\
        return 0;\n\
        }\n",
        "3.0\n12.0\n0.0\n",
        "shapes",
    );
}

#[test]
fn try_null_coalesces() {
    check_stdout_everywhere(
        "fn getVal(): Int {\n\
        let o: Int? = 21;\n\
        let v = o ?? 0;\n\
        return v * 2;\n\
        }\n\
        fn getNull(): Int {\n\
        let o: Int? = null;\n\
        let v = o ?? 5;\n\
        return v * 2;\n\
        }\n\
        fn Main(): Int {\n\
        print(getVal());\n\
        print(getNull());\n\
        return 0;\n\
        }\n",
        "42\n10\n",
        "trynull",
    );
}

#[test]
fn try_result_propagates() {
    check_stdout_everywhere(
        "fn calc(x: Int): Result {\n\
        let r: Result = Result.Ok(x * 2);\n\
        let v = r?;\n\
        return Result.Ok(v + 1);\n\
        }\n\
        fn failer(): Result {\n\
        let r: Result = Result.Err(\"boom\");\n\
        let v = r?;\n\
        return Result.Ok(v);\n\
        }\n\
        fn Main(): Int {\n\
        print(calc(20).unwrap());\n\
        print(failer().isErr());\n\
        return 0;\n\
        }\n",
        "41\ntrue\n",
        "tryres",
    );
}

#[test]
fn try_pop_and_join() {
    check_stdout_everywhere(
        "fn popIt(): Int {\n\
        let arr = [10, 20];\n\
        let v = arr.pop() ?? 0;\n\
        return v + 1;\n\
        }\n\
        fn spawnIt(): Result {\n\
        let h = Thread.spawn(() => 40);\n\
        let v = h.join()?;\n\
        return Result.Ok(v + 2);\n\
        }\n\
        fn Main(): Int {\n\
        print(popIt());\n\
        print(spawnIt().unwrap());\n\
        return 0;\n\
        }\n",
        "21\n42\n",
        "trypop",
    );
}

#[test]
fn ternary_still_works() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let t = 1 > 2 ? 10 : 20;\n\
        print(t);\n\
        let u = 2 > 1 ? 1 : 2;\n\
        print(u);\n\
        return 0;\n\
        }\n",
        "20\n1\n",
        "ternary",
    );
}

#[test]
fn err_non_exhaustive_switch() {
    check_fails_with(
        "fn Main(): Int {\n\
        let o: Result = Result.Ok(42);\n\
        switch o {\n\
            case .Ok(v):\n\
                print(v);\n\
                pass;\n\
        }\n\
        return 0;\n\
        }\n",
        &["E108", "non-exhaustive", "Err"],
        "exh",
    );
}

#[test]
fn err_try_on_non_result() {
    check_fails_with(
        "fn Main(): Int {\n\
        let x = 5;\n\
        let y = x?;\n\
        return 0;\n\
        }\n",
        &["E108", "`?` needs a `Result` value"],
        "trynon",
    );
}

#[test]
fn err_try_in_int_fn() {
    check_fails_with(
        "fn Main(): Int {\n\
        let o: Result = Result.Ok(1);\n\
        let y = o?;\n\
        return 0;\n\
        }\n",
        &["E108", "enclosing function returning"],
        "tryret",
    );
}
