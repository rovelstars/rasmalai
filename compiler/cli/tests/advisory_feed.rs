#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use frontend::advisory::{feed_canonical_bytes, key_id_for_public_key, Advisory};
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::io::{Read, Write};
use std::process::{Command, Stdio};

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn pkg(tag: &str) -> crate::scratch::Scratch {
    let dir = crate::scratch::dir(&format!("advfeed-{tag}"));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/main.rnx\" }\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
    dir
}

fn write_deplock(dir: &std::path::Path, version: &str, artifact: Option<&str>) {
    let pin = artifact
        .map(|p| format!("            artifactDigest: \"{p}\",\n"))
        .unwrap_or_default();
    let text = format!(
        "// Project.deplock - GENERATED AUTOMATICALLY BY RNX LOCK - DO NOT EDIT\nexport default {{\n    version: 2,\n    packages: [\n        {{\n            name: \"@acme/widget\",\n            version: \"{version}\",\n            source: \"registry:@acme/widget@{version}\",\n            checksum: \"abc\",\n{pin}            dependencies: [],\n            tier: \"pure\",\n            capabilities: []\n        }}\n    ]\n}}\n"
    );
    std::fs::write(dir.join("Project.deplock"), text).unwrap();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn keypair() -> Ed25519KeyPair {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
}

fn sign_feed(key: &Ed25519KeyPair, advisories: Vec<Advisory>) -> String {
    let message = feed_canonical_bytes(&advisories);
    let signature = key.sign(&message);
    let public_key = key.public_key().as_ref().to_vec();
    let key_id = key_id_for_public_key(&public_key);
    let body = serde_json::json!({
        "advisories": advisories.iter().map(|a| serde_json::json!({
            "id": a.id,
            "package": a.package,
            "affected": a.affected,
            "artifacts": a.artifacts,
            "severity": a.severity,
            "title": a.title,
            "detail": a.detail,
        })).collect::<Vec<_>>(),
        "keyId": key_id,
        "publicKey": hex(&public_key),
        "signature": hex(signature.as_ref()),
    });
    body.to_string()
}

fn serve(body: String, spec: Option<&str>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let header = match spec {
        Some(s) => format!("rnx-registry-spec: {s}\r\n"),
        None => String::new(),
    };
    std::thread::spawn(move || {
        for _ in 0..4 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            let mut buf = vec![0u8; 65536];
            let mut head = Vec::new();
            loop {
                let Ok(n) = stream.read(&mut buf) else {
                    break;
                };
                if n == 0 {
                    break;
                }
                head.extend_from_slice(&buf[..n]);
                if head.windows(4).any(|w| w == b"\r\n\r\n") || head.len() > 65536 {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    url
}

fn feed_advisory(id: &str, affected: &str, artifacts: Vec<String>, severity: &str) -> Advisory {
    Advisory {
        id: id.to_string(),
        package: "@acme/widget".to_string(),
        affected: affected.to_string(),
        artifacts,
        severity: severity.to_string(),
        title: format!("{id} title"),
        detail: format!("{id} detail"),
    }
}

fn audit(dir: &std::path::Path, registry: &str, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["audit", "--path", dir.to_str().unwrap(), "--registry", registry];
    args.extend_from_slice(extra);
    Command::new(rnx())
        .args(&args)
        .env("RNX_ADVISORY_KEY", "")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn combined(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn critical_hold_fails_audit() {
    let dir = pkg("critical");
    write_deplock(&dir, "1.4.3", None);
    let key = keypair();
    let url = serve(sign_feed(&key, vec![feed_advisory("ADV-1", "^1.0.0", vec![], "critical")]), Some("0"));
    let out = audit(&dir, &url, &[]);
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    let text = combined(&out);
    assert!(text.contains("critical security hold"), "{text}");
    assert!(text.contains("ADV-1"), "{text}");
    assert!(text.contains("@acme/widget@1.4.3"), "{text}");
}

#[test]
fn critical_json_keeps_stdout_pure() {
    let dir = pkg("critical-json");
    write_deplock(&dir, "1.4.3", None);
    let key = keypair();
    let url = serve(sign_feed(&key, vec![feed_advisory("ADV-1", "^1.0.0", vec![], "critical")]), Some("0"));
    let out = audit(&dir, &url, &["--json"]);
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout stays pure report JSON");
    assert_eq!(v["tier"], "pure");
    assert_eq!(v["name"], "app");
}

#[test]
fn warning_warns_like_yank() {
    let dir = pkg("warning");
    write_deplock(&dir, "1.4.3", None);
    let key = keypair();
    let url = serve(sign_feed(&key, vec![feed_advisory("ADV-7", "^1.0.0", vec![], "warning")]), Some("0"));
    let out = audit(&dir, &url, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let text = combined(&out);
    assert!(
        text.contains("warning: package `@acme/widget@1.4.3` is affected by advisory ADV-7"),
        "{text}"
    );
}

#[test]
fn pin_exactness_gates_artifact_advisories() {
    let pin = format!("sha256:{}", "ab".repeat(32));
    let other = format!("sha256:{}", "cd".repeat(32));
    let key = keypair();
    let body = sign_feed(
        &key,
        vec![feed_advisory("ADV-9", "*", vec![pin.clone()], "critical")],
    );
    let url = serve(body, Some("0"));
    let dir = pkg("pin-miss");
    write_deplock(&dir, "1.0.0", Some(&other));
    let out = audit(&dir, &url, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    assert!(combined(&out).contains("no advisories apply"), "{}", combined(&out));
    let dir = pkg("pin-hit");
    write_deplock(&dir, "1.0.0", Some(&pin));
    let out = audit(&dir, &url, &[]);
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    assert!(combined(&out).contains("(pinned artifact)"), "{}", combined(&out));
}

#[test]
fn tampered_feed_fails_closed() {
    let dir = pkg("tamper");
    write_deplock(&dir, "1.4.3", None);
    let key = keypair();
    let mut body = sign_feed(&key, vec![feed_advisory("ADV-1", "^1.0.0", vec![], "critical")]);
    body = body.replace("ADV-1 title", "forged title");
    let url = serve(body, Some("0"));
    let out = audit(&dir, &url, &[]);
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    assert!(combined(&out).contains("signature verification failed"), "{}", combined(&out));
}

#[test]
fn missing_spec_header_fails_closed() {
    let dir = pkg("nospec");
    write_deplock(&dir, "1.4.3", None);
    let key = keypair();
    let url = serve(sign_feed(&key, vec![]), None);
    let out = audit(&dir, &url, &[]);
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    assert!(combined(&out).contains("rnx-registry-spec"), "{}", combined(&out));
}

#[test]
fn unavailable_feed_warns_and_passes() {
    let dir = pkg("unavail");
    write_deplock(&dir, "1.4.3", None);
    let out = audit(&dir, "http://127.0.0.1:1", &[]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    assert!(combined(&out).contains("advisory feed unavailable"), "{}", combined(&out));
}

#[test]
fn no_deplock_skips_advisory_check() {
    let dir = pkg("nodeplock");
    let key = keypair();
    let url = serve(sign_feed(&key, vec![feed_advisory("ADV-1", "*", vec![], "critical")]), Some("0"));
    let out = audit(&dir, &url, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    assert!(combined(&out).contains("skipping advisory check"), "{}", combined(&out));
}
