#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> crate::scratch::Scratch {
    let dir = crate::scratch::dir(&format!("dns-{tag}"));
    dir
}

fn run_backend(dir: &std::path::Path, src: &str, backend: Option<&str>) -> std::process::Output {
    let prog = dir.join("main.rnx");
    std::fs::write(&prog, src).unwrap();
    let mut cmd = Command::new(rnx());
    cmd.arg("run").env("NO_COLOR", "1");
    if let Some(b) = backend {
        cmd.arg("--backend").arg(b);
    }
    cmd.arg(&prog);
    cmd.output().unwrap()
}

fn expect_backends(tag: &str, src: &str, want: &str) {
    for backend in [Some("interpreter"), Some("cranelift"), Some("llvm")] {
        let dir = fresh_dir(&format!("{tag}-{}", backend.unwrap_or("interp")));
        let out = run_backend(&dir, src, backend);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{tag} {:?} failed: {stderr}", backend);
        assert_eq!(stdout, want, "{tag} {:?} stdout mismatch", backend);
    }
}

fn expect_failure_everywhere(tag: &str, src: &str, want_err: &str) {
    for backend in [Some("interpreter"), Some("cranelift"), Some("llvm")] {
        let dir = fresh_dir(&format!("{tag}-{}", backend.unwrap_or("interp")));
        let out = run_backend(&dir, src, backend);
        assert!(!out.status.success(), "{tag} {:?} unexpectedly succeeded", backend);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let combined = format!("{stdout}{stderr}");
        assert!(
            combined.contains(want_err),
            "{tag} {:?} missing `{want_err}` in: {combined}",
            backend
        );
    }
}

const IP_FAST_PATH: &str = r#"import { Dns } from "@std/net";
let v4 = await Dns.lookup("127.0.0.1");
print(v4);
let v6 = await Dns.lookup("::1");
print(v6);
"#;

#[test]
fn dns_ip_literals_resolve_synchronously() {
    expect_backends("ipfast", IP_FAST_PATH, "127.0.0.1\n::1\n");
}

const LOCALHOST: &str = r#"import { Dns } from "@std/net";
let ip = await Dns.lookup("localhost");
print(ip == "127.0.0.1" || ip == "::1");
"#;

#[test]
fn dns_localhost_resolves_off_thread() {
    expect_backends("localhost", LOCALHOST, "true\n");
}

const NXDOMAIN: &str = r#"import { Dns } from "@std/net";
let ip = await Dns.lookup("this-domain-definitely-does-not-exist.invalid");
print(ip);
"#;

#[test]
fn dns_unknown_host_rejects_cleanly() {
    expect_failure_everywhere("nxdomain", NXDOMAIN, "DNS resolution failed");
}

const FETCH_LOCALHOST: &str = r#"import { TcpStream, TcpListener } from "@std/net";
import { fetch, Response } from "@std/web";
import { ByteBuffer } from "@std/bytes";

fn s2b(s: String): Array<Int> {
    let out: Array<Int> = [];
    let i = 0;
    while i < s.length() {
        out.push(s.charCodeAt(i));
        i = i + 1;
    }
    return out;
}

fn b2s(data: Array<Int>): String {
    return ByteBuffer.fromArray(data).readString(0, data.length());
}

fn readReq(sh: TcpStream): String {
    let buf: Array<Int> = [];
    let done = false;
    while ! done {
        let chunk: Array<Int> = sh.read(1024).wait().unwrapOr([]);
        if chunk.length() == 0 {
            done = true;
        } else {
            let i = 0;
            while i < chunk.length() {
                buf.push(chunk[i]);
                i = i + 1;
            }
            let m = buf.length();
            if m >= 4 && buf[m - 4] == 13 && buf[m - 3] == 10 && buf[m - 2] == 13 && buf[m - 1] == 10 {
                done = true;
            }
        }
    }
    return b2s(buf);
}

fn writeAll(sh: TcpStream, data: Array<Int>): Int {
    let r = sh.write(data).wait();
    if r.isErr() {
        return 0 - 1;
    }
    return r.unwrap();
}

fn serveOne(lh: Int): Int {
    let listener = new TcpListener(lh);
    let accepted = listener.accept().wait();
    if accepted.isErr() {
        return 0 - 1;
    }
    let sh: TcpStream = accepted.unwrap();
    let req = readReq(sh);
    let eol = "\u{D}\u{A}";
    let body = "dns-ok";
    let res = "HTTP/1.1 200 OK" + eol + "Content-Length: 6" + eol + "Connection: close" + eol + eol + body;
    let w = writeAll(sh, s2b(res));
    sh.close();
    if w < 0 {
        return 0 - 3;
    }
    return req.length();
}

let listener = TcpListener.bind("127.0.0.1", 0);
let port = listener.port();
let base = "http://localhost:" + port.toString();
let srv = Thread.spawn((): Int => serveOne(listener.handle));
let r: Response = await fetch(base + "/hello");
print(r.status);
print(r.text());
let done = srv.join().unwrap();
print(done > 0);
listener.close();
"#;

#[test]
fn fetch_localhost_resolves_through_async_dns() {
    expect_backends("fetchdns", FETCH_LOCALHOST, "200\ndns-ok\ntrue\n");
}
