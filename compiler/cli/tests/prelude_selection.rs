#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn stock_prelude() -> String {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src/prelude/lib.rnx");
    std::fs::read_to_string(&path).expect("checkout prelude must exist")
}

fn write_package(root: &std::path::Path, name: &str, prelude: &str) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Project.config"),
        format!(
            "export default {{\n    project: {{ name: \"{name}\", version: \"0.1.0\" }},\n    entries: {{ main: \"src/lib.rnx\" }}\n}}\n"
        ),
    )
    .unwrap();
    std::fs::write(root.join("src").join("lib.rnx"), prelude).unwrap();
}

fn write_app(root: &std::path::Path, main: &str, prelude_alias: &str) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Project.config"),
        format!(
            "export default {{\n    project: {{ name: \"app\", version: \"0.1.0\" }},\n    dependencies: {{\n        \"@std/prelude\": {{ alias: \"{prelude_alias}\" }},\n        \"{prelude_alias}\": {{ path: \"deps/prelude\" }}\n    }}\n}}\n"
        ),
    )
    .unwrap();
    std::fs::write(root.join("src").join("main.rnx"), main).unwrap();
}

fn write_plain_app(root: &std::path::Path, main: &str) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Project.config"),
        "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("src").join("main.rnx"), main).unwrap();
}

fn run_build(project: &std::path::Path) -> (i32, String, String) {
    let mut cmd = std::process::Command::new(rnx());
    cmd.args(["build", project.join("src/main.rnx").to_str().unwrap()])
        .current_dir(project)
        .env("RNX_REGISTRY", "http://127.0.0.1:9");
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn run_binary(bin: &std::path::Path) -> i32 {
    std::process::Command::new(bin)
        .output()
        .unwrap()
        .status
        .code()
        .unwrap_or(-1)
}

fn artifact_of(stdout: &str) -> std::path::PathBuf {
    let bin = stdout
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("artifact: "))
        .map(|s| s.trim().trim_end_matches(" (fresh)").to_string())
        .expect("build prints artifact path");
    std::path::PathBuf::from(bin)
}

#[test]
fn local_prelude_alias_is_ambient() {
    let dir = crate::scratch::dir("prelude-alias");
    let mut prelude = stock_prelude();
    prelude.push_str("\nexport fn preludeMarker(): Int {\n    return 42;\n}\n");
    write_package(
        &dir.join("deps").join("prelude"),
        "@local/prelude",
        &prelude,
    );
    write_app(
        &dir,
        "fn Main(): Int {\n    return preludeMarker();\n}\n",
        "@local/prelude",
    );
    let (code, stdout, stderr) = run_build(&dir);
    assert_eq!(code, 0, "stdout: {stdout}stderr: {stderr}");
    let bin = artifact_of(&stdout);
    let exec = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(exec.status.code(), Some(42));
}

#[test]
fn broken_prelude_reports_the_missing_name() {
    let dir = crate::scratch::dir("prelude-broken");
    write_package(
        &dir.join("deps").join("prelude"),
        "@local/broken",
        "export fn hello(): Int { return 1; }\n",
    );
    write_app(
        &dir,
        "fn Main(): Int {\n    return 0;\n}\n",
        "@local/broken",
    );
    let (code, stdout, stderr) = run_build(&dir);
    assert_ne!(code, 0, "a broken prelude must fail the build");
    let combined = format!("{stdout}{stderr}");
    assert!(combined.contains("E108"), "{combined}");
    assert!(combined.contains("Int"), "{combined}");
    assert!(combined.contains("@local/broken"), "{combined}");
}

const STOCK_SURFACE: &str = "async fn work(v: Int): Int {\n    return v;\n}\n\nasync fn run(): Int {\n    let a = await work(20);\n    return a + 22;\n}\n\nfn Main(): Int {\n    let d: Date = new Date();\n    let got = run().wait().unwrapOr(-1);\n    let ps: Array<Promise<Int>> = [Promise.resolve(1), Promise.reject<Int>(\"bad\")];\n    let settled: Array<PromiseResult<Int>> = Promise.allSettled<Int>(ps).wait().unwrap();\n    let cwd = rnx.cwd();\n    if got == 42 && settled[0].status == \"fulfilled\" && settled[1].status == \"rejected\" && cwd.length() > 0 {\n        return 0;\n    }\n    return 1;\n}\n";

#[test]
fn stock_prelude_surface_runs() {
    let dir = crate::scratch::dir("stock-surface");
    write_plain_app(&dir, STOCK_SURFACE);
    let (code, stdout, stderr) = run_build(&dir);
    assert_eq!(code, 0, "stdout: {stdout}stderr: {stderr}");
    let bin = artifact_of(&stdout);
    assert_eq!(run_binary(&bin), 0);
}

