use std::path::{Path, PathBuf};
use std::process::Command;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-p1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
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

    let bin_path = dir.join("p1_bin");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    match run.status.code() {
        Some(code) => assert_eq!(code, want as i32, "aot {tag} exit"),
        None => panic!("aot {tag} killed by signal: {run:?}"),
    }
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const ANY_CHURN_SRC: &str = "import { Map } from \"@std/collections\";\nfn hammer(m: Map, n: Int): Int {\n    let i = 0;\n    while i < n {\n        let v: Any = i;\n        m.set(1, v);\n        let g = m.get(1) ?? -1;\n        i = i + 1;\n    }\n    return 0;\n}\nfn Main(): Int {\n    let m = new Map();\n    m.set(1, 0);\n    let h0 = Thread.spawn((): Int => hammer(m, 1500));\n    let h1 = Thread.spawn((): Int => hammer(m, 1500));\n    let h2 = Thread.spawn((): Int => hammer(m, 1500));\n    let h3 = Thread.spawn((): Int => hammer(m, 1500));\n    let r = h0.join().unwrap() + h1.join().unwrap() + h2.join().unwrap() + h3.join().unwrap();\n    if r != 0 {\n        return 3;\n    }\n    if m.len() != 1 {\n        return 1;\n    }\n    print(\"any box churn ok\");\n    return 42;\n}\n";

#[test]
fn test_shared_any_box_churn_across_threads() {
    check_all_backends(ANY_CHURN_SRC, 42, &["any box churn ok".to_string()], "anychurn");
}

#[test]
fn test_box_refcount_hammer_at_native_layer() {
    for _ in 0..20 {
        let boxed = unsafe { runtime::native::rnx_any_box(runtime::native::TAG_INT, 7) };
        let mut handles = Vec::new();
        for _ in 0..8 {
            handles.push(std::thread::spawn(move || {
                for _ in 0..20000 {
                    unsafe {
                        runtime::native::rnx_any_retain(boxed);
                        runtime::native::rnx_any_release(boxed);
                    }
                }
            }));
        }
        for h in handles {
            h.join().expect("worker panicked");
        }
        unsafe { runtime::native::rnx_any_release(boxed) };
    }
}

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-p1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_package(dir: &Path, name: &str, files: &[(&str, &str)]) {
    std::fs::write(
        dir.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}\n}}\n"),
    )
    .unwrap();
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for (file, contents) in files {
        std::fs::write(src.join(file), contents).unwrap();
    }
}

#[test]
fn test_w201_round_trips_and_explains() {
    let parsed = diagnostics::Code::from_str("W201");
    assert_eq!(parsed, Some(diagnostics::Code::W201));
    let c = parsed.unwrap();
    assert_eq!(c.as_str(), "W201");
    assert!(c.is_warning());
    assert!(!c.title().is_empty());
    let fix = cli::explain("W201").expect("W201 has no explain fix");
    assert!(!fix.1.is_empty(), "W201 has empty fix");

    let out = Command::new(rnx())
        .arg("explain")
        .arg("W201")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).into_owned()
        + &String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(text.contains("W201"), "{text}");
    assert!(text.contains("unrecognized manifest"), "{text}");
}

#[test]
fn test_manifest_typo_emits_w201_warning() {
    let dir = fresh_dir("w201");
    write_package(&dir, "w201pkg", &[("main.rnx", "fn Main(): Int {\n    return 0;\n}\n")]);
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"w201pkg\",\n        version: \"0.1.0\",\n        bogus: 1\n    }\n}\n",
    )
    .unwrap();
    let out = Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("W201"), "{stderr}");
    assert!(stderr.contains("project.bogus"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
