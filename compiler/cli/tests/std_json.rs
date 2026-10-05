use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-json-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_backend(dir: &PathBuf, src: &str, backend: Option<&str>) -> std::process::Output {
    let prog = dir.join("main.rnx");
    std::fs::write(&prog, src).unwrap();
    let mut cmd = Command::new(rnx());
    cmd.arg("run").env("NO_COLOR", "1");
    if let Some(b) = backend {
        cmd.arg("--backend").arg(b);
    }
    cmd.arg(&prog);
    cmd.output().unwrap()
}

fn expect_backends(tag: &str, src: &str, want: &str) {
    for backend in [Some("interpreter"), Some("cranelift"), Some("llvm")] {
        let dir = fresh_dir(&format!("{tag}-{}", backend.unwrap_or("interp")));
        let out = run_backend(&dir, src, backend);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{tag} {:?} failed: {stderr}", backend);
        assert_eq!(stdout, want, "{tag} {:?} stdout mismatch", backend);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn expect_failure_everywhere(tag: &str, src: &str, want_err: &str) {
    for backend in [Some("interpreter"), Some("cranelift"), Some("llvm")] {
        let dir = fresh_dir(&format!("{tag}-{}", backend.unwrap_or("interp")));
        let out = run_backend(&dir, src, backend);
        assert!(!out.status.success(), "{tag} {:?} unexpectedly succeeded", backend);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let combined = format!("{stdout}{stderr}");
        assert!(
            combined.contains(want_err),
            "{tag} {:?} missing `{want_err}` in: {combined}",
            backend
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

const SCALARS: &str = r#"import { JSON } from "@std/json";
fn Main(): Int {
    print(JSON.stringify(JSON.parse("null")));
    print(JSON.stringify(JSON.parse("true")));
    print(JSON.stringify(JSON.parse("false")));
    print(JSON.stringify(JSON.parse("12345")));
    print(JSON.stringify(JSON.parse("-7")));
    print(JSON.stringify(JSON.parse("3.1415")));
    print(JSON.stringify(JSON.parse("3.0")));
    print(JSON.stringify(JSON.parse("\"hello world\"")));
    return 0;
}
"#;

#[test]
fn json_scalars_round_trip() {
    expect_backends(
        "scalars",
        SCALARS,
        "null\ntrue\nfalse\n12345\n-7\n3.1415\n3.0\n\"hello world\"\n",
    );
}

const NESTED: &str = r#"import { JSON } from "@std/json";
fn Main(): Int {
    let doc = "\{\"name\": \"Rasmalai\", \"version\": 1, \"features\": [\"fast\", \"arc\", \"mio\"], \"meta\": \{\"ok\": true}}";
    print(JSON.stringify(JSON.parse(doc)));
    return 0;
}
"#;

#[test]
fn json_nested_round_trip_is_canonical() {
    expect_backends(
        "nested",
        NESTED,
        "{\"features\":[\"fast\",\"arc\",\"mio\"],\"meta\":{\"ok\":true},\"name\":\"Rasmalai\",\"version\":1}\n",
    );
}

const ARR_NAV: &str = r#"import { JSON } from "@std/json";
fn Main(): Int {
    let it: Any = JSON.parse("[10, 20, 30]");
    print(it[0]);
    print(it[2]);
    print(__rnx_array_len(it));
    return 0;
}
"#;

#[test]
fn json_array_index_and_len() {
    expect_backends("arrnav", ARR_NAV, "10\n30\n3\n");
}

const OBJ_NAV: &str = r#"import { JSON } from "@std/json";
import { Map } from "@std/collections";
fn showItems(it: Any): Int {
    print(it[0]);
    print(it[1]);
    print(__rnx_array_len(it));
    return 0;
}
fn showMeta(nm: Any): Int {
    let sub = JSON.asMap(nm);
    let okv = sub.get("ok");
    if okv != null {
        print(okv);
        return 0;
    }
    print("none-o");
    return 1;
}
fn stepMeta(m: Map<String, Any>): Int {
    let nm = m.get("meta");
    if nm != null {
        return showMeta(nm);
    }
    print("none-m");
    return 1;
}
fn stepItems(m: Map<String, Any>): Int {
    let itv = m.get("items");
    if itv != null {
        showItems(itv);
        return stepMeta(m);
    }
    print("none-i");
    return 1;
}
fn Main(): Int {
    let m = JSON.parseObject("\{\"count\": 42, \"items\": [10, 20], \"meta\": \{\"ok\": true}}");
    let cv = m.get("count");
    if cv != null {
        print(cv);
        return stepItems(m);
    }
    print("none-c");
    return 1;
}
"#;

#[test]
fn json_object_lookup_across_backends() {
    expect_backends("objnav", OBJ_NAV, "42\n10\n20\n2\ntrue\n");
}

const ESCAPES: &str = r#"import { JSON } from "@std/json";
fn Main(): Int {
    let doc = "\{\"q\": \"a\\\"b\\\\c\\nd\", \"tab\": \"x\\ty\"}";
    print(JSON.stringify(JSON.parse(doc)));
    return 0;
}
"#;

#[test]
fn json_string_escapes_round_trip() {
    expect_backends("escapes", ESCAPES, "{\"q\":\"a\\\"b\\\\c\\nd\",\"tab\":\"x\\ty\"}\n");
}

const BAD_UNQUOTED: &str = r#"import { JSON } from "@std/json";
fn Main(): Int {
    let data: Any = JSON.parse("\{unquoted: 1}");
    print(data);
    return 0;
}
"#;

const BAD_TRUNCATED: &str = r#"import { JSON } from "@std/json";
fn Main(): Int {
    let data: Any = JSON.parse("[");
    print(data);
    return 0;
}
"#;

#[test]
fn json_invalid_syntax_fails_cleanly() {
    expect_failure_everywhere("badkey", BAD_UNQUOTED, "json parse error");
    expect_failure_everywhere("badtrunc", BAD_TRUNCATED, "json parse error");
}

fn deep_doc(depth: usize) -> String {
    format!(
        "import {{ JSON }} from \"@std/json\";\nfn Main(): Int {{\n    print(JSON.stringify(JSON.parse(\"{}\")));\n    return 0;\n}}\n",
        "[".repeat(depth) + &"]".repeat(depth)
    )
}

#[test]
fn json_max_depth_fails_cleanly() {
    expect_failure_everywhere("depth", &deep_doc(70), "json max depth exceeded");
}

const TYPED_BASIC: &str = r#"import { JSON } from "@std/json";
struct Point {
    let x: Int = 0;
    let y: Int = 0;
    let label: String = "";
}
fn Main(): Int {
    let p = JSON.parse<Point>("\{\"y\": 7, \"x\": 3, \"label\": \"hi\", \"extra\": [1, 2]}");
    print(p.x);
    print(p.y);
    print(p.label);
    return 0;
}
"#;

#[test]
fn json_parse_typed_basic() {
    expect_backends("typedbasic", TYPED_BASIC, "3\n7\nhi\n");
}

const TYPED_PARITY: &str = r#"import { JSON } from "@std/json";
struct Item {
    let id: Int = 0;
    let name: String = "";
    let score: Float = 0.0;
    let ok: Bool = false;
}
fn Main(): Int {
    let doc = "\{\"id\": 7, \"name\": \"name-7\", \"tags\": [\"a\", \"b\", \"c\"], \"score\": 1.5, \"ok\": true}";
    let t = JSON.parse<Item>(doc);
    let m = JSON.parseObject(doc);
    print(t.id);
    print(m.get("id"));
    print(t.name);
    print(m.get("name"));
    print(t.score);
    print(m.get("score"));
    print(t.ok);
    print(m.get("ok"));
    return 0;
}
"#;

#[test]
fn json_parse_typed_matches_untyped() {
    expect_backends(
        "typedparity",
        TYPED_PARITY,
        "7\n7\nname-7\nname-7\n1.5\n1.5\ntrue\ntrue\n",
    );
}

const TYPED_NESTED: &str = r#"import { JSON } from "@std/json";
struct Outer {
    let count: Int = 0;
    let items: Any = null;
    let meta: Any = null;
}
fn Main(): Int {
    let o = JSON.decode<Outer>("\{\"count\": 42, \"items\": [10, 20], \"meta\": \{\"ok\": true}}");
    print(o.count);
    print(o.items[1]);
    let sub = JSON.asMap(o.meta);
    print(sub.get("ok"));
    return 0;
}
"#;

#[test]
fn json_decode_typed_nested() {
    expect_backends("typednested", TYPED_NESTED, "42\n20\ntrue\n");
}

const TYPED_ERRORS: &str = r#"import { JSON } from "@std/json";
struct Point {
    let x: Int = 0;
}
fn Main(): Int {
    let p = JSON.parse<Point>("[1, 2]");
    print(p.x);
    return 0;
}
"#;

#[test]
fn json_parse_typed_rejects_non_object() {
    expect_failure_everywhere("typedobj", TYPED_ERRORS, "typed decode needs a JSON object");
}

const TYPED_RFC: &str = r#"import { JSON } from "@std/json";
struct Rec {
    let neg: Int = 0;
    let pi: Float = 0.0;
    let esc: String = "";
    let empty: Any = null;
    let nest: Any = null;
}
fn Main(): Int {
    let r = JSON.parse<Rec>("\{\"neg\": -12, \"pi\": 3.25, \"esc\": \"a\\\"b\\\\c\\u00e9\", \"empty\": \{\"a\": []}, \"nest\": [[1], \{\"k\": null}]}");
    print(r.neg);
    print(r.pi);
    print(r.esc);
    print(JSON.stringify(r.empty));
    print(JSON.stringify(r.nest));
    return 0;
}
"#;

#[test]
fn json_parse_typed_rfc8259() {
    expect_backends(
        "typedrfc",
        TYPED_RFC,
        "-12\n3.25\na\"b\\cé\n{\"a\":[]}\n[[1],{\"k\":null}]\n",
    );
}
