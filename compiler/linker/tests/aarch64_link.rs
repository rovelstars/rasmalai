#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use linker::{LinkInput, native};
use std::path::{Path, PathBuf};
use std::process::Command;

fn tool(name: &str) -> Option<PathBuf> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn assembler() -> Option<PathBuf> {
    tool("aarch64-linux-gnu-as")
        .or_else(|| tool("aarch64-linux-gnu-gcc").map(|_| PathBuf::from("aarch64-linux-gnu-as")))
}

fn qemu() -> Option<PathBuf> {
    tool("qemu-aarch64-static").or_else(|| tool("qemu-aarch64"))
}

fn skip(test: &str, reason: &str) {
    eprintln!("skipping `{test}`: {reason}");
}

fn assemble(dir: &Path, name: &str, src: &str) -> PathBuf {
    let asm = assembler().expect("assembler checked by caller");
    let source = dir.join(format!("{name}.s"));
    let object = dir.join(format!("{name}.o"));
    std::fs::write(&source, src).unwrap();
    let out = Command::new(&asm)
        .arg("-o")
        .arg(&object)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "assemble {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    object
}

fn aarch64_lib(root: &Path) -> PathBuf {
    root.join("lib")
        .join("rustlib")
        .join("aarch64-unknown-linux-gnu")
        .join("lib")
}

fn empty_archive(path: &Path) {
    let out = Command::new("ar").arg("cr").arg(path).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn sysroot_with_stubs(tag: &str) -> crate::scratch::Scratch {
    let root = crate::scratch::dir(&format!("a64-sysroot-{tag}"));
    let lib = aarch64_lib(&root);
    std::fs::create_dir_all(&lib).unwrap();
    let empty = lib.join("empty.o");
    std::fs::write(&empty, b"").unwrap();
    let out = Command::new(assembler().expect("assembler"))
        .arg("-o")
        .arg(&empty)
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(b"\n")?;
            child.wait()
        })
        .unwrap();
    assert!(out.success(), "assemble empty object");
    for name in [
        "crt1.o",
        "Scrt1.o",
        "rcrt1.o",
        "crti.o",
        "crtbeginT.o",
        "crtbeginS.o",
        "crtend.o",
        "crtendS.o",
        "crtn.o",
    ] {
        std::fs::copy(&empty, lib.join(name)).unwrap();
    }
    for name in ["libc.a", "libm.a", "libgcc.a", "libgcc_eh.a"] {
        empty_archive(&lib.join(name));
    }
    root
}

fn static_opts() -> native::StaticOpts {
    native::StaticOpts {
        entry: "_start".to_string(),
        icf: true,
        target: "aarch64-unknown-linux-gnu".to_string(),
        strip: false,
        lib_dirs: Vec::new(),
        runpath: None,
    }
}

fn link_static(object: &Path, out: &Path, sysroot: &Path) {
    let bytes = std::fs::read(object).unwrap();
    native::native_link_fully_static_in(
        &[LinkInput::ObjectBytes(bytes)],
        out,
        &static_opts(),
        &sysroot::Sysroot::custom(sysroot.to_path_buf()),
    )
    .unwrap();
}

fn run_qemu(binary: &Path, sysroot: Option<&Path>) -> std::process::Output {
    let qemu = qemu().expect("qemu checked by caller");
    let mut cmd = Command::new(qemu);
    if let Some(root) = sysroot {
        cmd.arg("-L").arg(root);
    }
    cmd.arg(binary);
    cmd.output().unwrap()
}

fn machine(path: &Path) -> u16 {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[0..4], b"\x7fELF");
    u16::from_le_bytes([bytes[18], bytes[19]])
}

fn elf_type(path: &Path) -> u16 {
    let bytes = std::fs::read(path).unwrap();
    u16::from_le_bytes([bytes[16], bytes[17]])
}

const EXIT_42: &str = ".global _start\n_start:\n    mov x0, #42\n    mov x8, #93\n    svc #0\n";

#[test]
fn static_syscall_exits_42() {
    let Some(_) = assembler() else {
        return skip("static_syscall_exits_42", "aarch64-linux-gnu-as is absent");
    };
    let Some(_) = qemu() else {
        return skip("static_syscall_exits_42", "qemu-aarch64-static is absent");
    };
    let dir = crate::scratch::dir("a64-exit");
    let sysroot = sysroot_with_stubs("exit");
    let object = assemble(&dir, "exit", EXIT_42);
    let out = dir.join("exit");
    link_static(&object, &out, &sysroot);
    assert_eq!(elf_type(&out), 2, "expected ET_EXEC");
    assert_eq!(machine(&out), 183);
    let run = run_qemu(&out, None);
    assert_eq!(run.status.code(), Some(42), "{run:?}");
}

