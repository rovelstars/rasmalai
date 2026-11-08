#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

// Regression pin for heap temporaries that are stored into fields or ignored.
//
// Before the fix, `this.field = new Inner()` retained the value twice: once
// for the field and once for the never-released call temporary. The same
// happened for ignored `new`/call results and for class field defaults. The
// process kept every constructed object alive.
//
// These tests run each backend in a subprocess and count deinit output.

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(src: &str, backend: &str) -> String {
    let dir = scratch::dir(&format!("arc-cleanup-{backend}"));
    let file = dir.join("main.rnx");
    std::fs::write(&file, src).unwrap();
    let mut cmd = Command::new(rnx());
    cmd.arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(&file)
        .env("NO_COLOR", "1");
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{backend} failed: {stderr}{stdout}");
    assert!(
        !stderr.contains("rnx panic") && !stderr.contains("rnx_release"),
        "{backend} panicked: {stderr}"
    );
    stdout
}

const CLEANUP: &str = r#"class Inner {
    let n: Int = 1
    deinit { print("inner") }
}

class Outer {
    let a: Inner
    let b: Inner
    init() {
        this.a = new Inner()
        this.b = makeInner()
    }
    deinit { print("outer") }
}

fn makeInner(): Inner {
    return new Inner()
}

fn makeOuter(): Outer {
    return new Outer()
}

fn Main(): Int {
    let i = 0
    while i < 3 {
        let o = new Outer()
        i += 1
    }
    print("pass1")
    i = 0
    while i < 2 {
        makeOuter()
        i += 1
    }
    print("pass2")
    return 0
}
"#;

#[test]
fn stored_and_ignored_heap_values_are_released() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run(CLEANUP, backend);
        let inner = out.matches("inner").count();
        let outer = out.matches("outer").count();
        assert_eq!(inner, 10, "{backend} inner deinit count\n{out}");
        assert_eq!(outer, 5, "{backend} outer deinit count\n{out}");
        assert!(out.contains("pass1") && out.contains("pass2"), "{backend}: {out}");
    }
}

const RECEIVER_TEMP: &str = r#"class Leaf {
    let n: Int = 0
    fn value(): Int { return this.n }
    deinit { print("leaf-drop") }
}

class Holder {
    let leaf: Leaf
    init() {
        let l = new Leaf()
        this.leaf = l
    }
    deinit { print("holder-drop") }
}

fn Main(): Int {
    let i = 0
    while i < 3 {
        let h = new Holder()
        if h.leaf.value() == 0 {
            let x = 1
        }
        let named = h.leaf
        if named.value() == 0 {
            let y = 1
        }
        i += 1
    }
    print("done")
    return 0
}
"#;

#[test]
fn inlined_receiver_temp_is_released() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run(RECEIVER_TEMP, backend);
        let leaf = out.matches("leaf-drop").count();
        let holder = out.matches("holder-drop").count();
        assert_eq!(leaf, 3, "{backend} leaf deinit count\n{out}");
        assert_eq!(holder, 3, "{backend} holder deinit count\n{out}");
        assert!(out.contains("done"), "{backend}: {out}");
    }
}

// A constructor temporary passed straight into another constructor is used
// more than once (its own field defaults plus the outer init argument), so
// the single-use sweeps never released it. Each loop trip leaked the inner
// object while the outer one was freed.
const NESTED_NEW: &str = r#"class D {
    let x: Int = 1
    deinit { print("d-drop") }
}

class F {
    let d: D
    init(d: D) { this.d = d }
    deinit { print("f-drop") }
}

fn Main(): Int {
    let i = 0
    while i < 50 {
        new F(new D())
        i += 1
    }
    print("done")
    return 0
}
"#;

#[test]
fn nested_constructor_temps_are_released() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run(NESTED_NEW, backend);
        assert_eq!(out.matches("d-drop").count(), 50, "{backend}\n{out}");
        assert_eq!(out.matches("f-drop").count(), 50, "{backend}\n{out}");
        assert!(out.contains("done"), "{backend}: {out}");
    }
}

// Returning an owned object field hands a retained reference to the caller.
// The call result is a fresh temporary; when it dies in a loop it must be
// released or every trip leaks the object and everything it owns.
const FIELD_RETURN: &str = r#"class Damage {
    let xs: Array<Int> = []
    let ys: Array<Int> = []
    fn add(x: Int, y: Int) {
        this.xs.push(x)
        this.ys.push(y)
    }
    fn count(): Int { return this.xs.length() }
    deinit { print("damage-drop") }
}

class F {
    let d: Damage
    init(d: Damage) { this.d = d }
    fn damage(): Damage { return this.d }
    deinit { print("f-drop") }
}

fn Main(): Int {
    let i = 0
    while i < 50 {
        let f = new F(new Damage())
        let d = f.damage()
        d.add(i, i)
        if d.count() < 0 {
            print("unreachable")
        }
        i += 1
    }
    print("done")
    return 0
}
"#;

#[test]
fn returned_field_temp_is_released() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run(FIELD_RETURN, backend);
        assert_eq!(out.matches("damage-drop").count(), 50, "{backend}\n{out}");
        assert_eq!(out.matches("f-drop").count(), 50, "{backend}\n{out}");
        assert!(out.contains("done"), "{backend}: {out}");
    }
}

// A damage packet built and consumed inside loop branches: the unbound
// construction temporary is used several times (field defaults plus the
// init argument) away from the loop latch, so only a cross-block death
// analysis releases it exactly once per path instead of once per frame.
const DAMAGE_LOOP: &str = r#"class Damage {
    let xs: Array<Int> = []
    let ys: Array<Int> = []
    let ws: Array<Int> = []
    let hs: Array<Int> = []
    fn add(x: Int, y: Int, w: Int, h: Int) {
        this.xs.push(x)
        this.ys.push(y)
        this.ws.push(w)
        this.hs.push(h)
    }
    fn count(): Int { return this.xs.length() }
    deinit { print("damage-drop") }
}

