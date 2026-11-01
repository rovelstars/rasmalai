#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(dir: &std::path::Path, src: &str, backend: &str) -> String {
    let file = dir.join("main.rnx");
    std::fs::write(&file, src).unwrap();
    let out = Command::new(rnx())
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(&file)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "{backend} failed: {stderr}{stdout}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("rnx panic"),
        "{backend} panicked: {stderr}"
    );
    stdout.trim().to_string()
}

fn backends() -> [&'static str; 3] {
    ["interpreter", "cranelift", "llvm"]
}

const STORED_FROM_CALLEE: &str = r#"class Counter {
    let n: Int = 0
    fn inc() { this.n += 1 }
}

class Tree {
    let handlers: Array<fn(): Void> = []
    let index: Array<Int> = []
    fn add(f: fn(): Void): Int {
        this.index.push(this.handlers.length())
        this.handlers.push(f)
        return this.index.length() - 1
    }
    fn fire(id: Int) { this.handlers[this.index[id]]() }
}

fn view(t: Tree, c: Counter): Int {
    return t.add(() => { c.inc() })
}

fn Main(): Int {
    let t = new Tree()
    let c = new Counter()
    let id = view(t, c)
    t.fire(id)
    print(c.n)
    return 0
}
"#;

const TWO_CLOSURES: &str = r#"class Counter {
    let n: Int = 0
    fn add(k: Int) { this.n += k }
}

class Box {
    let fs: Array<fn(): Void> = []
    fn push(f: fn(): Void) { this.fs.push(f) }
    fn fire(i: Int) { this.fs[i]() }
}

fn build(b: Box, a: Counter, c: Counter) {
    b.push(() => { a.add(1) })
    b.push(() => { c.add(10) })
}

fn Main(): Int {
    let b = new Box()
    let a = new Counter()
    let c = new Counter()
    build(b, a, c)
    b.fire(0)
    b.fire(1)
    b.fire(0)
    print(a.n, c.n)
    return 0
}
"#;

#[test]
fn stored_closure_keeps_its_capture_alive() {
    let dir = crate::scratch::dir("closure-stored");
    for backend in backends() {
        assert_eq!(run(&dir, STORED_FROM_CALLEE, backend), "1", "{backend}");
    }
}

#[test]
fn two_stored_closures_keep_separate_captures() {
    let dir = crate::scratch::dir("closure-two");
    for backend in backends() {
        assert_eq!(run(&dir, TWO_CLOSURES, backend), "2 10", "{backend}");
    }
}
