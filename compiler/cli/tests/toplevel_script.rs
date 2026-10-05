use cli::{RunOutcome, Value};
use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-toplevel-{tag}-{}", std::process::id()));
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
        Value::Int(v) if v == want => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    let got: Vec<String> = machine.output.clone();
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

fn run_binary(path: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    std::process::Command::new(rnx).arg("run").arg(path).args(extra).output().unwrap()
}

#[test]
fn clean_top_level_execution() {
    check_all_backends(
        "let a = 10;\nlet b = 20;\nprint(a + b);\n",
        0,
        &["30".to_string()],
        "clean",
    );
}

#[test]
fn top_level_await_executes() {
    check_all_backends(
        "let data = await Promise.resolve(42);\nprint(data);\n",
        0,
        &["42".to_string()],
        "await",
    );
}

#[test]
fn top_level_return_sets_exit_code() {
    check_all_backends("if true {\n    return 7;\n}\n", 7, &[], "ret7");
}

#[test]
fn fallthrough_defaults_to_zero() {
    check_all_backends("print(\"hi\");\n", 0, &["hi".to_string()], "fallthrough");
}

#[test]
fn run_propagates_top_level_exit_code() {
    let dir = std::env::temp_dir().join(format!("rnx-tl-exit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "if true {\n    return 7;\n}\n").unwrap();
    let out = run_binary(&main, &[]);
    assert_eq!(out.status.code().unwrap(), 7);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn explicit_main_keeps_parity() {
    check_all_backends(
        "fn Main(): Int {\n    print(6 * 7);\n    return 5;\n}\n",
        5,
        &["42".to_string()],
        "explicit",
    );
}

#[test]
fn e112_on_imported_print() {
    let dir = std::env::temp_dir().join(format!("rnx-tl-e112-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("helpers.rnx"), "fn helper(): Int {\n    return 1;\n}\n\nprint(\"leak\");\n").unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { helper } from \"./helpers\";\n\nfn Main(): Int {\n    print(helper());\n    return 0;\n}\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e:?}"));
    let err = g.resolve().expect_err("expected E112");
    assert_eq!(err.code.as_str(), "E112");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let check = std::process::Command::new(rnx).arg("check").arg(&main).output().unwrap();
    assert!(!check.status.success());
    assert!(String::from_utf8(check.stderr).unwrap().contains("E112"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e112_on_imported_let() {
    let dir = std::env::temp_dir().join(format!("rnx-tl-e112l-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("helpers.rnx"), "let x = 1 + 2;\n").unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import \"./helpers\";\n\nfn Main(): Int {\n    return 0;\n}\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e:?}"));
    let err = g.resolve().expect_err("expected E112");
    assert_eq!(err.code.as_str(), "E112");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn valid_declarative_import_runs() {
    let dir = std::env::temp_dir().join(format!("rnx-tl-decl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("helpers.rnx"),
        "class Foo {\n    let x: Int;\n    init(x: Int) {\n        this.x = x;\n    }\n}\n\nfn bar(): Int {\n    return 41;\n}\n\nconst MAX = 100;\n",
    )
    .unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(
        &main,
        "import { bar } from \"./helpers\";\n\nlet v = bar() + 1;\nprint(v);\n",
    )
    .unwrap();
    let out = run_binary(&main, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rnx_global_version_and_cwd() {
    let out = cli::run_source(
        "print(rnx.version);\nprint(rnx.cwd().length() > 0);\n",
        "Main",
        vec![],
    );
    match out.outcome {
        RunOutcome::Value(Value::Int(0)) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(out.output.len(), 2);
    assert_eq!(out.output[0], env!("CARGO_PKG_VERSION"));
    assert_eq!(out.output[1], "true");
}

#[test]
fn rnx_global_args_skip_script_path() {
    let dir = std::env::temp_dir().join(format!("rnx-tl-args-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "print(rnx.args.length());\nprint(rnx.args[0]);\n").unwrap();
    let out = run_binary(&main, &["--", "alpha"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "1\nalpha\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rnx_exit_terminates_with_code() {
    let dir = std::env::temp_dir().join(format!("rnx-tl-exit3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "rnx.exit(3);\n").unwrap();
    let out = run_binary(&main, &[]);
    assert_eq!(out.status.code().unwrap(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lowercase_main_check_build_run_agree() {
    let dir = std::env::temp_dir().join(format!("rnx-lowmain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "fn main(): Int {\n    print(\"lower\");\n    return 3;\n}\n").unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let check = std::process::Command::new(rnx)
        .arg("check")
        .arg(&main)
        .output()
        .unwrap();
    assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&main)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code(), Some(3));
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "lower\n");
    let _ = std::fs::remove_dir_all(&dir);
}
