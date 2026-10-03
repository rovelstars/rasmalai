const ASYNC_MAIN_CALLS_ASYNC_FN: &str = "async fn test(): Int {\n    return 40 + 2;\n}\nasync fn main(): Int {\n    let v = await test();\n    print(v);\n    return 0;\n}";

const ASYNC_CHAIN: &str = "async fn one(): Int {\n    return 1;\n}\nasync fn two(): Int {\n    let a = await one();\n    let b = await one();\n    return a + b;\n}\nasync fn main(): Int {\n    print(await two());\n    return 0;\n}";

const ASYNC_DEEP_RECURSION: &str = "async fn depth(n: Int): Int {\n    if n <= 0 {\n        return 0;\n    }\n    return (await depth(n - 1)) + 1;\n}\nasync fn main(): Int {\n    print(await depth(20));\n    return 0;\n}";

const ASYNC_LOOP_AWAITS: &str = "async fn tick(n: Int): Int {\n    return n * 10;\n}\nasync fn main(): Int {\n    let a = await tick(1);\n    let b = (await tick(2)) + a;\n    print(a + b);\n    return 0;\n}";

const ASYNC_LAMBDA_IIFE: &str = "async fn main(): Int {\n    let v = await (async() => 42)();\n    print(v);\n    return 0;\n}";

const ASYNC_LAMBDA_CAPTURE: &str = "async fn main(): Int {\n    let base = 100;\n    let get = async () => base + 23;\n    let add = async (x) => x + 1;\n    print(await get());\n    print(await add(41));\n    return 0;\n}";

const ASYNC_LAMBDA_AWAITS_INNER: &str = "async fn fetch(n: Int): Int {\n    return n * 2;\n}\nasync fn main(): Int {\n    let v = await (async() => await fetch(21))();\n    print(v);\n    return 0;\n}";

const AWAIT_LOCAL_CLOSURE_CALL: &str = "async fn main(): Int {\n    let f = async () => 7;\n    let v = await f();\n    print(v);\n    return 0;\n}";

const SYNC_MAIN_UNAFFECTED: &str = "fn main(): Int {\n    print(6 * 7);\n    return 0;\n}";

#[test]
fn async_main_calling_async_fn_resolves_value() {
    let out = wasm_playground::run(ASYNC_MAIN_CALLS_ASYNC_FN);
    assert!(out.contains("42"), "{out}");
    assert!(out.contains("=> 0"), "{out}");
    assert!(!out.contains("obj#"), "{out}");
}

#[test]
fn async_chained_awaits_resolve() {
    let out = wasm_playground::run(ASYNC_CHAIN);
    assert!(out.contains("2\n=> 0"), "{out}");
    assert!(!out.contains("obj#"), "{out}");
}

#[test]
fn async_deep_recursion_terminates() {
    let out = wasm_playground::run(ASYNC_DEEP_RECURSION);
    assert!(out.contains("20\n=> 0"), "{out}");
    assert!(!out.contains("obj#"), "{out}");
}

#[test]
fn async_awaits_in_loop_accumulate() {
    let out = wasm_playground::run(ASYNC_LOOP_AWAITS);
    assert!(out.contains("40\n=> 0"), "{out}");
    assert!(!out.contains("obj#"), "{out}");
}

#[test]
fn async_lambda_iife_await_resolves() {
    let out = wasm_playground::run(ASYNC_LAMBDA_IIFE);
    assert!(out.contains("42\n=> 0"), "{out}");
    assert!(!out.contains("obj#"), "{out}");
}

#[test]
fn async_lambda_capture_and_params() {
    let out = wasm_playground::run(ASYNC_LAMBDA_CAPTURE);
    assert!(out.contains("123"), "{out}");
    assert!(out.contains("42"), "{out}");
    assert!(!out.contains("obj#"), "{out}");
}

#[test]
fn async_lambda_awaiting_inner_call() {
    let out = wasm_playground::run(ASYNC_LAMBDA_AWAITS_INNER);
    assert!(out.contains("42\n=> 0"), "{out}");
    assert!(!out.contains("obj#"), "{out}");
}

#[test]
fn await_local_closure_call_resolves() {
    let out = wasm_playground::run(AWAIT_LOCAL_CLOSURE_CALL);
    assert!(out.contains("7\n=> 0"), "{out}");
    assert!(!out.contains("obj#"), "{out}");
}

#[test]
fn sync_main_still_runs() {
    let out = wasm_playground::run(SYNC_MAIN_UNAFFECTED);
    assert!(out.contains("42\n=> 0"), "{out}");
}

#[test]
fn async_sources_check_clean() {
    for src in [
        ASYNC_MAIN_CALLS_ASYNC_FN,
        ASYNC_CHAIN,
        ASYNC_DEEP_RECURSION,
        ASYNC_LOOP_AWAITS,
        ASYNC_LAMBDA_IIFE,
        ASYNC_LAMBDA_CAPTURE,
        ASYNC_LAMBDA_AWAITS_INNER,
        AWAIT_LOCAL_CLOSURE_CALL,
        SYNC_MAIN_UNAFFECTED,
    ] {
        assert_eq!(wasm_playground::check(src), String::new(), "{src}");
    }
}

#[test]
fn async_lambda_returning_non_int_resolves() {
    let out = wasm_playground::run("fn main(): Int {\n    let f = (async(): String => \"s\")();\n    switch f.wait() {\n    case .Ok(v): print(v); pass;\n    case .Err(e): print(e); pass;\n    }\n    return 0;\n}\n");
    assert!(out.contains("s\n=> 0"), "{out}");
}

#[test]
fn await_inside_loop_body_allowed() {
    let out = wasm_playground::check("async fn tick(n: Int): Int {\n    return n;\n}\nasync fn main(): Int {\n    let i = 0;\n    while i < 1 {\n        print(await tick(i));\n        i = i + 1;\n    }\n    return 0;\n}\n");
    assert_eq!(out, String::new(), "{out}");
}

const ASYNC_MAIN_NO_RETURN_NO_AWAIT: &str = "async fn main(){\nprint(\"hello\")\n}";

#[test]
fn async_main_without_return_terminates() {
    let out = wasm_playground::run(ASYNC_MAIN_NO_RETURN_NO_AWAIT);
    assert!(out.contains("hello"), "{out}");
    assert!(out.contains("=> 0"), "{out}");
}
