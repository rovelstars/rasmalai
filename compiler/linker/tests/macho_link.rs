#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use linker::macho::{MachInput, link};
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

fn assemble(dir: &Path, name: &str, triple: &str, src: &str) -> PathBuf {
    let mc = tool("llvm-mc").expect("llvm-mc checked by caller");
    let source = dir.join(format!("{name}.s"));
    let object = dir.join(format!("{name}.o"));
    std::fs::write(&source, src).unwrap();
    let out = Command::new(mc)
        .arg(format!("-triple={triple}"))
        .args(["-filetype=obj", "-o"])
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

fn tbd() -> String {
    "--- !tapi-tbd\ntbd-version: 4\ntargets: [ arm64-macos, x86_64-macos ]\ninstall-name: /usr/lib/libSystem.B.dylib\nexports:\n  - targets: [ arm64-macos, x86_64-macos ]\n    symbols: [ _exit, _write, __tlv_bootstrap ]\n...\n".to_string()
}

fn bind_table(path: &Path) -> String {
    let objdump = tool("llvm-objdump").expect("llvm-objdump checked by caller");
    let out = Command::new(objdump)
        .args(["--macho", "--bind"])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn rebase_table(path: &Path) -> String {
    let objdump = tool("llvm-objdump").expect("llvm-objdump checked by caller");
    let out = Command::new(objdump)
        .args(["--macho", "--rebase"])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn section_bytes(path: &Path, wanted: &str) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    let ncmds = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
    let mut at = 32usize;
    for _ in 0..ncmds {
        let cmd = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if cmd == 0x19 {
            let nsects = u32::from_le_bytes(bytes[at + 64..at + 68].try_into().unwrap()) as usize;
            let mut sect_at = at + 72;
            for _ in 0..nsects {
                let name = &bytes[sect_at..sect_at + 16];
                let end = name.iter().position(|&b| b == 0).unwrap_or(16);
                if &name[..end] == wanted.as_bytes() {
                    let size =
                        u64::from_le_bytes(bytes[sect_at + 40..sect_at + 48].try_into().unwrap())
                            as usize;
                    let offset =
                        u32::from_le_bytes(bytes[sect_at + 48..sect_at + 52].try_into().unwrap())
                            as usize;
                    return bytes[offset..offset + size].to_vec();
                }
                sect_at += 80;
            }
        }
        at += size;
    }
    panic!("section {wanted} not found");
}

fn headers(path: &Path) -> String {
    let objdump = tool("llvm-objdump").expect("llvm-objdump checked by caller");
    let out = Command::new(objdump)
        .args(["--macho", "--private-headers"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "llvm-objdump: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const ARM_SRC: &str = r#"
.text
.globl _start
_start:
    adrp x0, msg@PAGE
    add x0, x0, msg@PAGEOFF
    adrp x1, _exit@GOTPAGE
    ldr x1, [x1, _exit@GOTPAGEOFF]
    bl _exit
.data
msg:
    .asciz "hi"
"#;

const X86_SRC: &str = r#"
.intel_syntax noprefix
.text
.globl _start
_start:
    lea rdi, [rip + msg]
    mov rax, qword ptr [rip + _exit@GOTPCREL]
    call _exit
.data
msg:
    .asciz "hi"
"#;

const ARM_TLS_SRC: &str = r#"
.section __DATA,__thread_vars,thread_local_variables
.globl _x
_x:
    .quad __tlv_bootstrap
    .quad 0
    .quad _x$tlv$init
.globl _y
_y:
    .quad __tlv_bootstrap
    .quad 0
    .quad _y$tlv$init
.section __DATA,__thread_data,thread_local_regular
_x$tlv$init:
    .quad 7
_y$tlv$init:
    .quad 9
.text
.globl _start
_start:
    adrp x0, _x@TLVPPAGE
    ldr x0, [x0, _x@TLVPPAGEOFF]
    adrp x1, _y@TLVPPAGE
    ldr x1, [x1, _y@TLVPPAGEOFF]
    ret
"#;

const ARM_DATA_CONST_SRC: &str = r#"
.section __DATA_CONST,__const
.globl _p
_p:
    .quad _start
.text
.globl _start
_start:
    ret
"#;

const X86_TLS_SRC: &str = r#"
.section __DATA,__thread_vars,thread_local_variables
.globl _x
_x:
    .quad __tlv_bootstrap
    .quad 0
    .quad _x$tlv$init
.globl _y
_y:
    .quad __tlv_bootstrap
    .quad 0
    .quad _y$tlv$init
.section __DATA,__thread_data,thread_local_regular
_x$tlv$init:
    .quad 7
_y$tlv$init:
    .quad 9
.text
.globl _start
_start:
    movq _x@TLVP(%rip), %rax
    movq _y@TLVP(%rip), %rdi
    ret
"#;

fn link_fixture(tag: &str, triple: &str, src: &str) -> (crate::scratch::Scratch, PathBuf) {
    let dir = crate::scratch::dir(&format!("macho-{tag}"));
    let object = assemble(&dir, tag, triple, src);
    let exe = dir.join(format!("{tag}.exe"));
    link(
        &[
            MachInput::ObjectPath(object),
            MachInput::Tbd(tbd(), "libSystem.tbd".to_string()),
        ],
        &exe,
        "start",
    )
    .unwrap();
    (dir, exe)
}

#[test]
fn arm64_macho_executable_structure() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(_) = tool("llvm-objdump") else {
        return eprintln!("skipping: llvm-objdump is absent");
    };
    let (_dir, exe) = link_fixture("arm", "arm64-apple-macos11", ARM_SRC);
    let text = headers(&exe);
    assert!(text.contains("MH_MAGIC_64"), "{text}");
    assert!(text.contains("EXECUTE"), "{text}");
    assert!(text.contains("ARM64"), "{text}");
    assert!(text.contains("segname __PAGEZERO"), "{text}");
    assert!(text.contains("segname __TEXT"), "{text}");
    assert!(text.contains("segname __DATA"), "{text}");
    assert!(text.contains("segname __LINKEDIT"), "{text}");
    assert!(text.contains("LC_MAIN"), "{text}");
    assert!(text.contains("LC_LOAD_DYLIB"), "{text}");
    assert!(text.contains("/usr/lib/libSystem.B.dylib"), "{text}");
    assert!(text.contains("LC_CODE_SIGNATURE"), "{text}");
    assert!(text.contains("LC_DYLD_INFO_ONLY"), "{text}");
}

#[test]
fn x86_64_macho_executable_structure() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(_) = tool("llvm-objdump") else {
        return eprintln!("skipping: llvm-objdump is absent");
    };
    let (_dir, exe) = link_fixture("x86", "x86_64-apple-macos10.15", X86_SRC);
    let text = headers(&exe);
    assert!(text.contains("MH_MAGIC_64"), "{text}");
    assert!(text.contains("EXECUTE"), "{text}");
    assert!(text.contains("X86_64"), "{text}");
    assert!(text.contains("LC_MAIN"), "{text}");
    assert!(text.contains("LC_CODE_SIGNATURE"), "{text}");
}

#[test]
fn code_signature_blob_is_well_formed() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let (_dir, exe) = link_fixture("sig", "arm64-apple-macos11", ARM_SRC);
    let text = headers(&exe);
    let mut dataoff = None;
    let mut datasize = None;
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.contains("cmd LC_CODE_SIGNATURE") {
            for follow in &lines[i..lines.len().min(i + 5)] {
                if let Some(value) = follow.trim().strip_prefix("dataoff ") {
                    dataoff = value.parse::<usize>().ok();
                }
                if let Some(value) = follow.trim().strip_prefix("datasize ") {
                    datasize = value.parse::<usize>().ok();
                }
            }
        }
    }
    let dataoff = dataoff.expect("LC_CODE_SIGNATURE dataoff");
    let datasize = datasize.expect("LC_CODE_SIGNATURE datasize");
    let bytes = std::fs::read(&exe).unwrap();
    let blob = &bytes[dataoff..dataoff + datasize];
    assert_eq!(
        u32::from_be_bytes(blob[0..4].try_into().unwrap()),
        0xfade0cc0
    );
    assert_eq!(
        u32::from_be_bytes(blob[4..8].try_into().unwrap()) as usize,
        datasize
    );
    assert_eq!(datasize % 16, 0, "signature payload is 16-byte aligned");
    assert_eq!(u32::from_be_bytes(blob[8..12].try_into().unwrap()), 1);
    let cd_len = u32::from_be_bytes(blob[24..28].try_into().unwrap()) as usize;
    let cd = &blob[20..20 + cd_len];
    assert_eq!(u32::from_be_bytes(cd[0..4].try_into().unwrap()), 0xfade0c02);
    assert_eq!(u32::from_be_bytes(cd[8..12].try_into().unwrap()), 0x20100);
    assert_eq!(cd[36], 32, "SHA-256 hash size");
    assert_eq!(cd[37], 2, "SHA-256 hash type");
    assert_eq!(cd[39], 12, "4 KiB signature pages");
    let slots = u32::from_be_bytes(cd[28..32].try_into().unwrap()) as usize;
    assert!(slots > 0);
    let hash_offset = u32::from_be_bytes(cd[16..20].try_into().unwrap()) as usize;
    assert_eq!(cd.len(), hash_offset + slots * 32);
}

#[test]
fn arm64_tlv_descriptors_are_relative() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let (_dir, exe) = link_fixture("tls-arm", "arm64-apple-macos11", ARM_TLS_SRC);
    let vars = section_bytes(&exe, "__thread_vars");
    assert_eq!(vars.len(), 48);
    let words: Vec<u64> = vars
        .chunks_exact(8)
        .map(|word| u64::from_le_bytes(word.try_into().unwrap()))
        .collect();
    assert_eq!(words[2], 0, "first thread-local data offset");
    assert_eq!(words[5], 8, "second thread-local data offset");
    let binds = bind_table(&exe);
    assert!(binds.contains("__tlv_bootstrap"), "{binds}");
    assert!(binds.contains("__thread_vars"), "{binds}");
    let text = headers(&exe);
    assert!(text.contains("__thread_vars"), "{text}");
    assert!(text.contains("__thread_data"), "{text}");
    assert!(text.contains("S_THREAD_LOCAL_VARIABLES"), "{text}");
}

