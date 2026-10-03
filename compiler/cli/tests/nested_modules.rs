use std::path::PathBuf;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-nested-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_config(dir: &std::path::Path, body: &str) {
    std::fs::write(dir.join("Project.config"), body).unwrap();
}

#[test]
fn nested_std_submodule_runs() {
    let dir = fresh_dir("std");
    write_config(
        &dir,
        "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    }\n}\n",
    );
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src").join("main.rnx"),
        "import { encodeRequestLine, statusCode, isSuccess } from \"@std/net/http\";\nfn Main(): Int {\n    print(encodeRequestLine(\"GET\", \"/health\"));\n    let code = statusCode(\"HTTP/1.1 404 Not Found\");\n    print(code);\n    print(isSuccess(200));\n    print(isSuccess(code));\n    return 0;\n}\n",
    )
    .unwrap();
    let run = std::process::Command::new(rnx()).arg("run").current_dir(&dir).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains("GET /health HTTP/1.1"), "{stdout}");
    assert!(stdout.contains("404"), "{stdout}");
    assert!(stdout.contains("true"), "{stdout}");
    assert!(stdout.contains("false"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nested_package_submodule_runs() {
    let dir = fresh_dir("pkg");
    let lib = dir.join("dep");
    let app = dir.join("app");
    std::fs::create_dir_all(lib.join("src").join("ops")).unwrap();
    std::fs::create_dir_all(app.join("src")).unwrap();
    write_config(
        &lib,
        "export default {\n    project: {\n        name: \"dep\",\n        version: \"0.1.0\",\n        entry: \"src/lib.rnx\"\n    }\n}\n",
    );
    write_config(
        &app,
        "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        dep: { path: \"../dep\" }\n    }\n}\n",
    );
    std::fs::write(lib.join("src").join("lib.rnx"), "fn base(): Int { return 40; }\n").unwrap();
    std::fs::write(
        lib.join("src").join("ops").join("deep.rnx"),
        "import { base } from \"../lib\";\nfn deep(): Int { return base() + 2; }\n",
    )
    .unwrap();
    std::fs::write(
        app.join("src").join("main.rnx"),
        "import { deep } from \"dep/ops/deep\";\nfn Main(): Int {\n    print(deep());\n    return 0;\n}\n",
    )
    .unwrap();
    let run = std::process::Command::new(rnx()).arg("run").current_dir(&app).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nested_doc_json_carries_slash_paths() {
    let dir = fresh_dir("doc");
    write_config(
        &dir,
        "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    }\n}\n",
    );
    std::fs::create_dir_all(dir.join("src").join("util")).unwrap();
    std::fs::write(
        dir.join("src").join("main.rnx"),
        "import { v } from \"./util/vec\";\nfn Main(): Int { return v(); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("util").join("vec.rnx"),
        "/** Vec helper. */\nexport fn v(): Int { return 7; }\n",
    )
    .unwrap();
    let out = std::process::Command::new(rnx())
        .arg("doc")
        .arg("--json")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let api = std::fs::read_to_string(dir.join("target").join("doc").join("api.json")).unwrap();
    assert!(api.contains("\"name\":\"util.vec\""), "{api}");
    assert!(api.contains("\"path\":\"util/vec\""), "{api}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stdlib_doc_json_lists_nested_module() {
    let dir = fresh_dir("stdjson");
    let out = std::process::Command::new(rnx())
        .arg("doc")
        .arg("--json")
        .arg("--stdlib")
        .arg("--out-dir")
        .arg(dir.join("api"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let api = std::fs::read_to_string(dir.join("api").join("api.json")).unwrap();
    assert!(api.contains("\"name\":\"net/http\""), "{api}");
    assert!(api.contains("\"name\":\"net\""), "{api}");
    let _ = std::fs::remove_dir_all(&dir);
}
