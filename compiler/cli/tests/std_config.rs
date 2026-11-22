#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn vendor() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("compiler/stdlib/src"))
        .expect("repo stdlib")
}

fn run_backend(tag: &str, src: &str, backend: &str) -> std::process::Output {
    let dir = crate::scratch::dir(&format!("std-config-{tag}-{backend}"));
    let prog = dir.join("main.rnx");
    std::fs::write(&prog, src).unwrap();
    Command::new(rnx())
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(&prog)
        .env("NO_COLOR", "1")
        .env("RNX_VENDOR_DIR", vendor())
        .output()
        .expect("run rnx")
}

fn expect_everywhere(tag: &str, src: &str, want: &str) {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run_backend(tag, src, backend);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{tag} {backend} failed: {stderr}");
        assert_eq!(stdout, want, "{tag} {backend} stdout");
    }
}

const PARSE: &str = r#"import { Config } from "@std/config";

fn Main(): Int {
    let text = "const base = \{ a: 1, b: true, native: \"lib\" }\nconst lib = target.os == \"windows\" ? \"zlibstatic\" : \"z\"\nexport default \{ project: \{ name: \"demo\", version: \"0.1.0\", mask: 0xAB, bits: 0b1010 }, deps: \{ ...base, z: lib }, list: [1, 2, 3] }";
    let doc = Config.parseWith(text, "windows", "x86_64", "msvc");
    let project = doc.object("project");
    println!(project?.text("name"), project?.text("version"));
    println!(project?.number("mask"), project?.number("bits"));
    let deps = doc.object("deps");
    println!(deps?.number("a"), deps?.flag("b"), deps?.text("z"), deps?.text("native"));
    println!(doc.list("list")?.number(2));
    let rendered = Config.stringify(doc);
    let again = Config.parse(rendered);
    println!(Config.stringify(again) == rendered);
    return 0;
}
"#;

const PARSE_WANT: &str = "demo 0.1.0\n171 10\n1 true zlibstatic lib\n3\ntrue\n";

const REJECT: &str = r#"import { Config } from "@std/config";

fn Main(): Int {
    let calls = Config.tryParse("export default \{ x: f() }");
    let floats = Config.tryParse("export default \{ x: 1.5 }");
    let interp = Config.tryParse("export default \{ x: \"a\{b}\" }");
    let imports = Config.tryParse("import \{ Map } from \"@std/collections\";\nexport default \{ x: 1 }");
    println!(calls.isErr(), floats.isErr(), interp.isErr(), imports.isErr());
    return 0;
}
"#;

const REJECT_WANT: &str = "true true true true\n";

const ABORT: &str = r#"import { Config } from "@std/config";

fn Main(): Int {
    let doc = Config.parse("export default \{ x: f() }");
    return 0;
}
"#;

const CANON: &str = r#"import { Config } from "@std/config";

fn Main(): Int {
    let doc = Config.parse("export default \{ name: \"demo\", count: 2, flag: true, nested: \{ x: 1 }, list: [\"a\", \"b\"] }");
    print!(Config.stringify(doc));
    return 0;
}
"#;

const CANON_WANT: &str = "export default {\n    name: \"demo\",\n    count: 2,\n    flag: true,\n    nested: {\n        x: 1,\n    },\n    list: [\"a\", \"b\"],\n}\n";

const MANIFEST: &str = r#"import { Config } from "@std/config";

fn Main(): Int {
    let text = "const libs = \{ zlib: \{ native: \"z\", system: true } }\nexport default \{ project: \{ name: \"colony\", version: \"0.4.0\" }, dependencies: \{ ...libs, gui: target.arch == \"aarch64\" ? \{ native: \"gui_arm\" } : \{ native: \"gui_x86\" } } }";
    let doc = Config.parseWith(text, "linux", "x86_64", "gnu");
    let deps = doc.object("dependencies");
    println!(doc.object("project")?.text("name"), deps?.object("zlib")?.flag("system"));
    println!(deps?.object("gui")?.text("native"));
    return 0;
}
"#;

const MANIFEST_WANT: &str = "colony true\ngui_x86\n";

#[test]
fn config_parses_and_writes_on_every_backend() {
    expect_everywhere("parse", PARSE, PARSE_WANT);
}

#[test]
fn config_rejects_code_outside_the_subset() {
    expect_everywhere("reject", REJECT, REJECT_WANT);
}

#[test]
fn config_writer_matches_the_documented_canonical_form() {
    expect_everywhere("canon", CANON, CANON_WANT);
}

#[test]
fn config_reads_a_realistic_manifest_shape() {
    expect_everywhere("manifest", MANIFEST, MANIFEST_WANT);
}

#[test]
fn config_parse_aborts_with_the_parse_error() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run_backend("abort", ABORT, backend);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "abort {backend} unexpectedly succeeded");
        assert!(
            stderr.contains("config parse error") && stderr.contains("unknown config name `f`"),
            "abort {backend} stderr: {stderr}"
        );
    }
}
