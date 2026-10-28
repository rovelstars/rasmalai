#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use runtime::comptime::InterpComptime;
use runtime::machine::Machine;
use runtime::value::Value;

fn tree(tag: &str, files: &[(&str, &str)]) -> crate::scratch::Scratch {
    let dir = crate::scratch::dir(&format!("rt-macro-{tag}"));
    for (name, src) in files {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, src).unwrap();
    }
    dir
}

fn expanded(dir: &crate::scratch::Scratch, entry: &str) -> frontend::ast::Module {
    let graph = frontend::modules::ModuleGraph::build(&dir.join(entry)).unwrap_or_else(|e| panic!("{e}"));
    let mut m = graph.resolve().unwrap_or_else(|e| panic!("{e}"));
    let mut opts = frontend::macros::MacroOptions::new(dir.to_path_buf());
    let a = frontend::macros::expand_phase_a(&mut m, &mut opts);
    assert!(a.iter().all(|d| d.code.is_warning()), "{a:?}");
    let runner = InterpComptime::build(&m).unwrap_or_else(|e| panic!("{e}"));
    let b = frontend::macros::expand_phase_b(
        &mut m,
        &mut opts,
        runner.as_ref().map(|r| r as &dyn frontend::macros::ComptimeRunner),
    );
    let hard: Vec<_> = b.into_iter().filter(|d| !d.code.is_warning()).collect();
    assert!(hard.is_empty(), "{hard:?}");
    frontend::desugar::desugar(&mut m);
    m
}

fn run(dir: &crate::scratch::Scratch, entry: &str, func: &str) -> (Result<Value, String>, Vec<String>) {
    let m = expanded(dir, entry);
    let lowered = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    assert!(lir::verify::verify(&lowered).is_empty());
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(lowered));
    let mut machine = Machine::new(leaked);
    let out = match machine.call(func, vec![]) {
        Ok(v) => Ok(v),
        Err(e) => Err(format!("{e:?}")),
    };
    (out, machine.output.clone())
}

fn ok(dir: &crate::scratch::Scratch, entry: &str, func: &str) -> Value {
    match run(dir, entry, func) {
        (Ok(v), _) => v,
        (Err(e), _) => panic!("{func}: {e}"),
    }
}

fn expand_errs(dir: &crate::scratch::Scratch, entry: &str) -> Vec<diagnostics::Diagnostic> {
    let graph = frontend::modules::ModuleGraph::build(&dir.join(entry)).unwrap_or_else(|e| panic!("{e}"));
    let mut m = graph.resolve().unwrap_or_else(|e| panic!("{e}"));
    let mut opts = frontend::macros::MacroOptions::new(dir.to_path_buf());
    let mut all = frontend::macros::expand_phase_a(&mut m, &mut opts);
    let runner = InterpComptime::build(&m).unwrap_or_else(|e| panic!("{e}"));
    all.extend(frontend::macros::expand_phase_b(
        &mut m,
        &mut opts,
        runner.as_ref().map(|r| r as &dyn frontend::macros::ComptimeRunner),
    ));
    all.into_iter().filter(|d| !d.code.is_warning()).collect()
}

#[test]
fn builtin_derive_clone_debug_run() {
    let dir = tree(
        "clone",
        &[(
            "main.rnx",
            "#[Derive(Clone, Debug)] struct Pair { let a: Int; let b: Int }\nfn Main(): Int {\n  let p = Pair(10, 20);\n  let q = p.clone();\n  if q.a + q.b != 30 { return 0; }\n  if p.debug() == \"Pair(a=10, b=20)\" { return 1; }\n  return 0;\n}\n",
        )],
    );
    assert_eq!(ok(&dir, "main.rnx", "Main"), Value::Int(1));
}

#[test]
fn class_derive_clone_runs() {
    let dir = tree(
        "classclone",
        &[(
            "main.rnx",
            "#[Derive(Clone, Debug)] class Box { let v: Int; init(v: Int) { this.v = v } }\nfn Main(): Int {\n  let b = new Box(7);\n  let c = b.clone();\n  if c.v != 7 { return 0; }\n  if b.debug() == \"Box(v=7)\" { return 1; }\n  return 0;\n}\n",
        )],
    );
    assert_eq!(ok(&dir, "main.rnx", "Main"), Value::Int(1));
}

const DESCRIBE_RNX: &str = "#[CompTime]\nfn Describe(info: Array<String>): Array<String> {\n  let name = info[0];\n  return [\"fn label(): String \\{ return \\\"\" + name + \"\\\"; }\"];\n}\n";

