#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, crate::scratch::Scratch) {
    let dir = crate::scratch::dir(&format!("unwrap-payload-{tag}"));
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

fn dyn_calls(module: &lir::instr::Module, name: &str) -> Vec<String> {
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
    for name in ["Main", "fallbackLen", "strictLen"] {
        let leftover = dyn_calls(&module, name);
        assert!(leftover.is_empty(), "{name}: dynamic calls survived lowering: {leftover:?}");
    }
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(v) if v == want => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    assert_eq!(machine.output, want_out.to_vec(), "interpreter {tag} stdout");

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

const UNWRAP_PAYLOAD_SRC: &str = r#"import fs from "@std/fs";

fn fallbackLen(r: Result<String, String>): Int {
    let t = r.unwrapOr("");
    return t.len();
}

fn strictLen(r: Result<String, String>): Int {
    let t = r.unwrap();
    return t.trim().len();
}

fn Main(): Int {
    let path = "PAYLOAD_PATH";
    if fs.writeText(path, "rasmalai", .Overwrite).isErr() { return 1; }
    let text = fs.readText(path).unwrapOr("");
    print(text.len());
    if text.trim().len() != 8 { return 2; }
    if fs.readText(path).unwrapOr("").len() != 8 { return 6; }
    let raw = fs.readText(path).unwrap();
    print(raw.slice(0, 3));
    let ok = Result.Ok("payload");
    if ok.unwrap().len() != 7 { return 3; }
    let mapped = Result.Ok("ab").map((s: String): String => s + "!").unwrap();
    if mapped.len() != 3 { return 7; }
    let miss = Result.Err<String, String>("boom");
    if miss.unwrapOr("fallback").len() != 8 { return 4; }
    if fallbackLen(fs.readText(path)) != 8 { return 5; }
    if strictLen(fs.readText(path)) != 8 { return 8; }
    return 0;
}
"#;

const ANNOTATED_AND_INFERRED_SRC: &str = r#"fn text(ok: Bool): Result<String, String> {
    if ok { return Result.Ok("abcd"); }
    return Result.Err<String, String>("bad");
}

fn num(ok: Bool): Result<Int, String> {
    if ok { return Result.Ok(41); }
    return Result.Err<Int, String>("bad");
}

fn Main(): Int {
    let annotated: String = text(true).unwrap();
    print(annotated.len());
    let annotatedOr: String = text(false).unwrapOr("xyz");
    print(annotatedOr.len());
    let inferred = text(true).unwrap();
    print(inferred.trim().len());
    let inferredOr = text(false).unwrapOr("wxyz");
    print(inferredOr.len());
    let ni: Int = num(true).unwrap();
    let nio: Int = num(false).unwrapOr(7);
    print(ni + nio);
    let inferredInt = num(true).unwrap();
    let inferredOrInt = num(false).unwrapOr(1);
    print(inferredInt + inferredOrInt);
    print(num(true).unwrap().toString().len());
    return 0;
}
"#;

#[test]
fn result_unwrap_keeps_payload_when_annotated_and_inferred() {
    check_all_backends(
        ANNOTATED_AND_INFERRED_SRC,
        0,
        &[
            "4".to_string(),
            "3".to_string(),
            "4".to_string(),
            "4".to_string(),
            "48".to_string(),
            "42".to_string(),
            "2".to_string(),
        ],
        "result-unwrap-annotated",
    );
}

#[test]
fn result_unwrap_payload_methods_run_on_every_backend() {
    let dir = crate::scratch::dir("unwrap-payload-data");
    let path = dir.join("payload.txt");
    let src = UNWRAP_PAYLOAD_SRC.replace("PAYLOAD_PATH", &path.to_string_lossy().replace('\\', "/"));
    check_all_backends(
        &src,
        0,
        &["8".to_string(), "ras".to_string()],
        "result-unwrap-payload",
    );
}
