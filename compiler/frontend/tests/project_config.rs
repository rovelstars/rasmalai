#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

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
    let dir = crate::scratch::dir("pcfg");
    std::fs::write(dir.join("Project.config"), src).unwrap();
    let cfg = project::ProjectConfig::load_from_dir(&dir).unwrap().unwrap();
    cfg
}

#[test]
fn basic_manifest_evaluates() {
    let c = load(BASIC);
    assert_eq!(c.name, "my_app");
    assert_eq!(c.entries.main, "src/main.rnx");
    assert_eq!(
        c.dependencies["utils"],
        DependencySpec::Path { path: std::path::PathBuf::from("../utils"), link: frontend::license::LinkPreference::Auto }
    );
    assert_eq!(
        c.dependencies["algo"],
        DependencySpec::Git {
            git: "https://github.com/example/algo".to_string(),
            rev: "main".to_string(),
            link: frontend::license::LinkPreference::Auto,
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
        sysroots: Default::default(),
        packages: vec![
            LockedPackage {
                name: "app".to_string(),
                alias: None,
                version: "0.1.0".to_string(),
                source: "root".to_string(),
                checksum: "4792fbf8e9c7ee3f9d1eda17d5efab674330f431fb914deb660e45e28add3000".to_string(),
                artifact_digest: Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                license: Some("MIT".to_string()),
                link: Some("static".to_string()),
                dependencies: vec!["dep 0.2.0".to_string()],
                tier: "pure".to_string(),
                capabilities: vec![],
            },
            LockedPackage {
                name: "dep".to_string(),
                alias: None,
                version: "0.2.0".to_string(),
                source: "path:../dep".to_string(),
                checksum: "19a45518b6b76282bafa62db49ef24accd4ef11584050e5f25cb1302336a17ca".to_string(),
                artifact_digest: None,
                license: Some("MIT".to_string()),
                link: Some("static".to_string()),
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

#[test]
fn redact_replaces_project_version_only() {
    let src = "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\",\n        description: \"mentions version \\\"9.9.9\\\" inside\"\n    },\n    dependencies: {\n        dep: { version: \"4.5.6\" }\n    }\n}\n";
    let facts = project::redact::publish_facts(src).unwrap_or_else(|e| panic!("facts failed: {e}"));
    assert_eq!(facts.name, "probe");
    assert_eq!(facts.version, "1.2.3");
    assert!(facts.redacted.contains("version: \"0.0.0-published\""), "{}", facts.redacted);
    assert!(facts.redacted.contains("version: \"4.5.6\""), "dep version must survive:\n{}", facts.redacted);
    assert!(facts.redacted.contains("9.9.9"), "description text must survive:\n{}", facts.redacted);
    let back = project::redact::rehydrate_project_version(&facts.redacted, "1.2.3").unwrap();
    assert!(back.contains("version: \"1.2.3\""), "{back}");
}

#[test]
fn redact_survives_comments_and_braces_in_strings() {
    let src = "export default {\n    // version: \"0.0.0\" in a comment\n    /* version: \"0.0.0\" in a block, braces { } here */\n    project: {\n        name: \"probe\", // trailing\n        version: \"1.2.3\",\n        description: \"a brace-free string\"\n    }\n}\n";
    let facts = project::redact::publish_facts(src).unwrap_or_else(|e| panic!("facts failed: {e}"));
    assert_eq!(facts.version, "1.2.3");
    assert!(facts.redacted.contains("version: \"0.0.0-published\""), "{}", facts.redacted);
    assert!(facts.redacted.contains("// version: \"0.0.0\" in a comment"), "{}", facts.redacted);
}

#[test]
fn rehydrate_refuses_foreign_placeholder() {
    let src = "export default {\n    project: {\n        name: \"probe\",\n        version: \"9.9.9\"\n    }\n}\n";
    let err = project::redact::rehydrate_project_version(src, "1.2.3").unwrap_err();
    assert_eq!(err.code, diagnostics::Code::E108);
}

#[test]
fn redact_rejects_missing_project_version() {
    let src = "export default {\n    project: {\n        name: \"probe\"\n    }\n}\n";
    let err = project::redact::publish_facts(src).unwrap_err();
    assert_eq!(err.code, diagnostics::Code::E108);
}

#[test]
fn publish_facts_pass_artifacts_declaration_through() {
    let src = "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\"\n    },\n    artifacts: \"ghcr.io/org/sqlite3\"\n}\n";
    let facts = project::redact::publish_facts(src).unwrap_or_else(|e| panic!("facts failed: {e}"));
    assert_eq!(facts.artifacts.as_deref(), Some("ghcr.io/org/sqlite3"));
    let plain = "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\"\n    }\n}\n";
    let facts = project::redact::publish_facts(plain).unwrap_or_else(|e| panic!("facts failed: {e}"));
    assert_eq!(facts.artifacts, None);
    let keyed = "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\"\n    },\n    artifacts: { native: { ref: \"ghcr.io/org/x:1.0.0\", digest: \"sha256:abc\" } }\n}\n";
    let err = project::redact::publish_facts(keyed).unwrap_err();
    assert_eq!(err.code, diagnostics::Code::E108);
    assert!(err.message.contains("keyed `artifacts` tables are rejected"), "{}", err.message);
}

#[test]
fn alias_to_path_dependency_parses() {
    let src = "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    dependencies: {\n        \"@std/prelude\": { alias: \"@uefi/prelude\" },\n        \"@uefi/prelude\": { path: \"../uefi-prelude\" }\n    }\n}\n";
    let c = load(src);
    assert_eq!(
        c.dependencies["@std/prelude"],
        DependencySpec::Alias {
            package: "@uefi/prelude".to_string(),
            version: None,
        }
    );
    assert_eq!(
        c.dependencies["@uefi/prelude"],
        DependencySpec::Path {
            path: std::path::PathBuf::from("../uefi-prelude"),
            link: frontend::license::LinkPreference::Auto,
        }
    );
}

#[test]
fn alias_with_inline_version_parses() {
    let src = "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    dependencies: {\n        \"@std/prelude\": { alias: \"@uefi/prelude\", version: \"0.2.2\" }\n    }\n}\n";
    let c = load(src);
    assert_eq!(
        c.dependencies["@std/prelude"],
        DependencySpec::Alias {
            package: "@uefi/prelude".to_string(),
            version: Some("0.2.2".to_string()),
        }
    );
}

#[test]
fn alias_rejects_every_other_source_key() {
    for other in ["path", "git", "url", "native", "system", "from", "checksum", "rev", "tag", "branch", "link"] {
        let value = match other {
            "path" => "{ alias: \"@uefi/prelude\", path: \"../x\" }".to_string(),
            "git" => "{ alias: \"@uefi/prelude\", git: \"https://example.com/x\" }".to_string(),
            "url" => "{ alias: \"@uefi/prelude\", url: \"https://example.com/x.tgz\" }".to_string(),
            "native" => "{ alias: \"@uefi/prelude\", native: \"z\" }".to_string(),
            "system" => "{ alias: \"@uefi/prelude\", system: true }".to_string(),
            "from" => "{ alias: \"@uefi/prelude\", from: \"libz.so\" }".to_string(),
            "checksum" => "{ alias: \"@uefi/prelude\", checksum: \"abc\" }".to_string(),
            "rev" => "{ alias: \"@uefi/prelude\", rev: \"v1\" }".to_string(),
            "tag" => "{ alias: \"@uefi/prelude\", tag: \"v1\" }".to_string(),
            "branch" => "{ alias: \"@uefi/prelude\", branch: \"main\" }".to_string(),
            _ => "{ alias: \"@uefi/prelude\", link: \"static\" }".to_string(),
        };
        let src = format!(
            "export default {{\n    project: {{ name: \"app\", version: \"0.1.0\" }},\n    dependencies: {{ \"@std/prelude\": {value} }}\n}}\n"
        );
        let issues = project::validate_manifest_text(&src, None);
        assert!(
            issues.iter().any(|i| i.error
                && i.code == diagnostics::Code::E108
                && i.message.contains("`alias`")
                && i.message.contains(other)),
            "{other}: {issues:?}"
        );
    }
}

#[test]
fn alias_rejects_invalid_package_name() {
    let src = "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    dependencies: { \"@std/prelude\": { alias: \"Bad_Name\" } }\n}\n";
    let issues = project::validate_manifest_text(src, None);
    assert!(
        issues.iter().any(|i| i.error
            && i.code == diagnostics::Code::E108
            && i.message.contains("not a valid package name")),
        "{issues:?}"
    );
}

#[test]
fn alias_rejects_invalid_inline_version() {
    let src = "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    dependencies: { \"@std/prelude\": { alias: \"@uefi/prelude\", version: \"nonsense!!\" } }\n}\n";
    let issues = project::validate_manifest_text(src, None);
    assert!(
        issues.iter().any(|i| i.error
            && i.code == diagnostics::Code::E108
            && i.message.contains("not a valid range")),
        "{issues:?}"
    );
}

#[test]
fn alias_dep_writes_back_to_manifest() {
    let src = "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    dependencies: {\n        \"@std/prelude\": { alias: \"@uefi/prelude\", version: \"0.2.2\" }\n    }\n}\n";
    let c = load(src);
    let manifest = project::Manifest { project: Some(c.clone()), workspace: None };
    let text = project::manifest_to_rnx(&manifest);
    assert!(text.contains("alias: \"@uefi/prelude\""), "{text}");
    assert!(text.contains("version: \"0.2.2\""), "{text}");
    let (back, _) =
        project::parse_manifest_for_publish(&text).unwrap_or_else(|e| panic!("reparse failed: {e}"));
    assert_eq!(back.expect("project").dependencies, c.dependencies);
}

#[test]
fn lock_alias_round_trips() {
    use frontend::deplock::{LockedPackage, ProjectDepLock};
    let lock = ProjectDepLock {
        version: 2,
        sysroots: Default::default(),
        packages: vec![LockedPackage {
            name: "@std/prelude".to_string(),
            alias: Some("@uefi/prelude".to_string()),
            version: "0.2.2".to_string(),
            source: "registry:@uefi/prelude@0.2.2".to_string(),
            checksum: "a".repeat(64),
            artifact_digest: None,
            license: Some("MIT".to_string()),
            link: Some("static".to_string()),
            dependencies: Vec::new(),
            tier: "pure".to_string(),
            capabilities: vec![],
        }],
    };
    let text = lock.to_rnx();
    let name_at = text.find("name: \"@std/prelude\"").expect("name");
    let alias_at = text.find("alias: \"@uefi/prelude\"").expect("alias");
    let version_at = text.find("version: \"0.2.2\"").expect("version");
    assert!(name_at < alias_at && alias_at < version_at, "{text}");
    assert_eq!(ProjectDepLock::parse(&text).unwrap(), lock);
}

#[test]
fn redact_preserves_alias_and_inline_version() {
    let src = "export default {\n    project: { name: \"probe\", version: \"1.2.3\" },\n    dependencies: {\n        \"@std/prelude\": { alias: \"@uefi/prelude\", version: \"0.2.2\" }\n    }\n}\n";
    let facts = project::redact::publish_facts(src).unwrap_or_else(|e| panic!("facts failed: {e}"));
    assert!(facts.redacted.contains("alias: \"@uefi/prelude\""), "{}", facts.redacted);
    assert!(facts.redacted.contains("version: \"0.2.2\""), "{}", facts.redacted);
}

#[test]
fn project_targets_parse_and_round_trip() {
    let src = "export default {\n    project: {\n        name: \"boot\",\n        version: \"0.1.0\",\n        targets: [\"x86_64-unknown-uefi\", \"wasm32-unknown-wasi\"]\n    }\n}\n";
    let c = load(src);
    assert_eq!(
        c.targets,
        vec!["x86_64-unknown-uefi".to_string(), "wasm32-unknown-wasi".to_string()]
    );
    let manifest = project::Manifest { project: Some(c.clone()), workspace: None };
    let text = project::manifest_to_rnx(&manifest);
    assert!(
        text.contains("targets: [\"x86_64-unknown-uefi\", \"wasm32-unknown-wasi\"]"),
        "{text}"
    );
    let (back, _) =
        project::parse_manifest_for_publish(&text).unwrap_or_else(|e| panic!("reparse failed: {e}"));
    assert_eq!(back.expect("project").targets, c.targets);
}

#[test]
fn project_targets_reject_an_unknown_triple() {
    let src = "export default {\n    project: { name: \"app\", version: \"0.1.0\", targets: [\"x86_64-pc-windows-msvc\"] }\n}\n";
    let dir = crate::scratch::dir("pcfg-targets");
    std::fs::write(dir.join("Project.config"), src).unwrap();
    let err = project::ProjectConfig::load_from_dir(&dir).expect_err("unknown target rejected");
    assert_eq!(err.code, diagnostics::Code::E108);
    assert!(err.message.contains("x86_64-pc-windows-msvc"), "{}", err.message);
    let issues = project::validate_manifest_text(src, None);
    assert!(
        issues
            .iter()
            .any(|issue| issue.error
                && issue.code == diagnostics::Code::E108
                && issue.message.contains("x86_64-pc-windows-msvc")),
        "{issues:?}"
    );
}

#[test]
fn project_targets_accept_an_empty_array() {
    let src = "export default {\n    project: { name: \"app\", version: \"0.1.0\", targets: [] }\n}\n";
    let c = load(src);
    assert!(c.targets.is_empty());
    let issues = project::validate_manifest_text(src, None);
    assert!(issues.is_empty(), "{issues:?}");
}


#[test]
fn capabilities_parse_and_round_trip() {
    let src = "export default {\n    project: { name: \"probe\", version: \"1.2.3\" },\n    capabilities: [\"fs\", \"log\"]\n}\n";
    let c = load(src);
    assert_eq!(c.capabilities, vec!["fs".to_string(), "log".to_string()]);
    let manifest = project::Manifest { project: Some(c.clone()), workspace: None };
    let text = project::manifest_to_rnx(&manifest);
    assert!(text.contains("capabilities: [\"fs\", \"log\"]"), "{text}");
    let (back, _) =
        project::parse_manifest_for_publish(&text).unwrap_or_else(|e| panic!("reparse failed: {e}"));
    assert_eq!(back.expect("project").capabilities, c.capabilities);
    let redacted = project::redact::redact_project_version(&text).unwrap();
    assert!(redacted.contains("capabilities: [\"fs\", \"log\"]"), "{redacted}");
}

#[test]
fn capabilities_reject_an_unknown_name() {
    let src = "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    capabilities: [\"telepathy\"]\n}\n";
    let dir = crate::scratch::dir("pcfg-caps");
    std::fs::write(dir.join("Project.config"), src).unwrap();
    let err = project::ProjectConfig::load_from_dir(&dir).expect_err("unknown capability rejected");
    assert_eq!(err.code, diagnostics::Code::E108);
    assert!(err.message.contains("telepathy"), "{}", err.message);
    let issues = project::validate_manifest_text(src, None);
    assert!(
        issues.iter().any(|issue| issue.error
            && issue.code == diagnostics::Code::E108
            && issue.message.contains("telepathy")),
        "{issues:?}"
    );
}
