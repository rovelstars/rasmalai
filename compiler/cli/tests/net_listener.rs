use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-listen-{tag}-{}", std::process::id()));
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

const NET_IMPORT: &str = "import { TcpStream, TcpListener } from \"@std/net\";\n\n";

const RAW_HELPERS: &str = r#"async fn drain(s: TcpStream, want: Int): Promise<Array<Int>> {
    let acc: Array<Int> = [];
    while acc.length() < want {
        let chunk: Array<Int> = await s.read(want);
        if chunk.length() == 0 {
            return acc;
        }
        let k = 0;
        while k < chunk.length() {
            acc.push(chunk[k]);
            k = k + 1;
        }
    }
    return acc;
}

fn s2b(s: String): Array<Int> {
    let out: Array<Int> = [];
    let i = 0;
    while i < s.length() {
        out.push(s.charCodeAt(i));
        i = i + 1;
    }
    return out;
}

fn acceptRaw(lh: Int): Int {
    let r = __rnx_net_listener_accept_start(lh);
    if r == 0 - 1 {
        return 0 - 1;
    }
    if r < 0 {
        return __rnx_net_listener_accept_wait(0 - r);
    }
    return r;
}

fn readSome(sh: Int, max: Int): Array<Int> {
    let out: Array<Int> = [];
    let n = __rnx_net_recv_or_wait(sh, max);
    if n <= 0 {
        return out;
    }
    let i = 0;
    while i < n {
        out.push(__rnx_net_recv_get(sh, i));
        i = i + 1;
    }
    return out;
}

fn readExact(sh: Int, want: Int): Array<Int> {
    let out: Array<Int> = [];
    while out.length() < want {
        let n = __rnx_net_recv_or_wait(sh, want - out.length());
        if n <= 0 {
            return out;
        }
        let i = 0;
        while i < n {
            out.push(__rnx_net_recv_get(sh, i));
            i = i + 1;
        }
    }
    return out;
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
"#;

const ECHO_ONCE: &str = r#"fn echoOne(lh: Int): Int {
    let sh = acceptRaw(lh);
    if sh < 0 {
        return 1;
    }
    let got = readExact(sh, 4);
    let w = writeAll(sh, got);
    __rnx_net_close(sh);
    if w != got.length() {
        return 2;
    }
    return 0;
}

let listener = TcpListener.bind("127.0.0.1", 0);
let port = listener.port();
print(port > 0);
let srv = Thread.spawn((): Int => echoOne(listener.handle));
let c: TcpStream = await TcpStream.connect("127.0.0.1", port);
let ping: Array<Int> = [112, 105, 110, 103];
print(await c.write(ping));
let back: Array<Int> = await drain(c, 4);
print(back.length());
print(back[0]);
print(back[1]);
print(back[2]);
print(back[3]);
c.close();
let done = srv.join().unwrap();
print(done);
listener.close();
"#;

#[test]
fn listener_accept_and_echo_round_trip() {
    let src = NET_IMPORT.to_string() + RAW_HELPERS + ECHO_ONCE;
    expect_backends("echo", &src, "true\n4\n4\n112\n105\n110\n103\n0\n");
}

const ECHO_TEN: &str = r#"fn echoTen(lh: Int): Int {
    let i = 0;
    while i < 10 {
        let sh = acceptRaw(lh);
        if sh < 0 {
            return 100 + i;
        }
        let got = readExact(sh, 3);
        let w = writeAll(sh, got);
        __rnx_net_close(sh);
        if w != got.length() {
            return 200 + i;
        }
        i = i + 1;
    }
    return 0;
}

let listener = TcpListener.bind("127.0.0.1", 0);
let port = listener.port();
let srv = Thread.spawn((): Int => echoTen(listener.handle));
async fn dialAll(port: Int, i: Int, ok: Int): Promise<Int> {
    if i >= 10 {
        return ok;
    }
    let c: TcpStream = await TcpStream.connect("127.0.0.1", port);
    let ping: Array<Int> = [i, i + 1, i + 2];
    let w: Int = await c.write(ping);
    let back: Array<Int> = await drain(c, 3);
    c.close();
    let hit = 0;
    if w == 3 && back.length() == 3 && back[0] == i {
        hit = 1;
    }
    return await dialAll(port, i + 1, ok + hit);
}

let ok: Int = await dialAll(port, 0, 0);
let done = srv.join().unwrap();
print(ok);
print(done);
listener.close();
"#;

#[test]
fn listener_accepts_ten_sequential_clients() {
    let src = NET_IMPORT.to_string() + RAW_HELPERS + ECHO_TEN;
    expect_backends("ten", &src, "10\n0\n");
}