#[test]
fn user_derive_in_fixture_package_compiles_and_runs() {
    let dir = tree(
        "userderive",
        &[
            ("derive.rnx", DESCRIBE_RNX),
            (
                "main.rnx",
                "import { Describe } from \"./derive\";\n#[Derive(Describe)] struct Point { let x: Int; let y: Int }\nfn Main(): Int {\n  let p = Point(1, 2);\n  if p.label() == \"Point\" { return 1; }\n  return 0;\n}\n",
            ),
        ],
    );
    assert_eq!(ok(&dir, "main.rnx", "Main"), Value::Int(1));
}

#[test]
fn comptime_lut_value_correct_at_runtime() {
    let dir = tree(
        "lut",
        &[(
            "main.rnx",
            "#[CompTime]\nfn Table(): Array<Int> {\n  let t: Array<Int> = vec![10, 20, 30];\n  return t;\n}\nfn Main(): Int { static LUT: Array<Int> = CompTime!(Table()); return LUT[0] + LUT[1] + LUT[2]; }\n",
        )],
    );
    assert_eq!(ok(&dir, "main.rnx", "Main"), Value::Int(60));
}

#[test]
fn comptime_fn_call_splices_literal() {
    let dir = tree(
        "six",
        &[(
            "main.rnx",
            "#[CompTime]\nfn Six(): Int { return 6; }\nconst N: Int = CompTime!(Six());\nfn Main(): Int { return N * 7; }\n",
        )],
    );
    assert_eq!(ok(&dir, "main.rnx", "Main"), Value::Int(42));
}

#[test]
fn static_assert_fail_blocks_build_with_span() {
    let dir = tree(
        "safail",
        &[("main.rnx", "fn Main(): Int { StaticAssert!(1 == 2, \"bad\"); return 1; }\n")],
    );
    let errs = expand_errs(&dir, "main.rnx");
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].code.as_str(), "E123");
    assert!(errs[0].span.is_some());
}

#[test]
fn fuel_cap_trips_e117() {
    let dir = tree(
        "fuel",
        &[(
            "main.rnx",
            "#[CompTime]\nfn Spin(): Int {\n  let i = 0;\n  while true {\n    i = i + 1;\n  }\n  return i;\n}\nconst N: Int = CompTime!(Spin());\nfn Main(): Int { return N; }\n",
        )],
    );
    let errs = expand_errs(&dir, "main.rnx");
    assert!(errs.iter().any(|d| d.code.as_str() == "E117"), "{errs:?}");
}

#[test]
fn depth_cap_trips_e118() {
    let dir = tree(
        "depth",
        &[(
            "main.rnx",
            "#[CompTime]\nfn Down(n: Int): Int { return Down(n); }\nconst N: Int = CompTime!(Down(0));\nfn Main(): Int { return N; }\n",
        )],
    );
    let errs = expand_errs(&dir, "main.rnx");
    assert!(errs.iter().any(|d| d.code.as_str() == "E118"), "{errs:?}");
}

#[test]
fn sandbox_denies_io_in_comptime() {
    let dir = tree(
        "sandbox",
        &[(
            "main.rnx",
            "#[CompTime]\nfn Loud(): Int { print(\"hi\"); return 1; }\nconst N: Int = CompTime!(Loud());\nfn Main(): Int { return N; }\n",
        )],
    );
    let errs = expand_errs(&dir, "main.rnx");
    assert!(errs.iter().any(|d| d.code.as_str() == "E122"), "{errs:?}");
}

#[test]
fn macro_expansion_is_byte_identical_across_runs() {
    let src = "#[Derive(Clone, Debug)] struct S { let a: Int }\nfn Main(): Int { let v = vec![1]; StaticAssert!(1 < 2, \"x\"); return v[0]; }\n";
    let once = || {
        let dir = tree("det", &[("main.rnx", src)]);
        let m = expanded(&dir, "main.rnx");
        format!("{m:?}")
    };
    assert_eq!(once(), once());
}

#[test]
fn embed_bytes_roundtrip_at_runtime() {
    let dir = tree(
        "embed",
        &[
            ("icon.bin", "ABC"),
            ("main.rnx", "fn Main(): Int { let b = EmbedBytes!(\"icon.bin\"); return b[0] + b[1] + b[2]; }\n"),
        ],
    );
    assert_eq!(ok(&dir, "main.rnx", "Main"), Value::Int(65 + 66 + 67));
}
