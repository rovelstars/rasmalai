#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use frontend::deplock::{LockedPackage, ProjectDepLock};
use frontend::license::LinkPreference;
use frontend::modules::resolve_alias_spec;
use frontend::project::{self, DependencySpec};
use std::path::PathBuf;

fn config(src: &str) -> project::ProjectConfig {
    let (cfg, _) = project::parse_manifest_for_publish(src)
        .unwrap_or_else(|e| panic!("parse failed: {e}"));
    cfg.expect("project table")
}

fn manifest(deps: &str) -> String {
    format!(
        "export default {{\n    project: {{ name: \"app\", version: \"0.1.0\" }},\n    dependencies: {{ {deps} }}\n}}\n"
    )
}

#[test]
fn alias_chain_resolves_to_final_spec() {
    let cfg = config(&manifest(
        "a: { alias: \"b\" }, b: { alias: \"c\" }, c: { path: \"../c\" }",
    ));
    let (name, spec) =
        resolve_alias_spec(&PathBuf::from("."), &cfg, "a", &mut Vec::new()).unwrap();
    assert_eq!(name, "c");
    assert_eq!(
        spec,
        DependencySpec::Path {
            path: PathBuf::from("../c"),
            link: LinkPreference::Auto,
        }
    );
}

#[test]
fn alias_cycle_names_the_chain() {
    let cfg = config(&manifest("a: { alias: \"b\" }, b: { alias: \"a\" }"));
    let err = resolve_alias_spec(&PathBuf::from("."), &cfg, "a", &mut Vec::new()).unwrap_err();
    assert_eq!(err.code, diagnostics::Code::E108);
    assert!(err.message.contains("a -> b -> a"), "{}", err.message);
}

#[test]
fn alias_chain_over_limit_fails() {
    let mut deps = String::new();
    for i in 0..17 {
        deps.push_str(&format!("a{i}: {{ alias: \"a{}\" }}, ", i + 1));
    }
    deps.push_str("a17: { path: \"../end\" }");
    let cfg = config(&manifest(&deps));
    let err = resolve_alias_spec(&PathBuf::from("."), &cfg, "a0", &mut Vec::new()).unwrap_err();
    assert_eq!(err.code, diagnostics::Code::E108);
    assert!(err.message.contains("chain"), "{}", err.message);
}

