#[path = "../../testkit/pack.rs"]
mod pack_fixture;
#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

const MUSL: &str = "x86_64-unknown-linux-musl";
const ARM_MUSL: &str = "aarch64-unknown-linux-musl";

fn rnx() -> &'static str {
    env!("CARGO_BIN_EXE_rnx")
}

fn vendor() -> String {
    format!("{}/../stdlib/src", env!("CARGO_MANIFEST_DIR"))
}

fn run_in(dir: &Path, cache: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut cmd = std::process::Command::new(rnx());
    cmd.args(args)
        .current_dir(dir)
        .env("RNX_CACHE_HOME", cache)
        .env("RNX_VENDOR_DIR", vendor())
        .env("RNX_REGISTRY", "http://127.0.0.1:9")
        .env_remove("RNX_SYSROOT")
        .env_remove("RNX_SYSROOT_OCI")
        .env_remove("RNX_OFFLINE")
        .env_remove("CI");
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().unwrap()
}

fn run(dir: &Path, cache: &Path, args: &[&str]) -> Output {
    run_in(dir, cache, &[], args)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn combined(output: &Output) -> String {
    format!("{}{}", stdout(output), stderr(output))
}

fn project(tag: &str, targets: Option<&str>) -> (crate::scratch::Scratch, PathBuf) {
    let root = crate::scratch::dir(&format!("pack-proj-{tag}"));
    let targets = match targets {
        Some(list) => format!(",\n        targets: [{list}]"),
        None => String::new(),
    };
    std::fs::write(
        root.join("Project.config"),
        format!(
            "export default {{\n    project: {{\n        name: \"pack-probe\",\n        version: \"0.1.0\"{targets}\n    }}\n}}\n"
        ),
    )
    .unwrap();
    let src = root.join("src").join("main.rnx");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, "fn Main(): Int {\n    return 0;\n}\n").unwrap();
    (root, src)
}

fn cache(tag: &str) -> crate::scratch::Scratch {
    crate::scratch::dir(&format!("pack-cache-{tag}"))
}

fn pack(tag: &str, triple: &str, salt: &str) -> (crate::scratch::Scratch, String) {
    let dir = crate::scratch::dir(&format!("pack-fixture-{tag}"));
    let digest = crate::pack_fixture::write_pack(&dir, triple, salt);
    (dir, digest)
}

fn install(cache: &Path, dir: &Path) -> String {
    let out = run(
        dir,
        cache,
        &["sysroot", "add", "--local", dir.to_str().unwrap()],
    );
    assert!(out.status.success(), "{}", combined(&out));
    let text = stdout(&out);
    let digest = text
        .split_whitespace()
        .last()
        .expect("install prints the digest")
        .to_string();
    digest
}

#[test]
fn local_install_list_path_verify_remove_prune() {
    let cache = cache("lifecycle");
    let (fixture, digest) = pack("lifecycle", MUSL, "one");
    let installed = install(&cache, &fixture);
    assert_eq!(installed, digest);

    let list = run(&cache, &cache, &["sysroot", "list"]);
    assert!(list.status.success(), "{}", combined(&list));
    let text = stdout(&list);
    let header = text.lines().next().unwrap_or("");
    for column in ["TRIPLE", "DIGEST", "TIER", "CURRENT"] {
        assert!(header.contains(column), "missing {column} in {header:?}");
    }
    assert!(text.contains(MUSL), "{text}");
    assert!(text.contains(&digest), "{text}");
    assert!(
        text.lines().nth(1).unwrap_or("").trim_end().ends_with('*'),
        "{text}"
    );

    let json = run(&cache, &cache, &["sysroot", "list", "--json"]);
    assert!(json.status.success(), "{}", combined(&json));
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&json.stdout).expect("JSON array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["triple"], MUSL);
    assert_eq!(rows[0]["digest"], digest.as_str());
    assert_eq!(rows[0]["tier"], "2");
    assert_eq!(rows[0]["current"], true);

    let path = run(&cache, &cache, &["sysroot", "path", MUSL]);
    assert!(path.status.success(), "{}", combined(&path));
    let expected = cache.join("sysroots").join(MUSL).join(&digest);
    assert_eq!(stdout(&path).trim(), expected.to_string_lossy());

    let verify = run(&cache, &cache, &["sysroot", "verify", MUSL]);
    assert!(verify.status.success(), "{}", combined(&verify));
    assert!(stdout(&verify).contains("verified"));

    let (second, second_digest) = pack("lifecycle-two", MUSL, "two");
    assert_ne!(second_digest, digest);
    install(&cache, &second);
    let pruned = run(&cache, &cache, &["sysroot", "prune"]);
    assert!(pruned.status.success(), "{}", combined(&pruned));
    assert!(stdout(&pruned).contains("pruned 1 pack(s)"));
    let list = run(&cache, &cache, &["sysroot", "list", "--json"]);
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["digest"], second_digest.as_str());
    assert!(!expected.exists(), "prune must drop the old digest");

    let removed = run(&cache, &cache, &["sysroot", "remove", MUSL]);
    assert!(removed.status.success(), "{}", combined(&removed));
    assert!(stdout(&removed).contains("removed 1 pack(s)"));
    let missing = run(&cache, &cache, &["sysroot", "path", MUSL]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(stderr(&missing).contains("E124"), "{}", stderr(&missing));
}

