use cranelift::jit::emit_object;

static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(tag: &str) -> std::path::PathBuf {
    let n = CTR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rnx-clobj-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn lower_src(src: &str, tag: &str) -> lir::instr::Module {
    let dir = scratch(&format!("src-{tag}"));
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn cc_parts() -> (String, Option<String>) {
    let cc = ["cc", "gcc", "clang"]
        .into_iter()
        .find(|name| {
            std::env::var_os("PATH").map_or(false, |paths| {
                std::env::split_paths(&paths).any(|d| d.join(name).is_file())
            })
        })
        .expect("system cc driver required");
    let has = |name: &str| {
        std::env::var_os("PATH").map_or(false, |paths| {
            std::env::split_paths(&paths).any(|d| d.join(name).is_file())
        })
    };
    let ld = if has("mold") {
        Some("-fuse-ld=mold".to_string())
    } else if has("ld.lld") {
        Some("-fuse-ld=lld".to_string())
    } else {
        None
    };
    (cc.to_string(), ld)
}

fn link_cc(obj: &[u8], tag: &str) -> std::path::PathBuf {
    let dir = scratch(&format!("link-{tag}"));
    std::fs::write(dir.join("user.o"), obj).unwrap();
    std::fs::write(dir.join("rt.a"), runtime::archive::BYTES).unwrap();
    let exe = dir.join("prog");
    let (cc, ld) = cc_parts();
    let mut cmd = std::process::Command::new(&cc);
    if let Some(flag) = ld {
        cmd.arg(flag);
    }
    let out = cmd
        .arg(dir.join("user.o"))
        .arg(dir.join("rt.a"))
        .args(["-lpthread", "-ldl", "-lm", "-lc", "-o"])
        .arg(&exe)
        .output()
        .unwrap_or_else(|e| panic!("spawn {cc}: {e}"));
    assert!(out.status.success(), "cc link failed: {}", String::from_utf8_lossy(&out.stderr));
    exe
}

fn run_exe(exe: &std::path::Path) -> (i32, String) {
    let out = std::process::Command::new(exe).output().unwrap_or_else(|e| panic!("run {}: {e}", exe.display()));
    let code = out.status.code().unwrap_or(-1);
    (code, String::from_utf8_lossy(&out.stdout).into_owned())
}

fn cl_exe(src: &str, tag: &str, entry: &str) -> std::path::PathBuf {
    let lir = lower_src(src, tag);
    let obj = emit_object(&lir, "rnx_module", entry).unwrap_or_else(|e| panic!("{e}"));
    link_cc(&obj, tag)
}

fn llvm_exe(src: &str, tag: &str) -> std::path::PathBuf {
    let lir = lower_src(src, tag);
    let obj = llvm::codegen::emit_object(&lir, "rnx_module", "Main", llvm::codegen::OptLevel::Dev, None)
        .unwrap_or_else(|e| panic!("{e}"));
    link_cc(&obj, tag)
}

const MATH_SRC: &str = "fn Fib(n: Int): Int { let a = 0 let b = 1 let i = 0 while i < n { let t = a + b a = b b = t i += 1 } return a } fn Double(x: Int): Int { return x * 2 } fn Pick(a: Int, b: Int): Int { return (a > b ? a : b) + Double(1) } fn Main(): Int { print(Fib(20)) print(Pick(3, 9) + Double(4)) return 0 }";

const STRARR_SRC: &str = r#"fn Main(): Int { let greeting = "Hello"; let target = "World"; let msg = greeting + ", " + target + "!"; print(msg); let list = [10, 20, 30]; list.push(40); let sum = 0; for x in list { sum = sum + x; } print(sum); if msg == "Hello, World!" && sum == 100 && list.length == 4 { return 0; } return 1; }"#;

const CLOSURE_SRC: &str = "fn Main(): Int { let f = (): Int => 42; let factor = 10; let mult = (x: Int): Int => x * factor; let r = f() + mult(5); print(r); if r == 92 { return 0; } return 1; }";

#[test]
fn object_int_math_calls() {
    let (code, out) = run_exe(&cl_exe(MATH_SRC, "math", "Main"));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("6765"), "{out}");
    assert!(out.contains("19"), "{out}");
    let (lcode, lout) = run_exe(&llvm_exe(MATH_SRC, "math-llvm"));
    assert_eq!((lcode, lout), (code, out));
}

#[test]
fn object_strings_arrays() {
    let (code, out) = run_exe(&cl_exe(STRARR_SRC, "strarr", "Main"));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Hello, World!"), "{out}");
    assert!(out.contains("100"), "{out}");
    let (lcode, lout) = run_exe(&llvm_exe(STRARR_SRC, "strarr-llvm"));
    assert_eq!((lcode, lout), (code, out));
}

#[test]
fn object_closures() {
    let (code, out) = run_exe(&cl_exe(CLOSURE_SRC, "clo", "Main"));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("92"), "{out}");
    let (lcode, lout) = run_exe(&llvm_exe(CLOSURE_SRC, "clo-llvm"));
    assert_eq!((lcode, lout), (code, out));
}

