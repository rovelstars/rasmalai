use std::path::PathBuf;

const BYTE_SRC: &str = "import { ByteBuffer } from \"@std/bytes\";\nfn Main(): Int {\n    let buf = ByteBuffer.allocate(16);\n    let i = 0;\n    while i < 16 { buf.writeUInt8(i, 10 + i); i = i + 1; }\n    unsafe {\n        let p = Pointer.fromAddress<Byte>(buf.address());\n        p.write(99);\n        assert(p.read() == 99, \"byte 0\");\n        assert(buf.readUInt8(0) == 99, \"buf 0\");\n        assert(buf.readUInt8(1) == 11, \"neighbour 1\");\n        assert(buf.readUInt8(7) == 17, \"neighbour 7\");\n        assert(buf.readUInt8(15) == 25, \"tail\");\n    }\n    print(\"byte-ok\");\n    return 0;\n}\n";

const LIBC_SRC: &str = "import { ByteBuffer } from \"@std/bytes\";\nimport {\n    fn strlen(s: Pointer<Byte>): Int,\n    fn puts(s: Pointer<Byte>): Int\n} from native \"c\"\nfn Main(): Int {\n    let buf = ByteBuffer.allocate(32);\n    let msg = \"Hello from Native C!\";\n    let i = 0;\n    while i < 20 { buf.writeUInt8(i, msg.charCodeAt(i)); i = i + 1; }\n    buf.writeUInt8(20, 0);\n    unsafe {\n        let p = Pointer.fromAddress<Byte>(buf.address());\n        let len = strlen(p);\n        print(len);\n        assert(len == 20, \"strlen\");\n        puts(p);\n    }\n    return 0;\n}\n";

const ZLIB_SRC: &str = "import {\n    fn compressBound(sourceLen: Int): Int\n} from native \"z\"\nfn Main(): Int {\n    let bound = compressBound(100);\n    print(bound);\n    assert(bound > 100, \"bound grows\");\n    return 0;\n}\n";

fn write_proj(src: &str, tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-ffi-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"t\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    let main = dir.join("src").join("main.rnx");
    std::fs::write(&main, src).unwrap();
    (dir, main)
}

fn rnx() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rnx"))
}

fn run_backend(main: &std::path::Path, backend: &str) -> std::process::Output {
    std::process::Command::new(rnx())
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(main)
        .output()
        .unwrap()
}

fn run_ok(main: &std::path::Path, backend: &str) -> String {
    let out = run_backend(main, backend);
    assert!(
        out.status.success(),
        "{backend}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn build(main: &std::path::Path, out: &std::path::Path, release: bool) {
    let mut cmd = std::process::Command::new(rnx());
    cmd.arg("build").arg(main).arg("-o").arg(out);
    if release {
        cmd.arg("--release");
    }
    let build = cmd.output().unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
}

#[test]
fn byte_store_writes_one_byte_on_all_backends() {
    let (dir, main) = write_proj(BYTE_SRC, "byte");
    for backend in ["interpreter", "cranelift", "llvm"] {
        assert_eq!(run_ok(&main, backend), "byte-ok\n", "{backend}");
    }
    for release in [false, true] {
        let out = dir.join(if release { "t_br" } else { "t_b" });
        build(&main, &out, release);
        let run = std::process::Command::new(&out).output().unwrap();
        assert!(run.status.success());
        assert_eq!(String::from_utf8(run.stdout).unwrap(), "byte-ok\n");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn libc_strlen_puts_on_all_backends() {
    let (dir, main) = write_proj(LIBC_SRC, "libc");
    for backend in ["interpreter", "cranelift", "llvm"] {
        assert_eq!(
            run_ok(&main, backend),
            "20\nHello from Native C!\n",
            "{backend}"
        );
    }
    for release in [false, true] {
        let out = dir.join(if release { "t_cr" } else { "t_c" });
        build(&main, &out, release);
        let run = std::process::Command::new(&out).output().unwrap();
        assert!(run.status.success());
        assert_eq!(
            String::from_utf8(run.stdout).unwrap(),
            "20\nHello from Native C!\n"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn zlib_compress_bound_links_libz() {
    let (dir, main) = write_proj(ZLIB_SRC, "zlib");
    for backend in ["interpreter", "cranelift", "llvm"] {
        let stdout = run_ok(&main, backend);
        let bound: i64 = stdout.trim().parse().expect("bound");
        assert!(bound > 100, "{backend}: {bound}");
    }
    let out = dir.join("t_z");
    build(&main, &out, false);
    let run = std::process::Command::new(&out).output().unwrap();
    assert!(run.status.success());
    let bound: i64 = String::from_utf8(run.stdout).unwrap().trim().parse().expect("bound");
    assert!(bound > 100, "{bound}");
    #[cfg(unix)]
    {
        let ldd = std::process::Command::new("ldd").arg(&out).output().unwrap();
        let text = String::from_utf8_lossy(&ldd.stdout).into_owned();
        assert!(text.contains("libz.so"), "libz linked:\n{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
