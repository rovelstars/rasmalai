use frontend::project::eval::{ConfigTarget, ConfigValue, eval_module};
use frontend::project::{self, DependencySpec};
use frontend::parser::Parser;

fn target(os: &str, arch: &str) -> ConfigTarget {
    ConfigTarget { os: os.to_string(), arch: arch.to_string(), env: "gnu".to_string() }
}

fn linux() -> ConfigTarget {
    target("linux", "x86_64")
}

fn eval(src: &str, target: ConfigTarget) -> ConfigValue {
    let module = Parser::parse_module(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
    eval_module(&module, target).unwrap_or_else(|e| panic!("eval failed: {e}"))
}

fn eval_err(src: &str) -> diagnostics::Diagnostic {
    let module = Parser::parse_module(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
    eval_module(&module, linux()).expect_err("expected eval error")
}

const BASIC: &str = "export default {\n    project: {\n        name: \"my_app\",\n        version: \"0.1.0\",\n        description: \"High-performance app with native FFI\"\n    },\n    entries: { main: \"src/main.rnx\" },\n    dependencies: {\n        utils: { path: \"../utils\" },\n        algo: { git: \"https://github.com/example/algo\", branch: \"main\" }\n    },\n    permissions: [{ perm: \"fs:read:/data\", reason: \"seed\" }, \"term:write\"]\n}\n";

fn load(src: &str) -> project::ProjectConfig {
    let dir = std::env::temp_dir().join(format!("rnx-pcfg-{}-{}", std::process::id(), src.len()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.config"), src).unwrap();
    let cfg = project::ProjectConfig::load_from_dir(&dir).unwrap().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    cfg
}

#[test]
fn basic_manifest_evaluates() {
    let c = load(BASIC);
    assert_eq!(c.name, "my_app");
    assert_eq!(c.entries.main, "src/main.rnx");
    assert_eq!(
        c.dependencies["utils"],
        DependencySpec::Path { path: std::path::PathBuf::from("../utils") }
    );
    assert_eq!(
        c.dependencies["algo"],
        DependencySpec::Git {
            git: "https://github.com/example/algo".to_string(),
            rev: "main".to_string()
        }
    );
    assert_eq!(
        c.permissions,
        Some(vec![
            project::PermissionDecl {
                perm: "fs:read:/data".to_string(),
                reason: Some("seed".to_string())
            },
            project::PermissionDecl { perm: "term:write".to_string(), reason: None }
        ])
    );
}

#[test]
fn spread_merges_common_deps() {
    let src = "const commonDeps = { json: \"1.2.0\" }\n\
        export default {\n    project: { name: \"a\", version: \"0.1.0\" },\n    dependencies: { ...commonDeps, extra: \"^2.0.0\" }\n}\n";
    let v = eval(src, linux());
    let deps = v.get("dependencies").expect("deps");
    assert_eq!(deps.get("json"), Some(&ConfigValue::String("1.2.0".to_string())));
    assert_eq!(deps.get("extra"), Some(&ConfigValue::String("^2.0.0".to_string())));
}

#[test]
fn target_switch_selects_per_os() {
    let src = "export default {\n    project: { name: \"a\", version: \"0.1.0\" },\n    dependencies: {\n        zlib: switch (target.os) { case \"windows\": { native: \"zlibstatic\", system: true } default: { native: \"z\", system: true } }\n    }\n}\n";
    let win = eval(src, target("windows", "x86_64"));
    let zlib = win.get("dependencies").unwrap().get("zlib").unwrap();
    assert_eq!(zlib.get("native"), Some(&ConfigValue::String("zlibstatic".to_string())));
    let lin = eval(src, linux());
    let zlib = lin.get("dependencies").unwrap().get("zlib").unwrap();
    assert_eq!(zlib.get("native"), Some(&ConfigValue::String("z".to_string())));
    let mac = eval(src, target("macos", "aarch64"));
    let zlib = mac.get("dependencies").unwrap().get("zlib").unwrap();
    assert_eq!(zlib.get("native"), Some(&ConfigValue::String("z".to_string())));
}

#[test]
fn ternary_selects_per_arch() {
    let src = "export default {\n    project: { name: \"a\", version: \"0.1.0\" },\n    dependencies: {\n        gui: target.arch == \"aarch64\" ? { native: \"gui_arm\" } : { native: \"gui_x86\" }\n    }\n}\n";
    let arm = eval(src, target("linux", "aarch64"));
    assert_eq!(
        arm.get("dependencies").unwrap().get("gui").unwrap().get("native"),
        Some(&ConfigValue::String("gui_arm".to_string()))
    );
    let x64 = eval(src, linux());
    assert_eq!(
        x64.get("dependencies").unwrap().get("gui").unwrap().get("native"),
        Some(&ConfigValue::String("gui_x86".to_string()))
    );
}

#[test]
fn sandbox_rejects_while_loops() {
    let e = eval_err("while true { }\nexport default { project: { name: \"a\", version: \"0.1.0\" } }\n");
    assert_eq!(e.code, diagnostics::Code::E108);
}

#[test]
fn sandbox_rejects_function_definitions() {
    let e = eval_err("fn helper(): Int { return 1; }\nexport default { project: { name: \"a\", version: \"0.1.0\" } }\n");
    assert_eq!(e.code, diagnostics::Code::E108);
    assert!(e.message.contains("function definitions"), "{e:?}");
}

#[test]
fn sandbox_rejects_imports() {
    let e = eval_err("import { x } from \"./x\"\nexport default { project: { name: \"a\", version: \"0.1.0\" } }\n");
    assert_eq!(e.code, diagnostics::Code::E108);
}

#[test]
fn sandbox_rejects_let_and_calls() {
    let module = Parser::parse_module(
        "export default { project: { name: \"a\", version: \"0.1.0\" } }\n",
    )
    .unwrap();
    assert!(eval_module(&module, linux()).is_ok());
    let e = eval_err("let x = 1;\nexport default { project: { name: \"a\", version: \"0.1.0\" } }\n");
    assert_eq!(e.code, diagnostics::Code::E108);
}

#[test]
fn entry_and_edition_are_hard_errors() {
    for src in [
        "export default {\n    project: { name: \"a\", version: \"0.1.0\", entry: \"src/main.rnx\" }\n}\n",
        "export default {\n    project: { name: \"a\", version: \"0.1.0\", edition: \"2026\" }\n}\n",
        "export default {\n    project: { name: \"a\", version: \"0.1.0\" },\n    entry: \"src/main.rnx\"\n}\n",
    ] {
        let issues = project::validate_manifest_text(src, None);
        assert!(
            issues.iter().any(|i| i.error && i.code == diagnostics::Code::E108 && i.message.contains("was removed")),
            "{src}: {issues:?}"
        );
    }
}

#[test]
fn unknown_top_level_key_warns_w201() {
    let issues = project::validate_manifest_text(
        "export default {\n    project: { name: \"a\", version: \"0.1.0\" },\n    frobnicate: 1\n}\n",
        None,
    );
    assert!(
        issues.iter().any(|i| !i.error && i.code == diagnostics::Code::W201 && i.message.contains("frobnicate")),
        "{issues:?}"
    );
}

#[test]
fn lockfile_round_trip_is_sorted_and_stable() {
    use frontend::deplock::{LockedPackage, ProjectDepLock};
    let lock = ProjectDepLock {
        version: 2,
        packages: vec![
            LockedPackage {
                name: "app".to_string(),
                version: "0.1.0".to_string(),
                source: "root".to_string(),
                checksum: "4792fbf8e9c7ee3f9d1eda17d5efab674330f431fb914deb660e45e28add3000".to_string(),
                dependencies: vec!["dep 0.2.0".to_string()],
                tier: "pure".to_string(),
                capabilities: vec![],
            },
            LockedPackage {
                name: "dep".to_string(),
                version: "0.2.0".to_string(),
                source: "path:../dep".to_string(),
                checksum: "19a45518b6b76282bafa62db49ef24accd4ef11584050e5f25cb1302336a17ca".to_string(),
                dependencies: vec![],
                tier: "pure".to_string(),
                capabilities: vec![],
            },
        ],
    };
    let text = lock.to_rnx();
    let again = ProjectDepLock::parse(&text).unwrap_or_else(|e| panic!("reparse failed: {e}"));
    assert_eq!(again, lock);
    assert!(text.find("name: \"app\"") < text.find("name: \"dep\""), "sorted:\n{text}");
    assert_eq!(ProjectDepLock::parse(&lock.to_rnx()).unwrap(), lock);
}
