#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn lower_src(src: &str, tag: &str) -> (lir::instr::Module, crate::scratch::Scratch) {
    let dir = crate::scratch::dir(&format!("erased-array-{tag}"));
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check_with_files(&m, &frontend::modules::merged_decl_files(&g));
    assert!(c.iter().all(|x| x.code.is_warning()), "{c:?}");
    let out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    (out, dir)
}

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, crate::scratch::Scratch) {
    let (mut out, dir) = lower_src(src, tag);
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
    let want_out: Vec<String> = want_out.to_vec();
    assert_eq!(machine.output.concat(), want_out.join("\n") + if want_out.is_empty() { "" } else { "\n" }, "interpreter {tag} stdout");

    let mut jit = cranelift::jit::Jit::compile(leaked).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), want, "cranelift {tag}");
    assert_eq!(llvm::codegen::execute(leaked, "Main").unwrap(), want, "llvm {tag}");

    for release in [false, true] {
        let rnx = env!("CARGO_BIN_EXE_rnx");
        let mut cmd = std::process::Command::new(rnx);
        cmd.arg("build").arg(dir.join("main.rnx"));
        if release {
            cmd.arg("--release");
        }
        let build = cmd.output().unwrap();
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
}

#[test]
fn manual_join_examples_native_and_aot() {
    check_all_backends(
        "fn Main(): Int {\n\
        let numbers: Array<Int> = [10, 20, 30, 40, 50];\n\
        println!(numbers.join(\",\"));\n\
        println!([1, 2, 3].join());\n\
        return 0;\n\
        }\n",
        0,
        &["10,20,30,40,50".to_string(), "123".to_string()],
        "manual-join",
    );
}

#[test]
fn string_and_empty_typed_arrays_join() {
    check_all_backends(
        "fn Main(): Int {\n\
        let words: Array<String> = [\"a\", \"b\", \"c\"];\n\
        println!(words.join(\"-\"));\n\
        let ints: Array<Int> = [];\n\
        println!(\"[\" + ints.join(\",\") + \"]\");\n\
        let texts: Array<String> = [];\n\
        println!(\"[\" + texts.join(\",\") + \"]\");\n\
        let flags: Array<Bool> = [];\n\
        println!(\"[\" + flags.join(\",\") + \"]\");\n\
        let reals: Array<Float> = [];\n\
        println!(\"[\" + reals.join(\",\") + \"]\");\n\
        return 0;\n\
        }\n",
        0,
        &[
            "a-b-c".to_string(),
            "[]".to_string(),
            "[]".to_string(),
            "[]".to_string(),
            "[]".to_string(),
        ],
        "empty-join",
    );
}

#[test]
fn erased_receiver_find_some_reversed_on_typed_arrays() {
    check_all_backends(
        "fn Main(): Int {\n\
        let xs: Array<Int> = [4, 9, 16];\n\
        println!(xs.findIndex((x: Int): Bool => x > 5));\n\
        println!(xs.some((x: Int): Bool => x == 9));\n\
        println!(xs.every((x: Int): Bool => x > 0));\n\
        let hit = xs.find((x: Int): Bool => x == 16);\n\
        println!(hit ?? -1);\n\
        println!(xs.reversed().join(\",\"));\n\
        println!([4, 9, 16].join(\" | \"));\n\
        let flags: Array<Bool> = [true, false];\n\
        println!(flags.join(\",\"));\n\
        let reals: Array<Float> = [1.5, 2.5];\n\
        println!(reals.join(\",\"));\n\
        return 0;\n\
        }\n",
        0,
        &[
            "1".to_string(),
            "true".to_string(),
            "true".to_string(),
            "16".to_string(),
            "16,9,4".to_string(),
            "4 | 9 | 16".to_string(),
            "true,false".to_string(),
            "1.5,2.5".to_string(),
        ],
        "erased-methods",
    );
}

#[test]
fn typed_array_into_erased_array_param_is_element_correct() {
    check_all_backends(
        ERASED_PARAM_SRC,
        0,
        &[
            "n=3 first=3".to_string(),
            "4".to_string(),
            "3".to_string(),
            "4".to_string(),
        ],
        "erased-param",
    );
}

#[test]
fn erased_param_signature_is_array_any() {
    let (module, _dir) = lower_src(ERASED_PARAM_SRC, "erased-param-sig");
    let id = module
        .functions
        .iter()
        .position(|f| f.name == "head" || f.name.rsplit('.').next() == Some("head"))
        .unwrap_or_else(|| {
            panic!(
                "head function; have {:?}",
                module.functions.iter().map(|f| f.name.as_str()).collect::<Vec<_>>()
            )
        });
    assert_eq!(
        module.functions[id].sig_params.first().cloned(),
        Some(lir::instr::LirType::Array(Box::new(lir::instr::LirType::Any))),
        "the generic Array<T> parameter erases to Array<Any>"
    );
}

const ERASED_PARAM_SRC: &str = "fn head<T>(xs: Array<T>): String {\n\
        return \"n=\" + xs.length() + \" first=\" + (\"\" + xs[0]);\n\
        }\n\
        fn firstOf<T>(xs: Array<T>): T {\n\
        return xs[0];\n\
        }\n\
        fn Main(): Int {\n\
        let xs: Array<Int> = [3, 4, 5];\n\
        println!(head(xs));\n\
        println!(firstOf(xs) + 1);\n\
        let it = new ArrayIter<Int>(xs);\n\
        println!(it.next() ?? -1);\n\
        println!(it.next() ?? -1);\n\
        return 0;\n\
        }\n";

#[test]
fn bare_array_annotation_on_typed_literal_boxed() {
    check_all_backends(
        "fn Main(): Int {\n\
        let wide: Array = [1, 2];\n\
        println!(wide[0] == 1);\n\
        println!(wide[0]);\n\
        println!(wide.join(\"-\"));\n\
        println!(\"len=\" + wide.length());\n\
        return 0;\n\
        }\n",
        0,
        &[
            "true".to_string(),
            "1".to_string(),
            "1-2".to_string(),
            "len=2".to_string(),
        ],
        "bare-literal",
    );
}
