#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

const STDLIB_API_URI: &str = "rasmalai://stdlib/api.json";
const NEW_MODULES: [&str; 5] = ["foundation", "async", "log", "uefi", "wasi"];

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

struct Session {
    child: std::process::Child,
    next_id: i64,
    reader: BufReader<std::process::ChildStdout>,
}

impl Session {
    fn start(envs: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(rnx());
        cmd.arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_remove("RNX_STDLIB_API");
        for (key, value) in envs {
            cmd.env(key, value);
        }
        let mut child = cmd.spawn().expect("spawn rnx mcp");
        let stdout = child.stdout.take().expect("stdout");
        Session { child, next_id: 1, reader: BufReader::new(stdout) }
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let stdin = self.child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("read");
        serde_json::from_str(&line).expect("valid json response")
    }

    fn initialize(&mut self) {
        let resp = self.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "mcp-catalog", "version": "0.0.0" },
            }),
        );
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05", "{resp:?}");
        let msg = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let stdin = self.child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
    }

    fn resource_uris(&mut self) -> Vec<String> {
        let resp = self.request("resources/list", serde_json::json!({}));
        resp["result"]["resources"]
            .as_array()
            .unwrap_or_else(|| panic!("resources array: {resp:?}"))
            .iter()
            .map(|r| r["uri"].as_str().expect("uri").to_string())
            .collect()
    }

    fn read_uri(&mut self, uri: &str) -> String {
        let resp = self.request("resources/read", serde_json::json!({ "uri": uri }));
        assert!(resp.get("error").is_none(), "resource {uri} failed: {resp:?}");
        resp["result"]["contents"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("resource {uri} has no text: {resp:?}"))
            .to_string()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn expected_modules() -> Vec<String> {
    let mut names: Vec<String> = stdlib::MODULES.iter().map(|m| m.to_string()).collect();
    names.sort();
    names
}

fn catalog_modules(text: &str) -> Vec<String> {
    let api: serde_json::Value = serde_json::from_str(text).expect("catalog parses");
    let mut names: Vec<String> = api["modules"]
        .as_array()
        .expect("catalog modules")
        .iter()
        .map(|m| m["name"].as_str().expect("module name").to_string())
        .collect();
    names.sort();
    names
}

fn fresh_cache(tag: &str) -> scratch::Scratch {
    let cache = scratch::dir(tag);
    std::fs::create_dir_all(cache.join("mcp")).expect("cache mcp dir");
    cache
}

fn cache_catalog_path(cache: &Path) -> std::path::PathBuf {
    cache.join("mcp").join("stdlib-api.json")
}

fn write_minimal_vendor(root: &Path) {
    let version = env!("CARGO_PKG_VERSION");
    for name in stdlib::MODULES {
        let top = name.split('/').next().unwrap_or(name);
        let dir = root.join(top);
        std::fs::create_dir_all(&dir).expect("vendor module dir");
        if dir.join("Project.config").is_file() {
            continue;
        }
        std::fs::write(
            dir.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"@std/{top}\",\n        version: \"{version}\"\n    }},\n    entries: {{\n        main: \"lib.rnx\"\n    }}\n}}\n"
            ),
        )
        .expect("vendor manifest");
        std::fs::write(
            dir.join("lib.rnx"),
            "fn placeholder(): Int {\n    return 0;\n}\n",
        )
        .expect("vendor source");
    }
    std::fs::write(
        root.join("net").join("http.rnx"),
        "fn status(): Int {\n    return 0;\n}\n",
    )
    .expect("vendor net/http");
}

#[test]
fn stdlib_catalog_covers_every_module() {
    let cache = fresh_cache("mcp-catalog-fresh");
    let mut s = Session::start(&[("RNX_CACHE_HOME", cache.to_str().unwrap())]);
    s.initialize();
    let uris = s.resource_uris();
    for name in stdlib::MODULES {
        let uri = format!("rasmalai://stdlib/{name}.json");
        assert!(uris.contains(&uri), "resources/list is missing {uri}: {uris:?}");
    }
    let text = s.read_uri(STDLIB_API_URI);
    let modules = catalog_modules(&text);
    assert_eq!(modules, expected_modules(), "catalog modules must equal stdlib::MODULES");
    for want in NEW_MODULES {
        assert!(modules.contains(&want.to_string()), "catalog is missing @std/{want}: {modules:?}");
    }
    let path = cache_catalog_path(&cache);
    assert!(path.is_file(), "catalog cache {} was not written", path.display());
    let cached: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read cache")).expect("cache parses");
    assert_eq!(cached["modules"].as_array().expect("modules").len(), stdlib::MODULES.len());
    let wasi = s.read_uri("rasmalai://stdlib/wasi.json");
    let shard: serde_json::Value = serde_json::from_str(&wasi).expect("wasi shard parses");
    assert_eq!(shard["modules"][0]["name"], "wasi", "{wasi:?}");
}

