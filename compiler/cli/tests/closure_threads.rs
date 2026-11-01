#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

const SPAWN_ROUNDS: &str = r#"import { AtomicInt, Thread } from "@std/sync"

fn workerN(n: Int): Int {
    return AtomicInt.byId(910).fetchAdd(1)
}

fn round(n: Int): Int {
    AtomicInt.byId(910).set(0)
    let hs: Array<Thread> = []
    for _ in 0..n { hs.push(Thread.spawn((): Int => workerN(n))) }
    for t in hs { t.join() }
    return AtomicInt.byId(910).get()
}

fn Main(): Int {
    let i = 0
    while i < 12 {
        if round(16) != 16 { return 1 }
        i++
    }
    print("done")
    return 0
}
"#;

fn run(dir: &std::path::Path, backend: &str) {
    let out = Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(dir.join("main.rnx"))
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "{backend} failed with {:?}: {stderr}{stdout}",
        out.status.code()
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("rnx panic"),
        "{backend} panicked: {stderr}"
    );
    assert_eq!(stdout.trim(), "done", "{backend}");
}

#[test]
fn spawn_with_closures_repeats_without_crashing() {
    let dir = crate::scratch::dir("closure-threads");
    std::fs::write(dir.join("main.rnx"), SPAWN_ROUNDS).unwrap();
    for backend in ["interpreter", "cranelift", "llvm"] {
        for _ in 0..3 {
            run(&dir, backend);
        }
    }
}
