use std::path::PathBuf;

fn fmt(src: &str) -> String {
    frontend::fmt::format_source(src).unwrap_or_else(|e| panic!("format failed: {e}"))
}

fn write_tmp(tag: &str, name: &str, src: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-fmt-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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
    let (dir, file) = write_tmp("check", "main.rnx", UNFORMATTED_SRC);
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
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_fmt_diff_output() {
    let (dir, file) = write_tmp("diff", "main.rnx", UNFORMATTED_SRC);
    let name = file.to_string_lossy().into_owned();
    let out = run_fmt(&["--diff", &name]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains('-') && text.contains('+'), "no diff markers in:\n{text}");
    assert!(text.contains("let x = 1 + 2;"), "missing formatted line in:\n{text}");
    let _ = std::fs::remove_dir_all(&dir);
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
    let (dir, file) = write_tmp("manifest", "Project.config", MANIFEST_SRC);
    let out = run_fmt(&[file.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("name: \"demo\""), "{text}");
    assert!(text.contains("physics_2d: {"), "{text}");
    assert!(text.contains("path: \"../x\""), "{text}");
    assert!(text.contains("// inline"), "{text}");
    let check = run_fmt(&["--check", file.to_str().unwrap()]);
    assert!(check.status.success(), "not idempotent: {}", String::from_utf8_lossy(&check.stderr));
    let _ = std::fs::remove_dir_all(&dir);
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