#[test]
fn corrupt_byte_fails_verify() {
    let cache = cache("corrupt");
    let (fixture, digest) = pack("corrupt", MUSL, "one");
    install(&cache, &fixture);
    let payload = cache
        .join("sysroots")
        .join(MUSL)
        .join(&digest)
        .join("lib")
        .join("rustlib")
        .join(MUSL)
        .join("lib")
        .join("libc.a");
    assert!(payload.is_file(), "{}", payload.display());
    std::fs::write(&payload, b"corrupted").unwrap();
    let verify = run(&cache, &cache, &["sysroot", "verify", MUSL]);
    assert_eq!(verify.status.code(), Some(1), "{}", combined(&verify));
    let text = stderr(&verify);
    assert!(text.contains("E124"), "{text}");
    assert!(text.contains("integrity digest mismatch"), "{text}");
}

#[test]
fn unsafe_manifests_and_symlinks_are_rejected() {
    let cache = cache("unsafe");
    let absolute = crate::scratch::dir("pack-abs");
    crate::pack_fixture::write_pack_with_search(&absolute, MUSL, "/usr/lib");
    let out = run(
        &cache,
        &cache,
        &["sysroot", "add", "--local", absolute.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("E124"), "{}", stderr(&out));
    assert!(stderr(&out).contains("absolute"), "{}", stderr(&out));

    let parent = crate::scratch::dir("pack-parent");
    crate::pack_fixture::write_pack_with_search(&parent, MUSL, "lib/../escape");
    let out = run(
        &cache,
        &cache,
        &["sysroot", "add", "--local", parent.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("normalized"), "{}", stderr(&out));

    #[cfg(unix)]
    {
        let (fixture, _) = pack("symlink", MUSL, "one");
        std::os::unix::fs::symlink("libc.a", fixture.join("lib").join("alias.a")).unwrap();
        let out = run(
            &cache,
            &cache,
            &["sysroot", "add", "--local", fixture.to_str().unwrap()],
        );
        assert_eq!(out.status.code(), Some(1));
        assert!(stderr(&out).contains("E124"), "{}", stderr(&out));
        assert!(stderr(&out).contains("symlink"), "{}", stderr(&out));
    }
}

#[test]
fn digest_assertion_and_local_triple_must_match() {
    let cache = cache("assert");
    let (fixture, digest) = pack("assert", MUSL, "one");
    let wrong = format!("sha256:{}", "0".repeat(64));
    let rejected = run(
        &cache,
        &cache,
        &[
            "sysroot",
            "add",
            "--local",
            fixture.to_str().unwrap(),
            "--digest",
            &wrong,
        ],
    );
    assert_eq!(rejected.status.code(), Some(1), "{}", combined(&rejected));
    assert!(
        stderr(&rejected).contains("digest mismatch"),
        "{}",
        stderr(&rejected)
    );

    let accepted = run(
        &cache,
        &cache,
        &[
            "sysroot",
            "add",
            MUSL,
            "--local",
            fixture.to_str().unwrap(),
            "--digest",
            &digest,
        ],
    );
    assert!(accepted.status.success(), "{}", combined(&accepted));

    let mismatch = run(
        &cache,
        &cache,
        &[
            "sysroot",
            "add",
            ARM_MUSL,
            "--local",
            fixture.to_str().unwrap(),
        ],
    );
    assert_eq!(mismatch.status.code(), Some(1), "{}", combined(&mismatch));
    assert!(
        stderr(&mismatch).contains("carries target"),
        "{}",
        stderr(&mismatch)
    );
}

#[test]
fn digest_is_deterministic_across_install_and_cache() {
    let (fixture, digest) = pack("deterministic", ARM_MUSL, "same");
    let first = cache("det-one");
    let second = cache("det-two");
    let installed_first = install(&first, &fixture);
    let installed_second = install(&second, &fixture);
    assert_eq!(installed_first, digest);
    assert_eq!(installed_second, digest);
    let again = install(&first, &fixture);
    assert_eq!(again, digest);
    let out = run(&first, &first, &["sysroot", "path", ARM_MUSL]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    assert_eq!(
        stdout(&out).trim(),
        first
            .join("sysroots")
            .join(ARM_MUSL)
            .join(&digest)
            .to_string_lossy()
    );
}

#[test]
fn lock_pins_sysroots_and_rejects_a_mismatch() {
    let cache = cache("lock");
    let (fixture, digest) = pack("lock", MUSL, "one");
    install(&cache, &fixture);
    let (root, main) = project("lock", Some(&format!("\"{MUSL}\"")));

    let lock = run(&root, &cache, &["lock"]);
    assert!(lock.status.success(), "{}", combined(&lock));
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(text.contains("version: 3"), "{text}");
    assert!(text.contains("sysroots: {"), "{text}");
    assert!(text.contains(MUSL), "{text}");
    assert!(text.contains(&digest), "{text}");
    let parsed = frontend::deplock::ProjectDepLock::parse(&text).unwrap();
    assert_eq!(parsed.sysroots.get(MUSL), Some(&digest));

    let v2 = "export default {\n    version: 2,\n    packages: []\n}\n";
    let parsed_v2 = frontend::deplock::ProjectDepLock::parse(v2).unwrap();
    assert_eq!(parsed_v2.version, 2);
    assert!(parsed_v2.sysroots.is_empty());
    std::fs::write(root.join("Project.deplock"), v2).unwrap();
    let relock = run(&root, &cache, &["lock"]);
    assert!(relock.status.success(), "{}", combined(&relock));
    let upgraded = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(upgraded.contains("version: 3"), "{upgraded}");
    assert!(upgraded.contains(&digest), "{upgraded}");

    let tampered = upgraded.replace(&digest, &format!("sha256:{}", "0".repeat(64)));
    std::fs::write(root.join("Project.deplock"), &tampered).unwrap();
    let build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--locked",
            "--emit-obj",
            "--target",
            MUSL,
        ],
    );
    assert_eq!(build.status.code(), Some(1), "{}", combined(&build));
    assert!(stderr(&build).contains("E124"), "{}", stderr(&build));
}

#[test]
fn lock_target_records_an_installed_pack_without_project_targets() {
    let cache = cache("lock-target");
    let (fixture, digest) = pack("lock-target", ARM_MUSL, "one");
    install(&cache, &fixture);
    let (root, _main) = project("lock-target", None);
    let lock = run(&root, &cache, &["lock", "--target", ARM_MUSL]);
    assert!(lock.status.success(), "{}", combined(&lock));
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(text.contains(ARM_MUSL), "{text}");
    assert!(text.contains(&digest), "{text}");
}

#[test]
fn offline_and_ci_missing_pack_fail_e124() {
    let (root, main) = project("offline", Some(&format!("\"{MUSL}\"")));
    let no_ci = cache("offline-ci");
    let build = run_in(
        &root,
        &no_ci,
        &[("CI", "1")],
        &[
            "build",
            main.to_str().unwrap(),
            "--emit-obj",
            "--target",
            MUSL,
        ],
    );
    assert_eq!(build.status.code(), Some(1), "{}", combined(&build));
    assert!(stderr(&build).contains("E124"), "{}", stderr(&build));
    assert!(
        stderr(&build).contains(&format!("rnx sysroot add {MUSL}")),
        "{}",
        stderr(&build)
    );

    let offline = cache("offline-flag");
    let build = run(
        &root,
        &offline,
        &[
            "build",
            main.to_str().unwrap(),
            "--emit-obj",
            "--target",
            MUSL,
            "--offline",
        ],
    );
    assert_eq!(build.status.code(), Some(1), "{}", combined(&build));
    assert!(stderr(&build).contains("E124"), "{}", stderr(&build));
}

#[test]
fn host_build_needs_no_pack() {
    let cache = cache("host");
    let (root, main) = project("host", None);
    let build = run(&root, &cache, &["build", main.to_str().unwrap()]);
    assert!(build.status.success(), "{}", combined(&build));
    let artifact = stdout(&build)
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("artifact: "))
        .map(|value| value.trim().trim_end_matches(" (fresh)").to_string())
        .expect("artifact path");
    // The build itself proves no pack was consulted; the empty store is the
    // assertion that selection stays host-only.
    assert!(Path::new(&artifact).is_file(), "{artifact}");
    let list = run(&cache, &cache, &["sysroot", "list", "--json"]);
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&list.stdout).unwrap();
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn v1_custom_sysroot_remains_readable() {
    let root = crate::scratch::dir("pack-v1-root");
    std::fs::write(root.join("sysroot.json"), "{\"version\": 1}\n").unwrap();
    let (project_root, main) = project("v1", None);
    let out = run(
        &project_root,
        &cache("v1"),
        &[
            "build",
            main.to_str().unwrap(),
            "--sysroot",
            root.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "{}", combined(&out));
}

struct Origin {
    bare: String,
}

fn start_origin(index: Vec<u8>, files: BTreeMap<String, Vec<u8>>) -> Origin {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let bare = format!("{addr}/rovelstars/rnx-sysroot");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => head.push(byte[0]),
                }
                if head.len() > 65536 {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&head).into_owned();
            let path = text
                .lines()
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let body = if let Some(reference) = path.rsplit('/').next() {
                if path.contains("/manifests/") {
                    if reference.starts_with("sha256:") {
                        files.get(reference).cloned()
                    } else {
                        Some(index.clone())
                    }
                } else if path.contains("/blobs/") {
                    files.get(reference).cloned()
                } else {
                    None
                }
            } else {
                None
            };
            match body {
                Some(bytes) => {
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&bytes);
                }
                None => {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
            let _ = stream.flush();
        }
    });
    Origin { bare }
}