#[test]
fn x86_64_tlv_descriptors_are_relative() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let (_dir, exe) = link_fixture("tls-x86", "x86_64-apple-macos10.15", X86_TLS_SRC);
    let vars = section_bytes(&exe, "__thread_vars");
    assert_eq!(vars.len(), 48);
    let words: Vec<u64> = vars
        .chunks_exact(8)
        .map(|word| u64::from_le_bytes(word.try_into().unwrap()))
        .collect();
    assert_eq!(words[2], 0);
    assert_eq!(words[5], 8);
    let binds = bind_table(&exe);
    assert!(binds.contains("__tlv_bootstrap"), "{binds}");
}

#[test]
fn data_const_segment_is_laid_out() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let (_dir, exe) = link_fixture("dconst", "arm64-apple-macos11", ARM_DATA_CONST_SRC);
    let text = headers(&exe);
    assert!(text.contains("segname __DATA_CONST"), "{text}");
    assert!(text.contains("initprot r--"), "{text}");
    let rebase = rebase_table(&exe);
    assert!(rebase.contains("__DATA_CONST"), "{rebase}");
}

#[test]
fn two_links_are_byte_identical() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let dir = crate::scratch::dir("macho-det");
    let object = assemble(&dir, "det", "arm64-apple-macos11", ARM_SRC);
    let bytes = std::fs::read(&object).unwrap();
    let first = dir.join("first");
    let second = dir.join("second");
    let inputs = |bytes: Vec<u8>| {
        vec![
            MachInput::Object(bytes),
            MachInput::Tbd(tbd(), "libSystem.tbd".to_string()),
        ]
    };
    link(&inputs(bytes.clone()), &first, "start").unwrap();
    link(&inputs(bytes), &second, "start").unwrap();
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );
}