#[test]
fn static_absolute_pc_relative_and_adrp() {
    let Some(_) = assembler() else {
        return skip("static_absolute_pc_relative_and_adrp", "assembler absent");
    };
    let Some(_) = qemu() else {
        return skip("static_absolute_pc_relative_and_adrp", "qemu absent");
    };
    let src = r#"
.global _start
_start:
    adrp x1, msg
    add x1, x1, :lo12:msg
    adrp x2, msg_ptr
    add x2, x2, :lo12:msg_ptr
    ldr x2, [x2]
    cmp x1, x2
    b.ne fail
    mov x0, #1
    mov x2, #3
    mov x8, #64
    svc #0
    mov x0, #42
    b done
fail:
    mov x0, #1
done:
    mov x8, #93
    svc #0
.data
msg:
    .ascii "ok\n"
    .byte 0
.balign 8
msg_ptr:
    .quad msg
"#;
    let dir = crate::scratch::dir("a64-abs");
    let sysroot = sysroot_with_stubs("abs");
    let object = assemble(&dir, "abs", src);
    let out = dir.join("abs");
    link_static(&object, &out, &sysroot);
    let run = run_qemu(&out, None);
    assert_eq!(run.status.code(), Some(42), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "ok\n");
}

#[test]
fn static_branches_and_conditions() {
    let Some(_) = assembler() else {
        return skip("static_branches_and_conditions", "assembler absent");
    };
    let Some(_) = qemu() else {
        return skip("static_branches_and_conditions", "qemu absent");
    };
    let src = r#"
.global _start
_start:
    mov x0, #40
    bl add_two
    cmp x0, #42
    b.ne fail
    tbnz x0, #0, fail
    tbz x0, #1, fail
    b done
fail:
    mov x0, #1
done:
    mov x8, #93
    svc #0
add_two:
    add x0, x0, #2
    ret
"#;
    let dir = crate::scratch::dir("a64-branch");
    let sysroot = sysroot_with_stubs("branch");
    let object = assemble(&dir, "branch", src);
    let out = dir.join("branch");
    link_static(&object, &out, &sysroot);
    let run = run_qemu(&out, None);
    assert_eq!(run.status.code(), Some(42), "{run:?}");
}

#[test]
fn static_got_load() {
    let Some(_) = assembler() else {
        return skip("static_got_load", "assembler absent");
    };
    let Some(_) = qemu() else {
        return skip("static_got_load", "qemu absent");
    };
    let src = r#"
.global _start
_start:
    adrp x0, :got:target
    ldr x0, [x0, :got_lo12:target]
    ldr w0, [x0]
    mov x8, #93
    svc #0
.data
.balign 4
target:
    .word 7
"#;
    let dir = crate::scratch::dir("a64-got");
    let sysroot = sysroot_with_stubs("got");
    let object = assemble(&dir, "got", src);
    let out = dir.join("got");
    link_static(&object, &out, &sysroot);
    let run = run_qemu(&out, None);
    assert_eq!(run.status.code(), Some(7), "{run:?}");
}

#[test]
fn static_tls_local_exec() {
    let Some(_) = assembler() else {
        return skip("static_tls_local_exec", "assembler absent");
    };
    let Some(_) = qemu() else {
        return skip("static_tls_local_exec", "qemu absent");
    };
    let src = r#"
.global _start
_start:
    adrp x0, tls_var
    add x0, x0, :lo12:tls_var
    sub x0, x0, #16
    msr tpidr_el0, x0
    mrs x1, tpidr_el0
    add x1, x1, :tprel_hi12:tls_var
    add x1, x1, :tprel_lo12_nc:tls_var
    ldr w0, [x1]
    mov x8, #93
    svc #0
.section .tdata,"awT",@progbits
.balign 4
tls_var:
    .word 42
"#;
    let dir = crate::scratch::dir("a64-tlsle");
    let sysroot = sysroot_with_stubs("tlsle");
    let object = assemble(&dir, "tlsle", src);
    let out = dir.join("tlsle");
    link_static(&object, &out, &sysroot);
    let run = run_qemu(&out, None);
    assert_eq!(run.status.code(), Some(42), "{run:?}");
}

