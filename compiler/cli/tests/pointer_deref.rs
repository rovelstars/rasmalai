use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-ptrderef-{tag}-{}", std::process::id()));
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

fn lower_err(src: &str, tag: &str) -> diagnostics::Diagnostic {
    let dir = std::env::temp_dir().join(format!("rnx-ptrderef-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let err = lir::lower::lower(&m).expect_err("expected lowering error");
    let _ = std::fs::remove_dir_all(&dir);
    err
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

    let bin_path = dir.join("ptrderef_bin");
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
    assert_eq!(run.status.code().unwrap(), want as i32, "aot {tag} exit");
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const PRELUDE: &str = "import { ByteBuffer } from \"@std/bytes\";\n";

#[test]
fn raw_read_write_volatile() {
    let src = format!(
        "{PRELUDE}\nfn Main(): Int {{\n\
        let buf = ByteBuffer.allocate(32);\n\
        unsafe {{\n\
        let ptr: Pointer<Int> = Pointer.fromAddress<Int>(buf.address());\n\
        ptr.write(42);\n\
        assert(ptr.read() == 42, \"rw\");\n\
        ptr.writeVolatile(999);\n\
        assert(ptr.readVolatile() == 999, \"vol\");\n\
        assert(buf.readInt32LE(0) == 999, \"visible\");\n\
        print(\"raw-ok\");\n\
        }}\n\
        return 0;\n\
        }}\n"
    );
    check_all_backends(&src, 0, &["raw-ok".to_string()], "raw");
}

#[test]
fn float_bool_pointees() {
    let src = format!(
        "{PRELUDE}\nfn Main(): Int {{\n\
        let buf = ByteBuffer.allocate(32);\n\
        unsafe {{\n\
        let fptr: Pointer<Float> = Pointer.fromAddress<Float>(buf.address());\n\
        fptr.write(1.5);\n\
        assert(fptr.read() == 1.5, \"float\");\n\
        let bptr: Pointer<Bool> = Pointer.fromAddress<Bool>(buf.address());\n\
        bptr.write(true);\n\
        assert(bptr.read() == true, \"bool\");\n\
        bptr.write(false);\n\
        assert(bptr.read() == false, \"bool2\");\n\
        print(\"scalar-ok\");\n\
        }}\n\
        return 0;\n\
        }}\n"
    );
    check_all_backends(&src, 0, &["scalar-ok".to_string()], "scalar");
}

#[test]
fn deref_sugar_and_arithmetic() {
    let src = format!(
        "{PRELUDE}\nfn Main(): Int {{\n\
        let buf = ByteBuffer.allocate(32);\n\
        unsafe {{\n\
        let ptr: Pointer<Int> = Pointer.fromAddress<Int>(buf.address());\n\
        *ptr = 42;\n\
        assert(*ptr == 42, \"sugar\");\n\
        *(ptr + 8) = 7;\n\
        assert(*(ptr + 8) == 7, \"arith\");\n\
        assert(buf.readInt32LE(8) == 7, \"visible\");\n\
        print(\"sugar-ok\");\n\
        }}\n\
        return 0;\n\
        }}\n"
    );
    check_all_backends(&src, 0, &["sugar-ok".to_string()], "sugar");
}

#[test]
fn read_outside_unsafe_is_e202() {
    let err = lower_err(
        "fn Main(): Int {\n\
        let p: Pointer<Int>;\n\
        unsafe {\n\
        p = Pointer.fromAddress<Int>(8);\n\
        }\n\
        let v = p.read();\n\
        return 0;\n\
        }\n",
        "e202read",
    );
    assert_eq!(err.code, diagnostics::Code::E202, "{err:?}");
    assert!(err.message.contains("unsafe"), "{err:?}");
}

#[test]
fn write_outside_unsafe_is_e202() {
    let err = lower_err(
        "fn Main(): Int {\n\
        let p: Pointer<Int>;\n\
        unsafe {\n\
        p = Pointer.fromAddress<Int>(8);\n\
        }\n\
        p.write(1);\n\
        return 0;\n\
        }\n",
        "e202write",
    );
    assert_eq!(err.code, diagnostics::Code::E202, "{err:?}");
}

#[test]
fn from_address_outside_unsafe_is_e202() {
    let err = lower_err(
        "fn Main(): Int {\n\
        let ptr = Pointer.fromAddress<Int>(8);\n\
        return 0;\n\
        }\n",
        "e202from",
    );
    assert_eq!(err.code, diagnostics::Code::E202, "{err:?}");
}

#[test]
fn untyped_pointer_read_needs_annotation() {
    let err = lower_err(
        "fn Main(): Int {\n\
        unsafe {\n\
        let ptr = Pointer.fromAddress(8);\n\
        let v = ptr.read();\n\
        }\n\
        return 0;\n\
        }\n",
        "untyped",
    );
    assert_eq!(err.code, diagnostics::Code::E108, "{err:?}");
    assert!(err.message.contains("Pointer<Int>"), "{err:?}");
}

#[test]
fn stack_token_read_fails_cleanly() {
    let (module, _dir) = resolve_src(
        "fn Main(): Int {\n\
        let x: Int = 5;\n\
        unsafe {\n\
        let p: Pointer<Int> = &x;\n\
        let v = p.read();\n\
        }\n\
        return 0;\n\
        }\n",
        "token",
    );
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]);
    match r {
        Err(runtime::machine::ExecError::Fatal(m)) => assert!(m.contains("stack-local token"), "{m}"),
        other => panic!("expected clean stack-token failure, got {other:?}"),
    }
}

#[test]
fn address_of_local_rejected_on_jit() {
    let (module, _dir) = resolve_src(
        "fn Main(): Int {\n\
        let x: Int = 5;\n\
        unsafe {\n\
        let p = &x;\n\
        print(p);\n\
        }\n\
        return 0;\n\
        }\n",
        "addrof",
    );
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let r = cranelift::jit::Jit::compile(leaked);
    match r {
        Err(e) => assert!(e.message.contains("stack local"), "{e}"),
        Ok(_) => panic!("expected jit rejection of address-of-local"),
    }
}