#[test]
fn object_backend_parity_mixed() {
    let src = "class Point { let x: Int; let y: Int; init(x: Int, y: Int) { this.x = x; this.y = y; } fn sum(): Int { return this.x + this.y; } } fn Fib(n: Int): Int { if n < 2 { return n; } return Fib(n - 1) + Fib(n - 2); } fn Main(): Int { let pt = new Point(10, 32); let s = pt.sum() + Fib(15); print(s); let t = [1, 2, 3]; print(t.len()); let f = (x: Int): Int => x * 2; if f(21) != 42 { return 1; } print(42); return 0; }";
    let (ccode, cout) = run_exe(&cl_exe(src, "parity", "Main"));
    let (lcode, lout) = run_exe(&llvm_exe(src, "parity-llvm"));
    assert_eq!(ccode, 0, "{cout}");
    assert_eq!(lcode, 0, "{lout}");
    assert_eq!(cout, lout, "cranelift-object vs llvm-object stdout diverged");
    assert!(cout.contains("652"), "{cout}");
    assert!(cout.contains('3'), "{cout}");
    assert!(cout.contains("42"), "{cout}");
}

#[test]
fn object_lowercase_main_entry() {
    let (code, out) = run_exe(&cl_exe("fn main(): Int { print(7) return 0 }", "entrymain", "main"));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains('7'), "{out}");
}

#[test]
fn object_emit_deterministic() {
    let lir = lower_src(CLOSURE_SRC, "det");
    let a = emit_object(&lir, "rnx_module", "Main").unwrap_or_else(|e| panic!("{e}"));
    let b = emit_object(&lir, "rnx_module", "Main").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a, b, "same input must produce identical object bytes");
}

#[test]
fn object_unknown_entry_errors() {
    let lir = lower_src(CLOSURE_SRC, "err");
    let e = emit_object(&lir, "rnx_module", "Nope").unwrap_err();
    assert!(e.message.contains("unknown entry"), "{}", e.message);
}

#[test]
fn object_native_link_runs() {
    if linker::host_triple().0 != "x86_64-unknown-linux-gnu" {
        return;
    }
    let lir = lower_src(MATH_SRC, "native");
    let obj = emit_object(&lir, "rnx_module", "Main").unwrap_or_else(|e| panic!("{e}"));
    let dir = scratch("native-link");
    let exe = dir.join("prog-native");
    linker::native::native_link_static(
        &[
            linker::LinkInput::ObjectBytes(obj),
            linker::LinkInput::ArchiveBytes(runtime::archive::BYTES.to_vec()),
        ],
        &exe,
        &linker::native::StaticOpts { entry: "_start".to_string(), icf: false, target: "x86_64-unknown-linux-gnu".to_string(), strip: false },
    )
    .unwrap_or_else(|e| panic!("native link failed: {e}"));
    let (ncode, nout) = run_exe(&exe);
    let (ccode, cout) = run_exe(&cl_exe(MATH_SRC, "native-cc", "Main"));
    assert_eq!((ncode, nout), (ccode, cout));
}
