#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::{Path, PathBuf};
use std::process::Command;

const FOO_C: &str = "long foo_add(long a, long b) { return a + b; }\n";

const CONSUMER_MAIN: &str = "import { fn foo_add(a: Int, b: Int): Int } from native \"foo\"\nfn Main(): Int {\n    unsafe {\n        let r = foo_add(20, 22);\n        print(r);\n        assert(r == 42, \"add\");\n    }\n    return 0;\n}\n";

const ZLIB_MAIN: &str = "import {\n    fn compressBound(sourceLen: Int): Int\n} from native \"z\"\nfn Main(): Int {\n    unsafe {\n        let bound = compressBound(100);\n        print(bound);\n        assert(bound > 100, \"bound grows\");\n    }\n    return 0;\n}\n";

fn rnx() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rnx"))
}

fn build_foo_archive(workdir: &Path) -> Vec<u8> {
    std::fs::create_dir_all(workdir).unwrap();
    let src = workdir.join("foo.c");
    std::fs::write(&src, FOO_C).unwrap();
    let obj = workdir.join("foo.o");
    let archive = workdir.join("libfoo.a");
    let compile = Command::new("cc")
        .arg("-O2")
        .arg("-c")
        .arg(&src)
        .arg("-o")
        .arg(&obj)
        .output()
        .expect("cc must be installed for native link tests");
    assert!(
        compile.status.success(),
        "cc -c failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let ar = Command::new("ar")
        .arg("cqD")
        .arg(&archive)
        .arg(&obj)
        .output()
        .expect("ar must be installed for native link tests");
    assert!(
        ar.status.success(),
        "ar failed: {}",
        String::from_utf8_lossy(&ar.stderr)
    );
    let ranlib = Command::new("ranlib")
        .arg("-D")
        .arg(&archive)
        .output()
        .expect("ranlib must be installed for native link tests");
    assert!(
        ranlib.status.success(),
        "ranlib failed: {}",
        String::from_utf8_lossy(&ranlib.stderr)
    );
    std::fs::read(&archive).unwrap()
}

struct Fixture {
    _guard: crate::scratch::Scratch,
    main: PathBuf,
    consumer: PathBuf,
}

fn fixture(tag: &str, plant_blob: bool) -> Fixture {
    let guard = crate::scratch::dir(&format!("native-prefer-{tag}"));
    let author = guard.join("author");
    std::fs::create_dir_all(author.join("src")).unwrap();
    std::fs::write(
        author.join("Project.config"),
        "export default {\n    project: {\n        name: \"author-pkg\",\n        version: \"0.1.0\"\n    },\n    artifacts: \"example.com/org/author\",\n    dependencies: {\n        foo: { native: \"foo\", from: \"libfoo.a\", link: \"static\" }\n    },\n    permissions: [\"unsafe:ffi\", \"unsafe:raw_memory\"]\n}\n",
    )
    .unwrap();
    std::fs::write(author.join("src").join("lib.rnx"), "fn f(): Int { return 1; }\n").unwrap();
    if plant_blob {
        let bytes = build_foo_archive(&guard.join("build"));
        let staged = author
            .join(".rnx-cache")
            .join("artifacts")
            .join("runtime")
            .join("libfoo.a");
        std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
        std::fs::write(&staged, &bytes).unwrap();
    }
    let consumer = guard.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Project.config"),
        "export default {\n    project: {\n        name: \"consumer-pkg\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        \"author-pkg\": { path: \"../author\" }\n    }\n}\n",
    )
    .unwrap();
    let main = consumer.join("src").join("main.rnx");
    std::fs::write(&main, CONSUMER_MAIN).unwrap();
    Fixture { _guard: guard, main, consumer }
}

fn build(main: &Path, extra: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(rnx());
    cmd.arg("build").arg(main);
    for a in extra {
        cmd.arg(a);
    }
    cmd.output().unwrap()
}

fn artifact_of(out: &std::process::Output) -> String {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "build failed:\n{text}");
    text.lines()
        .rev()
        .find_map(|l| l.strip_prefix("artifact: "))
        .map(|s| s.trim().trim_end_matches(" (fresh)").to_string())
        .unwrap_or_else(|| panic!("build prints artifact path:\n{text}"))
}