#[test]
fn alias_inline_version_resolves_to_registry_spec() {
    let cfg = config(&manifest(
        "\"@std/prelude\": { alias: \"@uefi/prelude\", version: \"0.2.2\" }",
    ));
    let (name, spec) = resolve_alias_spec(
        &PathBuf::from("."),
        &cfg,
        "@std/prelude",
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(name, "@uefi/prelude");
    assert_eq!(
        spec,
        DependencySpec::Semver {
            version: "0.2.2".to_string(),
            link: LinkPreference::Auto,
        }
    );
}

#[test]
fn alias_without_target_or_version_needs_a_lock() {
    let cfg = config(&manifest("\"@std/prelude\": { alias: \"@uefi/prelude\" }"));
    let dir = scratch::dir("alias-lock");
    let err = resolve_alias_spec(
        dir.path(),
        &cfg,
        "@std/prelude",
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.code, diagnostics::Code::E115);
    assert!(err.hint.as_deref().unwrap_or_default().contains("rnx lock"));
}

#[test]
fn lock_generation_records_alias() {
    let dir = scratch::dir("alias-lock-gen");
    std::fs::create_dir_all(dir.join("deps").join("uefi").join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    dependencies: {\n        \"@std/prelude\": { alias: \"@uefi/prelude\" },\n        \"@uefi/prelude\": { path: \"deps/uefi\" }\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("deps").join("uefi").join("Project.config"),
        "export default {\n    project: { name: \"@uefi/prelude\", version: \"0.2.2\" }\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("deps").join("uefi").join("src").join("main.rnx"),
        "fn Main(): Int { return 0; }\n",
    )
    .unwrap();
    let cfg = project::ProjectConfig::load_from_dir(dir.path())
        .unwrap()
        .expect("project");
    let lock = ProjectDepLock::resolve(dir.path(), &cfg).unwrap();
    let alias = lock
        .packages
        .iter()
        .find(|p| p.name == "@std/prelude")
        .expect("alias entry");
    assert_eq!(alias.alias.as_deref(), Some("@uefi/prelude"));
    assert!(alias.source.starts_with("path:"), "{}", alias.source);
    assert!(lock
        .packages
        .iter()
        .any(|p| p.name == "@uefi/prelude" && p.alias.is_none()));
    let text = lock.to_rnx();
    assert!(text.contains("alias: \"@uefi/prelude\""), "{text}");
    let again = ProjectDepLock::parse(&text).unwrap();
    assert_eq!(again, lock);
    frontend::deplock::verify_lock(dir.path(), &cfg, &lock).unwrap();
}

static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct CwdGuard(std::path::PathBuf);

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

#[test]
fn std_alias_loads_the_local_target() {
    let _lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch::dir("std-alias");
    std::fs::create_dir_all(dir.join("deps").join("real").join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    dependencies: {\n        \"@std/prelude\": { alias: \"@local/real\" },\n        \"@local/real\": { path: \"deps/real\" }\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("deps").join("real").join("Project.config"),
        "export default {\n    project: { name: \"@local/real\", version: \"0.1.0\" }\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("deps").join("real").join("src").join("main.rnx"),
        "export fn marker(): Int { return 42; }\n",
    )
    .unwrap();
    frontend::stdvfs::clear();
    frontend::stdvfs::set("prelude", "// stock prelude\n".to_string());
    frontend::stdvfs::set("time", "// stock time\n".to_string());
    let previous = std::env::current_dir().unwrap();
    let _cwd = CwdGuard(previous);
    std::env::set_current_dir(dir.path()).unwrap();
    let prelude = frontend::modules::load_std_module_source("prelude").unwrap();
    assert!(prelude.contains("marker"), "{prelude}");
    assert!(!prelude.contains("stock prelude"), "{prelude}");
    let time = frontend::modules::load_std_module_source("time").unwrap();
    assert_eq!(time, "// stock time\n");
    frontend::stdvfs::clear();
}

#[test]
fn lock_alias_source_mismatch_is_rejected() {
    let dir = scratch::dir("alias-verify");
    std::fs::create_dir_all(dir.join("deps").join("uefi").join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    dependencies: {\n        \"@std/prelude\": { alias: \"@uefi/prelude\" },\n        \"@uefi/prelude\": { path: \"deps/uefi\" }\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("deps").join("uefi").join("Project.config"),
        "export default {\n    project: { name: \"@uefi/prelude\", version: \"0.1.0\" }\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("deps").join("uefi").join("src").join("main.rnx"),
        "fn Main(): Int { return 0; }\n",
    )
    .unwrap();
    let cfg = project::ProjectConfig::load_from_dir(dir.path())
        .unwrap()
        .expect("project");
    let lock = ProjectDepLock {
        version: frontend::deplock::LOCK_VERSION,
        sysroots: Default::default(),
        packages: vec![LockedPackage {
            name: "@std/prelude".to_string(),
            alias: Some("@uefi/prelude".to_string()),
            version: "0.1.0".to_string(),
            source: "path:deps/other".to_string(),
            checksum: "0".repeat(64),
            artifact_digest: None,
            license: Some("MIT".to_string()),
            link: Some("static".to_string()),
            dependencies: Vec::new(),
            tier: "pure".to_string(),
            capabilities: Vec::new(),
        }],
    };
    let err = frontend::deplock::verify_lock(dir.path(), &cfg, &lock).unwrap_err();
    assert_eq!(err.code, diagnostics::Code::E108);
    assert!(err.message.contains("lock alias mismatch"), "{}", err.message);
    assert!(err.message.contains("@uefi/prelude"), "{}", err.message);
}
