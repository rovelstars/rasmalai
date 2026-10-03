use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-anb-{tag}-{}", std::process::id()));
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

    let bin_path = dir.join("anb_bin");
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

#[test]
fn arrow_expression_body() {
    check_all_backends(
        "fn Main(): Int {\n\
        let numbers = [1, 2, 3, 4];\n\
        let doubled = numbers.map((n) => n * 2);\n\
        assert(doubled[0] == 2 && doubled[3] == 8, \"arrow expression body\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "arrow-expr",
    );
}

#[test]
fn arrow_block_body_with_capture() {
    check_all_backends(
        "fn Main(): Int {\n\
        let factor = 10;\n\
        let add_scaled = (a, b) => {\n\
        let sum = a + b;\n\
        return sum * factor;\n\
        };\n\
        assert(add_scaled(2, 3) == 50, \"arrow block body with capture\");\n\
        let done = () => \"done\";\n\
        assert(done() == \"done\", \"zero params\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "arrow-block",
    );
}

#[test]
fn nullish_coalescing() {
    check_all_backends(
        "fn Main(): Int {\n\
        let port_opt: Int? = null;\n\
        let active_port = port_opt ?? 8080;\n\
        assert(active_port == 8080, \"nullish coalescing fallback\");\n\
        let custom_port: Int? = 3000;\n\
        assert((custom_port ?? 8080) == 3000, \"nullish coalescing present\");\n\
        let zero_port: Int? = 0;\n\
        assert((zero_port ?? 8080) == 0, \"nullish keeps zero\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "nullish",
    );
}

#[test]
fn optional_chaining() {
    check_all_backends(
        "class Profile {\n\
        let name: String;\n\
        init(name: String) { this.name = name; }\n\
        }\n\
        class User {\n\
        let profile: Profile?;\n\
        init(profile: Profile?) { this.profile = profile; }\n\
        }\n\
        fn Main(): Int {\n\
        let user_with_profile = new User(new Profile(\"Alice\"));\n\
        let user_without_profile = new User(null);\n\
        let name1 = user_with_profile?.profile?.name;\n\
        assert(name1 == \"Alice\", \"optional chaining some\");\n\
        let name2 = user_without_profile?.profile?.name;\n\
        assert(name2 == null, \"optional chaining short-circuit null\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "optchain",
    );
}

#[test]
fn optional_chaining_calls() {
    check_all_backends(
        "fn Main(): Int {\n\
        let o: String? = \"bo\";\n\
        assert((o?.length() ?? -1) == 2, \"optcall method some\");\n\
        let n: String? = null;\n\
        assert(n?.length() == null, \"optcall method none\");\n\
        let m = o?.length() ?? 99;\n\
        assert(m == 2, \"optchain coalesce mix\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "optcall",
    );
}

#[test]
fn bytebuffer_endian() {
    check_all_backends(
        "import { ByteBuffer } from \"@std/bytes\";\n\
        fn Main(): Int {\n\
        let buf = ByteBuffer.allocate(32);\n\
        buf.writeUInt8(0, 0xAB);\n\
        buf.writeInt16LE(1, 0x1234);\n\
        buf.writeInt32BE(3, 0x01020304);\n\
        buf.writeFloat64LE(7, 3.1415926535);\n\
        assert(buf.readUInt8(0) == 0xAB, \"byte buffer uint8\");\n\
        assert(buf.readInt16LE(1) == 0x1234, \"byte buffer int16 le\");\n\
        assert(buf.readInt32BE(3) == 0x01020304, \"byte buffer int32 be\");\n\
        let f_val = buf.readFloat64LE(7);\n\
        assert(f_val > 3.14159 && f_val < 3.14160, \"byte buffer float64 le\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "bytes-endian",
    );
}

#[test]
fn bytebuffer_strings() {
    check_all_backends(
        "import { ByteBuffer } from \"@std/bytes\";\n\
        fn Main(): Int {\n\
        let str_buf = ByteBuffer.allocate(16);\n\
        let bytes_written = str_buf.writeString(0, \"Rasmalai\");\n\
        assert(bytes_written == 8, \"byte buffer write string len\");\n\
        assert(str_buf.readString(0, 8) == \"Rasmalai\", \"byte buffer read string\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "bytes-string",
    );
}