#[test]
fn differential_against_ld64_lld() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(ld) = tool("ld64.lld") else {
        return eprintln!("skipping: ld64.lld is absent");
    };
    let dir = crate::scratch::dir("macho-diff");
    let object = assemble(&dir, "diff", "arm64-apple-macos11", ARM_SRC);
    let ours = dir.join("ours");
    link(
        &[
            MachInput::ObjectPath(object.clone()),
            MachInput::Tbd(tbd(), "libSystem.tbd".to_string()),
        ],
        &ours,
        "start",
    )
    .unwrap();
    let libdir = dir.join("lib");
    std::fs::create_dir_all(&libdir).unwrap();
    std::fs::write(libdir.join("libSystem.tbd"), tbd()).unwrap();
    let theirs = dir.join("theirs");
    let out = Command::new(ld)
        .args([
            "-arch",
            "arm64",
            "-platform_version",
            "macos",
            "11.0",
            "11.0",
        ])
        .arg("-o")
        .arg(&theirs)
        .arg("-L")
        .arg(&libdir)
        .arg(&object)
        .args(["-lSystem", "-e", "_start"])
        .output()
        .unwrap();
    if !out.status.success() {
        eprintln!(
            "skipping differential: ld64.lld rejected the fixture: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    let a = headers(&ours);
    let b = headers(&theirs);
    assert!(a.contains("ARM64") && b.contains("ARM64"));
    assert!(a.contains("MH_EXECUTE") && b.contains("MH_EXECUTE"));
    for segment in ["__TEXT", "__DATA", "__LINKEDIT"] {
        assert!(a.contains(segment), "ours missing {segment}");
        assert!(b.contains(segment), "lld missing {segment}");
    }
    assert!(a.contains("/usr/lib/libSystem.B.dylib"));
    assert!(b.contains("/usr/lib/libSystem.B.dylib"));
    assert!(a.contains("LC_MAIN"));
    assert!(b.contains("LC_MAIN"));
    let objdump = tool("llvm-objdump").unwrap();
    for path in [&ours, &theirs] {
        let out = Command::new(&objdump)
            .args(["--macho", "--syms"])
            .arg(path)
            .output()
            .unwrap();
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.lines()
                .any(|line| line.contains(" g ") && line.contains(" _start")),
            "missing external _start in {}: {text}",
            path.display()
        );
        assert!(
            text.lines()
                .any(|line| line.contains("*UND*") && line.contains(" _exit")),
            "missing undefined _exit in {}: {text}",
            path.display()
        );
    }
}