#[test]
fn uefi_prelude_alias_selects_the_target_surface() {
    let uefi = "x86_64-unknown-uefi";
    let dir = crate::scratch::dir("uefi-prelude");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let prelude = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../stdlib/uefi_prelude")
        .canonicalize()
        .expect("uefi prelude package must exist");
    std::fs::write(
        dir.join("Project.config"),
        format!(
            "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\",\n        targets: [\"{uefi}\"]\n    }},\n    dependencies: {{\n        \"@std/prelude\": {{ alias: \"@uefi/prelude\" }},\n        \"@uefi/prelude\": {{ path: \"{}\" }}\n    }}\n}}\n",
            prelude.display()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("main.rnx"),
        "fn parse(text: String): Result<Int, String> {\n    if text == \"42\" {\n        return Result.Ok<Int, String>(42);\n    }\n    return Result.Err<Int, String>(\"not 42\");\n}\n\nfn Main(): Int {\n    let xs: Array<Int> = [1, 2, 3];\n    let hit = xs.find((n: Int): Bool => n == 2) ?? 0;\n    let label = \" uefi \".trim().toUpperCase();\n    let today: Date = new Date();\n    let parsed = parse(\"42\");\n    if label.length() > 0 && today != null && parsed.isOk() {\n        return 42;\n    }\n    return hit;\n}\n",
    )
    .unwrap();

    let mut cmd = std::process::Command::new(rnx());
    cmd.args([
        "build",
        dir.join("src/main.rnx").to_str().unwrap(),
        "--target",
        uefi,
        "--release",
        "--emit-obj",
    ])
    .current_dir(&dir)
    .env("RNX_REGISTRY", "http://127.0.0.1:9");
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(0), "stdout: {stdout}stderr: {stderr}");
    assert!(artifact_of(&stdout).is_file(), "the catalog must compile");

    std::fs::write(
        dir.join("src").join("main.rnx"),
        "fn Main(): Int {\n    println!(\"hello from uefi\");\n    return 0;\n}\n",
    )
    .unwrap();
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "the UEFI prelude must export the print macro: stdout: {stdout}stderr: {stderr}"
    );

    std::fs::write(
        dir.join("Project.config"),
        format!(
            "export default {{\n    project: {{ name: \"app\", version: \"0.1.0\" }},\n    dependencies: {{\n        \"@std/prelude\": {{ alias: \"@uefi/prelude\" }},\n        \"@uefi/prelude\": {{ path: \"{}\" }}\n    }}\n}}\n",
            prelude.display()
        ),
    )
    .unwrap();
    let (code, stdout, stderr) = {
        let mut cmd = std::process::Command::new(rnx());
        cmd.args(["build", dir.join("src/main.rnx").to_str().unwrap()])
            .current_dir(&dir)
            .env("RNX_REGISTRY", "http://127.0.0.1:9");
        let out = cmd.output().unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    };
    assert_ne!(code, 0, "a host target cannot provide the UEFI surface");
    let combined = format!("{stdout}{stderr}");
    assert!(combined.contains("E108"), "{combined}");
    assert!(combined.contains("@std/uefi"), "{combined}");
    assert!(combined.contains("firmware"), "{combined}");
    assert!(combined.contains("x86_64-unknown-linux-gnu"), "{combined}");
}

#[test]
fn prelude_without_promise_fails_async_with_e108() {
    let dir = crate::scratch::dir("prelude-no-promise");
    let prelude = stock_prelude()
        .replace(
            "export { Promise, PromiseWithResolvers, PromiseResult } from \"@std/async\";\n",
            "",
        )
        .replace(
            "export {\n    Promise,\n    PromiseWithResolvers,\n    PromiseResult\n} from \"@std/async\";\n",
            "",
        );
    assert!(!prelude.contains("from \"@std/async\""));
    write_package(
        &dir.join("deps").join("prelude"),
        "@local/no-promise",
        &prelude,
    );
    write_app(
        &dir,
        "async fn value(): Int {\n    return 42;\n}\n\nfn Main(): Int {\n    return value().wait().unwrapOr(1);\n}\n",
        "@local/no-promise",
    );
    let (code, stdout, stderr) = run_build(&dir);
    assert_ne!(code, 0, "async lowering without Promise must fail");
    let combined = format!("{stdout}{stderr}");
    assert!(combined.contains("E108"), "{combined}");
    assert!(combined.contains("Promise"), "{combined}");
    assert!(combined.contains("@local/no-promise"), "{combined}");
}

#[test]
fn prelude_without_io_hides_io_but_keeps_print_and_assert() {
    let dir = crate::scratch::dir("prelude-no-io");
    let prelude = stock_prelude().replace("pub class IO {}\n", "");
    write_package(&dir.join("deps").join("prelude"), "@local/no-io", &prelude);
    write_app(
        &dir,
        "fn Main(): Int {\n    IO.print(\"hidden\");\n    return 0;\n}\n",
        "@local/no-io",
    );
    let (code, stdout, stderr) = run_build(&dir);
    assert_ne!(
        code, 0,
        "IO must be unavailable without a prelude declaration"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(combined.contains("IO"), "{combined}");

    let ok = crate::scratch::dir("prelude-no-io-print");
    write_package(&ok.join("deps").join("prelude"), "@local/no-io", &prelude);
    write_app(
        &ok,
        "fn Main(): Int {\n    println!(\"ok\");\n    assert(1 == 1, \"one\");\n    return 0;\n}\n",
        "@local/no-io",
    );
    let (code, stdout, stderr) = run_build(&ok);
    assert_eq!(code, 0, "stdout: {stdout}stderr: {stderr}");
    let bin = artifact_of(&stdout);
    assert_eq!(run_binary(&bin), 0);
}