#[test]
fn stdlib_catalog_is_reused_without_regenerating() {
    let cache = fresh_cache("mcp-catalog-warm");
    let mut first = Session::start(&[("RNX_CACHE_HOME", cache.to_str().unwrap())]);
    first.initialize();
    let first_text = first.read_uri(STDLIB_API_URI);
    let path = cache_catalog_path(&cache);
    let first_bytes = std::fs::read(&path).expect("cache written");
    let first_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).expect("cache mtime");
    drop(first);

    let mut second = Session::start(&[("RNX_CACHE_HOME", cache.to_str().unwrap())]);
    second.initialize();
    let second_text = second.read_uri(STDLIB_API_URI);
    assert_eq!(first_text, second_text, "catalog content must be stable");
    assert_eq!(std::fs::read(&path).expect("cache read"), first_bytes);
    let second_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).expect("cache mtime");
    assert_eq!(
        second_mtime, first_mtime,
        "an unchanged stdlib source tree must not regenerate the catalog"
    );
}

#[test]
fn stdlib_catalog_rebuilds_a_stale_file() {
    let cache = fresh_cache("mcp-catalog-stale");
    let path = cache_catalog_path(&cache);
    std::fs::write(&path, "{\"modules\":[]}").expect("seed stale catalog");
    let mut s = Session::start(&[("RNX_CACHE_HOME", cache.to_str().unwrap())]);
    s.initialize();
    let text = s.read_uri(STDLIB_API_URI);
    assert_eq!(catalog_modules(&text), expected_modules());
    let cached: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read cache")).expect("cache parses");
    assert_eq!(
        cached["modules"].as_array().expect("modules").len(),
        stdlib::MODULES.len(),
        "the stale cache file must be replaced"
    );
}

#[test]
fn stdlib_catalog_rebuilds_when_sources_are_newer() {
    let vendor = scratch::dir("mcp-catalog-vendor");
    write_minimal_vendor(&vendor);
    let cache = fresh_cache("mcp-catalog-newer");
    let env = [
        ("RNX_CACHE_HOME", cache.to_str().unwrap()),
        ("RNX_VENDOR_DIR", vendor.to_str().unwrap()),
    ];
    let mut first = Session::start(&env);
    first.initialize();
    let first_text = first.read_uri(STDLIB_API_URI);
    assert_eq!(catalog_modules(&first_text), expected_modules());
    let path = cache_catalog_path(&cache);
    let first_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).expect("cache mtime");
    drop(first);

    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(
        vendor.join("math").join("lib.rnx"),
        "fn placeholder(): Int {\n    return 1;\n}\n",
    )
    .expect("touch vendor source");

    let mut second = Session::start(&env);
    second.initialize();
    let second_text = second.read_uri(STDLIB_API_URI);
    assert_eq!(catalog_modules(&second_text), expected_modules());
    let second_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).expect("cache mtime");
    assert!(
        second_mtime > first_mtime,
        "a newer stdlib source must regenerate the catalog"
    );
}

#[test]
fn stdlib_catalog_env_override_wins_over_the_cache() {
    let dir = scratch::dir("mcp-catalog-override");
    let explicit = dir.join("custom-api.json");
    std::fs::write(
        &explicit,
        "{\"modules\":[{\"name\":\"custom\",\"path\":\"\",\"docs\":{},\"functions\":[],\"classes\":[],\"enums\":[],\"constants\":[]}]}\n",
    )
    .expect("write explicit catalog");
    let cache = fresh_cache("mcp-catalog-override-cache");
    let mut s = Session::start(&[
        ("RNX_CACHE_HOME", cache.to_str().unwrap()),
        ("RNX_STDLIB_API", explicit.to_str().unwrap()),
    ]);
    s.initialize();
    let text = s.read_uri(STDLIB_API_URI);
    let api: serde_json::Value = serde_json::from_str(&text).expect("override parses");
    assert_eq!(api["modules"][0]["name"], "custom", "RNX_STDLIB_API must win: {text:?}");
    assert!(
        !cache_catalog_path(&cache).exists(),
        "an explicit RNX_STDLIB_API must not populate the cache"
    );
}

#[test]
fn stdlib_catalog_survives_concurrent_cold_reads() {
    let cache = fresh_cache("mcp-catalog-race");
    let mut a = Session::start(&[("RNX_CACHE_HOME", cache.to_str().unwrap())]);
    let mut b = Session::start(&[("RNX_CACHE_HOME", cache.to_str().unwrap())]);
    a.initialize();
    b.initialize();
    let (text_a, text_b) = std::thread::scope(|scope| {
        let ha = scope.spawn(|| a.read_uri(STDLIB_API_URI));
        let hb = scope.spawn(|| b.read_uri(STDLIB_API_URI));
        (ha.join().expect("first catalog"), hb.join().expect("second catalog"))
    });
    assert_eq!(catalog_modules(&text_a), expected_modules());
    assert_eq!(text_a, text_b, "concurrent cold reads must agree");
    let leftovers: Vec<String> = std::fs::read_dir(cache.join("mcp"))
        .expect("cache mcp dir")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_string_lossy().into_owned();
            name.starts_with('.').then_some(name)
        })
        .collect();
    assert!(leftovers.is_empty(), "temporary generation files left behind: {leftovers:?}");
}
