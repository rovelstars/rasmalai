use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-fetch-{tag}-{}", std::process::id()));
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
    for backend in [Some("interpreter"), Some("cranelift"), Some("llvm")] {
        let dir = fresh_dir(&format!("{tag}-{}", backend.unwrap_or("interp")));
        let out = run_backend(&dir, src, backend);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{tag} {:?} failed: {stderr}", backend);
        assert_eq!(stdout, want, "{tag} {:?} stdout mismatch", backend);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

const MOCK_AND_FETCH: &str = r#"import { TcpStream, TcpListener } from "@std/net";
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
    let status = "200 OK";
    let body = "Hello, World!";
    let framing = "Content-Length: 13";
    if req.startsWith("GET /missing ") {
        status = "404 Not Found";
        body = "missing";
        framing = "Content-Length: 7";
    }
    if req.startsWith("GET /chunked ") {
        status = "200 OK";
        body = "5" + eol + "Hello" + eol + "6" + eol + " World" + eol + "0" + eol + eol;
        framing = "Transfer-Encoding: chunked";
    }
    let res = "HTTP/1.1 " + status + eol + framing + eol + "Content-Type: text/plain" + eol + "X-Echo: yes" + eol + "Connection: close" + eol + eol + body;
    let w = writeAll(sh, s2b(res));
    __rnx_net_close(sh);
    if w < 0 {
        return 0 - 3;
    }
    return 0;
}

fn serveThree(lh: Int): Int {
    let a = serveOne(lh);
    if a != 0 {
        return a;
    }
    let b = serveOne(lh);
    if b != 0 {
        return b;
    }
    return serveOne(lh);
}

let listener = TcpListener.bind("127.0.0.1", 0);
let port = listener.port();
let base = "http://127.0.0.1:" + __rnx_int_to_str(port);
let srv = Thread.spawn((): Int => serveThree(listener.handle));
let r1: Response = await fetch(base + "/hello");
print(r1.status);
print(r1.text());
print((r1.headers.get("content-type") ?? ""));
print((r1.headers.get("content-length") ?? ""));
print((r1.headers.get("x-echo") ?? ""));
let r2: Response = await fetch(base + "/missing");
print(r2.status);
print(r2.text());
let r3: Response = await fetch(base + "/chunked");
print(r3.status);
print(r3.text());
let done = srv.join().unwrap();
print(done);
listener.close();
"#;

#[test]
fn fetch_parses_status_headers_and_bodies() {
    expect_backends(
        "fetch",
        MOCK_AND_FETCH,
        "200\nHello, World!\ntext/plain\n13\nyes\n404\nmissing\n200\nHello World\n0\n",
    );
}

const MOCK_AND_JSON: &str = r#"import { TcpStream, TcpListener } from "@std/net";
import { fetch, Response } from "@std/web";
import { JSON } from "@std/json";
import { Map } from "@std/collections";

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

fn showCount(m: Map<String, Any>): Int {
    let cv = m.get("count");
    if cv != null {
        print(cv);
        return 0;
    }
    print("none-count");
    return 1;
}

fn serveJson(lh: Int): Int {
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
    let status = "200 OK";
    let body = "\{\"status\": \"ok\", \"count\": 42}";
    let framing = "Content-Length: " + __rnx_int_to_str(body.length());
    if req.startsWith("GET /api ") {
        status = "200 OK";
    } else {
        status = "404 Not Found";
        body = "missing";
        framing = "Content-Length: 7";
    }
    let res = "HTTP/1.1 " + status + eol + framing + eol + "Content-Type: application/json" + eol + "Connection: close" + eol + eol + body;
    let w = writeAll(sh, s2b(res));
    __rnx_net_close(sh);
    if w < 0 {
        return 0 - 3;
    }
    return 0;
}

let listener = TcpListener.bind("127.0.0.1", 0);
let port = listener.port();
let base = "http://127.0.0.1:" + __rnx_int_to_str(port);
let srv = Thread.spawn((): Int => serveJson(listener.handle));
let r: Response = await fetch(base + "/api");
print(r.status);
let m = JSON.asMap(r.json());
let sv = m.get("status");
if sv != null {
    print(sv);
    return showCount(m);
}
print("none-status");
return 1;
"#;

#[test]
fn fetch_response_json_decodes_body() {
    expect_backends("fetchjson", MOCK_AND_JSON, "200\nok\n42\n");
}

#[test]
fn fetch_https_passes_scheme_gate_to_networking() {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let src = format!(
        "import {{ fetch, Response }} from \"@std/web\";\n\nlet r: Response = await fetch(\"https://127.0.0.1:{port}/\");\nprint(r.status);\n"
    );
    let dir = fresh_dir("https");
    let out = run_backend(&dir, &src, None);
    assert!(!out.status.success(), "https to closed port must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stdout}{stderr}");
    assert!(!combined.contains("HTTPS requires TLS"), "{combined}");
    assert!(combined.contains("connect failed"), "{combined}");
    let _ = std::fs::remove_dir_all(&dir);
}
