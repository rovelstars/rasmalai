use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-dns-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_backend(dir: &PathBuf, src: &str, backend: Option<&str>) -> std::process::Output {
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
    for backend in [None, Some("cranelift"), Some("llvm")] {
        let dir = fresh_dir(&format!("{tag}-{}", backend.unwrap_or("interp")));
        let out = run_backend(&dir, src, backend);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{tag} {:?} failed: {stderr}", backend);
        assert_eq!(stdout, want, "{tag} {:?} stdout mismatch", backend);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn expect_failure_everywhere(tag: &str, src: &str, want_err: &str) {
    for backend in [None, Some("cranelift"), Some("llvm")] {
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
        let _ = std::fs::remove_dir_all(&dir);
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
    let out = "";
    let i = 0;
    while i < data.length() {
        out = out + __rnx_string_from_char_code(data[i]);
        i = i + 1;
    }
    return out;
}

fn readReq(sh: Int): String {
    let buf: Array<Int> = [];
    let done = false;
    while ! done {
        let n = __rnx_net_recv_or_wait(sh, 1024);
        if n <= 0 {
            done = true;
        } else {
            let i = 0;
            while i < n {
                buf.push(__rnx_net_recv_get(sh, i));
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

fn writeAll(sh: Int, data: Array<Int>): Int {
    let i = 0;
    while i < data.length() {
        let r = __rnx_net_send_or_wait(sh, data[i]);
        if r < 0 {
            return r;
        }
        i = i + 1;
    }
    return i;
}

fn serveOne(lh: Int): Int {
    let r = __rnx_net_listener_accept_start(lh);
    let sh = r;
    if r < 0 {
        if r == 0 - 1 {
            return 0 - 1;
        }
        sh = __rnx_net_listener_accept_wait(0 - r);
        if sh < 0 {
            return 0 - 2;
        }
    }
    let req = readReq(sh);
    let eol = "\u{D}\u{A}";
    let body = "dns-ok";
    let res = "HTTP/1.1 200 OK" + eol + "Content-Length: 6" + eol + "Connection: close" + eol + eol + body;
    let w = writeAll(sh, s2b(res));
    __rnx_net_close(sh);
    if w < 0 {
        return 0 - 3;
    }
    return req.length();
}

let listener = TcpListener.bind("127.0.0.1", 0);
let port = listener.port();
let base = "http://localhost:" + __rnx_int_to_str(port);
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
