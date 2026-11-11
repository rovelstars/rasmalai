#![cfg(all(unix, target_os = "linux"))]

#[path = "../../testkit/scratch.rs"]
mod scratch;

use cli::dev::DevRunner;
use frontend::native::NativePrefer;
use std::path::Path;
use std::process::Command;

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

#[test]
fn dev_loads_a_bundled_artifact_blob() {
    let probe = Command::new("cc").arg("--version").output();
    if probe.map(|o| !o.status.success()).unwrap_or(true) {
        return;
    }
    let dir = scratch::dir("bundled-native");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf();
    unsafe {
        std::env::set_var("RNX_VENDOR_DIR", repo.join("compiler/stdlib/src"));
        std::env::set_var("RNX_CACHE_HOME", dir.join("cache"));
    }
    let so = dir.join("native_dep/.rnx-cache/artifacts/runtime/libtiny.so");
    std::fs::create_dir_all(so.parent().unwrap()).unwrap();
    let c = dir.join("tiny.c");
    write(&c, "long tiny_answer(void) { return 42; }\n");
    let built = Command::new("cc")
        .args(["-shared", "-fPIC", "-o"])
        .arg(&so)
        .arg(&c)
        .status()
        .unwrap();
    assert!(built.success());

    write(
        &dir.join("native_dep/Project.config"),
        "export default {\n    project: { name: \"native_dep\", version: \"0.1.0\" },\n    entries: { main: \"src/lib.rnx\" },\n    artifacts: \"example.invalid/org/native_dep\",\n    dependencies: {\n        tiny: { native: \"tiny\", from: \"libtiny.so\", version: \">=0.1.0\" }\n    }\n}\n",
    );
    write(&dir.join("native_dep/src/lib.rnx"), "fn unused(): Int { return 0; }\n");
    write(
        &dir.join("app/Project.config"),
        "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    dependencies: {\n        native_dep: { path: \"../native_dep\" }\n    }\n}\n",
    );
    let main = dir.join("app/src/main.rnx");
    write(
        &main,
        "import {\n    fn tiny_answer(): Int as tinyAnswer,\n} from native \"tiny\"\n\nfn Main(): Int {\n    let v = 0\n    unsafe {\n        v = tinyAnswer()\n    }\n    return v\n}\n",
    );

    let mut runner = DevRunner::new(&main, "Main", true, NativePrefer::None)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(runner.call("Main", &[]).unwrap(), 42);
}
