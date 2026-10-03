use frontend::parser::Parser;
use frontend::semantic;

fn errors(src: &str) -> Vec<diagnostics::Diagnostic> {
    let m = Parser::parse_module(src).expect("parses");
    semantic::check(&m).into_iter().filter(|d| !d.code.is_warning()).collect()
}

fn uses(src: &str) -> std::collections::BTreeSet<String> {
    let m = Parser::parse_module(src).expect("parses");
    semantic::prelude_uses(&m)
}

#[test]
fn string_methods_check_clean_with_types() {
    let src = "fn Main(): Int {\n\
        let n: Int = \"hello\".length();\n\
        let s: String = \"  a  \".trim();\n\
        let t: String = s.slice(0, 1);\n\
        let i: Int = s.indexOf(\"a\");\n\
        let c: String = s.concat(\"b\");\n\
        let k: Int = s.charCodeAt(0);\n\
        return n + i + k;\n\
    }\n";
    let errs = errors(src);
    assert!(errs.is_empty(), "{errs:?}");
    let used = uses(src);
    assert!(used.contains("String"), "{used:?}");
    assert!(used.contains("Int"), "{used:?}");
}

#[test]
fn string_length_property_checks_clean() {
    let src = "fn Main(): Int {\nlet n: Int = \"hello\".length;\nreturn n;\n}\n";
    assert!(errors(src).is_empty());
    assert!(uses(src).contains("String"));
}

#[test]
fn array_push_pop_length_check_clean() {
    let src = "fn Main(): Int {\n\
        let nums = [1, 2, 3];\n\
        nums.push(4);\n\
        let n: Int = nums.length();\n\
        let m: Int = nums.length;\n\
        let last = nums.pop().unwrap();\n\
        return n + m;\n\
    }\n";
    let errs = errors(src);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn arity_mismatch_is_e108() {
    let errs = errors("fn Main(): Int {\nlet n = \"hi\".length(1);\nreturn n;\n}\n");
    assert!(
        errs.iter().any(|d| d.code == diagnostics::Code::E108 && d.message.contains("takes 0 args")),
        "{errs:?}"
    );
}

#[test]
fn string_return_mismatch_is_e304() {
    let errs = errors("fn f(): String {\nreturn \"hi\".length();\n}\nfn Main(): Int { return 0; }\n");
    assert!(
        errs.iter().any(|d| d.code == diagnostics::Code::E304 && d.message.contains("`String`")),
        "{errs:?}"
    );
}

#[test]
fn method_on_wrong_receiver_is_e108() {
    let errs = errors("fn Main(): Int {\nlet s = \"foo\";\ns.push(1);\nreturn 0;\n}\n");
    assert!(
        errs.iter().any(|d| d.code == diagnostics::Code::E108 && d.message.contains("push")),
        "{errs:?}"
    );
}

#[test]
fn array_unknown_method_is_e108() {
    let errs = errors("fn Main(): Int {\nlet a = [1, 2];\nlet s = a.slice(\"bad\");\nreturn 0;\n}\n");
    assert!(
        errs.iter().any(|d| d.code == diagnostics::Code::E108 && d.message.contains("slice")),
        "{errs:?}"
    );
}

#[test]
fn map_filter_accepted_with_closure_callback() {
    for src in [
        "fn Main(): Int {\nlet a = [1, 2];\nlet b = a.map(x => x);\nreturn 0;\n}\n",
        "fn Main(): Int {\nlet a = [1, 2];\nlet b = a.filter(x => true);\nreturn 0;\n}\n",
    ] {
        let errs = errors(src);
        assert!(errs.is_empty(), "{src}: {errs:?}");
    }
}

#[test]
fn map_filter_reject_bad_callbacks() {
    for (src, want) in [
        (
            "fn Main(): Int {\nlet a = [1, 2];\nlet b = a.map(1);\nreturn 0;\n}\n",
            "needs a closure",
        ),
        (
            "fn Main(): Int {\nlet a = [1, 2];\nlet b = a.map((x: Int, y: Int) => x);\nreturn 0;\n}\n",
            "one-parameter",
        ),
        (
            "fn Main(): Int {\nlet a = [1, 2];\nlet b = a.filter((x: Int): Int => x);\nreturn 0;\n}\n",
            "Bool",
        ),
    ] {
        let errs = errors(src);
        assert!(
            errs.iter().any(|d| d.code == diagnostics::Code::E108 && d.message.contains(want)),
            "{src}: {errs:?}"
        );
    }
}

#[test]
fn null_result_methods_check_clean() {
    let src = "fn Main(): Int {\n\
        let x: Int? = 42;\n\
        let a: Bool = x != null;\n\
        let b: Bool = x == null;\n\
        let v = x ?? 0;\n\
        let w: Int? = null;\n\
        let r = Result.Ok(1);\n\
        let ok: Bool = r.isOk();\n\
        let err: Bool = r.isErr();\n\
        let u = r.unwrap();\n\
        return v + (w ?? 7) + u;\n\
    }\n";
    let errs = errors(src);
    assert!(errs.is_empty(), "{errs:?}");
    let used = uses(src);
    assert!(used.contains("Result"), "{used:?}");
}

#[test]
fn null_coalesce_and_checks_check_clean() {
    let src = "fn Main(): Int {\n\
        let x: Int? = 41;\n\
        let v = x ?? 0;\n\
        let n: Int? = null;\n\
        let w = n ?? 7;\n\
        return v + w;\n\
    }\n";
    let errs = errors(src);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn arg_type_mismatch_is_e108() {
    let errs = errors("fn Main(): Int {\nlet n = \"hi\".slice(\"a\", \"b\");\nreturn n;\n}\n");
    assert!(
        errs.iter().any(|d| d.code == diagnostics::Code::E108 && d.message.contains("Int")),
        "{errs:?}"
    );
}

#[test]
fn await_on_promise_result_resolves_unwrap_or() {
    let src = "async fn fetch(): Result<String, String> {\nreturn Result.Ok(\"live\");\n}\nasync fn load(): String {\nreturn (await fetch()).unwrapOr(\"d\");\n}\nfn Main(): Int {\nreturn 0;\n}\n";
    let errs = errors(src);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn await_on_promise_result_checks_unwrap_or_arity() {
    let src = "async fn fetch(): Result<String, String> {\nreturn Result.Ok(\"live\");\n}\nasync fn load(): String {\nreturn (await fetch()).unwrapOr();\n}\nfn Main(): Int {\nreturn 0;\n}\n";
    let errs = errors(src);
    assert!(
        errs.iter().any(|d| d.code == diagnostics::Code::E108 && d.message.contains("takes 1 args")),
        "{errs:?}"
    );
}
