use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-ergo-{tag}-{}", std::process::id()));
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
    let dir = std::env::temp_dir().join(format!("rnx-ergo-err-{tag}-{}", std::process::id()));
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
fn ergo_full_verification() {
    check_stdout_everywhere(
        "struct User {\n\
        let name: String;\n\
        let age: Int;\n\
        }\n\
        fn Main(): Int {\n\
        let code = 200;\n\
        let status = switch code {\n\
            case 200: \"OK\",\n\
            case 404: \"Not Found\",\n\
            default: \"Error\",\n\
        };\n\
        assert(status == \"OK\", \"switch expr\");\n\
        let count = 42;\n\
        let interp = \"value is ${count}\";\n\
        assert(interp == \"value is 42\", \"string interpolation\");\n\
        let opt_str: String? = \"rasmalai\";\n\
        print(opt_str ?? \"fallback\");\n\
        let pair = (100, \"items\");\n\
        let (total, unit) = pair;\n\
        assert(total == 100 && unit == \"items\", \"tuple destructure\");\n\
        let u = User(\"Alice\", 30);\n\
        let { name, age: userAge } = u;\n\
        assert(name == \"Alice\" && userAge == 30, \"object destructure\");\n\
        return 0;\n\
        }\n",
        "rasmalai\n",
        "full",
    );
}

#[test]
fn ergo_switch_expr_variants() {
    check_stdout_everywhere(
        "enum Status {\n\
        Active(Int),\n\
        Inactive,\n\
        }\n\
        fn Main(): Int {\n\
        let s: Status = Status.Active(7);\n\
        print(switch s {\n\
            case .Active(id): id,\n\
            case .Inactive: 0,\n\
        });\n\
        let t = switch 1 {\n\
            case 1: \"one\",\n\
            default: \"many\",\n\
        };\n\
        print(t);\n\
        return 0;\n\
        }\n",
        "7\none\n",
        "swexpr",
    );
}

#[test]
fn ergo_tuple_index_and_annotate() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let p: (Int, String) = (1, \"a\");\n\
        print(p.0);\n\
        print(p.1);\n\
        let q = p;\n\
        let (x, y) = q;\n\
        print(x + 1);\n\
        print(y);\n\
        return 0;\n\
        }\n",
        "1\na\n2\na\n",
        "tuples",
    );
}

#[test]
fn err_tuple_index_oob() {
    check_fails_with(
        "fn Main(): Int {\n\
        let p = (1, 2);\n\
        print(p.5);\n\
        return 0;\n\
        }\n",
        &["E108", "out of bounds"],
        "oob",
    );
}

#[test]
fn err_destructure_non_tuple() {
    check_fails_with(
        "fn Main(): Int {\n\
        let x = 5;\n\
        let (a, b) = x;\n\
        return 0;\n\
        }\n",
        &["E108", "non-tuple"],
        "notup",
    );
}

#[test]
fn err_destructure_missing_field() {
    check_fails_with(
        "struct User {\n\
        let name: String;\n\
        }\n\
        fn Main(): Int {\n\
        let u = User(\"Al\");\n\
        let { name, age } = u;\n\
        return 0;\n\
        }\n",
        &["E108", "unknown field"],
        "nofield",
    );
}

#[test]
fn err_destructure_duplicate() {
    check_fails_with(
        "fn Main(): Int {\n\
        let p = (1, 2);\n\
        let (a, a) = p;\n\
        return 0;\n\
        }\n",
        &["E108", "duplicate binding"],
        "dup",
    );
}

#[test]
fn err_switch_expr_mismatch() {
    check_fails_with(
        "fn Main(): Int {\n\
        let status = switch 1 {\n\
            case 1: \"one\",\n\
            default: 2,\n\
        };\n\
        return 0;\n\
        }\n",
        &["E108", "arms yield"],
        "mismatch",
    );
}

#[test]
fn err_switch_expr_non_exhaustive() {
    check_fails_with(
        "fn Main(): Int {\n\
        let status = switch 1 {\n\
            case 1: \"one\",\n\
        };\n\
        return 0;\n\
        }\n",
        &["E108", "exhaustive"],
        "exh",
    );
}

#[test]
fn tuple_across_fn_now_allowed() {
    check_stdout_everywhere(
        "fn div_rem(num: Int, den: Int): (Int, Int) {\n\
        return (num / den, num % den);\n\
        }\n\
        fn Main(): Int {\n\
        let (q, r) = div_rem(10, 3);\n\
        print(q);\n\
        print(r);\n\
        return 0;\n\
        }\n",
        "3\n1\n",
        "fnbound",
    );
}
