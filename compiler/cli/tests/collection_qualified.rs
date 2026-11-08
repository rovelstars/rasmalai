#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("repo root")
}

fn check(lib: &str, main: &str) -> (bool, String) {
    let dir = crate::scratch::dir("collection-qualified");
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"cq\", version: \"0.1.0\" },\n    entries: { main: \"main.rnx\" },\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib.rnx"), lib).unwrap();
    std::fs::write(dir.join("main.rnx"), main).unwrap();
    let out = Command::new(rnx())
        .arg("check")
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env("RNX_VENDOR_DIR", repo().join("compiler/stdlib/src"))
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

const LIB: &str = r#"class Maker {
    let v: Int;
    init() { this.v = 1; }
    fn go(): Int throws { throw 1; }
    static fn make(): Maker throws { throw 1; }
}
"#;

#[test]
fn imported_instance_method_rejects_throwing_collection_block() {
    let (ok, text) = check(
        LIB,
        "import { Maker } from \"./lib\";\n\
         fn F(children: Array<Int>): Int { return children.length(); }\n\
         fn Main(): Int { let m = new Maker(); let n = F { m.go() }; return n; }\n",
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("a throwing call cannot appear in a collection block"),
        "{text}"
    );
}

#[test]
fn imported_static_method_rejects_throwing_collection_block() {
    let (ok, text) = check(
        LIB,
        "import { Maker } from \"./lib\";\n\
         fn F(children: Array<Int>): Int { return children.length(); }\n\
         fn Main(): Int { let n = F { Maker.make() }; return n; }\n",
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("a throwing call cannot appear in a collection block"),
        "{text}"
    );
}
