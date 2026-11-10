#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, crate::scratch::Scratch) {
    let dir = crate::scratch::dir(&format!("deinit-{tag}"));
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

fn check_all_backends(src: &str, want: i64, want_out: &[String], tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(v) if v == want => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    let got = machine.output.concat();
    let want_out: Vec<String> = want_out.to_vec();
    assert_eq!(got, want_out.join("\n") + if want_out.is_empty() { "" } else { "\n" }, "interpreter {tag} stdout");

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
}

#[test]
fn user_deinit_runs_once_before_field_releases() {
    check_all_backends(
        "class Inner {\n\
        let v: Int\n\
        init(v: Int) { this.v = v }\n\
        deinit { println!(\"inner {this.v}\") }\n\
        }\n\
        class Outer {\n\
        let child: Inner\n\
        let v: Int\n\
        init(v: Int) {\n\
        this.v = v\n\
        this.child = new Inner(v + 1)\n\
        }\n\
        deinit { println!(\"outer {this.v}\") }\n\
        }\n\
        fn make(): Int {\n\
        let o = new Outer(1)\n\
        println!(\"body\")\n\
        return o.v\n\
        }\n\
        fn Main(): Int {\n\
        println!(\"start\")\n\
        let r = make()\n\
        println!(\"after {r}\")\n\
        return 0\n\
        }\n",
        0,
        &[
            "start".to_string(),
            "body".to_string(),
            "outer 1".to_string(),
            "inner 2".to_string(),
            "after 1".to_string(),
        ],
        "order",
    );
}

#[test]
fn user_deinit_runs_once_at_main_exit() {
    check_all_backends(
        "class Handle {\n\
        let id: Int\n\
        init(id: Int) { this.id = id }\n\
        deinit { println!(\"close {this.id}\") }\n\
        }\n\
        fn Main(): Int {\n\
        let a = new Handle(7)\n\
        println!(\"open\")\n\
        return 0\n\
        }\n",
        0,
        &["open".to_string(), "close 7".to_string()],
        "exit",
    );
}
