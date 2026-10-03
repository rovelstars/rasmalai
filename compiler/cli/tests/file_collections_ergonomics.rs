use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-fce-{tag}-{}", std::process::id()));
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

    let bin_path = dir.join("fce_bin");
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

const PRELUDE: &str = "import fs, { File, OpenMode } from \"@std/fs\";\n\
import { ByteBuffer } from \"@std/bytes\";\n\
import { Map, Set } from \"@std/collections\";\n\
class User {\n\
let id: Int;\n\
let name: String;\n\
init(id: Int, name: String) {\n\
this.id = id;\n\
this.name = name;\n\
}\n\
}\n";

#[test]
fn chained_coalesce() {
    check_all_backends(
        "fn Main(): Int {\n\
        let a: Int? = null;\n\
        let b: Int? = null;\n\
        let c: Int? = 42;\n\
        let fallback: Int = 100;\n\
        let resolved_chain = a ?? b ?? c ?? fallback;\n\
        assert(resolved_chain == 42, \"chained coalesce middle match\");\n\
        let all_none_chain = a ?? b ?? null ?? fallback;\n\
        assert(all_none_chain == 100, \"chained coalesce fallback match\");\n\
        let zero_chain = a ?? 0 ?? fallback;\n\
        assert(zero_chain == 0, \"chained coalesce keeps zero\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "chain",
    );
}

#[test]
fn lambda_return_inference() {
    check_all_backends(
        "fn Main(): Int {\n\
        let numbers = [1, 2, 3];\n\
        let stringified = numbers.map((n) => \"val:\" + n.toString());\n\
        assert(stringified.length() == 3, \"lambda return inferred length\");\n\
        assert(stringified[0] == \"val:1\", \"lambda return inferred elem 0\");\n\
        assert(stringified[2] == \"val:3\", \"lambda return inferred elem 2\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "infer",
    );
}

#[test]
fn generic_map_set() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let user_map = new Map<String, User>();\n\
        user_map.set(\"admin\", new User(1, \"Alice\"));\n\
        assert(user_map.has(\"admin\"), \"map has key\");\n\
        assert(!user_map.has(\"guest\"), \"map missing key\");\n\
        let admin_user = user_map.get(\"admin\") ?? new User(0, \"Default\");\n\
        assert(admin_user.name == \"Alice\", \"map get value content\");\n\
        let missing = user_map.get(\"guest\") ?? new User(0, \"Default\");\n\
        assert(missing.id == 0 && missing.name == \"Default\", \"map get with ??\");\n\
        let id_set = new Set<Int>();\n\
        id_set.add(10);\n\
        id_set.add(20);\n\
        id_set.add(10);\n\
        assert(id_set.len() == 2, \"generic set deduplication\");\n\
        assert(id_set.has(20), \"generic set contains\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "mapset",
    );
}

#[test]
fn file_binary_io() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let test_path = \"temp_binary_test.bin\";\n\
        defer fs.remove(test_path);\n\
        let out_file = File.open(test_path, OpenMode.Write);\n\
        assert(out_file.isOpen, \"file opened for writing\");\n\
        let out_buf = ByteBuffer.allocate(16);\n\
        out_buf.writeInt32BE(0, 0x12345678);\n\
        out_buf.writeString(4, \"Rasmalai\");\n\
        let written = out_file.writeBytes(out_buf, 0, 12).unwrap();\n\
        assert(written == 12, \"exact binary bytes written\");\n\
        out_file.close();\n\
        let in_file = File.open(test_path, OpenMode.Read);\n\
        assert(in_file.isOpen, \"file opened for reading\");\n\
        let in_buf = ByteBuffer.allocate(16);\n\
        let bytes_read = in_file.readBytes(in_buf, 0, 12).unwrap();\n\
        assert(bytes_read == 12, \"exact binary bytes read\");\n\
        assert(in_buf.readInt32BE(0) == 0x12345678, \"binary read matches int32\");\n\
        assert(in_buf.readString(4, 8) == \"Rasmalai\", \"binary read matches string\");\n\
        in_file.close();\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "fileio",
    );
}
