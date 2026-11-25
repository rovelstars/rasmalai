#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use frontend::deplock::{LockedPackage, ProjectDepLock};
use frontend::project::{self, DependencySpec, Manifest, ProjectConfig, RustSource};
use frontend::rustdep::graph::{
    self, BRIDGE_ABI_VERSION, RUST_GRAPH_VERSION, RustGraph, RustPackage,
};

fn load(src: &str) -> ProjectConfig {
    let dir = scratch::dir("rust-dep");
    std::fs::write(dir.join("Project.config"), src).unwrap();
    project::ProjectConfig::load_from_dir(&dir).unwrap().unwrap()
}

const BASE: &str = r#"export default {
    project: { name: "app", version: "0.1.0" },
    dependencies: {
        serde: { rust: "1" },
        wgpu: { rust: "0.20", features: ["trace", "wgsl-in"], default_features: false },
        parley: { rust: { git: "https://github.com/linebender/parley", rev: "abc123" }, features: ["std"] }
    }
}
"#;

#[test]
fn parses_rust_dependencies() {
    let config = load(BASE);
    assert_eq!(
        config.dependencies["serde"],
        DependencySpec::Rust {
            package: None,
            monomorphs: Vec::new(),
            source: RustSource::CratesIo,
            version: Some("1".to_string()),
            features: Vec::new(),
            default_features: true,
            link: frontend::license::LinkPreference::Auto,
        }
    );
    assert_eq!(
        config.dependencies["wgpu"],
        DependencySpec::Rust {
            package: None,
            monomorphs: Vec::new(),
            source: RustSource::CratesIo,
            version: Some("0.20".to_string()),
            features: vec!["trace".to_string(), "wgsl-in".to_string()],
            default_features: false,
            link: frontend::license::LinkPreference::Auto,
        }
    );
    assert_eq!(
        config.dependencies["parley"],
        DependencySpec::Rust {
            package: None,
            monomorphs: Vec::new(),
            source: RustSource::Git {
                url: "https://github.com/linebender/parley".to_string(),
                rev: Some("abc123".to_string()),
            },
            version: None,
            features: vec!["std".to_string()],
            default_features: true,
            link: frontend::license::LinkPreference::Auto,
        }
    );
}

#[test]
fn parses_registry_rust_dependency() {
    let config = load(
        "export default { project: { name: \"app\", version: \"0.1.0\" }, dependencies: { inner: { rust: \"2\", registry: \"private\" } } }\n",
    );
    assert_eq!(
        config.dependencies["inner"],
        DependencySpec::Rust {
            package: None,
            monomorphs: Vec::new(),
            source: RustSource::Registry("private".to_string()),
            version: Some("2".to_string()),
            features: Vec::new(),
            default_features: true,
            link: frontend::license::LinkPreference::Auto,
        }
    );
}

#[test]
fn package_alias_selects_the_cargo_crate_name() {
    let config = load(
        "export default { project: { name: \"app\", version: \"0.1.0\" }, dependencies: { serde: { rust: \"1\", package: \"serde_crate\" } } }\n",
    );
    let DependencySpec::Rust { package, .. } = &config.dependencies["serde"] else {
        panic!("expected rust dependency");
    };
    assert_eq!(package.as_deref(), Some("serde_crate"));
    let requests = graph::requests_from_config(&config);
    assert_eq!(requests[0].name, "serde");
    assert_eq!(requests[0].cargo_name, "serde_crate");
}

#[test]
fn parses_monomorph_declarations() {
    let config = load(
        "export default { project: { name: \"app\", version: \"0.1.0\" }, dependencies: { probe: { rust: \"1\", monomorphs: [[\"i32\"], [\"f64\"]] } } }\n",
    );
    let DependencySpec::Rust { monomorphs, .. } = &config.dependencies["probe"] else {
        panic!("expected rust dependency");
    };
    assert_eq!(
        monomorphs,
        &vec![vec!["i32".to_string()], vec!["f64".to_string()]]
    );
    let text = project::manifest_to_rnx(&Manifest { project: Some(config), workspace: None });
    assert!(text.contains("monomorphs: [[\"i32\"], [\"f64\"]]"));
}

