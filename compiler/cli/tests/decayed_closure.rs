#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(dir: &std::path::Path, src: &str, backend: &str) -> String {
    let file = dir.join(format!("{backend}.rnx"));
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
    assert!(
        !stderr.contains("rnx_release on zero count")
            && !stdout.contains("rnx_release on zero count"),
        "{backend} over-released: {stderr}{stdout}"
    );
    stdout.trim().to_string()
}

fn backends() -> [&'static str; 3] {
    ["interpreter", "cranelift", "llvm"]
}

const STRONG_THIS: &str = r#"class Counter {
    let n: Int = 0
    let onTick: fn(): Void = () => {}
    fn inc() { this.n += 1 }
    fn install() { this.onTick = () => { this.inc() } }
    fn fire() { this.onTick() }
}

fn Main(): Int {
    let c = new Counter()
    c.install()
    c.fire()
    c.fire()
    print(c.n)
    return 0
}
"#;

const STRONG_TWO_INSTANCES: &str = r#"class Counter {
    let n: Int = 0
    let onTick: fn(): Void = () => {}
    fn inc() { this.n += 1 }
    fn install() { this.onTick = () => { this.inc() } }
    fn fire() { this.onTick() }
}

fn Main(): Int {
    let a = new Counter()
    let b = new Counter()
    a.install()
    b.install()
    a.fire()
    a.fire()
    b.fire()
    print(a.n, b.n)
    return 0
}
"#;

const WEAK_ALIVE: &str = r#"class Counter {
    let n: Int = 0
    fn inc() { this.n += 1 }
    fn make(): fn(): Void {
        return fn decay(this) { this.inc() }
    }
}

fn Main(): Int {
    let c = new Counter()
    let f = c.make()
    f()
    f()
    print(c.n)
    return 0
}
"#;

const WEAK_AFTER_DROP: &str = r#"class Counter {
    let n: Int = 0
    fn inc() { this.n += 1 }
    deinit { print("destroyed") }
    fn make(): fn(): Void {
        return fn decay(this) { print("ran"); this.inc() }
    }
}

fn take(): fn(): Void {
    let c = new Counter()
    return c.make()
}

fn Main(): Int {
    let f = take()
    print("after take")
    f()
    print("after call")
    return 0
}
"#;

const FOUR_HANDLERS: &str = r#"class Appearance {
    let scheme: String = "system"
    let resets: Int = 0
}

class Tree {
    let handlers: Array<fn(): Void> = []
    fn button(f: fn(): Void) { this.handlers.push(f) }
    fn fire(i: Int) { this.handlers[i]() }
}

class Build {
    let tree: Tree
    init(tree: Tree) { this.tree = tree }
    fn button(f: fn(): Void) { this.tree.button(f) }
}

fn screen(b: Build, s: Appearance) {
    b.button(() => { print("tag-system"); s.scheme = "system" })
    b.button(() => { print("tag-light"); s.scheme = "light" })
    b.button(() => { print("tag-dark"); s.scheme = "dark" })
    b.button(() => { print("tag-reset"); s.scheme = "system"; s.resets += 1 })
}

fn Main(): Int {
    let t = new Tree()
    let b = new Build(t)
    let s = new Appearance()
    screen(b, s)
    t.fire(0)
    t.fire(1)
    t.fire(2)
    t.fire(3)
    print(s.scheme, s.resets)
    return 0
}
"#;

const THREE_MUTATIONS: &str = r#"class State {
    let count: Int = 0
    let name: String = "init"
    fn rename(s: String) { this.name = s }
}

class Box {
    let fs: Array<fn(): Void> = []
    fn push(f: fn(): Void) { this.fs.push(f) }
    fn fire(i: Int) { this.fs[i]() }
}

fn build(b: Box, s: State) {
    b.push(() => { s.count += 1 })
    b.push(() => { s.name = "changed" })
    b.push(() => { s.rename("setter") })
}

fn Main(): Int {
    let b = new Box()
    let s = new State()
    build(b, s)
    b.fire(0)
    b.fire(1)
    b.fire(2)
    print(s.count, s.name)
    return 0
}
"#;

#[test]
fn strong_this_survives_the_creating_method() {
    let dir = crate::scratch::dir("decay-strong");
    for backend in backends() {
        assert_eq!(run(&dir, STRONG_THIS, backend), "2", "{backend}");
    }
}

#[test]
fn strong_this_keeps_instances_separate() {
    let dir = crate::scratch::dir("decay-strong-two");
    for backend in backends() {
        assert_eq!(run(&dir, STRONG_TWO_INSTANCES, backend), "2 1", "{backend}");
    }
}

#[test]
fn decayed_this_runs_while_alive() {
    let dir = crate::scratch::dir("decay-alive");
    for backend in backends() {
        assert_eq!(run(&dir, WEAK_ALIVE, backend), "2", "{backend}");
    }
}

#[test]
fn decayed_this_skips_after_drop() {
    let dir = crate::scratch::dir("decay-dead");
    for backend in backends() {
        let out = run(&dir, WEAK_AFTER_DROP, backend);
        assert!(out.contains("destroyed"), "{backend}: {out}");
        assert!(!out.contains("ran"), "{backend}: {out}");
        assert!(out.contains("after call"), "{backend}: {out}");
    }
}

#[test]
fn four_handlers_all_fire_in_order() {
    let dir = crate::scratch::dir("decay-four");
    for backend in backends() {
        let out = run(&dir, FOUR_HANDLERS, backend);
        let tags: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("tag-"))
            .collect();
        assert_eq!(
            tags,
            ["tag-system", "tag-light", "tag-dark", "tag-reset"],
            "{backend}: {out}"
        );
        assert!(out.ends_with("system 1"), "{backend}: {out}");
    }
}

#[test]
fn stored_closures_mutate_an_object_without_over_release() {
    let dir = crate::scratch::dir("decay-mut");
    for backend in backends() {
        assert_eq!(run(&dir, THREE_MUTATIONS, backend), "1 setter", "{backend}");
    }
}
