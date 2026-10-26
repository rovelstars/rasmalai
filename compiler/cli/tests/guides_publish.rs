#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::io::{Read, Write};

fn rnx(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_rnx"));
    cmd.args(args).current_dir(dir);
    cmd.output().unwrap()
}

fn serve_publish_once() -> (String, std::sync::mpsc::Receiver<Vec<u8>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => head.push(byte[0]),
                Err(_) => break,
            }
            if head.len() > 65536 {
                break;
            }
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let mut length = 0usize;
        for line in text.lines().skip(1) {
            if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = rest.trim().parse::<usize>().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; length.min(16 * 1024 * 1024)];
        let mut read = 0usize;
        while read < body.len() {
            match stream.read(&mut body[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(_) => break,
            }
        }
        body.truncate(read);
        let _ = tx.send(body);
        let payload = "{\"success\": true, \"url\": \"/packages/probe\", \"version\": \"0.1.0\", \"manifestDigest\": \"ab\"}";
        let head = format!(
            "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(payload.as_bytes());
    });
    (url, rx)
}

#[test]
fn publish_posts_collected_guides_to_mock_registry() {
    let (registry, rx) = serve_publish_once();
    let dir = crate::scratch::dir("guides-pub-body");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"probe\",\n        version: \"0.1.0\",\n        description: \"Probe package\"\n    },\n    entries: {\n        main: \"src/main.rnx\",\n        docs: \"docs\"\n    },\n    license: \"MIT\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("main.rnx"),
        "fn Main(): Int {\n    return 0;\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("LICENSE"),
        "MIT License\n\nPermission is hereby granted.\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("docs").join("advanced")).unwrap();
    std::fs::write(
        dir.join("docs").join("getting-started.md"),
        "---\ntitle: \"Getting Started\"\ndescription: \"First steps.\"\nsection: \"Basics\"\norder: \"1\"\n---\n\n# Getting Started\n\nHello.\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("docs").join("advanced").join("perf.md"),
        "# Perf\n\nGo fast.\n",
    )
    .unwrap();
    let fmt = rnx(&dir, &["fmt"]);
    assert!(
        fmt.status.success(),
        "{}",
        String::from_utf8_lossy(&fmt.stderr)
    );
    let out = rnx(
        &dir,
        &["publish", "--registry", &registry, "--token", "test-token"],
    );
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let body = rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
    let frame: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(frame["name"], "probe");
    assert_eq!(frame["version"], "0.1.0");
    let guides = frame["guides"]
        .as_array()
        .expect("frame carries a guides array");
    assert_eq!(guides.len(), 2);
    assert_eq!(guides[0]["slug"], "getting-started");
    assert_eq!(guides[0]["title"], "Getting Started");
    assert_eq!(guides[0]["source"], "# Getting Started\n\nHello.\n");
    assert_eq!(guides[0]["description"], "First steps.");
    assert_eq!(guides[0]["section"], "Basics");
    assert_eq!(guides[0]["order"], "1");
    assert_eq!(guides[1]["slug"], "perf");
    assert_eq!(guides[1]["title"], "Perf");
    assert!(guides[1]["section"].is_null(), "{}", guides[1]["section"]);
    assert!(guides[1]["order"].is_null(), "{}", guides[1]["order"]);
    for entry in guides {
        assert!(entry.get("html").is_none(), "{entry}");
        assert!(entry["slug"].as_str().is_some(), "{entry}");
        assert!(entry["title"].as_str().is_some(), "{entry}");
        assert!(entry["source"].as_str().is_some(), "{entry}");
    }
}
