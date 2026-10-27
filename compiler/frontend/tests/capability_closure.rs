#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use frontend::capabilities::{self, GraphInput};
use frontend::modules::ModuleGraph;

fn tree(tag: &str, files: &[(&str, &str)]) -> crate::scratch::Scratch {
    let dir = crate::scratch::dir(&format!("cc-{tag}"));
    for (name, src) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, src).unwrap();
    }
    dir
}

fn file_source(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if let Some(rest) = text.strip_prefix("@std/") {
        return frontend::modules::load_std_module_source(rest)
            .unwrap_or_else(|e| panic!("cannot load @{rest}: {e}"));
    }
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn analyze_graph(g: &ModuleGraph) -> frontend::capabilities::CapabilityAnalysisReport {
    let mut owned: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut paths: Vec<std::path::PathBuf> = g.files.iter().map(|f| f.path.clone()).collect();
    paths.sort();
    for path in &paths {
        owned.push((path.clone(), file_source(path)));
    }
    let inputs: Vec<GraphInput> = g
        .files
        .iter()
        .map(|f| {
            let src = owned.iter().find(|(p, _)| p == &f.path).unwrap();
            GraphInput { path: f.path.clone(), module: &f.module, source: src.1.as_str() }
        })
        .collect();
    capabilities::analyze_file_graph(&inputs, capabilities::DEFAULT_FUEL)
        .unwrap_or_else(|e| panic!("analysis failed: {}", e.message()))
}

fn cap_set(report: &frontend::capabilities::CapabilityAnalysisReport) -> Vec<String> {
    let mut caps: Vec<String> = report.capabilities.iter().map(|c| c.to_string()).collect();
    caps.sort();
    caps
}

const WRAP_LIB: &str = "export fn helper(): Int {\n    File.open(\"/tmp/chain.txt\", FileMode.Read);\n    return 0;\n}\n";

#[test]
fn bare_import_attributes_transitively() {
    let dir = tree(
        "bare",
        &[
            ("main.rnx", "import { helper } from \"./lib\";\nfn Main(): Int {\n    helper();\n    return 0;\n}\n"),
            ("lib.rnx", WRAP_LIB),
        ],
    );
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let caps = cap_set(&analyze_graph(&g));
    assert!(caps.contains(&"fs:read:/tmp/**".to_string()), "{caps:?}");
}

#[test]
fn alias_import_attributes_identically() {
    let direct = tree(
        "direct",
        &[(
            "main.rnx",
            "import { helper } from \"./lib\";\nfn Main(): Int {\n    helper();\n    return 0;\n}\n",
        ), ("lib.rnx", WRAP_LIB)],
    );
    let aliased = tree(
        "alias",
        &[(
            "main.rnx",
            "import { helper as h } from \"./lib\";\nfn Main(): Int {\n    h();\n    return 0;\n}\n",
        ), ("lib.rnx", WRAP_LIB)],
    );
    let g_direct = ModuleGraph::build(&direct.join("main.rnx")).unwrap();
    let g_alias = ModuleGraph::build(&aliased.join("main.rnx")).unwrap();
    assert_eq!(cap_set(&analyze_graph(&g_direct)), cap_set(&analyze_graph(&g_alias)));
}

#[test]
fn wrapper_library_chain_two_hops() {
    let dir = tree(
        "twohop",
        &[
            ("main.rnx", "import { mid } from \"./mid\";\nfn Main(): Int {\n    mid(\"/tmp/deep.txt\");\n    return 0;\n}\n"),
            ("mid.rnx", "import { leaf } from \"./leaf\";\nexport fn mid(p: String): Int {\n    leaf(p);\n    return 0;\n}\n"),
            ("leaf.rnx", "export fn leaf(p: String): Int {\n    File.open(p, FileMode.Read);\n    return 0;\n}\n"),
        ],
    );
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let caps = cap_set(&analyze_graph(&g));
    assert!(caps.contains(&"fs:delegated".to_string()), "{caps:?}");
    assert!(caps.contains(&"fs:read:/tmp/**".to_string()), "{caps:?}");
}

#[test]
fn delegated_param_through_wrapper_stays_delegated() {
    let dir = tree(
        "deleg",
        &[
            ("main.rnx", "import { load } from \"./lib\";\nfn Main(): Int {\n    load(\"/tmp/x\");\n    return 0;\n}\n"),
            ("lib.rnx", "export fn load(path: String): Int {\n    File.open(path, FileMode.Read);\n    return 0;\n}\n"),
        ],
    );
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let caps = cap_set(&analyze_graph(&g));
    assert!(caps.contains(&"fs:delegated".to_string()), "{caps:?}");
}

#[test]
fn mutated_delegated_through_wrapper_is_s201() {
    let dir = tree(
        "s201",
        &[
            ("main.rnx", "import { load } from \"./lib\";\nfn Main(): Int {\n    load(\"/tmp\");\n    return 0;\n}\n"),
            ("lib.rnx", "export fn load(path: String): Int {\n    File.open(path + \"/x\", FileMode.Read);\n    return 0;\n}\n"),
        ],
    );
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let mut owned: Vec<(std::path::PathBuf, String)> = Vec::new();
    for f in &g.files {
        owned.push((f.path.clone(), file_source(&f.path)));
    }
    let inputs: Vec<GraphInput> = g
        .files
        .iter()
        .map(|f| {
            let src = owned.iter().find(|(p, _)| p == &f.path).unwrap();
            GraphInput { path: f.path.clone(), module: &f.module, source: src.1.as_str() }
        })
        .collect();
    match capabilities::analyze_file_graph(&inputs, capabilities::DEFAULT_FUEL) {
        Err(frontend::capabilities::SecurityDiagnostic::S201 { .. }) => {}
        other => panic!("expected S201, got {other:?}"),
    }
}

#[test]
fn std_wrapper_chain_attributes_through_real_fs() {
    let dir = tree(
        "stdfs",
        &[(
            "main.rnx",
            "import { readText } from \"@std/fs\";\nfn Main(): Int {\n    readText(\"/tmp/real.txt\");\n    return 0;\n}\n",
        )],
    );
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    assert!(g.files.iter().any(|f| f.path.to_string_lossy() == "@std/fs"));
    let caps = cap_set(&analyze_graph(&g));
    assert!(caps.contains(&"fs:read:/tmp/**".to_string()), "{caps:?}");
}

#[test]
fn std_wrapper_alias_matches_bare_spelling_charge() {
    let bare = tree(
        "wbare",
        &[(
            "main.rnx",
            "import { readText } from \"@std/fs\";\nfn Main(): Int {\n    readText(\"/tmp/real.txt\");\n    return 0;\n}\n",
        )],
    );
    let aliased = tree(
        "walias",
        &[(
            "main.rnx",
            "import { readText as rd } from \"@std/fs\";\nfn Main(): Int {\n    rd(\"/tmp/real.txt\");\n    return 0;\n}\n",
        )],
    );
    let g_bare = ModuleGraph::build(&bare.join("main.rnx")).unwrap();
    let g_alias = ModuleGraph::build(&aliased.join("main.rnx")).unwrap();
    assert_eq!(cap_set(&analyze_graph(&g_bare)), cap_set(&analyze_graph(&g_alias)));
}