#[test]
fn static_tls_initial_exec() {
    let Some(_) = assembler() else {
        return skip("static_tls_initial_exec", "assembler absent");
    };
    let Some(_) = qemu() else {
        return skip("static_tls_initial_exec", "qemu absent");
    };
    let src = r#"
.global _start
_start:
    adrp x0, tls_var
    add x0, x0, :lo12:tls_var
    sub x0, x0, #16
    msr tpidr_el0, x0
    adrp x1, :gottprel:tls_var
    ldr x1, [x1, :gottprel_lo12:tls_var]
    mrs x0, tpidr_el0
    add x1, x0, x1
    ldr w0, [x1]
    mov x8, #93
    svc #0
.section .tdata,"awT",@progbits
.balign 4
tls_var:
    .word 37
"#;
    let dir = crate::scratch::dir("a64-tlsie");
    let sysroot = sysroot_with_stubs("tlsie");
    let object = assemble(&dir, "tlsie", src);
    let out = dir.join("tlsie");
    link_static(&object, &out, &sysroot);
    let run = run_qemu(&out, None);
    assert_eq!(run.status.code(), Some(37), "{run:?}");
}

#[test]
fn static_pie_runs_and_carries_relative_relocations() {
    let Some(_) = assembler() else {
        return skip(
            "static_pie_runs_and_carries_relative_relocations",
            "assembler absent",
        );
    };
    let Some(_) = qemu() else {
        return skip(
            "static_pie_runs_and_carries_relative_relocations",
            "qemu absent",
        );
    };
    let src = r#"
.global _start
_start:
    adrp x0, start_ptr
    add x0, x0, :lo12:start_ptr
    mov x0, #9
    mov x8, #93
    svc #0
.data
.balign 8
start_ptr:
    .quad _start
"#;
    let dir = crate::scratch::dir("a64-pie");
    let sysroot = sysroot_with_stubs("pie");
    let object = assemble(&dir, "pie", src);
    let bytes = std::fs::read(&object).unwrap();
    let out = dir.join("pie");
    native::native_link_static_pie_in(
        &[LinkInput::ObjectBytes(bytes)],
        &out,
        &static_opts(),
        &sysroot::Sysroot::custom(sysroot.to_path_buf()),
    )
    .unwrap();
    assert_eq!(elf_type(&out), 3, "expected ET_DYN");
    assert_eq!(machine(&out), 183);
    let relocs = Command::new("readelf")
        .arg("-rW")
        .arg(&out)
        .output()
        .unwrap();
    assert!(relocs.status.success());
    let text = String::from_utf8_lossy(&relocs.stdout);
    assert!(text.contains("R_AARCH64_RELATIVE"), "{text}");
    let run = run_qemu(&out, None);
    assert_eq!(run.status.code(), Some(9), "{run:?}");
}

#[test]
fn deterministic_static_output() {
    let Some(_) = assembler() else {
        return skip("deterministic_static_output", "assembler absent");
    };
    let dir = crate::scratch::dir("a64-det");
    let sysroot = sysroot_with_stubs("det");
    let object = assemble(&dir, "det", EXIT_42);
    let bytes = std::fs::read(&object).unwrap();
    let first = dir.join("first");
    let second = dir.join("second");
    let root = sysroot::Sysroot::custom(sysroot.to_path_buf());
    native::native_link_fully_static_in(
        &[LinkInput::ObjectBytes(bytes.clone())],
        &first,
        &static_opts(),
        &root,
    )
    .unwrap();
    native::native_link_fully_static_in(
        &[LinkInput::ObjectBytes(bytes)],
        &second,
        &static_opts(),
        &root,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap(),
        "two links of the same inputs differ"
    );
}

fn copy_file(from: &Path, to: &Path) {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::copy(from, to).unwrap_or_else(|e| panic!("copy {}: {e}", from.display()));
}

