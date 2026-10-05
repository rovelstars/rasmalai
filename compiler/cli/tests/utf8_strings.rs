use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-utf8-{tag}-{}", std::process::id()));
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
fn lengths_equality_concat() {
    check_all_backends(
        "fn Main(): Int {\n\
        let cafe = \"café\";\n\
        assert(cafe.length() == 4, \"accented length\");\n\
        assert(cafe == \"café\", \"accented equality\");\n\
        assert(\"日本語\".length() == 3, \"cjk length\");\n\
        assert(\"🚀\".length() == 1, \"emoji length\");\n\
        assert(\"\".length() == 0, \"empty length\");\n\
        assert((\"caf\" + \"é\") == \"café\", \"concat accent\");\n\
        assert((\"🚀\" + \"✨\").length() == 2, \"concat emoji length\");\n\
        assert(cafe.charCodeAt(3) == 233, \"accent scalar\");\n\
        assert(\"🚀\".charCodeAt(0) == 128640, \"emoji scalar\");\n\
        print(cafe);\n\
        return 0;\n\
        }\n",
        0,
        &["café".to_string()],
        "lengths",
    );
}

#[test]
fn byte_buffer_from_string_sizes_by_utf8_bytes() {
    check_all_backends(
        "import { ByteBuffer } from \"@std/bytes\";\n\
        fn Main(): Int {\n\
        let ascii = ByteBuffer.fromString(\"hi\");\n\
        assert(ascii.length() == 2, \"ascii byte count\");\n\
        assert(ascii.readUInt8(0) == 104, \"ascii first byte\");\n\
        assert(ascii.readUInt8(1) == 105, \"ascii second byte\");\n\
        assert(ascii.readString(0, 2) == \"hi\", \"ascii round trip\");\n\
        let accent = ByteBuffer.fromString(\"héllo\");\n\
        assert(accent.length() == 6, \"accent byte count\");\n\
        assert(accent.readString(0, accent.length()) == \"héllo\", \"accent round trip\");\n\
        assert(ByteBuffer.fromString(\"\").length() == 0, \"empty stays empty\");\n\
        let cjk = ByteBuffer.fromString(\"日本語\");\n\
        assert(cjk.length() == 9, \"cjk byte count\");\n\
        assert(cjk.readString(0, cjk.length()) == \"日本語\", \"cjk round trip\");\n\
        let emoji = ByteBuffer.fromString(\"🚀\");\n\
        assert(emoji.length() == 4, \"emoji byte count\");\n\
        assert(emoji.readString(0, 4) == \"🚀\", \"emoji round trip\");\n\
        print(accent.readString(0, accent.length()));\n\
        return 0;\n\
        }\n",
        0,
        &["héllo".to_string()],
        "from-string",
    );
}
