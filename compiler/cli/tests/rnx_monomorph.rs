#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

#[test]
fn rnx_source_generics_monomorphize_into_native_functions() {
    let dir = scratch::dir("rnx-monomorph");
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"mono\", version: \"0.1.0\", engine: \">=0.3.2\" },\n    entries: { main: \"src/main.rnx\" },\n}\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/main.rnx"),
        "#[Monomorph(Int, Float)]\n\
         fn scale<T>(value: T, by: T): T {\n    \
             return value * by\n\
         }\n\
         fn Main(): Int {\n    \
             let a = scaleI32(6, 7)\n    \
             let b = scaleF64(1.5, 2.0)\n    \
             println!(a, b)\n    \
             return a\n\
         }\n",
    )
    .unwrap();

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = Command::new(rnx).arg("build").current_dir(&dir).output().unwrap();
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let exe = dir.join(".rnx-cache/build/release/mono");
    let out = Command::new(&exe).output().unwrap();
    assert_eq!(out.status.code(), Some(42), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "42 3.0");
}

#[test]
fn rnx_source_monomorphs_use_trait_conformance() {
    let dir = scratch::dir("rnx-monomorph-trait");
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"mono-trait\", version: \"0.1.0\", engine: \">=0.3.2\" },\n    entries: { main: \"src/main.rnx\" },\n}\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/main.rnx"),
        "trait Doubler { fn double(): Int { return 0 } }\n\
         class Num with Doubler {\n    \
             let value: Int\n    \
             init(v: Int) { this.value = v }\n    \
             fn double(): Int { return this.value * 2 }\n\
         }\n\
         #[Monomorph(Num)]\n\
         fn apply<T>(v: T): Int { return v.double() }\n\
         fn Main(): Int { return applyNum(new Num(21)) }\n",
    )
    .unwrap();

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = Command::new(rnx).arg("build").current_dir(&dir).output().unwrap();
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let exe = dir.join(".rnx-cache/build/release/mono-trait");
    let out = Command::new(&exe).output().unwrap();
    assert_eq!(out.status.code(), Some(42), "{}", String::from_utf8_lossy(&out.stderr));
}
