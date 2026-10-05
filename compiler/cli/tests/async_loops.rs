use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-asyloop-{tag}-{}", std::process::id()));
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

const WHILE_ACC: &str = r#"async fn add(x: Int): Promise<Int> {
    return x * 2;
}

async fn total(n: Int): Promise<Int> {
    let sum = 0;
    let i = 0;
    while i < n {
        sum = sum + await add(i);
        i = i + 1;
    }
    return sum;
}

print(await total(10));
"#;

#[test]
fn while_loop_async_accumulator() {
    expect_backends("acc", WHILE_ACC, "90\n");
}

const FOR_STREAM: &str = r#"async fn dbl(x: Int): Promise<Int> {
    return x * 2;
}

async fn fsum(xs: Array<Int>): Promise<Int> {
    let sum = 0;
    for x in xs {
        sum = sum + await dbl(x);
    }
    return sum;
}

async fn produce(cell: Array<Int>): Promise<Array<Int>> {
    if cell[0] >= 4 {
        return [];
    }
    cell[0] = cell[0] + 1;
    return [cell[0] * 10];
}

async fn consume(): Promise<Int> {
    let cell: Array<Int> = [0];
    let sum = 0;
    while true {
        let chunk: Array<Int> = await produce(cell);
        if chunk.length() == 0 {
            break;
        }
        sum = sum + chunk[0];
    }
    return sum;
}

print(await fsum([1, 2, 3, 4]));
print(await consume());
"#;

#[test]
fn for_loop_and_stream_consume_until_eof() {
    expect_backends("stream", FOR_STREAM, "20\n100\n");
}

const STILL_REJECTED: &str = r#"async fn bad(n: Int): Promise<Int> {
    let i = 0;
    while await check(i) {
        i = i + 1;
    }
    return i;
}

print(await bad(3));
"#;

#[test]
fn await_in_loop_condition_still_rejected() {
    let dir = fresh_dir("cond");
    let out = run_backend(&dir, STILL_REJECTED, None);
    assert!(!out.status.success(), "condition await must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stdout}{stderr}");
    assert!(combined.contains("await is only supported"), "{combined}");
    let _ = std::fs::remove_dir_all(&dir);
}

const AWAIT_RESULT_INLINE: &str = r#"async fn fetch(): Result<String, String> {
    return Result.Ok("live");
}

async fn load(): String {
    return (await fetch()).unwrapOr("d");
}

async fn loadVar(): String {
    let r = await fetch();
    return r.unwrapOr("d");
}

print(await load());
print(await loadVar());
"#;

#[test]
fn await_promise_result_inline_unwrap_or() {
    // Inline `(await p).unwrapOr(..)` without a rebind keeps an Any
    // receiver through lowering; native backends still need an annotated
    // rebind, so this locks the interpreter path from the worker-err
    // fallback regression.
    let dir = fresh_dir("await-result");
    let out = run_backend(&dir, AWAIT_RESULT_INLINE, Some("interpreter"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "await-result interp failed: {stderr}");
    assert_eq!(stdout, "live\nlive\n", "await-result stdout mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}
