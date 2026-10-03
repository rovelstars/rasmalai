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
fn bindgen_sort_orders_json_derived_names() {
    let dir = std::env::temp_dir().join(format!("rnx-bindgen-sort-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("rnx-bindgen")
        .join("src");
    std::fs::create_dir_all(&dir).unwrap();
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
    let _ = std::fs::remove_dir_all(&dir);
}