#[test]
fn dynamic_glibc_write() {
    let Some(_) = assembler() else {
        return skip("dynamic_glibc_write", "assembler absent");
    };
    let Some(_) = qemu() else {
        return skip("dynamic_glibc_write", "qemu absent");
    };
    let root = Path::new("/usr/aarch64-linux-gnu");
    let libc = root.join("lib").join("libc.so.6");
    let libm = root.join("lib").join("libm.so.6");
    let loader = root.join("lib").join("ld-linux-aarch64.so.1");
    if !libc.is_file() || !loader.is_file() {
        return skip("dynamic_glibc_write", "aarch64 glibc is absent");
    }
    let src = r#"
.global _start
_start:
    mov x0, #1
    adrp x1, msg
    add x1, x1, :lo12:msg
    mov x2, #3
    bl write
    mov x0, #0
    mov x8, #93
    svc #0
.data
msg:
    .ascii "hi\n"
"#;
    let dir = crate::scratch::dir("a64-dyn");
    let object = assemble(&dir, "dyn", src);
    let sysroot = sysroot_with_stubs("dyn");
    let lib = aarch64_lib(&sysroot);
    std::fs::copy(lib.join("empty.o"), lib.join("DynEmpty.o")).unwrap();
    copy_file(&libc, &lib.join("libc.so.6"));
    copy_file(&loader, &lib.join("ld-linux-aarch64.so.1"));
    if libm.is_file() {
        copy_file(&libm, &lib.join("libm.so.6"));
    }
    let out = dir.join("dyn");
    let bytes = std::fs::read(&object).unwrap();
    let opts = native::DynamicOpts {
        entry: "_start".to_string(),
        icf: true,
        target: "aarch64-unknown-linux-gnu".to_string(),
        lib_dirs: vec![lib.clone()],
        libs: Vec::new(),
        runpath: Some(lib.display().to_string()),
        strip: false,
    };
    native::native_link_dynamic_in(
        &[LinkInput::ObjectBytes(bytes)],
        &out,
        &opts,
        &sysroot::Sysroot::custom(sysroot.path().to_path_buf()),
    )
    .unwrap();
    assert_eq!(machine(&out), 183);
    let run = run_qemu(&out, Some(root));
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "hi\n");

    let again = dir.join("dyn-again");
    let bytes = std::fs::read(&object).unwrap();
    native::native_link_dynamic_in(
        &[LinkInput::ObjectBytes(bytes)],
        &again,
        &opts,
        &sysroot::Sysroot::custom(sysroot.path().to_path_buf()),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&out).unwrap(),
        std::fs::read(&again).unwrap(),
        "two dynamic links of the same inputs differ"
    );
}

#[test]
fn compiler_emitted_aarch64_object_links_and_runs() {
    let Some(_) = assembler() else {
        return skip(
            "compiler_emitted_aarch64_object_links_and_runs",
            "assembler absent",
        );
    };
    let Some(_) = qemu() else {
        return skip(
            "compiler_emitted_aarch64_object_links_and_runs",
            "qemu absent",
        );
    };
    let dir = crate::scratch::dir("a64-codegen");
    let source = dir.join("main.rnx");
    std::fs::write(&source, "fn Main(): Int {\n    return 42;\n}\n").unwrap();
    let mut cfg = cli::CompileConfig::default();
    cfg.jobs = 1;
    cfg.memory_cap = u64::MAX;
    let built = match cli::build_files_staged_cfg(
        source.to_str().unwrap(),
        "Main",
        true,
        1,
        Some("aarch64-unknown-linux-gnu"),
        &cfg,
    ) {
        Ok(built) => built,
        Err(diags) => panic!("compile aarch64 object: {diags:?}"),
    };
    assert!(!built.objects.is_empty());
    let sysroot = sysroot_with_stubs("codegen");
    let start = assemble(
        &dir,
        "start",
        ".global _start\n_start:\n    bl Main\n    mov x8, #93\n    svc #0\n",
    );
    let start_bytes = std::fs::read(&start).unwrap();
    let mut inputs: Vec<LinkInput> = built
        .objects
        .into_iter()
        .map(LinkInput::ObjectBytes)
        .collect();
    inputs.push(LinkInput::ObjectBytes(start_bytes));
    let out = dir.join("codegen");
    native::native_link_fully_static_in(
        &inputs,
        &out,
        &static_opts(),
        &sysroot::Sysroot::custom(sysroot.to_path_buf()),
    )
    .unwrap();
    assert_eq!(machine(&out), 183);
    let run = run_qemu(&out, None);
    assert_eq!(run.status.code(), Some(42), "{run:?}");
}

#[test]
fn differential_lld_static() {
    let Some(_) = assembler() else {
        return skip("differential_lld_static", "assembler absent");
    };
    let Some(_) = qemu() else {
        return skip("differential_lld_static", "qemu absent");
    };
    let Some(ld) = tool("aarch64-linux-gnu-ld").or_else(|| tool("ld.lld")) else {
        return skip("differential_lld_static", "no aarch64 lld/ld");
    };
    let dir = crate::scratch::dir("a64-diff");
    let sysroot = sysroot_with_stubs("diff");
    let object = assemble(&dir, "diff", EXIT_42);
    let ours = dir.join("ours");
    link_static(&object, &ours, &sysroot);
    let theirs = dir.join("theirs");
    let out = Command::new(&ld)
        .arg("-o")
        .arg(&theirs)
        .arg("-e")
        .arg("_start")
        .arg(&object)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let a = run_qemu(&ours, None);
    let b = run_qemu(&theirs, None);
    assert_eq!(a.status.code(), b.status.code());
    assert_eq!(a.stdout, b.stdout);
    let sections = Command::new("readelf")
        .arg("-SW")
        .arg(&ours)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&sections.stdout);
    assert!(text.contains(".text"), "{text}");
    assert!(text.contains("AX"), "{text}");
    assert!(text.contains(".symtab"), "{text}");
}
