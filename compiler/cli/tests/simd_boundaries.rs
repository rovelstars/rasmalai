use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-bound-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check(&m);
    let errors: Vec<_> = c.iter().filter(|d| !d.code.is_warning()).collect();
    assert!(errors.is_empty(), "{errors:?}");
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
    let _ = std::fs::remove_dir_all(&dir);
}

const PARAM_SRC: &str = "import { Vec4f } from \"@std/simd\";\n\nfn dbl(v: Vec4f): Vec4f {\n    return v * Vec4f.splat(2.0);\n}\n\nfn Main(): Int {\n    let got = Int(dbl(new Vec4f(1.0, 2.0, 3.0, 4.0)).get(2));\n    print(\"param:\", got);\n    if got == 6 {\n        return 42;\n    }\n    return 1;\n}\n";

const RETURN_SRC: &str = "import { Vec4f } from \"@std/simd\";\n\nfn mk(a: Float): Vec4f {\n    return new Vec4f(a, a, a, a);\n}\n\nfn Main(): Int {\n    let got = Int(mk(3.0).get(1));\n    print(\"return:\", got);\n    if got == 3 {\n        return 42;\n    }\n    return 1;\n}\n";

const SQRT_SRC: &str = "import { Vec4f } from \"@std/simd\";\n\nfn Main(): Int {\n    let r = new Vec4f(4.0, 9.0, 16.0, 25.0).sqrt();\n    let a = Int(r.x());\n    let b = Int(r.y());\n    let c = Int(r.get(2));\n    let d = Int(r.w());\n    print(\"sqrt:\", a, b, c, d);\n    if a == 2 && b == 3 && c == 4 && d == 5 {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_vec4f_argument_crosses_function_boundary() {
    check_all_backends(PARAM_SRC, 42, &["param: 6".to_string()], "vec-param");
}

#[test]
fn test_vec4f_return_crosses_function_boundary() {
    check_all_backends(RETURN_SRC, 42, &["return: 3".to_string()], "vec-return");
}

#[test]
fn test_vec4f_sqrt_lanes() {
    check_all_backends(SQRT_SRC, 42, &["sqrt: 2 3 4 5".to_string()], "vec-sqrt");
}
