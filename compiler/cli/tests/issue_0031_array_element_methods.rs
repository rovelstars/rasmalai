#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, crate::scratch::Scratch) {
    let dir = crate::scratch::dir(&format!("array-elem-method-{tag}"));
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check_with_files(&m, &frontend::modules::merged_decl_files(&g));
    assert!(c.iter().all(|x| x.code.is_warning()), "{c:?}");
    let mut out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    lir::opt::optimize_lir(&mut out, 1, "Main");
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    (out, dir)
}

fn dynamic_calls(module: &lir::instr::Module, name: &str) -> Vec<String> {
    let Some(id) = module.fn_index.get(name).copied() else {
        return Vec::new();
    };
    module.functions[id]
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter_map(|i| match i {
            lir::instr::Instr::Call {
                target: lir::instr::CallTarget::Dyn { method, .. },
                ..
            } => Some(method.clone()),
            _ => None,
        })
        .collect()
}

fn check_all_backends(src: &str, want: i64, want_out: &[String], tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leftover = dynamic_calls(&module, "Main");
    assert!(leftover.is_empty(), "{tag}: dynamic calls survived lowering: {leftover:?}");
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(v) if v == want => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    assert_eq!(machine.output.concat(), want_out.join("\n") + if want_out.is_empty() { "" } else { "\n" }, "interpreter {tag} stdout");

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
    let bin = String::from_utf8_lossy(&build.stdout)
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("artifact: "))
        .map(|s| s.trim().trim_end_matches(" (fresh)").to_string())
        .expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), want as i32, "aot {tag} exit");
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
}

const PUSHED_THREAD_ARRAY_SRC: &str = r#"fn worker(): Int { return 42; }

fn Main(): Int {
    let hs: Array<Thread> = [];
    hs.push(Thread.spawn((): Int => worker()));
    let r = hs[0].join();
    let v = r.unwrap() as Int;
    println!(v);
    if v == 42 { return 42; }
    return 1;
}
"#;

#[test]
fn pushed_thread_array_element_join_runs_on_every_backend() {
    check_all_backends(
        PUSHED_THREAD_ARRAY_SRC,
        42,
        &["42".to_string()],
        "pushed-thread-array",
    );
}

const IMPORTED_THREAD_ARRAY_SRC: &str = r#"import { Thread } from "@std/sync";

fn worker(): Int { return 42; }

fn Main(): Int {
    let hs: Array<Thread> = [];
    hs.push(Thread.spawn(worker));
    let v = hs[0].join().unwrap() as Int;
    println!(v);
    if v == 42 { return 42; }
    return 1;
}
"#;

#[test]
fn imported_thread_class_array_element_join_runs_on_every_backend() {
    check_all_backends(
        IMPORTED_THREAD_ARRAY_SRC,
        42,
        &["42".to_string()],
        "imported-thread-array",
    );
}

const CLASS_ARRAY_SRC: &str = r#"class Counter {
    let n: Int = 0;
    fn bump(by: Int): Int {
        this.n = this.n + by;
        return this.n;
    }
    fn value(): Int { return this.n; }
}

fn Main(): Int {
    let xs: Array<Counter> = [];
    xs.push(new Counter());
    xs.push(new Counter());
    xs[0].bump(20);
    xs[1].bump(20);
    let i = 0;
    let total = xs[i].value() + xs[1].value();
    xs[i].bump(2);
    println!(total);
    if total == 40 && xs[i].value() == 22 { return 42; }
    return 1;
}
"#;

#[test]
fn class_array_element_method_runs_on_every_backend() {
    check_all_backends(CLASS_ARRAY_SRC, 42, &["40".to_string()], "class-array");
}

const NESTED_INDEX_SRC: &str = r#"class Cell {
    let n: Int = 0;
    fn set(v: Int): Int {
        this.n = v;
        return this.n;
    }
    fn get(): Int { return this.n; }
}

fn rows(): Array<Array<Cell>> {
    let row: Array<Cell> = [];
    row.push(new Cell());
    let out: Array<Array<Cell>> = [];
    out.push(row);
    return out;
}

fn Main(): Int {
    let grid = rows();
    grid[0][0].set(42);
    let v = grid[0][0].get();
    println!(v);
    if v == 42 { return 42; }
    return 1;
}
"#;

#[test]
fn nested_index_method_call_keeps_element_type() {
    check_all_backends(NESTED_INDEX_SRC, 42, &["42".to_string()], "nested-index");
}

#[test]
fn untyped_empty_array_still_reports_e108_with_the_annotation_hint() {
    let src = "fn worker(): Int { return 1; }\nfn Main(): Int {\n    let hs = [];\n    hs.push(Thread.spawn((): Int => worker()));\n    let r = hs[0].join();\n    println!(\"joined\");\n    return 0;\n}\n";
    let dir = crate::scratch::dir("array-elem-method-untyped");
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&main)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(!build.status.success(), "untyped empty array must stay rejected");
    let text = String::from_utf8_lossy(&build.stdout).into_owned()
        + &String::from_utf8_lossy(&build.stderr);
    assert!(text.contains("`join`"), "diagnostic must name the method: {text}");
    assert!(
        text.contains("Array<Thread> = []"),
        "hint must name the annotation that now compiles: {text}"
    );
    assert!(!text.contains("1:1"), "stale import-line span: {text}");
}