#[test]
fn oci_add_installs_a_pack_and_checks_annotations() {
    let (fixture, digest) = pack("oci", MUSL, "oci");
    let archive = frontend::oci::archive_pack_dir(&fixture).unwrap();
    let assembly = frontend::oci::assemble_index(&[frontend::oci::AssembleInput {
        role: "sysroot".to_string(),
        title: "sysroot.tar.gz".to_string(),
        kind: "sysroot".to_string(),
        target: MUSL.to_string(),
        platform: frontend::oci::platform_for_triple(MUSL),
        generator: "rnx-sysroot 0.2.1".to_string(),
        upstream: format!("musl@sha256:{}", "cd".repeat(32)),
        bytes: archive,
    }])
    .unwrap();
    let mut files: BTreeMap<String, Vec<u8>> = assembly.manifests.iter().cloned().collect();
    files.extend(assembly.blobs.iter().cloned());
    let origin = start_origin(assembly.index_bytes, files);
    let cache = cache("oci");

    let out = run_in(
        &cache,
        &cache,
        &[("RNX_SYSROOT_OCI", origin.bare.as_str())],
        &["sysroot", "add", MUSL],
    );
    assert!(out.status.success(), "{}", combined(&out));
    assert!(stdout(&out).contains(&digest), "{}", stdout(&out));
    let path = run(&cache, &cache, &["sysroot", "path", MUSL]);
    assert_eq!(path.status.code(), Some(0), "{}", combined(&path));
    assert!(Path::new(stdout(&path).trim()).is_dir());

    let wrong = run_in(
        &cache,
        &cache,
        &[("RNX_SYSROOT_OCI", origin.bare.as_str())],
        &["sysroot", "add", ARM_MUSL],
    );
    assert_eq!(wrong.status.code(), Some(1), "{}", combined(&wrong));
    assert!(stderr(&wrong).contains("E124"), "{}", stderr(&wrong));
    assert!(
        stderr(&wrong).contains("no `sysroot` manifest"),
        "{}",
        stderr(&wrong)
    );
}