fn combined(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn run_bin(bin: &str) -> String {
    let run = Command::new(bin).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    String::from_utf8(run.stdout).unwrap()
}

#[test]
fn bundled_blob_links_statically_by_default() {
    let fx = fixture("default", true);
    let out = build(&fx.main, &[]);
    let bin = artifact_of(&out);
    assert_eq!(run_bin(&bin), "42\n");
    #[cfg(unix)]
    {
        let nm = Command::new("nm").arg(&bin).output().unwrap();
        assert!(nm.status.success());
        let symbols = String::from_utf8_lossy(&nm.stdout).into_owned();
        assert!(
            symbols.lines().any(|l| l.contains(" T foo_add") || l.ends_with(" T foo_add")),
            "foo_add statically linked:\n{symbols}"
        );
        let ldd = Command::new("ldd").arg(&bin).output().unwrap();
        let linked = String::from_utf8_lossy(&ldd.stdout).into_owned();
        assert!(!linked.contains("libfoo"), "no host libfoo:\n{linked}");
    }
}

#[test]
fn prefer_bundled_links_statically() {
    let fx = fixture("flag-bundled", true);
    let out = build(&fx.main, &["--prefer-bundled"]);
    let bin = artifact_of(&out);
    assert_eq!(run_bin(&bin), "42\n");
    #[cfg(unix)]
    {
        let nm = Command::new("nm").arg(&bin).output().unwrap();
        assert!(nm.status.success());
        let symbols = String::from_utf8_lossy(&nm.stdout).into_owned();
        assert!(
            symbols.lines().any(|l| l.contains(" T foo_add") || l.ends_with(" T foo_add")),
            "foo_add statically linked:\n{symbols}"
        );
    }
}

#[test]
fn missing_bundled_blob_fails_closed() {
    let fx = fixture("missing-blob", false);
    let out = build(&fx.main, &[]);
    assert!(!out.status.success(), "missing blob must fail the build");
    let text = combined(&out);
    assert!(text.contains("libfoo.a"), "names the blob:\n{text}");
    assert!(text.contains("foo"), "names the library:\n{text}");
    assert!(text.contains("--prefer-system"), "names the remedy:\n{text}");
}

#[test]
fn prefer_system_with_no_host_library_fails_without_fallback() {
    let fx = fixture("flag-system", true);
    let out = build(&fx.main, &["--prefer-system"]);
    assert!(!out.status.success(), "no host libfoo exists, so the link must fail");
    let text = combined(&out);
    assert!(text.contains("foo"), "names the library:\n{text}");
}

#[test]
fn locked_build_enforces_effective_choice() {
    let fx = fixture("locked", true);
    let lock = Command::new(rnx())
        .arg("lock")
        .current_dir(&fx.consumer)
        .output()
        .unwrap();
    assert!(
        lock.status.success(),
        "lock failed: {}",
        String::from_utf8_lossy(&lock.stderr)
    );
    let out = build(&fx.main, &["--locked"]);
    let bin = artifact_of(&out);
    assert_eq!(run_bin(&bin), "42\n");
    let flipped = build(&fx.main, &["--locked", "--prefer-system"]);
    assert!(!flipped.status.success(), "changed preference must not reuse the lock");
    let text = combined(&flipped);
    assert!(
        text.contains("source mismatch") || text.contains("system library `foo` not found"),
        "changed preference fails closed instead of reusing the lock:\n{text}"
    );
}

#[test]
fn prefer_system_uses_host_library() {
    let guard = crate::scratch::dir("native-prefer-system");
    let root = guard.join("app");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Project.config"),
        "export default {\n    project: {\n        name: \"sysapp\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        zlib: { native: \"z\", system: true }\n    }\n}\n",
    )
    .unwrap();
    let main = root.join("src").join("main.rnx");
    std::fs::write(&main, ZLIB_MAIN).unwrap();
    let out = build(&main, &["--prefer-system"]);
    let bin = artifact_of(&out);
    let stdout = run_bin(&bin);
    let bound: i64 = stdout.trim().parse().expect("bound");
    assert!(bound > 100, "{bound}");
    #[cfg(unix)]
    {
        let ldd = Command::new("ldd").arg(&bin).output().unwrap();
        let linked = String::from_utf8_lossy(&ldd.stdout).into_owned();
        assert!(linked.contains("libz.so"), "libz linked:\n{linked}");
    }
}

#[test]
fn prefer_bundled_without_blob_fails_closed() {
    let guard = crate::scratch::dir("native-prefer-bundled-missing");
    let root = guard.join("app");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Project.config"),
        "export default {\n    project: {\n        name: \"sysapp\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        zlib: { native: \"z\", system: true }\n    }\n}\n",
    )
    .unwrap();
    let main = root.join("src").join("main.rnx");
    std::fs::write(&main, ZLIB_MAIN).unwrap();
    let out = build(&main, &["--prefer-bundled"]);
    assert!(!out.status.success(), "no bundled zlib blob exists");
    let text = combined(&out);
    assert!(text.contains("names no blob"), "fail-closed diagnostic:\n{text}");
    assert!(text.contains("from:"), "names the remedy:\n{text}");
}

#[test]
fn prefer_flags_conflict() {
    let fx = fixture("conflict", true);
    let out = build(&fx.main, &["--prefer-system", "--prefer-bundled"]);
    assert!(!out.status.success(), "conflicting flags must be rejected");
    let text = combined(&out);
    assert!(
        text.contains("prefer-bundled") || text.contains("prefer_system"),
        "names the conflict:\n{text}"
    );
}
