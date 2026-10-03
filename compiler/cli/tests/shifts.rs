use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-shifts-{tag}-{}", std::process::id()));
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

    let bin_path = dir.join("shifts_bin");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8(run.stdout).unwrap(), want, "aot {tag} stdout");
}

#[test]
fn shift_arithmetic_logical_and_masking() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        assert(1 << 3 == 8, \"shl\");\n\
        assert((-8) >> 2 == -2, \"shr keeps sign\");\n\
        assert((-1) >>> 1 == 0x7FFFFFFFFFFFFFFF, \"zshr zero-fills\");\n\
        assert(1 << 67 == 8, \"shift amount masks to 6 bits\");\n\
        assert((-1) >>> 66 == 0x3FFFFFFFFFFFFFFF, \"zshr masks too\");\n\
        print(\"shifts ok\");\n\
        return 0;\n\
        }\n",
        "shifts ok\n",
        "arith",
    );
}

#[test]
fn shift_precedence_and_compound_assign() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        assert((1 + 1) << 2 == 8, \"additive binds tighter\");\n\
        assert(1 << 2 + 1 == 8, \"rhs additive binds tighter\");\n\
        assert(8 >> 1 < 8, \"relational binds looser\");\n\
        let x = 1;\n\
        x <<= 3;\n\
        assert(x == 8, \"shleq\");\n\
        x >>= 1;\n\
        assert(x == 4, \"shreq\");\n\
        x >>>= 1;\n\
        assert(x == 2, \"zshreq\");\n\
        let nested: Array<Array<Int>> = [[1], [2, 3]];\n\
        assert(nested[1][0] == 2, \"nested generics still parse\");\n\
        print(\"prec ok\");\n\
        return 0;\n\
        }\n",
        "prec ok\n",
        "prec",
    );
}
