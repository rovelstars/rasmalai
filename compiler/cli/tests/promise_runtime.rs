use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-promise-{tag}-{}", std::process::id()));
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
fn resolve_and_reject_unwrap_via_wait() {
    check_all_backends(
        "fn Main(): Int {\n\
        let a: Result<Int, String> = Promise.resolve(41).wait();\n\
        print(a.unwrap());\n\
        let b: Result<Int, String> = Promise.reject<Int>(\"boom\").wait();\n\
        switch b {\n\
        case .Ok(v): print(v); pass;\n\
        case .Err(e): print(e); pass;\n\
        }\n\
        return 0;\n\
        }\n",
        0,
        &["41".to_string(), "boom".to_string()],
        "resolve",
    );
}

#[test]
fn then_catch_finally_chains() {
    check_all_backends(
        "fn Main(): Int {\n\
        let a: Result<Int, String> = Promise.resolve(20).then<Int>((v) => v + 1).wait();\n\
        print(a.unwrap());\n\
        let b: Result<Int, String> = Promise.reject<Int>(\"x\").catchReject<Int>((e) => 99).wait();\n\
        print(b.unwrap());\n\
        let c: Result<Int, String> = Promise.resolve(7).finallyDo(() => print(\"fin\")).wait();\n\
        print(c.unwrap());\n\
        return 0;\n\
        }\n",
        0,
        &[
            "21".to_string(),
            "99".to_string(),
            "fin".to_string(),
            "7".to_string(),
        ],
        "chains",
    );
}

#[test]
fn all_preserves_index_order() {
    check_all_backends(
        "fn Main(): Int {\n\
        let ps: Array<Promise<Int>> = [Promise.resolve(1), Promise.resolve(2), Promise.resolve(3)];\n\
        let all: Result<Array<Int>, String> = Promise.all<Int>(ps).wait();\n\
        let got = all.unwrap();\n\
        print(got[0]);\n\
        print(got[1]);\n\
        print(got[2]);\n\
        return 0;\n\
        }\n",
        0,
        &["1".to_string(), "2".to_string(), "3".to_string()],
        "all",
    );
}

#[test]
fn race_settles_first() {
    check_all_backends(
        "fn Main(): Int {\n\
        let ps: Array<Promise<Int>> = [Promise.resolve(1), Promise.resolve(2)];\n\
        print(Promise.race<Int>(ps).wait().unwrap());\n\
        return 0;\n\
        }\n",
        0,
        &["1".to_string()],
        "race",
    );
}

#[test]
fn all_settled_keeps_values_and_reasons() {
    check_all_backends(
        "fn Main(): Int {\n\
        let ps: Array<Promise<Int>> = [Promise.resolve(5), Promise.reject<Int>(\"bad\")];\n\
        let all: Result<Array<PromiseResult<Int>>, String> = Promise.allSettled<Int>(ps).wait();\n\
        let got: Array<PromiseResult<Int>> = all.unwrap();\n\
        print(got[0].status);\n\
        let v0 = got[0].value;\n\
        if v0 == null {\n\
        print(-1);\n\
        } else {\n\
        print(v0);\n\
        }\n\
        print(got[1].status);\n\
        print(got[1].reason);\n\
        return 0;\n\
        }\n",
        0,
        &[
            "fulfilled".to_string(),
            "5".to_string(),
            "rejected".to_string(),
            "bad".to_string(),
        ],
        "allsettled",
    );
}

#[test]
fn any_first_success_and_aggregate_failure() {
    check_all_backends(
        "fn Main(): Int {\n\
        let mixed: Array<Promise<Int>> = [Promise.reject<Int>(\"no\"), Promise.resolve(5)];\n\
        let first: Result<Int, String> = Promise.any<Int>(mixed).wait();\n\
        print(first.unwrap());\n\
        let doomed: Array<Promise<Int>> = [Promise.reject<Int>(\"a\"), Promise.reject<Int>(\"b\")];\n\
        let last: Result<Int, String> = Promise.any<Int>(doomed).wait();\n\
        switch last {\n\
        case .Ok(v): print(v); pass;\n\
        case .Err(e): print(e); pass;\n\
        }\n\
        return 0;\n\
        }\n",
        0,
        &[
            "5".to_string(),
            "AggregateError: all promises rejected".to_string(),
        ],
        "any",
    );
}

#[test]
fn with_resolvers_settles_from_worker_thread() {
    check_all_backends(
        "import { AtomicInt } from \"@std/sync\";\n\
        fn settler(wr: PromiseWithResolvers<Int>): Int {\n\
        wr.resolve(42);\n\
        AtomicInt.byId(70001).set(1);\n\
        return 0;\n\
        }\n\
        fn Main(): Int {\n\
        AtomicInt.byId(70001).set(0);\n\
        let wr = Promise.withResolvers<Int>();\n\
        let h = Thread.spawn((): Int => settler(wr));\n\
        let r: Result<Int, String> = wr.promise.wait();\n\
        h.join();\n\
        print(r.unwrap());\n\
        print(AtomicInt.byId(70001).get());\n\
        return 0;\n\
        }\n",
        0,
        &["42".to_string(), "1".to_string()],
        "resolvers",
    );
}

#[test]
fn wait_blocks_until_settlement() {
    check_all_backends(
        "import { Clock } from \"@std/time\";\n\
        fn settler(wr: PromiseWithResolvers<Int>): Int {\n\
        let t0 = Clock.mono().toMillis();\n\
        while Clock.mono().toMillis() - t0 < 200 {\n\
        }\n\
        wr.resolve(9);\n\
        return 0;\n\
        }\n\
        fn Main(): Int {\n\
        let wr = Promise.withResolvers<Int>();\n\
        let h = Thread.spawn((): Int => settler(wr));\n\
        let t0 = Clock.mono().toMillis();\n\
        let r: Result<Int, String> = wr.promise.wait();\n\
        let dt = Clock.mono().toMillis() - t0;\n\
        h.join();\n\
        print(r.unwrap());\n\
        assert(dt >= 100, \"wait returned before settlement\");\n\
        return 0;\n\
        }\n",
        0,
        &["9".to_string()],
        "blocking",
    );
}

fn check_src_fails(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("rnx-promise-neg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"neg\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/main.rnx"), src).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx)
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .arg("src/main.rnx")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.status.code(), Some(1), "{text}");
    text
}

#[test]
fn wait_inside_async_is_e108() {
    let text = check_src_fails(
        "async fn f(): Int {\n    let p = Promise.resolve(1);\n    return p.wait();\n}\nfn Main(): Int {\n    return 0;\n}\n",
        "wait",
    );
    assert!(text.contains("E108"), "{text}");
    assert!(text.contains("wait()"), "{text}");
}
