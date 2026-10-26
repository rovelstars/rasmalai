#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn fmt(src: &str) -> String {
    frontend::fmt::format_source(src).unwrap_or_else(|e| panic!("format failed: {e}"))
}

fn minify_src(src: &str) -> String {
    frontend::minify::minify_source(src)
        .unwrap_or_else(|e| panic!("minify failed: {e:?}"))
}

fn scrub_spans_and_docs(dbg: &str) -> String {
    let mut out = String::with_capacity(dbg.len());
    let bytes = dbg.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if dbg[i..].starts_with("Span { start: ") {
            out.push_str("Span{}");
            let tail = &dbg[i + "Span { start: ".len()..];
            match tail.find(" }") {
                Some(end) => i += "Span { start: ".len() + end + 2,
                None => i = bytes.len(),
            }
        } else if dbg[i..].starts_with("docs: \"") {
            out.push_str("docs: \"\"");
            let mut j = i + "docs: \"".len();
            while j < bytes.len() {
                if bytes[j] == b'\\' {
                    j += 2;
                } else if bytes[j] == b'"' {
                    j += 1;
                    break;
                } else {
                    j += 1;
                }
            }
            i = j;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn decl_signature(src: &str) -> String {
    let module = frontend::parser::Parser::parse_module(src)
        .unwrap_or_else(|e| panic!("parse failed: {e}"));
    let kinds: Vec<String> = module
        .decls
        .iter()
        .map(|d| scrub_spans_and_docs(&format!("{:?}", d.node)))
        .collect();
    format!("{} decls\n{}", kinds.len(), kinds.join("\n"))
}

fn write_tmp(tag: &str, name: &str, src: &str) -> (crate::scratch::Scratch, std::path::PathBuf) {
    let dir = crate::scratch::dir(&format!("fmt-{tag}"));
    let file = dir.join(name);
    std::fs::write(&file, src).unwrap();
    (dir, file)
}

fn run_fmt(args: &[&str]) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    std::process::Command::new(rnx)
        .arg("fmt")
        .args(args)
        .output()
        .unwrap()
}

const MESSY_SRC: &str = "fn total( xs:Array )  :Int{\n\n\n    let sum=0;\n    let i=0;\n    while(i<xs.length){\n        let j=0;\n        while(j<2){\n            sum=sum+xs[i]*j;\n            j=j+1;\n        }\n        i=i+1;\n    }\n    return sum;\n}\n";

#[test]
fn test_fmt_idempotency() {
    let once = fmt(MESSY_SRC);
    let twice = fmt(&once);
    assert_eq!(once, twice, "second pass differs:\n{once}\n---\n{twice}");
    assert!(once.contains("    let sum = 0;"), "{once:?}");
    assert!(!once.contains("  \n"), "trailing whitespace remains");
    assert!(!once.contains("\n\n\n"), "blank lines not collapsed");
}

const COMMENT_SRC: &str = "//! module docs stay here\n/// triple slash stays a plain comment\n// leading line comment\nfn Main(): Int {\n    // inner line comment\n    /* inline block */ let x = 1;\n    /*\n       multi line block\n       keeps stars\n    */\n    return x; // trailing comment\n}\n";

#[test]
fn test_fmt_comment_preservation() {
    let out = fmt(COMMENT_SRC);
    for comment in [
        "//! module docs stay here",
        "/// triple slash stays a plain comment",
        "// leading line comment",
        "// inner line comment",
        "/* inline block */",
        "multi line block",
        "keeps stars",
        "// trailing comment",
    ] {
        assert!(out.contains(comment), "lost comment {comment:?} in:\n{out}");
    }
    for line in out.lines() {
        assert_eq!(line, line.trim_end(), "trailing whitespace on {line:?}");
    }
    assert!(out.lines().any(|l| l == "    // inner line comment"), "bad indent in:\n{out}");
    let again = fmt(&out);
    assert_eq!(out, again, "comment formatting not idempotent");
}

const UNFORMATTED_SRC: &str = "fn Main():Int{\nlet x=1+2;\nreturn x;\n}\n";

#[test]
fn test_fmt_check_flag_exit_codes() {
    let (_g, file) = write_tmp("check", "main.rnx", UNFORMATTED_SRC);
    let name = file.to_string_lossy().into_owned();
    let check = run_fmt(&["--check", &name]);
    assert_eq!(check.status.code(), Some(1), "expected exit 1");
    assert!(
        String::from_utf8_lossy(&check.stderr).contains("main.rnx"),
        "stderr should name the file"
    );
    let fix = run_fmt(&[&name]);
    assert!(fix.status.success(), "{}", String::from_utf8_lossy(&fix.stderr));
    let recheck = run_fmt(&["--check", &name]);
    assert_eq!(recheck.status.code(), Some(0), "expected exit 0");
}

#[test]
fn test_fmt_diff_output() {
    let (_g, file) = write_tmp("diff", "main.rnx", UNFORMATTED_SRC);
    let name = file.to_string_lossy().into_owned();
    let out = run_fmt(&["--diff", &name]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains('-') && text.contains('+'), "no diff markers in:\n{text}");
    assert!(text.contains("let x = 1 + 2;"), "missing formatted line in:\n{text}");
}

const BRACE_SRC: &str = "fn add(a:Int,b:Int):Int{\nlet x=a+b*2;\nif(x>0){\nreturn x;}\nreturn 0;\n}\n";
const BRACE_EXPECTED: &str = "fn add(a: Int, b: Int): Int {\n    let x = a + b * 2;\n    if x > 0 {\n        return x;\n    }\n    return 0;\n}\n";

#[test]
fn test_fmt_operators_and_braces() {
    assert_eq!(fmt(BRACE_SRC), BRACE_EXPECTED);
}

const GENERIC_SRC: &str = "class Colony {\n    let crew: Array <Miner>;\n    let prices: Map<String, Array<Int>>;\n    fn census(belt: Array < Asteroid >): Int {\n        let i = 0;\n        while i < belt.length {\n            i = i + 1;\n        }\n        if i >= 10 && i <= 20 {\n            return 1;\n        }\n        return 0;\n    }\n}\n";

#[test]
fn test_fmt_generic_brackets_stay_tight() {
    let out = fmt(GENERIC_SRC);
    for tight in [
        "let crew: Array<Miner>;",
        "let prices: Map<String, Array<Int>>;",
        "fn census(belt: Array<Asteroid>): Int {",
    ] {
        assert!(out.contains(tight), "missing {tight:?} in:\n{out}");
    }
    for spaced in ["i < belt.length", "i >= 10", "i <= 20"] {
        assert!(out.contains(spaced), "comparison lost spacing {spaced:?} in:\n{out}");
    }
    let again = fmt(&out);
    assert_eq!(out, again, "generic formatting not idempotent");
}

const MANIFEST_SRC: &str = "// sample\nexport default {\n    project: {\n        name: \"demo\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        physics_2d: { path: \"../x\" } // inline\n    }\n}\n";

#[test]
fn test_fmt_manifest_normalizes() {
    let (_g, file) = write_tmp("manifest", "Project.config", MANIFEST_SRC);
    let out = run_fmt(&[file.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("name: \"demo\""), "{text}");
    assert!(text.contains("physics_2d: {"), "{text}");
    assert!(text.contains("path: \"../x\""), "{text}");
    assert!(text.contains("// inline"), "{text}");
    let check = run_fmt(&["--check", file.to_str().unwrap()]);
    assert!(check.status.success(), "not idempotent: {}", String::from_utf8_lossy(&check.stderr));
}

const ARROW_SRC: &str = "fn F():Int{\nlet a=(x)=>x*2;\nlet b=x=>x+1;\nlet c=(a: Int,b: Int):Int=>a+b;\nlet d=()=>{return 42;};\n}\n";
#[test]
fn test_fmt_arrow_lambdas_round_trip() {
    let once = fmt(ARROW_SRC);
    assert!(once.contains("let a = (x) => x * 2;"), "{once:?}");
    assert!(once.contains("let b = x => x + 1;"), "{once:?}");
    assert!(once.contains("let c = (a: Int, b: Int): Int => a + b;"), "{once:?}");
    assert!(once.contains("let d = () => {"), "{once:?}");
    assert!(!once.contains("fn("), "formatter must never emit fn()=>: {once:?}");
    let twice = fmt(&once);
    assert_eq!(once, twice, "arrow formatting not idempotent");
    frontend::parser::Parser::parse_module(&once)
        .unwrap_or_else(|e| panic!("formatted arrows fail to parse: {e}"));
}

const DOCSTRING_SRC: &str = "/**\n * Title of component\n *\n * Example:\n *   let x = 10;\n *   let y = x * 2;\n *\n * - Item 1\n *   - Subitem A\n */\nfn sample_doc(): Void {}\n";

#[test]
fn test_fmt_docstring_verbatim() {
    let out = fmt(DOCSTRING_SRC);
    assert_eq!(out, DOCSTRING_SRC, "doc comment mangled:\n{out}");
    for line in out.lines() {
        assert_eq!(line, line.trim_end(), "trailing whitespace on {line:?}");
    }
}

const GENERIC_DECL_SRC: &str = "class ArrayIter<T> with Iterator<T> {\n    let arr: Array<T>;\n}\nclass Box<T> {\n    let v: T;\n}\ninterface Stack<T> {\n    fn push(item: T);\n}\nfn use_map(m: Map<String, Int>): Int {\n    return m.len();\n}\n";

#[test]
fn test_fmt_generic_with_extends_snug() {
    let out = fmt(GENERIC_DECL_SRC);
    for tight in [
        "class ArrayIter<T> with Iterator<T> {",
        "class Box<T> {",
        "interface Stack<T> {",
        "fn use_map(m: Map<String, Int>): Int {",
    ] {
        assert!(out.contains(tight), "missing {tight:?} in:\n{out}");
    }
    assert!(!out.contains(" < "), "spurious generic spacing in:\n{out}");
    let again = fmt(&out);
    assert_eq!(out, again, "generic decl formatting not idempotent");
    frontend::parser::Parser::parse_module(&out)
        .unwrap_or_else(|e| panic!("formatted generics fail to parse: {e}"));
}

const NATIVE_ALIAS_SRC: &str = "import {\n fn sqlite3_column_text(arg0: Pointer<Byte>, iCol: Int): Pointer<Byte> as __rnx_sqlite3_column_text,\n fn sqlite3_backup_init(pDest: Pointer<Byte>, zDestName: Pointer<Byte>): Pointer<Pointer<Byte>> as __rnx_sqlite3_backup_init\n} from native \"sqlite3\"\n";

#[test]
fn test_fmt_native_alias_pointer_return_stays_tight() {
    let out = fmt(NATIVE_ALIAS_SRC);
    for tight in [
        "fn sqlite3_column_text(arg0: Pointer<Byte>, iCol: Int): Pointer<Byte> as __rnx_sqlite3_column_text,",
        "fn sqlite3_backup_init(pDest: Pointer<Byte>, zDestName: Pointer<Byte>): Pointer<Pointer<Byte>> as __rnx_sqlite3_backup_init",
    ] {
        assert!(out.contains(tight), "missing {tight:?} in:\n{out}");
    }
    assert!(!out.contains("Pointer <"), "spurious generic spacing in:\n{out}");
    let again = fmt(&out);
    assert_eq!(out, again, "native alias formatting not idempotent");
    frontend::parser::Parser::parse_module(&out)
        .unwrap_or_else(|e| panic!("formatted native alias fails to parse: {e}"));
}

const SHIFT_SRC: &str = "fn Main(): Int {\n    let x = 1<<3;\n    let y = x>>1;\n    let z = x>>>1;\n    x<<=1;\n    return x;\n}\n";

#[test]
fn test_fmt_shift_operators_stay_joined() {
    let out = fmt(SHIFT_SRC);
    for tight in ["1 << 3", "x >> 1", "x >>> 1", "x <<= 1"] {
        assert!(out.contains(tight), "missing {tight:?} in:\n{out}");
    }
    let again = fmt(&out);
    assert_eq!(out, again, "shift formatting not idempotent");
}

const UNARY_SRC: &str = "import { Clock } from \"@std/time\";\nfn f(g: Array<fn(Int): Bool>, ok: Bool): Int {\n    if !ok {\n        return (1);\n    }\n    let y = -5;\n    while !ok {\n        ok = !ok;\n    }\n    return -ok;\n}\n";

#[test]
fn test_fmt_unary_and_control_spacing() {
    let out = fmt(UNARY_SRC);
    for tight in [
        "import { Clock } from \"@std/time\";",
        "Array<fn(Int): Bool>",
        "if !ok {",
        "while !ok {",
        "ok = !ok;",
        "return -ok;",
        "return (1);",
    ] {
        assert!(out.contains(tight), "missing {tight:?} in:\n{out}");
    }
    for bad in ["Array <", "if!ok", "while!ok", "! ok", "return(", "return - "] {
        assert!(!out.contains(bad), "bad spacing {bad:?} in:\n{out}");
    }
    let again = fmt(&out);
    assert_eq!(out, again, "unary formatting not idempotent");
    frontend::parser::Parser::parse_module(&out)
        .unwrap_or_else(|e| panic!("formatted unary fails to parse: {e}"));
}

const JOINED_SRC: &str = "import { Map } from \"@std/collections\";\nimport { Clock } from \"@std/time\";\nimport { Aye, Bee } from \"@std/cee\";\nimport { fn f(x: Int): Int as g, fn h(x: Int): Int as k } from native \"libx\";\nexport const K: Int = 1; export const J: Int = 2;\nexport fn alpha(x: Int): Int { return x + 1; } export fn beta(y: Int): Int { let t = y * 2; return t - 1; }\npub class Pair {\n    let a: Int;\n    let b: Int;\n    init(a: Int, b: Int) { this.a = a; this.b = b; }\n    fn sum(): Int { return this.a + this.b; }\n} enum E { Ok(Int), Err(String), }\ninterface I { fn run(): Int; }\nfn flow(n: Int): Int { let i = 0; while i < n { if i == 2 { i = i + 1; } else if i == 3 { i = i + 10; } else { i = i + 100; } i = i + 1; } for x in [1, 2, 3] { i = i + x; } return i; }\nfn looped(ok: Bool): Int { do { print(\"x\"); } while ok; return 0; }\nfn guarded(v: Int?): Int { guard let x = v else { return -1; } return x; }\nfn tried(ok: Bool): Int { try { if ok { return 1; } return 2; } catch(e) { return 3; } finally { print(\"done\"); } return 4; }\nfn switched(x: Int): Int { switch x { case 1: return 10; case 2: return 20; default: return 0; } }\nfn lambdas(): Int { let a = (x) => x * 2; let d = () => { return 42; }; return a(21) + d(); }\nfn checks(v: Any): Int { if v is Int { return 1; } let s = v as String; return 0; }\n";

#[test]
fn test_fmt_converges_on_joined_statements() {
    let formatted = fmt(JOINED_SRC);
    assert_eq!(fmt(&formatted), formatted, "not idempotent");
    let min = minify_src(JOINED_SRC);
    assert!(
        !min.contains('\n') || min.contains('"'),
        "minified fixture should carry no newlines outside strings: {min:?}"
    );
    let refmt = fmt(&min);
    assert_eq!(fmt(&refmt), refmt, "reformat not idempotent");
    assert_eq!(
        refmt, formatted,
        "fmt(minify(x)) != fmt(x):\n--- fmt(x):\n{formatted}\n--- fmt(minify(x)):\n{refmt}"
    );
    assert_eq!(
        decl_signature(&refmt),
        decl_signature(&formatted),
        "AST drift between fmt(x) and fmt(minify(x))"
    );
    for line in refmt.lines() {
        assert_eq!(line, line.trim_end(), "trailing whitespace on {line:?}");
    }
    assert!(
        !refmt.lines().any(|l| l.len() > 120),
        "unbroken long line remains"
    );
    for stay in [
        "import { Map } from \"@std/collections\";",
        "} else if",
        "} else {",
        "} while ok;",
        "} catch (e) {",
        "} finally {",
        "} from native \"libx\";",
    ] {
        assert!(refmt.contains(stay), "lost same-line continuation {stay:?} in:\n{refmt}");
    }
}

#[test]
fn test_fmt_breaks_semiless_import_tail() {
    let src = "import { fn f(x: Int): Int as g } from native \"libx\" export const K: Int = 1;\n";
    let out = fmt(src);
    assert!(
        out.contains("} from native \"libx\"\nexport const K: Int = 1;"),
        "import tail not split:\n{out}"
    );
    assert_eq!(fmt(&out), out, "not idempotent");
    let refmt = fmt(&minify_src(src));
    assert_eq!(
        refmt, out,
        "fmt(minify(x)) != fmt(x):\n--- fmt(x):\n{out}\n--- fmt(minify(x)):\n{refmt}"
    );
    frontend::parser::Parser::parse_module(&out)
        .unwrap_or_else(|e| panic!("split import fails to parse: {e}"));

    let live = "import { fn sqlite3_open(path: Pointer<Byte>): Int as rawOpen } from native \"sqlite3\" export const SQLITE_OK: Int = 0;\n";
    let live_out = fmt(live);
    assert!(
        live_out.contains("} from native \"sqlite3\"\nexport const SQLITE_OK: Int = 0;"),
        "live chunk junction not split:\n{live_out}"
    );
    frontend::parser::Parser::parse_module(&live_out)
        .unwrap_or_else(|e| panic!("live junction fails to parse: {e}"));

    let reexport = "export { Aye } from \"cee\" export const J: Int = 2;\n";
    let re_out = fmt(reexport);
    assert!(
        re_out.contains("} from \"cee\"\nexport const J: Int = 2;"),
        "re-export tail not split:\n{re_out}"
    );
    assert_eq!(fmt(&re_out), re_out, "not idempotent");

    let trailing = "import { Aye } from \"cee\" // trailing\nexport const J: Int = 2;\n";
    let trail_out = fmt(trailing);
    assert!(trail_out.contains("// trailing"), "comment lost:\n{trail_out}");
    frontend::parser::Parser::parse_module(&trail_out)
        .unwrap_or_else(|e| panic!("comment case fails to parse: {e}"));

    let expr = "fn f(): Int { let s = \"a\" + \"b\"; return 0; } export const K: Int = 1;\n";
    let expr_out = fmt(expr);
    assert!(expr_out.contains("\"a\" + \"b\""), "expression string wrongly split:\n{expr_out}");
    frontend::parser::Parser::parse_module(&expr_out)
        .unwrap_or_else(|e| panic!("expression case fails to parse: {e}"));
}

fn corpus_paths() -> Vec<(&'static str, std::path::PathBuf)> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [
        ("stdlib-prelude", root.join("../stdlib/src/prelude/lib.rnx")),
        ("stdlib-collections", root.join("../stdlib/src/collections/lib.rnx")),
        ("stdlib-json", root.join("../stdlib/src/json/lib.rnx")),
        (
            "sqlite-index",
            root.join("tests/fixtures/fmt_convergence/sqlite_index.rnx"),
        ),
        (
            "sqlite-gen",
            root.join("tests/fixtures/fmt_convergence/sqlite_gen.rnx"),
        ),
    ]
    .into_iter()
    .collect()
}

#[test]
fn test_fmt_converges_on_corpus() {
    for (tag, path) in corpus_paths() {
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let formatted = fmt(&src);
        assert_eq!(
            fmt(&formatted),
            formatted,
            "{tag}: fmt output not idempotent"
        );
        let min = minify_src(&src);
        let refmt = fmt(&min);
        assert_eq!(fmt(&refmt), refmt, "{tag}: reformat not idempotent");
        assert_eq!(
            decl_signature(&refmt),
            decl_signature(&formatted),
            "{tag}: AST drift between fmt(x) and fmt(minify(x))"
        );
        assert_eq!(
            minify_src(&refmt),
            minify_src(&formatted),
            "{tag}: token streams differ between fmt(x) and fmt(minify(x))"
        );
    }
}