class Holder {
    let d: Damage
    init(d: Damage) { this.d = d }
    deinit { print("holder-drop") }
}

fn Main(): Int {
    let i = 0
    while i < 50 {
        if i % 2 == 0 {
            let h = new Holder(new Damage())
            h.d.add(i, i, 1, 1)
        } else {
            let h = new Holder(new Damage())
            if h.d.count() < 0 {
                print("bad count")
            }
        }
        i += 1
    }
    print("done")
    return 0
}
"#;

#[test]
fn multireturn_damage_loop_is_released() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run(DAMAGE_LOOP, backend);
        assert_eq!(out.matches("damage-drop").count(), 50, "{backend}\n{out}");
        assert_eq!(out.matches("holder-drop").count(), 50, "{backend}\n{out}");
        assert!(out.contains("done"), "{backend}: {out}");
        assert!(!out.contains("bad count"), "{backend}: {out}");
    }
}

// A buffer whose address is derived and used through a pointer must stay
// alive until the pointer dies, even when the buffer itself is never read
// again. Releasing it right after `address()` frees memory that is still
// reachable through the derived pointer.
const PINNED_BUFFER: &str = r#"import { ByteBuffer } from "@std/bytes";
fn Main(): Int {
    let i = 0
    let total = 0
    while i < 100 {
        let b = ByteBuffer.allocate(8)
        let k = b.address()
        unsafe {
            let p = Pointer.fromAddress<Byte>(k)
            p.write(i)
            total += p.read()
        }
        i += 1
    }
    print(total)
    return 0
}
"#;

#[test]
fn buffer_with_derived_pointer_is_not_freed_early() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run(PINNED_BUFFER, backend);
        assert!(out.contains("4950"), "{backend}: {out}");
    }
}

// Field arrays read through GetField inside loops must keep exactly one
// owner: the field itself. Releasing the loaded temporary twice corrupts
// the array; never releasing it leaks one reference per trip.
const FIELD_ARRAYS: &str = r#"class Shortcuts {
    let keys: Array<String> = []
    let vals: Array<Int> = []
    fn bind(k: String, v: Int): Bool {
        let i = 0
        while i < this.keys.length() {
            if this.keys[i] == k {
                this.vals[i] = v
                return true
            }
            i += 1
        }
        this.keys.push(k)
        this.vals.push(v)
        return false
    }
    fn get(k: String): Int {
        let i = 0
        while i < this.keys.length() {
            if this.keys[i] == k {
                return this.vals[i]
            }
            i += 1
        }
        return -1
    }
}

fn Main(): Int {
    let s = new Shortcuts()
    let i = 0
    while i < 50 {
        s.bind("k", i)
        i += 1
    }
    print(s.get("k"))
    print(s.get("missing"))
    return 0
}
"#;

#[test]
fn field_arrays_are_neither_leaked_nor_double_freed() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run(FIELD_ARRAYS, backend);
        assert!(out.contains("49"), "{backend}: {out}");
        assert!(out.contains("-1"), "{backend}: {out}");
    }
}

// A borrowed method receiver forwarded through a wrapper into inlined getters
// inside a loop. The inliner skips the move-back for the borrowed receiver
// and releases the entry slot instead; the loop pass must keep that entry
// copy inside the loop. Hoisting it to the preheader used to release one
// retain N times (rnx_free size mismatch), while the old move-back stranded
// one reference per call (missing deinit below).
const LOOP_FORWARDED_RECEIVER: &str = r#"class Damage {
    let xs: Array<Int> = []
    let ys: Array<Int> = []
    fn add(x: Int, y: Int) {
        this.xs.push(x)
        this.ys.push(y)
    }
    fn count(): Int { return this.xs.length() }
    fn xAt(i: Int): Int { return this.xs[i] }
    fn yAt(i: Int): Int { return this.ys[i] }
    deinit { print("damage-drop") }
}

fn hits(dm: Damage): Bool {
    let i = 0
    while i < dm.count() {
        if dm.xAt(i) < 0 {
            return true
        }
        if dm.yAt(i) < 0 {
            return true
        }
        i += 1
    }
    return false
}

fn outer(dm: Damage): Bool {
    let i = 0
    while i < 3 {
        if hits(dm) {
            return true
        }
        i += 1
    }
    return false
}

fn Main(): Int {
    let i = 0
    while i < 50 {
        let d = new Damage()
        d.add(i, i)
        if outer(d) {
            print("bad")
        }
        i += 1
    }
    print("done")
    return 0
}
"#;

#[test]
fn loop_forwarded_receiver_is_released_once_per_trip() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run(LOOP_FORWARDED_RECEIVER, backend);
        assert_eq!(out.matches("damage-drop").count(), 50, "{backend}\n{out}");
        assert!(out.contains("done"), "{backend}: {out}");
        assert!(!out.contains("bad"), "{backend}: {out}");
    }
}

const DISCARDED_BORROWED_RETURN: &str = r#"class C {
    let x: Int = 1
    fn bump(): C {
        print("touch")
        return this
    }
    deinit { print("drop") }
}

fn Main(): Int {
    let c = new C()
    c.bump()
    print("alive", c.x)
    return 0
}
"#;

#[test]
fn discarded_method_return_keeps_receiver_alive() {
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run(DISCARDED_BORROWED_RETURN, backend);
        assert!(out.contains("alive 1"), "{backend}: {out}");
        assert_eq!(out.matches("drop").count(), 1, "{backend}: {out}");
    }
}
