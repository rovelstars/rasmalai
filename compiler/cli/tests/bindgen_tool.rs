#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::PathBuf;
use std::process::Command;

fn rnx() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rnx"))
}

fn tool_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("rnx-bindgen")
        .join("src")
        .join("main.rnx")
}

#[test]
fn bindgen_tool_checks_clean() {
    let out = Command::new(rnx())
        .arg("check")
        .arg(tool_src())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

const SORT_DRIVER: &str = r#"import { sortFns } from "./emit";
import { FnDecl } from "./extract";
import { JSON } from "@std/json";
fn Main(): Int {
    let m = JSON.parseObject("\{\"b\": \"compressBound\", \"a\": \"compress2\"}");
    let x = "" + m.get("b");
    let y = "" + m.get("a");
    if !(y < x) {
        print("cmp-broken");
        return 1;
    }
    let fns: Array<FnDecl> = [];
    fns.push(FnDecl(x, "Int", []));
    fns.push(FnDecl(y, "Int", []));
    let s = sortFns(fns);
    if s[0].name != "compress2" || s[1].name != "compressBound" {
        print("sort-broken");
        return 1;
    }
    print("sort-ok");
    return 0;
}
"#;

#[test]
fn bindgen_case_and_rename_emit_idiomatic_wrappers() {
    let dir = crate::scratch::dir("bindgen-case");
    std::fs::write(
        dir.join("mini.h"),
        "int mini_open(const char *db_path, int flags);\nvoid mini_close(int handle);\n#define mini_version 3\n",
    )
    .unwrap();
    let out = dir.join("gen");
    let run = Command::new(rnx())
        .arg("bindgen")
        .arg("--header")
        .arg(dir.join("mini.h"))
        .arg("--native-lib")
        .arg("mini")
        .arg("--pkg-name")
        .arg("mini")
        .arg("--out")
        .arg(&out)
        .arg("--prefix")
        .arg("mini_")
        .arg("--case")
        .arg("camel")
        .arg("--rename")
        .arg("mini_open=openDb")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let generated = std::fs::read_to_string(out.join("bindgen.rnx")).unwrap();
    assert!(generated.contains("unsafe fn openDb("), "{generated}");
    assert!(generated.contains("unsafe fn miniClose("), "{generated}");
    assert!(
        generated.contains(
            "fn mini_open(dbPath: Pointer<Byte>, flags: Int): Int as __rnx_mini_open"
        ),
        "{generated}"
    );
    assert!(
        generated.contains("return __rnx_mini_open(dbPath, flags);"),
        "{generated}"
    );
    assert!(
        generated.contains("export const mini_version: Int = 3"),
        "{generated}"
    );
    let fmt = Command::new(rnx())
        .arg("fmt")
        .arg("--check")
        .arg(out.join("bindgen.rnx"))
        .output()
        .unwrap();
    assert!(
        fmt.status.success(),
        "{}",
        String::from_utf8_lossy(&fmt.stderr)
    );
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"mini\", version: \"0.1.0\" },\n    entries: { main: \"src/main.rnx\" }\n}\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src").join("main.rnx"),
        "import { openDb } from \"../gen/bindgen.rnx\"\nfn Main(): Int {\n    return 0;\n}\n",
    )
    .unwrap();
    let check = Command::new(rnx())
        .arg("check")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn bindgen_sort_orders_json_derived_names() {
    let dir = crate::scratch::dir("bindgen");
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("rnx-bindgen")
        .join("src");
    std::fs::write(dir.join("sort_driver.rnx"), SORT_DRIVER).unwrap();
    std::fs::write(
        dir.join("emit.rnx"),
        std::fs::read(src_dir.join("emit.rnx")).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.join("extract.rnx"),
        std::fs::read(src_dir.join("extract.rnx")).unwrap(),
    )
    .unwrap();
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = Command::new(rnx())
            .arg("run")
            .arg("--backend")
            .arg(backend)
            .arg(dir.join("sort_driver.rnx"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{backend}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "sort-ok\n",
            "{backend}"
        );
    }
}