#[test]
fn differential_against_ld64_lld_x86_64() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(ld) = tool("ld64.lld") else {
        return eprintln!("skipping: ld64.lld is absent");
    };
    let dir = crate::scratch::dir("macho-diff-x86");
    let object = assemble(&dir, "diffx", "x86_64-apple-macos10.15", X86_SRC);
    let ours = dir.join("ours");
    link(
        &[
            MachInput::ObjectPath(object.clone()),
            MachInput::Tbd(tbd(), "libSystem.tbd".to_string()),
        ],
        &ours,
        "start",
    )
    .unwrap();
    let libdir = dir.join("lib");
    std::fs::create_dir_all(&libdir).unwrap();
    std::fs::write(libdir.join("libSystem.tbd"), tbd()).unwrap();
    let theirs = dir.join("theirs");
    let out = Command::new(ld)
        .args([
            "-arch",
            "x86_64",
            "-platform_version",
            "macos",
            "10.15",
            "10.15",
        ])
        .arg("-o")
        .arg(&theirs)
        .arg("-L")
        .arg(&libdir)
        .arg(&object)
        .args(["-lSystem", "-e", "_start"])
        .output()
        .unwrap();
    if !out.status.success() {
        eprintln!(
            "skipping differential: ld64.lld rejected the fixture: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    let a = headers(&ours);
    let b = headers(&theirs);
    assert!(a.contains("X86_64") && b.contains("X86_64"));
    assert!(a.contains("EXECUTE") && b.contains("EXECUTE"));
    for segment in ["__TEXT", "__DATA", "__LINKEDIT"] {
        assert!(a.contains(segment), "ours missing {segment}");
        assert!(b.contains(segment), "lld missing {segment}");
    }
    assert!(a.contains("/usr/lib/libSystem.B.dylib"));
    assert!(b.contains("/usr/lib/libSystem.B.dylib"));
    assert!(a.contains("LC_MAIN") && b.contains("LC_MAIN"));
}