#[test]
fn rejects_invalid_rust_declarations() {
    let cases = [
        ("{ rust: \"1\", path: \"../x\" }", "cannot mix"),
        ("{ rust: \"1\", native: \"z\" }", "cannot mix"),
        ("{ rust: \"1\", rev: \"abc\" }", "requires a `git` source"),
        ("{ rust: \"1\", git: \"u\", registry: \"r\" }", "cannot combine"),
        ("{ rust: \"1\", features: [1] }", "non-empty strings"),
        ("{ rust: \"1\", default_features: \"yes\" }", "must be a boolean"),
        ("{ rust: \"1\", link: \"dynamic\" }", "link statically"),
        ("{ rust: { git: \"u\", branch: \"main\" } }", "unknown rust field"),
        ("{ rust: { version: \"1\" }, registry: \"r\" }", "not next to it"),
        ("{ rust: \"1\", package: \"bad name\" }", "non-empty crate name"),
        ("{ rust: \"1\", monomorphs: [[]] }", "must not be empty"),
        ("{ rust: \"1\", monomorphs: [\"i32\"] }", "arrays of Rust type names"),
    ];
    for (decl, expected) in cases {
        let src = format!(
            "export default {{ project: {{ name: \"app\", version: \"0.1.0\" }}, dependencies: {{ dep: {decl} }} }}\n"
        );
        let dir = scratch::dir("rust-dep-bad");
        std::fs::write(dir.join("Project.config"), &src).unwrap();
        let error = project::ProjectConfig::load_from_dir(&dir).unwrap_err();
        assert!(
            error.message.contains(expected),
            "{decl}: expected `{expected}`, got {}",
            error.message
        );
    }
}

#[test]
fn manifest_round_trips_rust_dependencies() {
    let config = load(BASE);
    let manifest = Manifest { project: Some(config), workspace: None };
    let text = project::manifest_to_rnx(&manifest);
    assert!(text.contains("rust: \"1\""));
    assert!(text.contains("features: [\"trace\", \"wgsl-in\"]"));
    let dir = scratch::dir("rust-dep-roundtrip");
    std::fs::write(dir.join("Project.config"), &text).unwrap();
    let again = project::ProjectConfig::load_from_dir(&dir).unwrap().unwrap();
    assert_eq!(again.dependencies, manifest.project.unwrap().dependencies);
}

fn graph() -> RustGraph {
    RustGraph {
        version: RUST_GRAPH_VERSION,
        packages: vec![
            RustPackage {
                name: "serde".to_string(),
                version: "1.0.228".to_string(),
                source: "registry+https://github.com/rust-lang/crates.io-index".to_string(),
                checksum: Some("abc".to_string()),
                features: vec!["derive".to_string()],
            },
            RustPackage {
                name: "wgpu".to_string(),
                version: "0.20.1".to_string(),
                source: "registry+https://github.com/rust-lang/crates.io-index".to_string(),
                checksum: None,
                features: vec!["trace".to_string(), "wgsl-in".to_string()],
            },
            RustPackage {
                name: "parley".to_string(),
                version: "0.5.0".to_string(),
                source: "git+https://github.com/linebender/parley?rev=abc123#abc123".to_string(),
                checksum: None,
                features: vec!["std".to_string()],
            },
        ],
        cargo_lock_digest: "sha256:deadbeef".to_string(),
        rustc_version: "1.98.1".to_string(),
        llvm_version: "22.1".to_string(),
        bridge_abi: BRIDGE_ABI_VERSION,
        artifact_digest: Some("sha256:cafe".to_string()),
    }
}

#[test]
fn deplock_embeds_and_parses_rust_graph() {
    let lock = ProjectDepLock {
        version: frontend::deplock::LOCK_VERSION,
        packages: vec![LockedPackage {
            name: "app".to_string(),
            alias: None,
            version: "0.1.0".to_string(),
            source: "root".to_string(),
            checksum: "0".repeat(64),
            artifact_digest: None,
            license: None,
            link: None,
            dependencies: Vec::new(),
            tier: "pure".to_string(),
            capabilities: Vec::new(),
        }],
        sysroots: Default::default(),
        rust: Some(graph()),
    };
    let text = lock.to_rnx();
    assert!(text.contains("rust: {"));
    assert!(text.contains("cargoLockDigest: \"sha256:deadbeef\""));
    assert!(text.contains("bridgeAbi: 1"));
    let parsed = ProjectDepLock::parse(&text).unwrap();
    assert_eq!(parsed.rust, Some(graph()));
}

#[test]
fn rust_graph_verifies_requests_and_detects_drift() {
    let config = load(BASE);
    assert!(graph::verify_rust_graph(&config, Some(&graph())).is_ok());
    assert!(graph::verify_rust_graph(&config, None).is_err());
    let mut stale = graph();
    stale.packages.remove(1);
    let error = graph::verify_rust_graph(&config, Some(&stale)).unwrap_err();
    assert!(error.message.contains("wgpu"));
    let mut no_feature = graph();
    no_feature.packages[1].features.clear();
    let error = graph::verify_rust_graph(&config, Some(&no_feature)).unwrap_err();
    assert!(error.message.contains("feature `trace`"));
}

#[test]
fn requests_are_sorted_and_deduplicated() {
    let config = load(BASE);
    let requests = graph::requests_from_config(&config);
    let names: Vec<&str> = requests.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["parley", "serde", "wgpu"]);
    let merged = graph::requests_from_configs(&[&config, &config]);
    let merged_names: Vec<&str> = merged.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(merged_names, names);
}
