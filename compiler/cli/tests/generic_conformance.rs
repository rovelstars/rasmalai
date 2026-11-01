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

fn check(src: &str) -> (bool, String) {
    let dir = crate::scratch::dir("generic-conformance");
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"gc\", version: \"0.1.0\" },\n    entries: { main: \"main.rnx\" },\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.rnx"), src).unwrap();
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

#[test]
fn a_generic_interface_does_not_reject_a_user_class() {
    let (ok, text) = check(
        "class T { let n: Int = 0 }\nfn Main(): Int { let t = new T() return t.n }\n",
    );
    assert!(ok, "{text}");
}

#[test]
fn enum_variant_named_like_a_method_stays_exhaustive() {
    let (ok, text) = check(
        "enum A { tap, toggle, dismiss }\n\
         class T { fn toggle(s: String): Int { return 1 } }\n\
         fn f(a: A): Int {\n\
             switch a {\n\
                 case .tap: return 1;\n\
                 case .toggle: return 2;\n\
                 case .dismiss: return 4;\n\
             }\n\
             return 0\n\
         }\n\
         fn Main(): Int { return f(A.tap) }\n",
    );
    assert!(ok, "{text}");
}

#[test]
fn an_enum_typed_array_is_a_valid_field() {
    let (ok, text) = check(
        "enum R { a, b }\n\
         class T {\n\
             let xs: Array<R> = []\n\
             fn add(r: R) { this.xs.push(r) }\n\
             fn len(): Int { return this.xs.length() }\n\
         }\n\
         fn Main(): Int { let t = new T() t.add(R.a) return t.len() }\n",
    );
    assert!(ok, "{text}");
}
