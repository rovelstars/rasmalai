use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-opit-{tag}-{}", std::process::id()));
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

const PRELUDE: &str = "import { Map, Set } from \"@std/collections\";\n\
struct Vec2 {\n\
let x: Float;\n\
let y: Float;\n\
}\n\
extension Vec2 {\n\
fn op_add(other: Vec2): Vec2 {\n\
return Vec2(this.x + other.x, this.y + other.y);\n\
}\n\
fn op_sub(other: Vec2): Vec2 {\n\
return Vec2(this.x - other.x, this.y - other.y);\n\
}\n\
fn op_index(idx: Int): Float {\n\
return idx == 0 ? this.x : this.y;\n\
}\n\
}\n\
class CounterIterator : Iterator<Int> {\n\
let current: Int;\n\
let max: Int;\n\
init(max: Int) {\n\
this.current = 0;\n\
this.max = max;\n\
}\n\
fn next(): Int? {\n\
if (this.current >= this.max) {\n\
return null;\n\
}\n\
let val = this.current;\n\
this.current = this.current + 1;\n\
return val;\n\
}\n\
}\n\
class Counter : Iterable<Int> {\n\
let max: Int;\n\
init(max: Int) { this.max = max; }\n\
fn iterator(): Iterator<Int> {\n\
return new CounterIterator(this.max);\n\
}\n\
}\n";

#[test]
fn operator_overloading() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let v1 = Vec2(1.5, 2.5);\n\
        let v2 = Vec2(3.5, 4.5);\n\
        let v3 = v1 + v2;\n\
        assert(v3.x == 5.0 && v3.y == 7.0, \"operator + on struct\");\n\
        let v4 = v2 - v1;\n\
        assert(v4.x == 2.0 && v4.y == 2.0, \"operator - on struct\");\n\
        assert(v1[0] == 1.5 && v1[1] == 2.5, \"operator [] indexing\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "oparith",
    );
}

#[test]
fn custom_iterable_loop() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let counter = new Counter(3);\n\
        let count_sum = 0;\n\
        for (n in counter) {{\n\
        count_sum = count_sum + n;\n\
        }}\n\
        assert(count_sum == 3, \"custom iterable for..in sum\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "iter",
    );
}

#[test]
fn collection_iteration() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let map = new Map<String, Int>();\n\
        map.set(\"alpha\", 1);\n\
        map.set(\"beta\", 2);\n\
        let key_count = 0;\n\
        for (k in map) {{\n\
        key_count = key_count + 1;\n\
        }}\n\
        assert(key_count == 2, \"map for..in keys\");\n\
        let set = new Set<Int>();\n\
        set.add(10);\n\
        set.add(20);\n\
        let set_sum = 0;\n\
        for (val in set) {{\n\
        set_sum = set_sum + val;\n\
        }}\n\
        assert(set_sum == 30, \"set for..in values\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "coliter",
    );
}
