const STATUS: &str = "import { HttpStatus, statusCode, statusReason, statusFromCode, isSuccess } from \"@std/web\";\nfn main(): Int {\n    print(statusCode(HttpStatus.NotFound));\n    print(statusReason(HttpStatus.Ok));\n    print(statusCode((statusFromCode(201) ?? HttpStatus.Ok)));\n    print(isSuccess(200));\n    return 0;\n}";

const HEADERS: &str = "import { Headers } from \"@std/web\";\nfn main(): Int {\n    let h = new Headers();\n    h.set(\"Content-Type\", \"text/plain\");\n    print((h.get(\"content-type\") ?? \"\"));\n    print(h.len());\n    return 0;\n}";

const URL_PARSE: &str = "import { URL } from \"@std/web\";\nfn main(): Int {\n    let u = new URL(\"https://example.com:8080/a/b?x=1#frag\");\n    print(u.href());\n    print(u.hostname(), u.port(), u.pathname());\n    let rel = new URL(\"../c\", \"https://example.com/a/b\");\n    print(rel.href());\n    return 0;\n}";

const PARAMS: &str = "import { URLSearchParams } from \"@std/web\";\nfn main(): Int {\n    let p = new URLSearchParams(\"a=1&a=2&b=3\");\n    print(p.getAll(\"a\").length());\n    print((p.get(\"b\") ?? \"\"));\n    print(p.toString());\n    return 0;\n}";

const CODEC: &str = "import { encodeComponent, decodeComponent } from \"@std/web\";\nfn main(): Int {\n    print(encodeComponent(\"a b\", true));\n    print(decodeComponent(\"a+b%20c\", true));\n    return 0;\n}";

const JSON_CODEC: &str = "import { JSON } from \"@std/json\";\nfn main(): Int {\n    print(JSON.stringify(JSON.parse(\"\\{\\\"n\\\": 7, \\\"t\\\": [true, null]}\")));\n    let m = JSON.parseObject(\"\\{\\\"n\\\": 7}\");\n    print((m.get(\"n\") ?? 0));\n    return 0;\n}";

#[test]
fn web_sync_check_clean() {
    for src in [STATUS, HEADERS, URL_PARSE, PARAMS, CODEC] {
        assert_eq!(wasm_playground::check(src), String::new(), "{src}");
    }
}

#[test]
fn web_sync_run() {
    let out = wasm_playground::run(STATUS);
    assert!(out.contains("404"), "{out}");
    assert!(out.contains("OK"), "{out}");
    assert!(out.contains("201"), "{out}");
    assert!(out.contains("=> 0"), "{out}");
    let out = wasm_playground::run(HEADERS);
    assert!(out.contains("text/plain"), "{out}");
    let out = wasm_playground::run(URL_PARSE);
    assert!(out.contains("https://example.com:8080/a/b?x=1#frag"), "{out}");
    assert!(out.contains("https://example.com/c"), "{out}");
    let out = wasm_playground::run(PARAMS);
    assert!(out.contains("a=1&a=2&b=3"), "{out}");
    let out = wasm_playground::run(CODEC);
    assert!(out.contains("a+b"), "{out}");
    assert!(out.contains("a b c"), "{out}");
}

#[test]
fn web_json_codec_run() {
    let out = wasm_playground::run(JSON_CODEC);
    assert!(out.contains("{\"n\":7,\"t\":[true,null]}"), "{out}");
    assert!(out.contains("\n7\n=> 0"), "{out}");
}
